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

/// 过滤查询词由 `previous` 变为 `current` 时，是否**必须丢弃对象选择**。
///
/// 只有「开始过滤」或「过滤条件变了」才丢：空白查询在 `filter_entries` 里等同不过滤
/// （可见集 = 全集），此时选择里的每一项都还看得见，丢它是误伤。
///
/// **内部自行 trim**（不依赖调用方先处理，免得文档与调用点各说一套——第一版就是这么
/// 写错的）：既与 `filter_entries` 对空白查询的处理保持一致（只敲一个空格不算开始
/// 过滤），也让「多敲一个尾随空格」不触发误清。
pub(crate) fn filter_drops_selection(previous: &str, current: &str) -> bool {
    let previous = previous.trim();
    let current = current.trim();
    previous != current && !current.is_empty()
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
            // 复位查询词记录：否则「关掉再打开」会把空框当成一次查询变化，
            // 白白清掉用户的选择。
            self.filter_query.clear();
            self.focus_handle.focus(window, cx);
            cx.notify();
        }
    }

    /// 依据输入框当前值重算过滤命中缓存；**查询词变化时丢弃对象选择**。
    ///
    /// 为什么必须丢（这不是外观问题）：过滤只影响展示，`entries` 与选择集合都不动，
    /// 于是一旦「先选了一批 → 再搜索」，过滤结果会带着**旧的**选中底色；而 ⌘⌫ 与
    /// 批量下载取的是 `selected_object_keys` **全集**，其中可能包含**当前不可见**的
    /// 对象。实测复现：选中 45 个 → ⌘F 搜出 5 行 → ⌘⌫ 的对话框写着「删除 45 个对象」，
    /// 而用户的视觉上下文是那 5 行。丢掉选择后，「看到的选中集 == 会被操作的集合」
    /// 才成立。
    ///
    /// 只在**查询词真的变了**时丢（判据见 `filter_drops_selection`）：本函数也会在
    /// 数据重载、翻页（`request_objects` 追加）之后被调用，那时清选择纯属误伤。
    pub(super) fn refresh_filter(&mut self, cx: &mut Context<Self>) {
        let query = self
            .object_filter
            .as_ref()
            .map(|editor| editor.read(cx).value().to_string());
        let effective = query.as_deref().unwrap_or_default().trim().to_string();
        // 判据自己会 trim，这里存 trim 后的值只是让下次比较省一次处理
        if filter_drops_selection(&self.filter_query, query.as_deref().unwrap_or_default()) {
            // 一并关掉行内重命名/右键菜单/详情：这些状态同样指向前一刻的行，
            // 被过滤掉后仍保留会变成「对着看不见的行操作」。
            self.clear_object_selection();
        }
        self.filter_query = effective;
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
