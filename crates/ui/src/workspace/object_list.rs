//! 对象列表渲染：内容区骨架、表头、行、状态条。

use super::*;

impl WorkspaceView {
    /// 中间内容区：对象列表（选中桶后异步加载，含前缀导航与翻页）。
    pub(super) fn render_content(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(_bucket) = self.selected_bucket.clone() else {
            let (title, hint) = if self.selected_account_id.is_some() {
                ("选择一个 Bucket", "从左侧列表选择空间，查看其中对象")
            } else {
                ("添加并选择账号", "点击左侧「添加账号」开始浏览云存储")
            };
            return self
                .with_file_drop(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .bg(theme.background)
                        .text_color(theme.muted_foreground)
                        .child(Icon::new(IconName::Inbox).text_size(tokens::icon_sm()))
                        .child(
                            div()
                                .text_size(tokens::heading())
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(theme.foreground)
                                .child(title),
                        )
                        .child(div().text_size(tokens::label()).child(hint)),
                    cx,
                )
                .into_any_element();
        };

        let mut content = self.with_file_drop(
            v_flex()
                .relative()
                .flex_1()
                .min_w_0()
                // min_h_0：允许被父容器压缩。缺省的 min-height:auto 会让 20+ 行
                // 内容把 content 撑出窗口，状态条被挤到可视区外（与数据重叠）。
                .min_h_0()
                .h_full()
                .overflow_hidden()
                .bg(theme.background),
            cx,
        );
        // 选中信息与下载动作在底部状态条里就地展示（见
        // render_object_status_bar）——不做独立于列表的选中条：那种
        // 「选中才出现」的条会把表头与全部行整体下推，选中/取消时
        // 内容跳动，Finder / Linear 都不这样做。
        if self.objects_state == AsyncState::Loading && self.entries.is_empty() {
            content = content.child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .text_color(theme.muted_foreground)
                    .child(Spinner::new())
                    .child("加载对象列表中…"),
            );
            return content.into_any_element();
        }

