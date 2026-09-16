//! 对象列表过滤（⌘F）：开关、命中计算与 Esc 关闭。

use super::*;

/// ⌘F 过滤：大小写不敏感子串匹配。命中对象 key 或目录前缀名任意即保留。
/// `None` query = 不过滤。返回保留项下标（指向 entries）。
pub(crate) fn filter_entries(entries: &[ListingEntry], query: Option<&str>) -> Vec<usize> {
    let Some(q) = query.map(str::trim).filter(|q| !q.is_empty()) else {
        return (0..entries.len()).collect();
    };
    let needle = q.to_lowercase();
    entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| match entry {
            ListingEntry::Object(o) => o.key.to_lowercase().contains(&needle),
            ListingEntry::CommonPrefix(p) => p.to_lowercase().contains(&needle),
        })
        .map(|(ix, _)| ix)
        .collect()
}

impl WorkspaceView {
    /// ⌘F：开/关对象列表过滤。开启时焦点入过滤框；关闭时清空查询。
    pub(super) fn handle_toggle_object_filter(
        &mut self,
        _: &ToggleObjectFilter,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.palette.is_some() || self.add_modal.is_some() {
            return;
        }
        if self.selected_bucket.is_none() {
            return;
        }
        if self.object_filter.is_some() {
            // 已开启 → 关闭（Esc 走 DismissFilter 也到这）
            self.close_object_filter(window, cx);
            return;
        }
        let editor = cx.new(|cx| InputState::new(window, cx).placeholder("过滤当前列表…"));
        cx.subscribe_in(&editor, window, |this, _, event: &InputEvent, _, cx| {
            if let InputEvent::Change = event {
                this.refresh_filter(cx);
            }
        })
        .detach();
        let focus_editor = editor.clone();
        focus_editor.update(cx, |state, cx| state.focus(window, cx));
        self.object_filter = Some(editor);
        self.refresh_filter(cx);
        cx.notify();
    }

    /// 关闭过滤：清空查询与命中缓存，焦点归还 Workspace。
    pub(super) fn close_object_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.object_filter.take().is_some() {
            self.filtered_ix = None;
            self.focus_handle.focus(window, cx);
            cx.notify();
        }
    }

    /// 依据输入框当前值重算过滤命中缓存。
    pub(super) fn refresh_filter(&mut self, cx: &mut Context<Self>) {
        let query = self
            .object_filter
            .as_ref()
            .map(|editor| editor.read(cx).value().to_string());
        self.filtered_ix = query
            .as_deref()
            .map(|q| filter_entries(&self.entries, Some(q)));
        cx.notify();
    }

    /// Esc 关闭过滤（context "ObjectFilter"）。
    pub(super) fn handle_dismiss_filter(
        &mut self,
        _: &DismissFilter,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // ⌘L 路径框与过滤条同处 toolbar 下方，共用 ObjectFilter context 的 Esc
        if self.path_input.take().is_some() {
            self.focus_handle.focus(window, cx);
            cx.notify();
            return;
        }
        if self.object_filter.is_some() {
            self.close_object_filter(window, cx);
        }
    }
}
