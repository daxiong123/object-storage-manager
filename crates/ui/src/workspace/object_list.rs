//! 对象列表渲染：内容区骨架、表头、行、状态条。

use super::*;

impl WorkspaceView {
    /// 表格上方的操作工具栏（对象区自己的 chrome）。
    ///
    /// 原先这些控件挤在统一标题栏右端；标题栏的职责是「窗口级导航」——
    /// 侧栏开关、前进/后退、当前位置——上传/过滤/更多属于**对象区**，
    /// 放在表格正上方更贴近它们作用的范围，也不再和窗口拖拽区抢位置。
    /// 布局对齐参考实现：**操作在左、搜索在右**。
    pub(super) fn render_object_toolbar(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        h_flex()
            .id("object-toolbar")
            .w_full()
            .flex_shrink_0()
            .items_center()
            .justify_between()
            .gap_2()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(theme.border)
            .child(
                h_flex()
                    .flex_shrink_0()
                    .items_center()
                    .gap_1()
                    // 操作组顺序与参照实现一致：上传（主色）· 新建目录 · 下载 · 更多
                    .child(
                        Button::new("toolbar-upload-files")
                            .icon(Icon::new(IconName::ArrowUp))
                            .label(if self.uploading {
                                "选择文件…"
                            } else {
                                "上传"
                            })
                            .primary()
                            .with_size(Size::Small)
                            .disabled(self.uploading)
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(|this, _, _, cx| this.start_files_upload(cx))),
                    )
                    .child(
                        Button::new("toolbar-create-folder")
                            .label("新建目录")
                            .with_size(Size::Small)
                            .disabled(self.creating_folder)
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_create_folder_overlay(window, cx)
                            })),
                    )
                    // 下载放在工具栏（参照实现如此），不再占底栏位置
                    .child(
                        Button::new("toolbar-download")
                            .label("下载")
                            .with_size(Size::Small)
                            .disabled(
                                self.downloading || self.selected_object_keys_vec().is_empty(),
                            )
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.start_object_download(window, cx)
                            })),
                    )
                    .child(
                        Button::new("toolbar-refresh")
                            .label("刷新")
                            .with_size(Size::Small)
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.handle_refresh(&Refresh, window, cx)
                            })),
                    )
                    .child(
                        div()
                            .relative()
                            .child(
                                Button::new("toolbar-more")
                                    .icon(Icon::new(IconName::ChevronDown))
                                    .label("更多")
                                    .with_size(Size::Small)
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation()
                                    })
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.toggle_top_more_menu(cx)),
                                    ),
                            )
                            .when(self.top_more_open, |button| {
                                button.child(deferred(
                                    anchored()
                                        .anchor(Anchor::TopRight)
                                        .offset(point(px(0.), px(4.)))
                                        .snap_to_window_with_margin(px(8.))
                                        .child(self.render_top_more_menu(theme, cx)),
                                ))
                            }),
                    ),
            )
            .child(self.render_object_filter(cx))
            .into_any_element()
    }

    /// 工具栏右端：过滤输入框（展开时）或搜索图标按钮。
    pub(super) fn render_object_filter(&self, cx: &mut Context<Self>) -> AnyElement {
        if let Some(editor) = &self.object_filter {
            return h_flex()
                .id("object-filter")
                .key_context("ObjectFilter")
                .w(tokens::text(220.))
                .items_center()
                .gap_1()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(div().flex_1().min_w_0().child(Input::new(editor).small()))
                .child(
                    Button::new("filter-close")
                        .icon(Icon::new(IconName::Close))
                        .ghost()
                        .with_size(Size::Small)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.close_object_filter(window, cx);
                        })),
                )
                .into_any_element();
        }
        Button::new("objects-filter")
            .icon(Icon::new(IconName::Search))
            .ghost()
            .with_size(Size::Small)
            .tooltip("过滤 ⌘F")
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _, window, cx| {
                this.handle_toggle_object_filter(&ToggleObjectFilter, window, cx);
            }))
            .into_any_element()
    }

    /// 中间内容区：对象列表（选中桶后异步加载，含前缀导航与翻页）。
    pub(super) fn render_content(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
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
                .bg(theme.background)
                // 工具栏在加载/失败分支之前就挂上：它是对象区的常驻 chrome，
                // 不该随加载态出现/消失（否则表头会上下跳）。
                .child(self.render_object_toolbar(theme, cx)),
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

        // 行内重命名的校验提示走横幅（行高固定，行内放不下第二行）
        if let Some(banner) = self.rename_validation_banner(theme, cx) {
            content = content.child(banner);
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

    /// 对象列表本体：虚拟列表（`uniform_list`）+ 行级操作列。
    ///
    /// **空白点击清空选择（B6）挂在滚动容器上**：`UniformList` 自己实现了
    /// `InteractiveElement`，它的 base 就是那个带 `BlockMouseExceptScroll`
    /// hitbox 的滚动容器，所以清空处理器仍落在「收得到空白点击」的那一层。
    /// 历史上试过**祖先**容器（内容区）的 capture 与 bubble 两版，都因那层
    /// hitbox 覆盖范围不可靠而失败——这里不是祖先，是滚动容器本身。
    /// 行处理器一律 `stop_propagation`，所以能冒泡到这里的一定没命中任何行：
    /// 判据不需要几何，也不依赖注册顺序。
    ///
    /// 行高必须是**确定**的（`tokens::row_height()`）：虚拟列表按**第 0 行**的
    /// 高度给所有行排版（`item_height * item_count`），行不能再由内容撑开。
    pub(super) fn render_object_list(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // 每帧只算一次「排序 × 过滤」，结果给行渲染闭包复用（见 display_order 注释）
        self.display_order =
            display_entry_order(&self.entries, self.object_sort, self.filtered_ix.as_deref());

        if self.entries.is_empty() && self.objects_state == AsyncState::Idle {
            return Self::object_list_placeholder(
                theme,
                IconName::Inbox,
                "此目录为空",
                "拖入文件或文件夹即可上传",
            );
        }
        // ⌘F 过滤开启时只渲染命中项；命中为空给出明确空态。
        // 过滤与排序都只影响展示：entries 全集与选择集合不动（Finder 语义）。
        if self.filtered_ix.is_some() && self.display_order.is_empty() {
            return Self::object_list_placeholder(
                theme,
                IconName::Search,
                "没有匹配的对象",
                "试试其他关键词，或按 Esc 清除过滤",
            );
        }

        let background = theme.background;
        let rows_theme = theme.clone();
        uniform_list(
            "object-list",
            self.display_order.len(),
            cx.processor(
                move |this: &mut Self,
                      range: Range<usize>,
                      _: &mut Window,
                      cx: &mut Context<Self>| {
                    this.object_rows_in_range(range, &rows_theme, cx)
                },
            ),
        )
        .track_scroll(&self.object_list_scroll)
        .flex_1() // row 主轴：宽度占满
        // h_full 必须显式：包裹层（row 容器）不拉伸子元素高度，缺省时
        // 滚动容器高度=内容自然高度 → 越过包裹层叠在状态条上、且无溢出
        // 不产生滚动（滚动条失效）。约束后溢出才成立，滚动条恢复。
        .h_full()
        .min_h_0()
        .bg(background)
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
        )
        .into_any_element()
    }

    /// 空态占位（整块居中）：目录为空 / 过滤无命中。
    ///
    /// 原先它是滚动容器的子元素；`uniform_list` 不接受子元素，所以改成与列表
    /// **互换**的整块占位。视觉上仍是「内容区中央的一段提示」，只是不再可滚动
    /// ——空态本来也没有可滚的内容。
    fn object_list_placeholder(
        theme: &Theme,
        icon: IconName,
        title: &'static str,
        hint: &'static str,
    ) -> AnyElement {
        div()
            .flex_1()
            .h_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_2()
            .text_color(theme.muted_foreground)
            .child(Icon::new(icon).text_size(tokens::icon_sm()))
            .child(
                div()
                    .text_size(tokens::heading())
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(theme.foreground)
                    .child(title),
            )
            .child(div().text_size(tokens::label()).child(hint))
            .into_any_element()
    }

    /// 表头（钉在滚动容器外，与状态条同级——滚动后仍在，Finder/Linear 语义）。
    pub(super) fn render_object_list_header(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // 表格规格照抄参照实现：单元格横向内边距 16px、表头与行同一高度
        // （36px，见 tokens::row_height）、表头底色 #fafafc、文字 12px。
        h_flex()
            .w_full()
            .flex_shrink_0()
            .h(tokens::row_height())
            .px_4()
            .gap_2()
            .border_b_1()
            .border_color(theme.border)
            .bg(theme.list_head)
            .text_size(tokens::label())
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(theme.foreground)
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
    ) -> AnyElement {
        let icon =
            crate::file_type::file_type_icon(&object.key, theme.muted_foreground, theme.accent);
        if renaming && let Some((_, editor)) = &self.renaming {
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
                        // 校验提示不在这里渲染，见 `rename_validation_banner`：
                        // 行高由虚拟列表定死，行内塞不下第二行文字。
                        .child(Input::new(editor).small()),
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

    /// 行内重命名的**校验提示**横幅（放在列表上方，而不是行内）。
    ///
    /// 虚拟列表按固定行高排版（`uniform_list` 取第 0 行的高度给所有行），行内再
    /// 挂一行 caption 会让这行比别的行高，被定高裁掉或与相邻行重叠。提示语因此
    /// 移到列表上方，与「加载更多失败」同一位置——两处都是「当前操作出了问题」
    /// 的横幅，位置统一反而更好找。
    ///
    /// 沿用行内版同一条过滤：把「请输入一个不同的新名称」当成正常输入过程，
    /// 不当错误提示。
    pub(super) fn rename_validation_banner(
        &self,
        theme: &Theme,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let (old_key, editor) = self.renaming.as_ref()?;
        let current_name = editor.read(cx).value().to_string();
        let message = rename_validation_message(old_key, &current_name, &self.entries)
            .filter(|message| message != "请输入一个不同的新名称")?;
        Some(
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
                .child(message)
                .into_any_element(),
        )
    }

    /// 虚拟列表的行渲染回调：只渲染可见区间 `range`（显示顺序里的**槽位**下标）。
    ///
    /// 越界一律跳过而不是 panic：`display_order` 与 `entries` 在同一次 render 内
    /// 一起更新，而本回调到 layout 阶段才被调用，理论上有窗口期。用 `get()` 兜住
    /// 后最坏表现是少画一行，既不会 panic 也不会画错行。
    pub(super) fn object_rows_in_range(
        &mut self,
        range: Range<usize>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut rows = Vec::with_capacity(range.len());
        for slot in range {
            let Some(&ix) = self.display_order.get(slot) else {
                continue;
            };
            let Some(entry) = self.entries.get(ix) else {
                continue;
            };
            rows.push(self.object_row(ix, entry, theme, cx));
        }
        rows
    }

    /// 单行渲染（目录前缀 / 云对象）。
    ///
    /// 每行显式固定为 `tokens::row_height()` 并设 `row_line_height()`：虚拟列表
    /// 按第 0 行给所有行定高，所以每一行都必须正好这么高——包括行内重命名那行
    /// （它的输入框是库固定的 24px，行盒已按此留量，见 tokens::row_height）。
    fn object_row(
        &self,
        ix: usize,
        entry: &ListingEntry,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match entry {
            ListingEntry::CommonPrefix(prefix) => {
                let label = display_name(prefix).to_string();
                let prefix_sel = prefix.clone();
                let prefix_nav = prefix.clone();
                h_flex()
                    .id(("object-row", ix))
                    .relative()
                    .w_full()
                    .h(tokens::row_height())
                    .items_center()
                    .px_4()
                    .gap_2()
                    .border_b_1()
                    .border_color(theme.table_row_border)
                    .text_size(tokens::body())
                    .line_height(tokens::row_line_height())
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
                            .child(Icon::new(IconName::Folder).text_color(theme.accent_foreground))
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
                    .into_any_element()
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
                h_flex()
                    .id(("object-row", ix))
                    .relative()
                    .w_full()
                    .h(tokens::row_height())
                    .items_center()
                    .px_4()
                    .gap_2()
                    .border_b_1()
                    .border_color(theme.table_row_border)
                    .text_size(tokens::body())
                    .line_height(tokens::row_line_height())
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
                    .child(self.render_object_name_cell(object, renaming, theme))
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
                            self.object_menu_open.as_deref() == Some(menu_open_key.as_str()),
                            |el| {
                                let at = self.object_menu_at.unwrap_or(point(px(8.), px(8.)));
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
                    .into_any_element()
            }
        }
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
            // 左段：条目计数（+ 当前排序）。参照实现左段是页码，我们的分页是
            // marker 式（没有真实页码），所以这里放真实计数，不编造页码。
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
        // 中段（弹性槽）：反馈消息优先，否则显示选中数量。
        // 中段占满剩余宽度，右段才始终贴右、不随消息出现而平移。
        let mut middle = div().flex_1().min_w_0().truncate();
        match &self.download_message {
            Some(message) => {
                middle = middle.text_color(if message.is_error {
                    theme.danger
                } else {
                    theme.success
                });
                middle = middle.child(message.text.clone());
            }
            None if !self.selected_object_keys.is_empty() => {
                middle = middle
                    .text_color(theme.foreground)
                    .child(format!("已选择 {} 个对象", self.selected_object_keys.len()));
            }
            None => {}
        }
        bar = bar.child(middle);
        // 右段：已加载数量 + 翻页
        bar = bar.child(
            div()
                .flex_shrink_0()
                .child(format!("已加载 {} 项", self.entries.len())),
        );
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
