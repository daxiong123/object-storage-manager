//! 对象多选语义：纯函数（单测锁死）+ 点击/键盘处理器。

use super::*;

/// 计算点击后的选中集合。`ordered_keys` 是当前列表中全部对象 key
/// （按展示顺序，不含目录前缀）；`selection` 是当前选中集合（有序）；
/// `anchor` 是范围选择的起点（上次普通/⌘点击的对象下标）。
pub(crate) fn apply_object_selection(
    intent: ObjectSelectionIntent,
    ordered_keys: &[String],
    selection: &indexmap::IndexSet<String>,
    anchor: Option<usize>,
    clicked: ClickedEntry,
) -> (indexmap::IndexSet<String>, Option<usize>, bool) {
    // 返回 (新选中集合, 新锚点, 是否触发预览)
    if intent.select_all {
        let all: indexmap::IndexSet<String> = ordered_keys.iter().cloned().collect();
        return (all, anchor, false);
    }
    match clicked {
        ClickedEntry::CommonPrefix(_) => (selection.clone(), anchor, false),
        ClickedEntry::None => (indexmap::IndexSet::new(), None, false),
        ClickedEntry::Object(key) => {
            let Some(ix) = intent.clicked_index else {
                return (selection.clone(), anchor, false);
            };
            if intent.shift {
                // ⇧Click：锚点→点击项范围；⌘⇧ 增量（保留原选择），纯 ⇧ 重置为范围
                let start = anchor.unwrap_or(ix).min(ix);
                let end = anchor.unwrap_or(ix).max(ix);
                let mut next = if intent.command {
                    selection.clone()
                } else {
                    indexmap::IndexSet::new()
                };
                for key in ordered_keys[start..=end].iter() {
                    next.insert(key.clone());
                }
                return (next, anchor, false);
            }
            if intent.command {
                // ⌘Click：切换；锚点更新为点击项（Finder 语义）
                let mut next = selection.clone();
                if next.shift_remove(&key) {
                    // 取消选中：锚点仍指向点击项
                    return (next, Some(ix), false);
                }
                next.insert(key);
                return (next, Some(ix), false);
            }
            // 普通 Click：单选主选，触发预览
            let mut next = indexmap::IndexSet::new();
            next.insert(key);
            (next, Some(ix), true)
        }
    }
}

/// 键盘导航纯决策：`keys` 是当前**展示顺序**下的对象 key（不含目录前缀）。
/// `anchor` 与 `keys` 同一坐标系。⇧ 扩选复用 `apply_object_selection` 的范围语义。
pub(crate) fn apply_object_keyboard_nav(
    keys: &[String],
    selection: &indexmap::IndexSet<String>,
    primary: Option<&str>,
    anchor: Option<usize>,
    direction: ObjectNavDirection,
    extend: bool,
) -> (indexmap::IndexSet<String>, Option<usize>, usize) {
    if keys.is_empty() {
        return (indexmap::IndexSet::new(), None, 0);
    }
    let current = primary
        .and_then(|key| keys.iter().position(|candidate| candidate == key))
        .or_else(|| {
            selection
                .last()
                .and_then(|key| keys.iter().position(|candidate| candidate == key))
        });
    let target = match (current, direction) {
        (None, ObjectNavDirection::Next) => 0,
        (None, ObjectNavDirection::Prev) => keys.len() - 1,
        (Some(index), ObjectNavDirection::Next) => (index + 1).min(keys.len() - 1),
        (Some(index), ObjectNavDirection::Prev) => index.saturating_sub(1),
    };
    let intent = ObjectSelectionIntent {
        command: false,
        shift: extend,
        select_all: false,
        clicked_index: Some(target),
    };
    let (next, next_anchor, _) = apply_object_selection(
        intent,
        keys,
        selection,
        if extend { anchor.or(current) } else { current },
        ClickedEntry::Object(keys[target].clone()),
    );
    (next, next_anchor, target)
}

/// 展示顺序下的对象 key（跳过目录前缀）。键盘导航与 Shift 范围用这套顺序。
pub(crate) fn visible_object_keys(
    entries: &[ListingEntry],
    display_order: &[usize],
) -> Vec<String> {
    display_order
        .iter()
        .filter_map(|&index| entries.get(index).and_then(object_key))
        .map(str::to_string)
        .collect()
}

/// 列表行「激活」判据：单击只做选择，**双击整行**才打开（对象 → 预览、目录 → 进入）。
///
/// Finder 语义。抽成纯函数是为了让两处行渲染（对象行 / 目录行）共用同一条判据，
/// 免得日后一处改了一处没改；单测锁死「单击不激活」这一点——把判据放松成
/// `>= 1` 会让单击既选中又打开，等于回到「点一下就跳走」。
///
/// `renaming`：行内重命名还在进行（即 mouse_down 里的提交校验失败、inline editor
/// 仍残留）时不激活，否则会跳走并丢掉编辑态。
pub(crate) fn row_activation(click_count: usize, renaming: bool) -> bool {
    click_count >= 2 && !renaming
}

