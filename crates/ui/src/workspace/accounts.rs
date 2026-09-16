//! 账号数据加载/选择，以及「添加账号」模态的接线与渲染。
//!
//! 异步数据加载全部经 `AppServices` 走后台执行器，窗口永不被 IO 阻塞。

use super::*;

impl WorkspaceView {
    /// 拉取账号列表（SQLite，快）。启动时与添加账号成功后调用。
    pub(super) fn load_accounts(&mut self, cx: &mut Context<Self>) {
        self.accounts_state = AsyncState::Loading;
        let services = Arc::clone(&self.services);
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { services.list_accounts() })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(accounts) => {
                        this.accounts = accounts;
                        this.accounts_state = AsyncState::Idle;
                    }
                    Err(e) => this.accounts_state = AsyncState::Failed(e.to_string()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// 选中账号并异步加载其空间列表。重复点击同一账号不重复请求（重试走 retry_buckets）。
    pub(super) fn select_account(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.selected_account_id.as_deref() == Some(id) {
            return;
        }
        self.selected_account_id = Some(id.to_string());
        self.buckets.clear();
        self.buckets_state = AsyncState::Loading;
        self.clear_bucket_selection();
        cx.notify();
        self.start_bucket_load(cx);
    }

    pub(super) fn handle_open_add_modal(
        &mut self,
        _: &AddAccount,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.add_modal.is_some() {
            return;
        }
        let services = Arc::clone(&self.services);
        let modal = cx.new(|cx| AddAccountModal::new(services, window, cx));
        cx.observe_in(&modal, window, Self::handle_add_modal_changed)
            .detach();
        modal.update(cx, |modal, cx| modal.focus_first(window, cx));
        self.add_modal = Some(modal);
        cx.notify();
    }

    /// 模态观察：置 done（保存成功 → 刷新账号列表）或 closed（取消）后丢弃实体。
    pub(super) fn handle_add_modal_changed(
        &mut self,
        modal: Entity<AddAccountModal>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (closed, done) = {
            let m = modal.read(cx);
            (m.closed(), m.done())
        };
        if !closed && !done {
            return; // saving/error 等常规通知，不处理
        }
        self.add_modal = None;
        window.focus(&self.focus_handle, cx);
        if done {
            self.load_accounts(cx);
        }
        cx.notify();
    }

    /// 「添加账号」模态遮罩：点击空白处请求关闭（保存中拒绝）；卡片内点击
    /// 已被卡片的 stop_propagation 挡住。
    pub(super) fn render_add_modal_overlay(
        &self,
        modal: &Entity<AddAccountModal>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mask = overlay::mask(theme)
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _window, cx| {
                    let Some(modal) = &this.add_modal else {
                        return;
                    };
                    if !modal.read(cx).saving() {
                        modal.update(cx, AddAccountModal::close);
                    }
                }),
            )
            .child(modal.clone());
        overlay::fade_in("add-account-overlay", mask)
    }
}
