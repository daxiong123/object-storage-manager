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
                        // 「上传 ▾」：一个按钮带下拉（参照实现如此），两个条目
                        // （上传文件… / 上传文件夹…）走同一条既有上传路径。
                        // 原先「上传文件夹」藏在「更多」里，发现成本高。
                        div()
                            .relative()
                            .child(
                                Button::new("toolbar-upload")
                                    .icon(Icon::new(IconName::ArrowUp))
                                    .label(if self.uploading {
                                        "选择文件…"
                                    } else {
                                        "上传"
                                    })
                                    .icon(Icon::new(IconName::ChevronDown))
                                    .primary()
                                    .with_size(Size::Small)
                                    .disabled(self.uploading)
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation()
                                    })
                                    .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
                                        this.toggle_toolbar_menu(
                                            ToolbarMenu::Upload,
                                            event.position(),
                                            cx,
                                        )
                                    })),
                            )
                            .when(self.toolbar_menu == Some(ToolbarMenu::Upload), |button| {
                                let at = self.toolbar_menu_at.unwrap_or(point(px(8.), px(8.)));
                                button.child(deferred(
                                    anchored()
                                        .anchor(Anchor::TopLeft)
                                        .position_mode(AnchoredPositionMode::Window)
                                        .position(at)
                                        .snap_to_window_with_margin(px(8.))
                                        .child(self.render_upload_menu(theme, cx)),
                                ))
                            }),
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
                                    .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
                                        this.toggle_toolbar_menu(
                                            ToolbarMenu::More,
                                            event.position(),
                                            cx,
                                        )
                                    })),
                            )
                            .when(self.toolbar_menu == Some(ToolbarMenu::More), |button| {
                                // 锚定用**触发点的窗口坐标**（与对象右键菜单同一写法）。
                                // 不要用 `anchor(...) + offset(...)` 的相对定位：它锚的是
                                // 「锚定元素所在容器的原点」，而这个按钮所在的容器会变
                                // （从标题栏挪到内容区工具栏后菜单就整体偏移了）。
                                let at = self.toolbar_menu_at.unwrap_or(point(px(8.), px(8.)));
                                button.child(deferred(
                                    anchored()
                                        .anchor(Anchor::TopLeft)
                                        .position_mode(AnchoredPositionMode::Window)
                                        .position(at)
                                        .snap_to_window_with_margin(px(8.))
                                        .child(self.render_top_more_menu(theme, cx)),
                                ))
                            }),
                    ),
            )
            .child(self.render_object_search(cx))
            // 刷新放在搜索框右侧（参照实现的工具栏右端是 搜索 + ⟳）：
            // 它是「重新拉取当前列表」，跟右侧这组读取类控件在一起更顺。
            .child(
                Button::new("toolbar-refresh")
                    .icon(Icon::new(IconName::Replace))
                    .ghost()
                    .with_size(Size::Small)
                    .tooltip("刷新")
                    .disabled(self.objects_state == AsyncState::Loading)
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(
                        cx.listener(|this, _, window, cx| {
                            this.handle_refresh(&Refresh, window, cx)
                        }),
                    ),
            )
            .into_any_element()
    }

    /// 「每页条数」菜单（参照实现右下角）。选项来自 `PAGE_LIMIT_CHOICES`。
    pub(super) fn render_page_limit_menu(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut menu = ui::menu::popup(theme);
        for (ix, limit) in PAGE_LIMIT_CHOICES.into_iter().enumerate() {
            let color = if limit == self.page_limit {
                theme.primary
            } else {
                theme.foreground
            };
            menu = menu.child(
                ui::menu::item(theme, ("page-limit-choice", ix), color, true)
                    .child(format!("{limit} 条/页"))
                    .on_click(cx.listener(move |this, _, _, cx| this.set_page_limit(limit, cx))),
            );
        }
        menu.into_any_element()
    }

    /// 切换每页条数：**丢弃选择并重载**。
    ///
    /// 为什么必须丢选择：换页大小会整体换掉可见集，而 ⌘⌫ / 批量下载取的是选择**全集**
    /// （不含可见性判断）——留着旧选择就是「删掉看不见的对象」那个风险。
    /// 与 ⌘F 过滤那边同一条纪律，见 agents.md 的 ⌘F 行。
    pub(super) fn set_page_limit(&mut self, limit: u32, cx: &mut Context<Self>) {
        self.page_limit_menu_open = false;
        if limit == self.page_limit {
            cx.notify();
            return;
        }
        self.page_limit = limit;
        // reload_objects 会清空 entries/选择并重新请求第一页
        self.reload_objects(cx);
    }

    /// 工具栏右端：**前缀搜索**（参照实现的「文件前缀搜索」）。
    ///
    /// 常驻显示（不再是 ⌘F 开关的浮层），右侧一个 🔍 提交按钮——参照实现是
    /// **显式提交**的查询，不是边打边筛。提交后把输入值当作列举前缀重新请求
    /// （见 `commit_prefix_search`），所以能查到还没加载出来的对象。
    /// 对象搜索：输入框 + 接合的放大镜按钮（形状与理由见 `ui::compact_search_field`）。
    ///
    /// 这里只管业务外壳：Esc 上下文（`ObjectFilter` → `DismissFilter`）、宽度、
    /// 以及不让点击穿到列表（列表容器上的空白点击会清空选择）。
    pub(super) fn render_object_search(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let Some(editor) = self.search_input.as_ref() else {
            return div().into_any_element();
        };
        h_flex()
            .id("object-search")
            .key_context("ObjectFilter")
            .w(tokens::text(220.))
            .items_center()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(ui::compact_search_field(
                editor,
                &theme,
                cx.listener(|this, _, window, cx| this.commit_prefix_search(window, cx)),
            ))
            .into_any_element()
    }

    /// 中间内容区：对象列表（选中桶后异步加载，含前缀导航与翻页）。
    pub(super) fn render_content(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // 显示顺序在**表头之前**算好并存起来：表头要拿它判断「是否已全选」，
        // 而表头先于列表渲染——放在 render_object_list 里会让表头读到上一帧的缓存。
        self.display_order = display_entry_order(&self.entries, self.object_sort);
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
                // 地址栏与工具栏都在加载/失败分支之前挂上：它们是对象区的常驻
                // chrome，不该随加载态出现/消失（否则表头会上下跳）。
                // 顺序对齐参照实现：地址栏行 → 操作工具栏 → 表格
                .child(self.render_address_bar(theme, cx))
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

    /// ⌘F：聚焦前缀搜索框（参照实现里搜索是工具栏上的常驻控件）。
    pub(super) fn handle_focus_object_search(
        &mut self,
        _: &FocusObjectSearch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.palette.is_some() || self.add_modal.is_some() || self.settings_modal.is_some() {
            return;
        }
        self.ensure_search_input(window, cx);
        if let Some(editor) = self.search_input.as_ref() {
            editor.update(cx, |state, cx| state.focus(window, cx));
        }
        cx.notify();
    }

    /// Esc：优先关 ⌘L 路径框，否则**清空**搜索框。
    ///
    /// 搜索框是常驻控件，Esc 不该把它藏起来（参照实现也没有隐藏态），所以这里是清空
    /// 而不是关闭；清空后列表保持当前前缀，需再回车才重新列举——与「显式提交」一致。
    pub(super) fn handle_dismiss_filter(
        &mut self,
        _: &DismissFilter,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.path_input.take().is_some() {
            self.focus_handle.focus(window, cx);
            cx.notify();
            return;
        }
        if let Some(editor) = self.search_input.clone() {
            editor.update(cx, |state, cx| state.set_value("", window, cx));
        }
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
        if self.entries.is_empty() && self.objects_state == AsyncState::Idle {
            return Self::object_list_placeholder(
                theme,
                IconName::Inbox,
                "此目录为空",
                "拖入文件或文件夹即可上传",
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
                // 重命名已改为弹层：遮罩 `occlude` 之后列表收不到点击，
                // 「点别处提交重命名」这一步随之取消（现由弹层自己的按钮/Esc/遮罩决定）。
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
        let visible_keys = visible_object_keys(&self.entries, &self.display_order);
        let all_visible_selected = !visible_keys.is_empty()
            && visible_keys
                .iter()
                .all(|k| self.selected_object_keys.contains(k));
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
                // 全选框（参照实现的表格第一列）：勾上＝选中当前可见的全部对象，
                // 取消＝清空；语义与 ⌘A 一致，走同一个 handler。
                div()
                    .w(tokens::col_check_width())
                    .flex_shrink_0()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        Checkbox::new("header-select-all")
                            .checked(all_visible_selected)
                            .on_click(cx.listener(|this, checked: &bool, window, cx| {
                                if *checked {
                                    this.handle_select_all(&SelectObjectAll, window, cx);
                                } else {
                                    this.clear_object_selection();
                                    cx.notify();
                                }
                            })),
                    ),
            )
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
            // 「操作」列表头（行末图标列）。这不只是补个文字：**表头必须用与数据行
            // 相同的固定宽度占位**，否则名称列（flex_1）会多吸收这一列的宽度，
            // 后面几列整体右移——实测过表头比数据行右偏 55px。
            .child(
                div()
                    .w(tokens::col_action_width())
                    .flex_shrink_0()
                    .child("操作"),
            )
    }

    /// 名称单元格。重命名已改为弹层（见 `rename.rs`），所以这里恒为纯文本——
    /// 行内编辑那套（行变 Input + 上方横幅提示）随之删除。
    pub(super) fn render_object_name_cell(
        &self,
        object: &CloudObject,
        theme: &Theme,
    ) -> AnyElement {
        let icon = crate::file_type::file_type_icon(&object.key, theme.mode);
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
                    // 目录行不参与对象选择，但列要对齐：首尾留同宽占位
                    .child(div().w(tokens::col_check_width()).flex_shrink_0())
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_2()
                            .child(
                                Icon::new(IconName::Folder)
                                    .text_color(crate::theme::file_icon_color(theme.mode)),
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
                    .child(div().w(tokens::col_action_width()).flex_shrink_0())
                    .into_any_element()
            }
            ListingEntry::Object(object) => {
                // 「勾选」与「当前行」是两件事，必须分开：
                // - `checked` = 在勾选集合里（只由复选框 / ⌘Click / ⇧Click / ⌘A 改变）
                // - `active`  = 当前行（不带动词的点击/方向键移动的就是它）
                // 高亮两者都给（否则点一行看不出「动作会作用在谁」），但**复选框只认
                // `checked`**——曾经两者共用同一个标志，于是「点几行看一眼」就把它们
                // 全勾上了，再按删除会一起删掉（见 selection.rs 的 `plain_click`）。
                let checked = self.selected_object_keys.contains(&object.key);
                let active =
                    checked || self.selected_object_key.as_deref() == Some(object.key.as_str());
                let key = object.key.clone();
                let check_key = object.key.clone();
                let actions_key = object.key.clone();
                let right_key = object.key.clone();
                let menu_open_key = object.key.clone();
                let dbl_key = object.key.clone();
                let download_key = object.key.clone();
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
                    .when(active, |row| row.bg(theme.selection))
                    .when(!active, |row| row.hover(|row| row.bg(theme.list_hover)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                            // 命中行 = 不再是空白点击（见列表容器的清空处理器）
                            cx.stop_propagation();
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
                    .child(
                        div()
                            .w(tokens::col_check_width())
                            .flex_shrink_0()
                            // 复选框自己吃掉 mouse_down，否则行处理器会把它当成
                            // 「点行」而先做一次选择，再叠加一次切换
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .child(Checkbox::new(("row-check", ix)).checked(checked).on_click(
                                cx.listener(move |this, _: &bool, _, cx| {
                                    this.toggle_object_key_selection(&check_key.clone(), cx)
                                }),
                            )),
                    )
                    .child(self.render_object_name_cell(object, theme))
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
                    .child(
                        // 「操作」列（参照实现的最后一列 = ⤓ ⋯）。下载是最常用动作，
                        // 提到行内省一次开菜单；其余动作留在 ⋯ 里。
                        // 两点与工具栏的「下载」不同：这里先**把选择收敛到本行**
                        // （`select_object_for_row_action`），所以未选中任何行时点行内
                        // 下载也按本行来，不需要先选。
                        div()
                            .w(tokens::col_action_width())
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap_1()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .child(
                                Button::new(("row-download", ix))
                                    .icon(Icon::new(IconName::ArrowDown))
                                    .ghost()
                                    .with_size(Size::Small)
                                    .tooltip("下载")
                                    .disabled(self.downloading)
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation()
                                    })
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.select_object_for_row_action(&download_key);
                                        this.start_object_download(window, cx);
                                    })),
                            )
                            .child(
                                Button::new(("row-actions", ix))
                                    .icon(Icon::new(IconName::Ellipsis))
                                    .ghost()
                                    .with_size(Size::Small)
                                    .tooltip("更多操作")
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation()
                                    })
                                    .on_click(cx.listener(
                                        move |this, event: &ClickEvent, _, cx| {
                                            this.open_row_actions_menu(
                                                &actions_key,
                                                event.position(),
                                                cx,
                                            );
                                        },
                                    )),
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
        let visible_count = self.entries.len();
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
        // 右段：每页条数选择器（参照实现右下角那一项）。服务端列举本就是
        // 「单页条数上限」语义，所以这是真值，不是装饰。
        bar = bar.child(
            div()
                .relative()
                .child(
                    Button::new("page-limit")
                        .label(format!("{} 条/页", self.page_limit))
                        .with_size(Size::Small)
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
                            this.page_limit_menu_open = !this.page_limit_menu_open;
                            this.page_limit_menu_at = Some(event.position());
                            cx.notify();
                        })),
                )
                .when(self.page_limit_menu_open, |el| {
                    let at = self.page_limit_menu_at.unwrap_or(point(px(8.), px(8.)));
                    el.child(deferred(
                        anchored()
                            .anchor(Anchor::TopRight)
                            .position_mode(AnchoredPositionMode::Window)
                            .position(at)
                            .snap_to_window_with_margin(px(8.))
                            .child(self.render_page_limit_menu(theme, cx)),
                    ))
                }),
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

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Modifiers, TestAppContext};
    use std::cell::Cell;
    use std::rc::Rc;

    /// 复刻「行 > 操作列包裹层 > ⋯ 按钮」的结构。
    /// 目的：验证点 ⋯ **不会**触发行选中（行处理器必须收不到 mouse_down）。
    struct Harness {
        row_down: Rc<Cell<usize>>,
        menu_opens: Rc<Cell<usize>>,
    }

    impl Render for Harness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let row_down = self.row_down.clone();
            let menu_opens = self.menu_opens.clone();
            div()
                .id("row")
                .size_full()
                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                    row_down.set(row_down.get() + 1);
                    cx.stop_propagation();
                })
                .child(
                    div()
                        .id("actions-wrap")
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_end()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            Button::new("actions-btn")
                                .icon(Icon::new(IconName::Ellipsis))
                                .ghost()
                                .with_size(Size::Small)
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .on_click(move |_, _, _| {
                                    menu_opens.set(menu_opens.get() + 1);
                                }),
                        ),
                )
        }
    }

    /// 返回 (行收到 mouse_down 的次数, 菜单被打开的次数)
    fn probe(cx: &mut TestAppContext) -> (usize, usize) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::init(cx);
        });
        let row_down = Rc::new(Cell::new(0));
        let menu_opens = Rc::new(Cell::new(0));
        let (r, m) = (row_down.clone(), menu_opens.clone());
        let (_view, cx) = cx.add_window_view(move |_window, _cx| Harness {
            row_down: r,
            menu_opens: m,
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        // 点在 ⋯ 按钮上：包裹层 justify_end，按钮在窗口右端的 40px 内（窗口 1920 宽）
        let at = point(px(1900.), px(540.));
        cx.simulate_mouse_move(at, None, Modifiers::default());
        cx.simulate_mouse_down(at, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(at, MouseButton::Left, Modifiers::default());
        cx.run_until_parked();
        (row_down.get(), menu_opens.get())
    }

    #[gpui::test]
    fn clicking_the_action_button_does_not_select_the_row(cx: &mut TestAppContext) {
        let (row_down, menu_opens) = probe(cx);
        assert_eq!(
            row_down, 0,
            "点 ⋯ 不该触发行选中（行处理器收到了 mouse_down）"
        );
        assert_eq!(menu_opens, 1, "点 ⋯ 应当打开菜单");
    }
}