pub(super) fn object_selection_ix(entries: &[ListingEntry], key: &str) -> Option<usize> {
    entries
        .iter()
        .filter_map(object_key)
        .position(|object_key| object_key == key)
}

pub(super) fn object_keys(entries: &[ListingEntry]) -> Vec<String> {
    entries
        .iter()
        .filter_map(object_key)
        .map(str::to_string)
        .collect()
}

impl WorkspaceView {
    /// 清空多选与主选（切桶/翻页/删除后）。
    pub(super) fn clear_object_selection(&mut self) {
        self.selected_object_key = None;
        self.selected_object_keys.clear();
        self.selection_anchor = None;
        self.renaming = None;
        self.object_menu_open = None;
        self.top_more_open = false;
        self.details_overlay_open = false;
    }

    pub(super) fn selected_cloud_object(&self) -> Option<&CloudObject> {
        let key = self.selected_object_key.as_ref()?;
        self.entries.iter().find_map(|e| match e {
            ListingEntry::Object(o) if o.key == *key => Some(o),
            _ => None,
        })
    }

    pub(super) fn selected_object_keys_vec(&self) -> Vec<String> {
        if self.selected_object_keys.is_empty() {
            self.selected_cloud_object()
                .map(|object| vec![object.key.clone()])
                .unwrap_or_default()
        } else {
            self.selected_object_keys.iter().cloned().collect()
        }
    }

    /// 行级操作按钮以“该行对象”为作用域，避免当前多选集合导致误批量操作。
    pub(super) fn select_object_for_row_action(&mut self, key: &str) {
        let anchor = object_selection_ix(&self.entries, key);
        self.selected_object_keys.clear();
        self.selected_object_keys.insert(key.to_string());
        self.selected_object_key = Some(key.to_string());
        self.selection_anchor = anchor;
        self.renaming = None;
    }

    /// 对象行点击 → 多选语义（规范 §7）。纯决策在 `apply_object_selection`，
    /// 这里只负责取上下文 + 回写状态；预览由文件名/预览按钮显式触发。
    pub(super) fn handle_object_row_click(
        &mut self,
        ix: usize,
        clicked: ClickedEntry,
        modifiers: gpui::Modifiers,
        cx: &mut Context<Self>,
    ) {
        let (baseline, baseline_anchor) =
            (self.selected_object_keys.clone(), self.selection_anchor);
        let ordered_keys = object_keys(&self.entries);
        let clicked_index = match &clicked {
            ClickedEntry::Object(key) => {
                ordered_keys.iter().position(|object_key| object_key == key)
            }
            ClickedEntry::CommonPrefix(_) => Some(ix),
            ClickedEntry::None => None,
        };
        let intent = ObjectSelectionIntent {
            command: modifiers.platform,
            shift: modifiers.shift,
            select_all: false,
            clicked_index,
        };
        let (next, anchor, _) = apply_object_selection(
            intent,
            &ordered_keys,
            &baseline,
            if baseline_anchor.is_some() {
                baseline_anchor
            } else {
                self.selection_anchor
            },
            clicked,
        );
        self.selected_object_keys = next;
        self.selected_object_key = self.selected_object_keys.last().cloned();
        self.selection_anchor = anchor;
        self.object_menu_open = None;
        self.top_more_open = false;
        cx.notify();
    }

    /// ⌘A：全选当前列表中的对象（不含目录前缀）。命令面板/模态打开时忽略。
    pub(super) fn handle_select_all(
        &mut self,
        _: &SelectObjectAll,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.palette.is_some() || self.add_modal.is_some() {
            return;
        }
        if self.selected_bucket.is_none() {
            return;
        }
        let intent = ObjectSelectionIntent {
            command: false,
            shift: false,
            select_all: true,
            clicked_index: None,
        };
        let ordered_keys = object_keys(&self.entries);
        let (next, anchor, _) = apply_object_selection(
            intent,
            &ordered_keys,
            &self.selected_object_keys,
            self.selection_anchor,
            ClickedEntry::None,
        );
        self.selected_object_keys = next;
        self.selected_object_key = self.selected_object_keys.last().cloned();
        self.selection_anchor = anchor;
        cx.notify();
    }

    pub(super) fn object_keyboard_nav_blocked(&self) -> bool {
        self.palette.is_some()
            || self.add_modal.is_some()
            || self.settings_modal.is_some()
            || self.renaming.is_some()
            || self.renaming_busy
            || self.path_input.is_some()
            || self.create_folder_input.is_some()
            || self.copy_move.is_some()
            || self.about_overlay_open
            || self.details_overlay_open
            || self.selected_bucket.is_none()
    }

    pub(super) fn handle_select_object_prev(
        &mut self,
        _: &SelectObjectPrev,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_object_selection(ObjectNavDirection::Prev, false, cx);
    }

    pub(super) fn handle_select_object_next(
        &mut self,
        _: &SelectObjectNext,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_object_selection(ObjectNavDirection::Next, false, cx);
    }

    pub(super) fn handle_select_object_prev_range(
        &mut self,
        _: &SelectObjectPrevRange,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_object_selection(ObjectNavDirection::Prev, true, cx);
    }

