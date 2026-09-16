//! 空间（Bucket）加载/选择/手填兜底/重试，以及命令面板的动态桶跳转命令。

use super::*;

impl WorkspaceView {
    /// 空间列表加载失败后的重试入口（保持当前选中账号重新请求）。
    pub(super) fn add_manual_bucket(&mut self, cx: &mut Context<Self>) {
        let Some(input) = self.manual_bucket_input.clone() else {
            return;
        };
        let name = input.read(cx).value().trim().to_string();
        if name.is_empty() {
            return;
        }
        let kind = self
            .accounts
            .iter()
            .find(|account| Some(account.id.as_str()) == self.selected_account_id.as_deref())
            .map(|account| account.provider)
            .unwrap_or(object_storage_domain::ProviderKind::Aliyun);
        if !self.buckets.iter().any(|bucket| bucket.name == name) {
            self.buckets.push(Bucket {
                name: name.clone(),
                kind,
                region: None,
            });
        }
        self.buckets_state = AsyncState::Idle;
        self.selected_bucket = Some(name);
        self.selected_object_key = None;
        self.reload_objects(cx);
        cx.notify();
    }

    pub(super) fn retry_buckets(&mut self, cx: &mut Context<Self>) {
        if self.selected_account_id.is_none() || self.buckets_state == AsyncState::Loading {
            return;
        }
        self.buckets_state = AsyncState::Loading;
        cx.notify();
        self.start_bucket_load(cx);
    }

    pub(super) fn start_bucket_load(&mut self, cx: &mut Context<Self>) {
        self.bucket_gen += 1;
        let generation = self.bucket_gen;
        let Some(account_id) = self.selected_account_id.clone() else {
            return;
        };
        let services = Arc::clone(&self.services);
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { services.list_buckets(&account_id) })
                .await;
            this.update(cx, |this, cx| {
                if this.bucket_gen != generation {
                    return; // 已切到别的账号，丢弃过期结果
                }
                match result {
                    Ok(buckets) => {
                        this.buckets = buckets;
                        this.buckets_state = AsyncState::Idle;
                    }
                    Err(e) => this.buckets_state = AsyncState::Failed(e.to_string()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// 选中桶并从根目录开始加载对象。重复点击同一桶不重复请求。
    pub(super) fn select_bucket(&mut self, name: &str, cx: &mut Context<Self>) {
        if self.selected_bucket.as_deref() == Some(name) {
            return;
        }
        self.selected_bucket = Some(name.to_string());
        self.current_prefix = None;
        // 跳桶 = 重上下文切换：清导航历史（历史只在桶内有意义）、关闭过滤条
        // （⌘R 刷新/翻页保留过滤词——只关这里，不动 reload_objects）
        self.nav_back.clear();
        self.nav_forward.clear();
        self.object_filter = None;
        self.filtered_ix = None;
        self.reload_objects(cx);
    }

    /// 清空对象区（切换账号/桶时）。
    pub(super) fn clear_bucket_selection(&mut self) {
        self.entries.clear();
        self.objects_state = AsyncState::Idle;
        self.loading_more = false;
        self.next_marker = None;
        self.current_prefix = None;
        self.nav_back.clear();
        self.nav_forward.clear();
        self.clear_object_selection();
        // 过滤命中缓存基于 entries，数据已换直接作废缓存与过滤条
        // （跳桶是重上下文切换，Finder 同样不保留过滤）。
        self.object_filter = None;
        self.filtered_ix = None;
        self.download_message = None;
    }

    pub(super) fn selected_provider_kind(&self) -> Option<ProviderKind> {
        let account_id = self.selected_account_id.as_ref()?;
        self.accounts
            .iter()
            .find(|account| account.id == *account_id)
            .map(|account| account.provider)
    }

    /// 「跳转到 Bucket」动态命令：当前账号下的每个空间一条，点击即选中
    /// （触发对象列表加载）。命令面板每次打开都重建，此处数据天然最新。
    ///
    /// 为什么不走 Action 派发：`execute_selected` 先 close 面板（焦点立即
    /// 归还 Workspace 根），Handler 里的 `dispatch_action` 是 **deferred**——
    /// 捕获的是派发调用时刻的焦点（已关闭的面板输入框），下一帧按该焦点
    /// 找 dispatch tree 节点落空，Action 静默丢失（E2 验收问题根因）。
    /// 因此直接经 WeakEntity 调用 WorkspaceView 方法，绕开焦点链。
    pub(super) fn bucket_jump_commands(&self, cx: &Context<Self>) -> Vec<PaletteCommand> {
        let weak = cx.weak_entity();
        self.buckets
            .iter()
            .map(|bucket| {
                let name = bucket.name.clone();
                let weak = weak.clone();
                PaletteCommand::handler(
                    format!("跳转：{name}"),
                    move |_window: &mut gpui::Window, cx: &mut gpui::App| {
                        let _ = weak.update(cx, |this, cx| {
                            this.select_bucket(&name, cx);
                        });
                    },
                )
            })
            .collect()
    }

    /// 命令面板「跳转：Bucket」入口（等价于侧栏点击：换桶 + 清对象区）。
    pub(super) fn handle_select_bucket_by_name(
        &mut self,
        action: &SelectBucketByName,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_bucket(&action.0, cx);
    }
}