        if let AsyncState::Failed(msg) = &self.objects_state {
            if self.entries.is_empty() {
                content = content.child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .text_color(theme.muted_foreground)
                        .child(
                            Icon::new(IconName::TriangleAlert)
                                .text_size(tokens::icon_sm())
                                .text_color(theme.danger),
                        )
                        .child(
                            div()
                                .text_size(tokens::heading())
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(theme.foreground)
                                .child("加载对象列表失败"),
                        )
                        .child(
                            div()
                                .max_w(tokens::text(480.))
                                .text_size(tokens::label())
                                .child(msg.clone()),
                        )
                        .child(
                            Button::new("objects-retry")
                                .label("重试")
                                .with_size(Size::Small)
                                .on_click(cx.listener(|this, _, _, cx| this.reload_objects(cx))),
                        ),
                );
                return content.into_any_element();
            }
            // 翻页失败但已有数据：保留列表，顶部横幅提示
            content = content.child(
                h_flex()
                    .mx_3()
                    .mt_2()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded(tokens::radius())
                    .text_color(theme.danger)
                    .text_size(tokens::label())
                    .child(Icon::new(IconName::TriangleAlert))
                    .child(format!("加载更多失败：{msg}")),
            );
        }

        content = content
            .child(self.render_object_list_header(theme, cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_object_list(theme, cx)),
            )
            .child(self.render_transfer_panel(theme, cx))
            .child(self.render_object_status_bar(theme, cx));
        content.into_any_element()
    }

    /// 对象列表本体：表格列布局 + 行级操作列。
    ///
    /// 空白点击清空选择（B6）挂在本滚动容器上：滚动容器带
    /// `BlockMouseExceptScroll` hitbox，落在其空白处的点击只有它收得到
    /// （历史上试过内容区容器的 capture 与 bubble 两版，都因这层 hitbox
    /// 的覆盖范围不可靠而失败）。行处理器一律 `stop_propagation`，所以能
    /// 冒泡到这里的一定没命中任何行 —— 判据不需要几何，也不依赖注册顺序。
    pub(super) fn render_object_list(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut list = v_flex()
            .id("object-list")
            .relative()
            .flex_1() // row 主轴：宽度占满
            // h_full 必须显式：包裹层（row 容器）不拉伸子元素高度，缺省时
            // 滚动容器高度=内容自然高度 → 越过包裹层叠在状态条上、且无溢出
            // 不产生滚动（滚动条失效）。约束后溢出才成立，滚动条恢复。
            .h_full()
            .min_h_0()
            .overflow_y_scroll()
            .bg(theme.background)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    // 编辑中先提交重命名（Finder：点别处即提交）；未提交成功
                    // （校验失败）则本次点击不当作空白处理。
                    if this.renaming.is_some() {
                        this.commit_rename(cx);
                        if this.renaming.is_some() {
                            return;
                        }
                    }
                    if this.selected_object_keys.is_empty() && this.selected_object_key.is_none() {
                        return;
                    }
                    // clear_object_selection 一并关掉行内菜单/详情弹层
                    this.clear_object_selection();
                    cx.notify();
                }),
            );

        if self.entries.is_empty() && self.objects_state == AsyncState::Idle {
            list = list.child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .text_color(theme.muted_foreground)
                    .child(Icon::new(IconName::Inbox).text_size(tokens::icon_sm()))
                    .child(
                        div()
                            .text_size(tokens::heading())
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(theme.foreground)
                            .child("此目录为空"),
                    )
                    .child(
                        div()
                            .text_size(tokens::label())
                            .child("拖入文件或文件夹即可上传"),
                    ),
            );
        }

        // ⌘F 过滤开启时只渲染命中项；命中为空给出明确空态。
        // 过滤与排序都只影响展示：entries 全集与选择集合不动（Finder 语义）。
        let display_order =
            display_entry_order(&self.entries, self.object_sort, self.filtered_ix.as_deref());
        let visible: Vec<(usize, &ListingEntry)> = display_order
            .into_iter()
            .filter_map(|ix| self.entries.get(ix).map(|e| (ix, e)))
            .collect();
        if self.filtered_ix.is_some() && visible.is_empty() {
            list = list.child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .text_color(theme.muted_foreground)
                    .child(Icon::new(IconName::Search).text_size(tokens::icon_sm()))
                    .child(
                        div()
                            .text_size(tokens::heading())
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(theme.foreground)
                            .child("没有匹配的对象"),
                    )
                    .child(
                        div()
                            .text_size(tokens::label())
                            .child("试试其他关键词，或按 Esc 清除过滤"),
                    ),
            );
            return list.into_any_element();
        }

        if !visible.is_empty() {
            list = list.children(self.object_list_rows(theme, visible, cx));
        }
        list.into_any_element()
    }

    /// 表头（钉在滚动容器外，与状态条同级——滚动后仍在，Finder/Linear 语义）。
    pub(super) fn render_object_list_header(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        h_flex()
            .w_full()
            .flex_shrink_0()
            .px_3()
            .py_2()
            .gap_2()
            .border_b_1()
            .border_color(theme.border)
            .bg(theme.list_head)
            .text_size(tokens::label())
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(theme.muted_foreground)
            .child(
                div()
                    .id("header-name")
                    .flex_1()
                    .min_w_0()
                    .hover(|el| el.text_color(theme.foreground))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.object_sort = this.object_sort.cycle_name();
                            cx.notify();
                        }),
                    )
                    .child(format!("名称{}", self.object_sort.name_mark())),
            )
            .child(
                div()
                    .id("header-size")
                    .w(tokens::col_size_width())
                    .flex_shrink_0()
                    .hover(|el| el.text_color(theme.foreground))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.object_sort = this.object_sort.cycle_size();
                            cx.notify();
                        }),
                    )
                    .child(format!("大小{}", self.object_sort.size_mark())),
            )
            .child(
                div()
                    .id("header-time")
                    .w(tokens::col_time_width())
                    .flex_shrink_0()
                    .hover(|el| el.text_color(theme.foreground))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.object_sort = this.object_sort.cycle_time();
                            cx.notify();
                        }),
                    )
                    .child(format!("最新修改时间{}", self.object_sort.time_mark())),
            )
    }

    pub(super) fn render_object_name_cell(
        &self,
        object: &CloudObject,
        renaming: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let icon =
            crate::file_type::file_type_icon(&object.key, theme.muted_foreground, theme.accent);
        if renaming && let Some((old_key, editor)) = &self.renaming {
            let current_name = editor.read(cx).value().to_string();
            let validation = rename_validation_message(old_key, &current_name, &self.entries)
                .filter(|message| message != "请输入一个不同的新名称");
            return h_flex()
                .flex_1()
                .min_w_0()
                .gap_2()
                .child(icon)
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .key_context("Renaming")
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                        .child(Input::new(editor).small())
                        .children(validation.map(|message| {
                            div()
                                .text_size(tokens::caption())
                                .text_color(theme.danger)
                                .child(message)
                        })),
                )
                .into_any_element();
        }
        h_flex()
            .flex_1()
            .min_w_0()
            .gap_2()
            .child(icon)
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .child(display_name(&object.key).to_string()),
            )
            .into_any_element()
    }

    /// 对象行渲染（目录前缀 / 云对象），供滚动容器内循环。
    pub(super) fn object_list_rows(
        &self,
        theme: &Theme,
        visible: Vec<(usize, &ListingEntry)>,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut rows: Vec<AnyElement> = Vec::new();
        for (ix, entry) in visible {
            match entry {
                ListingEntry::CommonPrefix(prefix) => {
                    let label = display_name(prefix).to_string();
                    let prefix_sel = prefix.clone();
                    let prefix_nav = prefix.clone();
                    rows.push(
                        h_flex()
                            .id(("object-row", ix))
                            .relative()
                            .w_full()
                            .px_3()
                            .py(tokens::row_pad_y())
                            .gap_2()
                            .border_b_1()
                            .border_color(theme.table_row_border)
                            .text_size(tokens::body())
                            .hover(|row| row.bg(theme.list_hover))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
                                    // 命中行 = 不再是空白点击：拦在列表容器之前，
                                    // 否则容器会把这次点击当成空白而清空刚做的选择
                                    cx.stop_propagation();
                                    if this.renaming.is_some() {
                                        this.commit_rename(cx);
                                        if this.renaming.is_some() {
                                            return;
                                        }
                                    }
                                    this.handle_object_row_click(
                                        ix,
                                        ClickedEntry::CommonPrefix(prefix_sel.clone()),
                                        event.modifiers,
                                        cx,
                                    );
                                }),
                            )
                            // 双击进入目录 —— 与对象行「双击整行预览」同一套 Finder 语义
                            // （判据见纯函数 `row_activation`）：单击只做选择（含 ⌘/⇧
                            // 语义），进入目录要双击。
                            .on_click(cx.listener(move |this, event: &ClickEvent, _window, cx| {
                                if !row_activation(event.click_count(), this.renaming.is_some()) {
                                    return;
                                }
                                this.open_prefix(prefix_nav.clone(), cx);
                            }))
                            .child(
                                h_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_2()
                                    .child(
                                        Icon::new(IconName::Folder)
                                            .text_color(theme.accent_foreground),
                                    )
                                    .child(div().min_w_0().truncate().child(label)),
                            )
                            .child(
                                div()
                                    .w(tokens::col_size_width())
                                    .flex_shrink_0()
                                    .text_color(theme.muted_foreground)
                                    .child("-"),
                            )
                            .child(
                                div()
                                    .w(tokens::col_time_width())
                                    .flex_shrink_0()
                                    .text_color(theme.muted_foreground)
                                    .child("-"),
                            )
                            .into_any_element(),
                    );
                }
                ListingEntry::Object(object) => {
                    let selected = self.selected_object_keys.contains(&object.key);
                    let renaming = self
                        .renaming
                        .as_ref()
                        .is_some_and(|(key, _)| key == &object.key);
                    let key = object.key.clone();
                    let right_key = object.key.clone();
                    let menu_open_key = object.key.clone();
                    let dbl_key = object.key.clone();
                    let size = format_size(object.size);
                    let time = format_time(object.put_time_millis);
                    rows.push(
                        h_flex()
                            .id(("object-row", ix))
                            .relative()
                            .w_full()
                            .px_3()
                            .py(tokens::row_pad_y())
                            .gap_2()
                            .border_b_1()
                            .border_color(theme.table_row_border)
                            .text_size(tokens::body())
                            .when(selected, |row| row.bg(theme.selection))
                            .when(!selected, |row| row.hover(|row| row.bg(theme.list_hover)))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                    // 命中行 = 不再是空白点击（见列表容器的清空处理器）
                                    cx.stop_propagation();
                                    if this.renaming.is_some() {
                                        this.commit_rename(cx);
                                        if this.renaming.is_some() {
                                            return;
                                        }
                                    }
                                    this.handle_object_row_click(
                                        ix,
                                        ClickedEntry::Object(key.clone()),
                                        event.modifiers,
                                        cx,
                                    );
                                }),
                            )
                            .on_mouse_down(
                                MouseButton::Right,
                                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                    // event.position 是窗口坐标，菜单以它为锚点
                                    this.toggle_object_menu(&right_key, event.position, cx);
                                }),
                            )
                            // 双击整行打开预览（Finder 语义；判据见纯函数
                            // `row_activation`）。单击仍只做选择：双击的第一次单击已经
                            // 把选中收敛到本行，这里再显式 scope 一次只为兜住「⌘/⇧ 双击」
                            // 这类组合键下的选中集合。
                            //
                            // 注意行上的 `on_mouse_down` 里有 `stop_propagation()`，
                            // 它不会吞掉本回调：gpui 在 capture 阶段就清掉了 pending
                            // mouse_down，click 监听在 bubble 阶段照常触发
                            // （elements/div.rs 的 mouse_up 处理器）。
                            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                                if !row_activation(event.click_count(), this.renaming.is_some()) {
                                    return;
                                }
                                this.select_object_for_row_action(&dbl_key);
                                this.open_preview_overlay(window, cx);
                            }))
                            .child(self.render_object_name_cell(object, renaming, theme, cx))
                            .child(
                                div()
                                    .w(tokens::col_size_width())
                                    .flex_shrink_0()
                                    .text_color(theme.muted_foreground)
                                    .text_size(tokens::label())
                                    .child(size),
                            )
                            .child(
                                div()
                                    .w(tokens::col_time_width())
                                    .flex_shrink_0()
                                    .text_color(theme.muted_foreground)
                                    .text_size(tokens::label())
                                    .child(time),
                            )
                            .child(
                                // 菜单按**右键的窗口坐标**定位，不挂在行上做相对锚定。
                                //
                                // `anchored()` 的默认行为（`position_mode` = Window 且未给
                                // `position`）是拿「锚定元素自身所在容器的原点」当锚点
                                // （anchored.rs 的 prepaint：anchor 角 + bounds.origin），
                                // 而行身处滚动容器内（列表 `overflow_y_scroll`），行的
                                // layout bounds 与视口不在同一坐标系，锚出来会偏；挂在整行
                                // 上时锚点还会落在行的**左端**，菜单直接越过行左边界压到
                                // 侧栏上。显式 `position()` + Window 模式是唯一不依赖
                                // 容器原点与滚动坐标系的锚法。
                                div().relative().when(
                                    self.object_menu_open.as_deref()
                                        == Some(menu_open_key.as_str()),
                                    |el| {
                                        let at =
                                            self.object_menu_at.unwrap_or(point(px(8.), px(8.)));
                                        el.child(deferred(
                                            anchored()
                                                // 光标点 = 菜单左上角
                                                .anchor(Anchor::TopLeft)
                                                .position_mode(AnchoredPositionMode::Window)
                                                .position(at)
                                                .snap_to_window_with_margin(px(8.))
                                                .child(self.render_object_menu(theme, cx)),
                                        ))
                                    },
                                ),
                            )
                            .into_any_element(),
                    );
                }
            }
        }
        rows
    }

    /// 对象区状态条：「共 N 项」+ 翻页。固定钉在内容区底部（Finder 语义），
    /// 恒定全宽，不随条数滚动或浮动——滚动容器里只放行，任何常驻元素
    /// 必须在滚动容器外（否则条数少时上浮、条数多时被卷走）。
    pub(super) fn render_object_status_bar(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let visible_count = match &self.filtered_ix {
            Some(ix) => ix.len(),
            None => self.entries.len(),
        };
        let mut bar = h_flex()
            .w_full()
            .flex_shrink_0() // 不被滚动区挤压，恒定钉底
            .px_3()
            .py_2()
            .gap_3()
            .border_t_1()
            .border_color(theme.border)
            .text_size(tokens::label())
            .text_color(theme.muted_foreground)
            .child(if visible_count == self.entries.len() {
                format!("共 {} 项", self.entries.len())
            } else {
                format!("显示 {visible_count} / 共 {} 项", self.entries.len())
            });
        if let Some(label) = self.object_sort.label() {
            bar = bar.child(
                div()
                    .text_color(theme.muted_foreground)
                    .child(format!("· {label}")),
            );
        }
        // 弹性槽：有反馈消息时占用（左对齐、超宽截断），否则空占位——
        // 保证右侧的选中信息与翻页按钮始终贴右，不随消息出现而平移。
        if let Some(message) = &self.download_message {
            let color = if message.is_error {
                theme.danger
            } else {
                theme.success
            };
            bar = bar.child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(color)
                    .child(message.text.clone()),
            );
        } else {
            bar = bar.child(div().flex_1());
        }
        // 选中信息 + 主操作就地展示在状态条里：选中/取消只改变这一行的内容，
        // 不会像独立选中条那样把表头与全部行整体推移。
        if !self.selected_object_keys.is_empty() {
            bar = bar
                .child(
                    div()
                        .flex_shrink_0()
                        .text_color(theme.foreground)
                        .child(format!("已选择 {} 个对象", self.selected_object_keys.len())),
                )
                .child(
                    Button::new("toolbar-download")
                        .icon(Icon::new(IconName::ArrowDown))
                        .label("下载")
                        .with_size(Size::Small)
                        .disabled(self.downloading)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.start_object_download(window, cx)
                        })),
                );
        }
        if self.next_marker.is_some() {
            bar = bar.child(
                Button::new("objects-load-more")
                    .label(if self.loading_more {
                        "加载中…"
                    } else {
                        "加载更多"
                    })
                    .loading(self.loading_more)
                    .disabled(self.loading_more)
                    .ghost()
                    .with_size(Size::Small)
                    .on_click(cx.listener(|this, _, _, cx| this.load_more(cx))),
            );
        }
        bar
    }
}
