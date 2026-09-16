//! 对象分页加载与桶内导航历史（⌘[ / ⌘] / ⌘L 的落点）。

use super::*;

/// 前缀搜索输入的规范化（纯函数，单测锁死）。
///
/// 规则刻意简单：去空白 → 空串即「回到桶根」（`None`）→ 去掉开头的 `/`
/// （用户常从地址栏复制 `oss://bucket/path` 那样的串，前导斜杠不该进 key）。
/// **不**强制补尾斜杠：参照实现的搜索是「文件前缀搜索」，输入 `2025` 应当列出
/// `2025…` 这些对象，补成 `2025/` 就只剩目录了。
pub(crate) fn normalize_prefix_query(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_start_matches('/').trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// 目录前缀的上一级（保持结尾 `/`）；已是根级返回 None。
pub fn parent_prefix(prefix: &str) -> Option<&str> {
    let trimmed = prefix.trim_end_matches('/');
    let idx = trimmed.rfind('/')?;
    Some(&prefix[..=idx])
}

impl WorkspaceView {
    /// 从头（当前前缀的第一页）重新加载对象。
    pub(super) fn reload_objects(&mut self, cx: &mut Context<Self>) {
        self.entries.clear();
        self.next_marker = None;
        self.clear_object_selection();
        // entries 将重建：作废过滤命中缓存（过滤条保留，加载完成后
        // refresh_filter 会按新数据重算；用同一词继续过滤是用户预期）
        self.download_message = None;
        self.objects_state = AsyncState::Loading;
        cx.notify();
        self.request_objects(None, cx);
    }

    /// 下钻到某个目录前缀（压导航历史，⌘[ 可回退）。
    pub(super) fn open_prefix(&mut self, prefix: String, cx: &mut Context<Self>) {
        // 目录切换 = 重上下文切换：关闭过滤条（与跳桶同理）
        self.search_input = None;
        self.push_nav_history();
        self.current_prefix = Some(prefix);
        self.reload_objects(cx);
    }

    /// 导航位置变更前：当前位置压入 back 栈并清空 forward 栈
    /// （浏览器语义：新跳转使 forward 失效）。
    /// 确保前缀搜索框存在（首次渲染时创建并订阅 Enter）。
    pub(super) fn ensure_search_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.search_input.is_some() {
            return;
        }
        let editor = cx.new(|cx| InputState::new(window, cx).placeholder("文件前缀搜索"));
        cx.subscribe_in(
            &editor,
            window,
            |this, _, event: &InputEvent, window, cx| {
                // 参照实现的搜索是**显式提交**（右边那个 🔍），不是边打边查
                if let InputEvent::PressEnter { .. } = event {
                    this.commit_prefix_search(window, cx);
                }
            },
        )
        .detach();
        self.search_input = Some(editor);
    }

    /// 提交前缀搜索：把输入值当作列举前缀，压历史并重新列举。
    ///
    /// 走 `push_nav_history` 是为了 ⌘[ 能退回搜索前的位置——参照实现里搜索也是一次
    /// 位置跳转（地址栏会跟着显示新前缀）。
    pub(super) fn commit_prefix_search(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.search_input.as_ref() else {
            return;
        };
        let raw = editor.read(cx).value().to_string();
        let normalized = normalize_prefix_query(&raw);
        if normalized == self.current_prefix {
            cx.notify();
            return;
        }
        self.push_nav_history();
        self.current_prefix = normalized;
        self.reload_objects(cx);
    }

    pub(super) fn push_nav_history(&mut self) {
        self.nav_back.push(self.current_prefix.clone());
        self.nav_forward.clear();
    }

    /// ⌘[：回退到上一个位置（桶内前缀栈，栈空不动）。
    pub(super) fn handle_nav_back(&mut self, cx: &mut Context<Self>) {
        let Some(previous) = self.nav_back.pop() else {
            return;
        };
        self.nav_forward.push(self.current_prefix.clone());
        self.current_prefix = previous;
        self.search_input = None;
        self.reload_objects(cx);
    }

    /// ⌘]：前进（回退后又点过新位置则 forward 已清空，不动）。
    pub(super) fn handle_nav_forward(&mut self, cx: &mut Context<Self>) {
        let Some(next) = self.nav_forward.pop() else {
            return;
        };
        self.nav_back.push(self.current_prefix.clone());
        self.current_prefix = next;
        self.search_input = None;
        self.reload_objects(cx);
    }

    pub(super) fn open_bucket_root(&mut self, cx: &mut Context<Self>) {
        if self.current_prefix.is_none() {
            return;
        }
        self.search_input = None;
        self.push_nav_history();
        self.current_prefix = None;
        self.reload_objects(cx);
    }

    /// 返回上一级目录（压历史）；已在根目录则无操作。
    #[allow(dead_code)]
    pub(super) fn go_up(&mut self, cx: &mut Context<Self>) {
        let Some(prefix) = self.current_prefix.clone() else {
            return;
        };
        if let Some(parent) = parent_prefix(&prefix).map(str::to_string) {
            self.open_prefix(parent, cx);
        }
    }

    /// 「加载更多」：带 marker 请求下一页并追加（错误时保留已加载内容）。
    pub(super) fn load_more(&mut self, cx: &mut Context<Self>) {
        if self.loading_more || self.next_marker.is_none() {
            return;
        }
        self.request_objects(self.next_marker.clone(), cx);
    }

    /// 对象列表请求核心：携带代号发起后台加载；marker 非空表示翻页追加。
    pub(super) fn request_objects(&mut self, marker: Option<String>, cx: &mut Context<Self>) {
        let is_more = marker.is_some();
        if is_more {
            self.loading_more = true;
        }
        self.object_gen += 1;
        let generation = self.object_gen;

        let Some(account_id) = self.selected_account_id.clone() else {
            return;
        };
        let Some(bucket) = self.selected_bucket.clone() else {
            return;
        };
        let region = self
            .buckets
            .iter()
            .find(|b| b.name == bucket)
            .and_then(|b| b.region.clone());
        let prefix = self.current_prefix.clone();
        // 在 spawn 之前取出：异步块按值捕获，直接在里面读 self 会让借用逃逸
        let page_limit = self.page_limit;
        let services = Arc::clone(&self.services);

        cx.spawn(async move |this, cx| {
            let request = ListObjectsRequest {
                bucket,
                prefix,
                delimiter: Some("/".into()),
                marker,
                limit: page_limit,
                region,
            };
            let result = cx
                .background_executor()
                .spawn(async move { services.list_objects(&account_id, request) })
                .await;
            this.update(cx, |this, cx| {
                if this.object_gen != generation {
                    return; // 已切换桶/前缀，丢弃过期结果
                }
                this.loading_more = false;
                match result {
                    Ok(page) => {
                        let marker = if page.has_more() {
                            page.next_marker.clone()
                        } else {
                            None
                        };
                        // 目录占位对象（key 以 / 结尾）：与 CommonPrefix 语义重复、
                        // 无文件内容（下载/预览/URL 均 404），列表不展示
                        let entries: Vec<ListingEntry> = page
                            .entries
                            .into_iter()
                            .filter(|entry| match entry {
                                ListingEntry::Object(object) => !is_directory_object(&object.key),
                                ListingEntry::CommonPrefix(_) => true,
                            })
                            .collect();
                        if is_more {
                            this.entries.extend(entries);
                        } else {
                            this.entries = entries;
                        }
                        this.next_marker = marker;
                        this.objects_state = AsyncState::Idle;
                    }
                    Err(e) => {
                        // 整页失败清空；翻页失败保留已加载数据（见 render_objects）
                        if !is_more {
                            this.entries.clear();
                        }
                        this.objects_state = AsyncState::Failed(e.to_string());
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// ⌘R：有选中空间则重载当前前缀的对象列表；否则刷新空间/账号。
    pub(super) fn handle_refresh(
        &mut self,
        _: &Refresh,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_bucket.is_some() {
            self.reload_objects(cx);
        } else if self.selected_account_id.is_some() {
            self.buckets_state = AsyncState::Loading;
            cx.notify();
            self.start_bucket_load(cx);
        } else {
            self.load_accounts(cx);
        }
    }

    pub(super) fn handle_navigate_back(
        &mut self,
        _: &NavigateBack,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.palette.is_some() || self.add_modal.is_some() || self.settings_modal.is_some() {
            return;
        }
        self.handle_nav_back(cx);
    }

    pub(super) fn handle_navigate_forward(
        &mut self,
        _: &NavigateForward,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.palette.is_some() || self.add_modal.is_some() || self.settings_modal.is_some() {
            return;
        }
        self.handle_nav_forward(cx);
    }
}