    pub(super) fn handle_select_object_next_range(
        &mut self,
        _: &SelectObjectNextRange,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_object_selection(ObjectNavDirection::Next, true, cx);
    }

    /// ↑↓/←→ 按当前展示顺序移动主选；⇧ 扩选。预览打开时只换对象、不扩选，并重载预览。
    /// 菜单/工具栏动作的目标集合。
    ///
    /// 菜单开着时用菜单记录的目标（见 `menu_targets_for`），否则就是普通选择集——
    /// 菜单栏、命令面板、快捷键走的都是这条回落路径。
    pub(super) fn action_target_keys(&self) -> Vec<String> {
        // 自守：不仅要求菜单开着，还要求**记录的目标里包含菜单所指的那一行**。
        // 这样即使哪条关闭路径忘了清空目标，也不会把陈旧目标当成动作对象——
        // 代价只是一次线性查找，换掉的是「删错对象」这类错误。
        if let Some(open) = self.object_menu_open.as_deref()
            && self
                .object_menu_targets
                .iter()
                .any(|candidate| candidate == open)
        {
            return self.object_menu_targets.clone();
        }
        self.selected_object_keys_vec()
    }

    /// 行首复选框：切换单个对象的选中态（等同 ⌘Click）。
    ///
    /// 刻意复用 `apply_object_selection`，不在复选框里另写一份「加入/移出集合」：
    /// 选择语义（主选、锚点、目录前缀不参与）只在一处定义，否则两处迟早不同步。
    pub(super) fn toggle_object_key_selection(&mut self, key: &str, cx: &mut Context<Self>) {
        let ordered_keys = object_keys(&self.entries);
        let Some(clicked_index) = ordered_keys.iter().position(|candidate| candidate == key) else {
            return;
        };
        let intent = ObjectSelectionIntent {
            command: true,
            shift: false,
            select_all: false,
            clicked_index: Some(clicked_index),
        };
        let (next, anchor, _) = apply_object_selection(
            intent,
            &ordered_keys,
            &self.selected_object_keys,
            self.selection_anchor,
            ClickedEntry::Object(key.to_string()),
        );
        self.selected_object_keys = next;
        self.selected_object_key = self.selected_object_keys.last().cloned();
        self.selection_anchor = anchor;
        self.object_menu_open = None;
        self.top_more_open = false;
        cx.notify();
    }

    pub(super) fn move_object_selection(
        &mut self,
        direction: ObjectNavDirection,
        extend: bool,
        cx: &mut Context<Self>,
    ) {
        if self.object_keyboard_nav_blocked() {
            return;
        }
        let extend = extend && !self.preview_overlay_open;
        let order =
            display_entry_order(&self.entries, self.object_sort, self.filtered_ix.as_deref());
        let keys = visible_object_keys(&self.entries, &order);
        if keys.is_empty() {
            return;
        }
        let display_anchor = self.selection_anchor.and_then(|index| {
            object_keys(&self.entries)
                .get(index)
                .and_then(|key| keys.iter().position(|candidate| candidate == key))
        });
        let (next, next_anchor, target) = apply_object_keyboard_nav(
            &keys,
            &self.selected_object_keys,
            self.selected_object_key.as_deref(),
            display_anchor,
            direction,
            extend,
        );
        let next_primary = keys.get(target).cloned();
        let changed = next != self.selected_object_keys || next_primary != self.selected_object_key;
        self.selected_object_keys = next;
        self.selected_object_key = next_primary;
        self.selection_anchor = next_anchor.and_then(|index| {
            keys.get(index)
                .and_then(|key| object_selection_ix(&self.entries, key))
        });
        self.object_menu_open = None;
        self.top_more_open = false;
        // 把新的主选滚进视野：虚拟列表只渲染可见区间，长列表里「选中了但看不见」
        // 等于没有反馈。`order` 含目录前缀，所以它的下标就是 uniform_list 的 item
        // 下标（`keys` 只含对象、不能直接用）。用 Nearest 非严格滚动：已在视野内
        // 就不动，避免方向键把列表来回拽。
        if let Some(selected) = self.selected_object_key.as_deref()
            && let Some(slot) = display_slot_of_key(&self.entries, &order, selected)
        {
            self.object_list_scroll
                .scroll_to_item(slot, ScrollStrategy::Nearest);
        }
        if self.preview_overlay_open && changed {
            self.start_object_preview(cx);
        } else {
            cx.notify();
        }
    }
}

/// 显示顺序里某个对象 Key 的**槽位**下标（= 虚拟列表 `uniform_list` 的 item 下标）。
///
/// 必须按 `order`（含目录前缀、已应用排序与过滤）数，不能按「只含对象的 keys」
/// 数：目录前缀同样占一个槽位，用 keys 的下标当槽位会把列表滚到错位置——行数少
/// 时看不出来，长列表里越滚越偏。与 `object_selection_ix` 是同一类索引陷阱。
pub(super) fn display_slot_of_key(
    entries: &[ListingEntry],
    order: &[usize],
    key: &str,
) -> Option<usize> {
    order
        .iter()
        .position(|&ix| entries.get(ix).and_then(object_key) == Some(key))
}
