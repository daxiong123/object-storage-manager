//! Transfer Engine：传输队列 + 状态机 + 事件驱动通知。
//!
//! 职责边界（agents.md §4/§5）：本 crate 只管「任务排队、并发调度、状态流转、
//! 事件广播」，**不知道任何云服务商与网络细节**——任务执行体由调用方以
//! [`TaskRunner`] 闭包注入（UI 层组装：闭包里调 AppServices → provider），
//! 运行时句柄由调用方提供（AppServices 的 tokio Runtime）。因此本 crate
//! 可用假 runner 做全状态机单测，不依赖 app/provider/gpui。
//!
//! 规范硬约束（spec §25 P0 / agents.md Transfer 行）：
//! - 系统睡眠 / 断网 → [`TransferEngine::suspend_all`] 把 Running 置为
//!   **Waiting**（绝不误标 Failed），唤醒/网络恢复 → [`TransferEngine::resume_all`]
//!   自动恢复。这两个入口由后续里程碑接 NSWorkspace willSleep/willWake 与
//!   NWPathMonitor，引擎只提供语义正确的 API 并保证可单测。
//! - 事件驱动，不轮询：状态每次变更 bump 一枚 watch 令牌，UI 订阅
//!   [`TransferEngine::subscribe`]，`changed().await` 后取快照刷新（gpui
//!   侧是常驻 async 任务，无定时器）。
//!
//! 取消/暂停语义：每个运行尝试的 wrapper 任务持 [`tokio::task::JoinHandle`]，
//! 暂停/挂起/取消即 `abort()`——future 在下一个 await 点被丢弃，reqwest 连接
//! 随之 teardown（下载从零重启：`File::create` 截断旧残留）。完成回调带
//! attempt 代号，过期回调（abort 与重跑竞态）直接丢弃，防旧结果覆盖新状态。

use std::collections::VecDeque;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::sync::watch;
use tokio::task::JoinHandle;

/// 传输任务唯一标识（引擎内自增，进程内唯一）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TransferId(u64);

/// 任务状态机。
///
/// 状态转移：
/// ```text
/// Queued → Running → Completed | Failed
///   ↑          │(abort)
///   └─ Paused / Waiting ←─ 用户暂停 / 系统挂起
/// 任意活动态 → Cancelled（用户取消）
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferState {
    /// 排队中（等并发槽位或引擎挂起）
    Queued,
    /// 传输中
    Running,
    /// 因系统睡眠/断网被引擎挂起（P0：恢复后自动继续，绝不是 Failed）
    Waiting,
    /// 用户手动暂停
    Paused,
    /// 已完成
    Completed,
    /// 已失败（`error` 字段为人话原因）
    Failed,
    /// 用户取消
    Cancelled,
}

impl TransferState {
    /// 中文展示标签（UI 直接用，不在此处染样式）。
    pub fn label(self) -> &'static str {
        match self {
            TransferState::Queued => "排队中",
            TransferState::Running => "传输中",
            TransferState::Waiting => "等待恢复",
            TransferState::Paused => "已暂停",
            TransferState::Completed => "已完成",
            TransferState::Failed => "失败",
            TransferState::Cancelled => "已取消",
        }
    }

    /// 是否处于可中止的活动状态（非终态）。
    pub fn is_active(self) -> bool {
        matches!(
            self,
            TransferState::Queued
                | TransferState::Running
                | TransferState::Waiting
                | TransferState::Paused
        )
    }

    /// 是否为终态（Completed/Failed/Cancelled）。
    pub fn is_finished(self) -> bool {
        !self.is_active()
    }
}

/// 任务类型。上传将在后续里程碑加入；下载携带云端三元组与本地目的地。
#[derive(Debug, Clone)]
pub enum TransferKind {
    /// 下载远端对象到本地文件（`dest` 为本地 `PathBuf`，`key` 为云端 `String`）
    Download {
        account_id: String,
        bucket: String,
        key: String,
        dest: PathBuf,
    },
}

/// 引擎调用 runner 时提取的执行参数（与 TransferKind::Download 一一对应）。
#[derive(Debug, Clone)]
pub struct TransferRequest {
    pub account_id: String,
    pub bucket: String,
    pub key: String,
    pub dest: PathBuf,
}

/// 任务执行体：引擎把 future spawn 到调用方提供的 tokio 运行时上。
/// 返回 `Ok(写入字节数)` / `Err(人话错误信息)`。
///
/// 暂停/挂起/取消 = 引擎 abort 该 future（下一个 await 点丢弃），
/// 不需要 runner 自觉配合取消。
pub type TaskRunner = Arc<
    dyn Fn(TransferRequest) -> Pin<Box<dyn Future<Output = Result<u64, String>> + Send>>
        + Send
        + Sync,
>;

/// 传输任务（对外只读快照）。
#[derive(Debug, Clone)]
pub struct TransferTask {
    pub id: TransferId,
    pub kind: TransferKind,
    /// UI 展示名（调用方入队时给定，如对象名末段）
    pub display_name: String,
    pub state: TransferState,
    /// 失败原因（仅 Failed 非空；人话文本，可直接展示）
    pub error: Option<String>,
    pub enqueued_at_millis: u64,
    pub finished_at_millis: Option<u64>,
}

struct TaskSlot {
    task: TransferTask,
    /// 当前运行尝试的 wrapper 任务句柄（仅 Running 持有；abort 即中止）
    handle: Option<JoinHandle<()>>,
    /// 当前运行尝试代号：完成回调据此丢弃过期结果（防旧 abort 竞态覆盖新状态）
    attempt: u64,
}

struct EngineState {
    tasks: Vec<TaskSlot>,
    /// FIFO 待启动队列（存 id；pump 时校验状态仍为 Queued）
    order: VecDeque<TransferId>,
    next_id: u64,
    next_attempt: u64,
    /// 引擎级挂起（系统睡眠/断网）：true 时不启动任何新任务
    suspended: bool,
}

impl EngineState {
    fn slot_mut(&mut self, id: TransferId) -> Option<&mut TaskSlot> {
        self.tasks.iter_mut().find(|slot| slot.task.id == id)
    }

    fn running_count(&self) -> usize {
        self.tasks
            .iter()
            .filter(|slot| slot.task.state == TransferState::Running)
            .count()
    }
}

struct EngineInner {
    max_parallel: usize,
    runner: TaskRunner,
    handle: tokio::runtime::Handle,
    state: Mutex<EngineState>,
    /// 状态变更令牌：每次 mutation bump 一枚；UI changed().await 后取快照
    changes: watch::Sender<()>,
}

impl EngineInner {
    fn lock(&self) -> MutexGuard<'_, EngineState> {
        // 锁毒化 = 持锁线程 panic：大声响应报，不静默（项目纪律）
        self.state
            .lock()
            .unwrap_or_else(|poisoned| panic!("TransferEngine 状态锁已毒化: {poisoned}"))
    }

    /// 完成回调（wrapper 任务在 fut 结束后调用）。
    /// 过期尝试（attempt 不符）与已移除任务的结果直接丢弃。
    fn complete(self: &Arc<Self>, id: TransferId, attempt: u64, result: Result<u64, String>) {
        {
            let mut st = self.lock();
            let Some(slot) = st.slot_mut(id) else {
                return; // 任务已被移除
            };
            if slot.attempt != attempt {
                return; // 过期回调（此轮已被 abort 并可能已重跑）
            }
            slot.handle = None;
            let now = now_millis();
            match result {
                Ok(_) => {
                    slot.task.state = TransferState::Completed;
                    slot.task.error = None;
                    slot.task.finished_at_millis = Some(now);
                }
                Err(message) => {
                    slot.task.state = TransferState::Failed;
                    slot.task.error = Some(message);
                    slot.task.finished_at_millis = Some(now);
                }
            }
        }
        self.pump_and_notify();
    }

    /// 尽量启动 Queued 任务填满并发槽位；未挂起才会真正启动。
    fn pump(self: &Arc<Self>) {
        let mut st = self.lock();
        if st.suspended {
            return; // 挂起期间不启动任何任务（Queued 原地等待恢复）
        }
        while st.running_count() < self.max_parallel {
            let Some(id) = st.order.pop_front() else {
                break;
            };
            // 先用不可变借用校验状态，避免与 next_attempt 的可变借用冲突
            let is_queued = st
                .tasks
                .iter()
                .find(|slot| slot.task.id == id)
                .is_some_and(|slot| slot.task.state == TransferState::Queued);
            if !is_queued {
                continue; // 已被暂停/取消/移除：跳过，继续下一个
            }
            let kind = st
                .tasks
                .iter()
                .find(|slot| slot.task.id == id)
                .map(|slot| slot.task.kind.clone())
                .expect("上方刚确认任务存在");
            let TransferKind::Download {
                account_id,
                bucket,
                key,
                dest,
            } = &kind; // 单变体模式必然匹配；新增 Upload 变体时此处编译报错即提醒处理
            let request = TransferRequest {
                account_id: account_id.clone(),
                bucket: bucket.clone(),
                key: key.clone(),
                dest: dest.clone(),
            };
            st.next_attempt += 1;
            let attempt = st.next_attempt;
            let slot = st.slot_mut(id).expect("上方刚确认任务存在");
            slot.attempt = attempt;
            slot.task.state = TransferState::Running;
            let future = (self.runner)(request);
            let engine = Arc::clone(self);
            // wrapper：fut 结束后回引擎登记结果；引擎 abort wrapper 即中止 fut
            let join = self.handle.spawn(async move {
                let result = future.await;
                engine.complete(id, attempt, result);
            });
            slot.handle = Some(join);
        }
    }

    /// pump + 广播变更令牌（每次状态突变统一出口）。
    fn pump_and_notify(self: &Arc<Self>) {
        self.pump();
        let _ = self.changes.send(());
    }
}

/// 传输引擎。克隆 [`Arc<TransferEngine>`] 或直接共享 `&self` 即可跨任务使用。
#[derive(Clone)]
pub struct TransferEngine {
    inner: Arc<EngineInner>,
}

impl TransferEngine {
    /// 创建引擎。`handle` 为任务执行运行时（AppServices 的 tokio Runtime），
    /// `runner` 为任务执行体，`max_parallel` 为并发上限（0 会被钳到 1）。
    pub fn new(handle: tokio::runtime::Handle, runner: TaskRunner, max_parallel: usize) -> Self {
        let (changes, _) = watch::channel(());
        Self {
            inner: Arc::new(EngineInner {
                max_parallel: max_parallel.max(1),
                runner,
                handle,
                state: Mutex::new(EngineState {
                    tasks: Vec::new(),
                    order: VecDeque::new(),
                    next_id: 0,
                    next_attempt: 0,
                    suspended: false,
                }),
                changes,
            }),
        }
    }

    /// 入队一个下载任务，返回任务 id。立即按并发余量启动。
    pub fn enqueue_download(
        &self,
        account_id: impl Into<String>,
        bucket: impl Into<String>,
        key: impl Into<String>,
        dest: PathBuf,
        display_name: impl Into<String>,
    ) -> TransferId {
        let mut st = self.inner.lock();
        st.next_id += 1;
        let id = TransferId(st.next_id);
        st.tasks.push(TaskSlot {
            task: TransferTask {
                id,
                kind: TransferKind::Download {
                    account_id: account_id.into(),
                    bucket: bucket.into(),
                    key: key.into(),
                    dest,
                },
                display_name: display_name.into(),
                state: TransferState::Queued,
                error: None,
                enqueued_at_millis: now_millis(),
                finished_at_millis: None,
            },
            handle: None,
            attempt: 0,
        });
        st.order.push_back(id);
        drop(st);
        self.inner.pump_and_notify();
        id
    }

    /// 用户暂停：Running 先 abort（下一个 await 点断开），再置 Paused；
    /// Queued 直接置 Paused（pump 跳过非 Queued）。其余状态无操作。
    pub fn pause(&self, id: TransferId) -> bool {
        let handle = {
            let mut st = self.inner.lock();
            let Some(slot) = st.slot_mut(id) else {
                return false;
            };
            match slot.task.state {
                TransferState::Queued => {
                    slot.task.state = TransferState::Paused;
                    None
                }
                TransferState::Running => {
                    slot.task.state = TransferState::Paused;
                    slot.handle.take()
                }
                _ => return false,
            }
        };
        if let Some(handle) = handle {
            handle.abort(); // 在锁外 abort，避免与 complete 回调同锁竞争窗口拉长
        }
        self.inner.pump_and_notify();
        true
    }

    /// 重新入队未完成任务（Paused/Waiting/Failed/Cancelled → Queued）
    /// 并尝试启动。Completed 是终态不可复用；失败重试也走这里。
    pub fn resume(&self, id: TransferId) -> bool {
        let mut st = self.inner.lock();
        let Some(slot) = st.slot_mut(id) else {
            return false;
        };
        match slot.task.state {
            TransferState::Paused
            | TransferState::Waiting
            | TransferState::Failed
            | TransferState::Cancelled => {
                slot.task.state = TransferState::Queued;
                slot.task.error = None;
                st.order.push_back(id);
                drop(st);
                self.inner.pump_and_notify();
                true
            }
            _ => false,
        }
    }

    /// 取消任务（活动任务先 abort；终态任务无操作）。
    pub fn cancel(&self, id: TransferId) -> bool {
        let handle = {
            let mut st = self.inner.lock();
            let Some(slot) = st.slot_mut(id) else {
                return false;
            };
            if !slot.task.state.is_active() {
                return false;
            }
            slot.task.state = TransferState::Cancelled;
            slot.task.finished_at_millis = Some(now_millis());
            slot.handle.take()
        };
        if let Some(handle) = handle {
            handle.abort();
        }
        self.inner.pump_and_notify();
        true
    }

    /// 从列表移除任务（活动任务先 abort）。
    pub fn remove(&self, id: TransferId) -> bool {
        let handle = {
            let mut st = self.inner.lock();
            let Some(index) = st.tasks.iter().position(|slot| slot.task.id == id) else {
                return false;
            };
            let mut slot = st.tasks.swap_remove(index);
            let handle = slot.handle.take();
            slot.task.state = TransferState::Cancelled;
            drop(slot);
            handle
        };
        if let Some(handle) = handle {
            handle.abort();
        }
        self.inner.pump_and_notify();
        true
    }

    /// 清除终态任务，返回清除数量。
    pub fn clear_finished(&self) -> usize {
        let mut st = self.inner.lock();
        let before = st.tasks.len();
        st.tasks.retain(|slot| slot.task.state.is_active());
        let removed = before - st.tasks.len();
        drop(st);
        if removed > 0 {
            self.inner.pump_and_notify();
        }
        removed
    }

    /// **P0 入口**：系统睡眠 / 网络断开时调用。
    /// 所有 Running 任务被 abort 并置为 **Waiting**（绝不误标 Failed）；
    /// Queued 任务保持排队，挂起期间不启动。唤醒/网络恢复调用
    /// [`Self::resume_all`] 自动继续。
    pub fn suspend_all(&self) {
        let handles: Vec<JoinHandle<()>> = {
            let mut st = self.inner.lock();
            st.suspended = true;
            st.tasks
                .iter_mut()
                .filter_map(|slot| {
                    if slot.task.state == TransferState::Running {
                        slot.task.state = TransferState::Waiting;
                        slot.handle.take()
                    } else {
                        None
                    }
                })
                .collect()
        };
        for handle in handles {
            handle.abort();
        }
        self.inner.pump_and_notify();
    }

    /// **P0 入口**：系统唤醒 / 网络恢复时调用。
    /// 解除挂起，所有 Waiting 任务回到 Queued 并按并发上限继续。
    pub fn resume_all(&self) {
        {
            let mut st = self.inner.lock();
            st.suspended = false;
            let waiting: Vec<TransferId> = st
                .tasks
                .iter()
                .filter(|slot| slot.task.state == TransferState::Waiting)
                .map(|slot| slot.task.id)
                .collect();
            for id in waiting {
                if let Some(slot) = st.slot_mut(id) {
                    slot.task.state = TransferState::Queued;
                    slot.task.error = None;
                    st.order.push_back(id);
                }
            }
        }
        self.inner.pump_and_notify();
    }

    /// 当前全部任务快照（入队顺序）。
    pub fn snapshot(&self) -> Vec<TransferTask> {
        self.inner
            .lock()
            .tasks
            .iter()
            .map(|slot| slot.task.clone())
            .collect()
    }

    /// 订阅状态变更令牌。每次任务状态突变 `changed()` 返回一次；
    /// UI 据此取 [`Self::snapshot`] 刷新（事件驱动，不轮询）。
    pub fn subscribe(&self) -> watch::Receiver<()> {
        self.inner.changes.subscribe()
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// 等待快照满足条件；超时即 panic（测试兜底，生产无轮询）。
    /// 必须用 await 让出 worker：#[tokio::test] 的用例跑在运行时上，
    /// 同步 sleep 会阻凋调度。
    async fn wait_for(
        engine: &TransferEngine,
        timeout_ms: u64,
        pred: impl Fn(&[TransferTask]) -> bool,
    ) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            if pred(&engine.snapshot()) {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "等待状态超时：{:?}",
                engine.snapshot()
            );
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }

    /// 立即成功的 runner（记录每次执行的 key）。
    fn instant_runner(log: Arc<Mutex<Vec<String>>>) -> TaskRunner {
        Arc::new(move |req: TransferRequest| {
            log.lock().unwrap().push(req.key);
            Box::pin(async move { Ok(42) })
        })
    }

    /// 带闸门的 runner：第 `block_on_call` 次（1 起）执行时挂在 Notify 上，
    /// 其余次数立即成功。返回控制句柄用于放行。
    fn gated_runner(
        log: Arc<Mutex<Vec<String>>>,
        block_on_call: usize,
    ) -> (TaskRunner, Arc<tokio::sync::Notify>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let gate = Arc::new(tokio::sync::Notify::new());
        let gate_for_runner = Arc::clone(&gate);
        let runner: TaskRunner = Arc::new(move |req: TransferRequest| {
            log.lock().unwrap().push(req.key);
            let call = calls.fetch_add(1, Ordering::SeqCst) + 1;
            if call == block_on_call {
                let gate = Arc::clone(&gate_for_runner);
                Box::pin(async move {
                    gate.notified().await;
                    Ok(7)
                })
            } else {
                Box::pin(async move { Ok(7) })
            }
        });
        (runner, gate)
    }

    fn download_key(kind: &TransferKind) -> &str {
        match kind {
            TransferKind::Download { key, .. } => key,
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn enqueue_runs_to_completion_and_reports_bytes() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let engine = TransferEngine::new(
            tokio::runtime::Handle::current(),
            instant_runner(Arc::clone(&log)),
            2,
        );
        let mut rx = engine.subscribe();
        let id1 =
            engine.enqueue_download("a1", "bkt", "k1.txt", PathBuf::from("/tmp/x1"), "k1.txt");
        let id2 =
            engine.enqueue_download("a1", "bkt", "k2.txt", PathBuf::from("/tmp/x2"), "k2.txt");
        wait_for(&engine, 2000, |tasks| {
            tasks.len() == 2 && tasks.iter().all(|t| t.state == TransferState::Completed)
        })
        .await;
        assert_eq!(log.lock().unwrap().len(), 2);
        let tasks = engine.snapshot();
        assert!(tasks.iter().all(|t| t.finished_at_millis.is_some()));
        assert_eq!(tasks[0].id, id1);
        assert_eq!(tasks[1].id, id2);
        rx.changed().await.expect("变更令牌应仍在");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrency_limit_holds_queue() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let (runner, gate) = gated_runner(Arc::clone(&log), 1);
        let engine = TransferEngine::new(tokio::runtime::Handle::current(), runner, 1);
        let id1 = engine.enqueue_download("a1", "bkt", "k1", PathBuf::from("/tmp/a"), "k1");
        let id2 = engine.enqueue_download("a1", "bkt", "k2", PathBuf::from("/tmp/b"), "k2");
        wait_for(&engine, 2000, |tasks| {
            tasks.iter().any(|t| t.state == TransferState::Running)
        })
        .await;
        // 第一个挂在闸门上，第二个必须保持 Queued
        let tasks = engine.snapshot();
        let t2 = tasks.iter().find(|t| t.id == id2).unwrap();
        assert_eq!(t2.state, TransferState::Queued);
        assert_eq!(log.lock().unwrap().len(), 1);
        // 放行 → 两个都完成
        gate.notify_one();
        wait_for(&engine, 2000, |tasks| {
            tasks.len() == 2 && tasks.iter().all(|t| t.state == TransferState::Completed)
        })
        .await;
        assert_eq!(log.lock().unwrap().len(), 2);
        assert_ne!(id1, id2);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pause_running_then_resume_completes() {
        let log = Arc::new(Mutex::new(Vec::new()));
        // 第 1 次调用挂起（被 abort 也不会返回），第 2 次（resume 后重跑）直接成功
        let (runner, _gate) = gated_runner(Arc::clone(&log), 1);
        let engine = TransferEngine::new(tokio::runtime::Handle::current(), runner, 1);
        let id = engine.enqueue_download("a1", "bkt", "k1", PathBuf::from("/tmp/a"), "k1");
        wait_for(&engine, 2000, |tasks| {
            tasks
                .first()
                .is_some_and(|t| t.state == TransferState::Running)
        })
        .await;
        assert!(engine.pause(id));
        assert_eq!(engine.snapshot()[0].state, TransferState::Paused);
        assert!(engine.resume(id));
        wait_for(&engine, 2000, |tasks| {
            tasks
                .first()
                .is_some_and(|t| t.state == TransferState::Completed)
        })
        .await;
        // 重跑 = runner 被调用了两次
        assert_eq!(log.lock().unwrap().len(), 2);
    }

    /// P0：挂起后必须是 Waiting，绝不允许出现 Failed。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn suspend_marks_waiting_never_failed_and_resume_all_recovers() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let (runner, gate) = gated_runner(Arc::clone(&log), 1);
        let engine = TransferEngine::new(tokio::runtime::Handle::current(), runner, 2);
        let id1 = engine.enqueue_download("a1", "bkt", "k1", PathBuf::from("/tmp/a"), "k1");
        let id2 = engine.enqueue_download("a1", "bkt", "k2", PathBuf::from("/tmp/b"), "k2");
        wait_for(&engine, 2000, |tasks| {
            tasks.iter().any(|t| t.state == TransferState::Running)
        })
        .await;
        engine.suspend_all();
        let tasks = engine.snapshot();
        // 挂起后：Running→Waiting，Queued 保持排队；没有任何 Failed
        assert!(tasks.iter().all(|t| t.state != TransferState::Failed));
        assert!(
            tasks
                .iter()
                .any(|t| t.state == TransferState::Waiting || t.state == TransferState::Queued)
        );
        assert_eq!(
            tasks.iter().find(|t| t.id == id1).map(|t| t.state).unwrap(),
            TransferState::Waiting
        );
        // 挂起期间新入队也不启动：runner 调用次数不增长（挂起前 k1/k2 可能
        // 都已开跑，基线取实测值）
        engine.enqueue_download("a1", "bkt", "k3", PathBuf::from("/tmp/c"), "k3");
        let baseline = log.lock().unwrap().len();
        assert!(baseline >= 1, "挂起前至少 k1 已开跑");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(log.lock().unwrap().len(), baseline);
        // 恢复：全部继续并完成（k1 重跑 + k3 首跑，共 +2）
        engine.resume_all();
        gate.notify_one();
        wait_for(&engine, 2000, |tasks| {
            tasks.len() == 3 && tasks.iter().all(|t| t.state == TransferState::Completed)
        })
        .await;
        // 恢复后确实重跑了任务：终量在 (baseline, baseline+3] 之间
        //（k1/k2 挂起时若仍在 Running 则重跑一次，k3 首跑；竞态下不超上界）
        let final_len = log.lock().unwrap().len();
        assert!(final_len > baseline, "恢复后应有任务重跑");
        assert!(
            final_len <= baseline + 3,
            "重跑次数不得超界：{final_len} vs {baseline}"
        );
        assert!(
            log.lock().unwrap().contains(&"k3".to_string()),
            "挂起后入队的 k3 只能在恢复后开跑"
        );
        let _ = id2;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn failure_records_error_and_retry_requeues() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_for_runner = Arc::clone(&calls);
        let runner: TaskRunner = Arc::new(move |req: TransferRequest| {
            log.lock().unwrap().push(req.key);
            let call = calls_for_runner.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                if call == 0 {
                    Err("网络中断：连接被重置".to_string())
                } else {
                    Ok(9)
                }
            })
        });
        let engine = TransferEngine::new(tokio::runtime::Handle::current(), runner, 1);
        let id = engine.enqueue_download("a1", "bkt", "k1", PathBuf::from("/tmp/a"), "k1");
        wait_for(&engine, 2000, |tasks| {
            tasks
                .first()
                .is_some_and(|t| t.state == TransferState::Failed)
        })
        .await;
        let task = &engine.snapshot()[0];
        assert_eq!(task.error.as_deref(), Some("网络中断：连接被重置"));
        assert!(task.finished_at_millis.is_some());
        // 失败任务允许重试：回 Queued 重跑成功
        assert!(engine.resume(id));
        wait_for(&engine, 2000, |tasks| {
            tasks
                .first()
                .is_some_and(|t| t.state == TransferState::Completed)
        })
        .await;
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancel_active_and_queued_without_running_them() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let (runner, gate) = gated_runner(Arc::clone(&log), 1);
        let engine = TransferEngine::new(tokio::runtime::Handle::current(), runner, 1);
        let id1 = engine.enqueue_download("a1", "bkt", "k1", PathBuf::from("/tmp/a"), "k1");
        let id2 = engine.enqueue_download("a1", "bkt", "k2", PathBuf::from("/tmp/b"), "k2");
        wait_for(&engine, 2000, |tasks| {
            tasks.iter().any(|t| t.state == TransferState::Running)
        })
        .await;
        // 取消排队中的任务：runner 不应被调用
        assert!(engine.cancel(id2));
        // 取消运行中的任务
        assert!(engine.cancel(id1));
        wait_for(&engine, 2000, |tasks| {
            tasks.iter().all(|t| t.state == TransferState::Cancelled)
        })
        .await;
        assert_eq!(log.lock().unwrap().len(), 1); // 只有 k1 跑过
        // 终态任务再取消/暂停：无操作
        assert!(!engine.cancel(id1));
        assert!(!engine.pause(id1));
        assert_eq!(
            engine.snapshot()[0].state,
            TransferState::Cancelled,
            "终态不得被改写"
        );
        gate.notify_one();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn remove_and_clear_finished() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let engine = TransferEngine::new(
            tokio::runtime::Handle::current(),
            instant_runner(Arc::clone(&log)),
            1,
        );
        let id1 = engine.enqueue_download("a1", "bkt", "k1", PathBuf::from("/tmp/a"), "k1");
        engine.enqueue_download("a1", "bkt", "k2", PathBuf::from("/tmp/b"), "k2");
        wait_for(&engine, 2000, |tasks| {
            tasks.len() == 2 && tasks.iter().all(|t| t.state == TransferState::Completed)
        })
        .await;
        assert_eq!(engine.clear_finished(), 2);
        assert!(engine.snapshot().is_empty());
        // 移除不存在的任务：false
        assert!(!engine.remove(id1));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pause_while_queued_never_starts() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let (runner, gate) = gated_runner(Arc::clone(&log), 1);
        let engine = TransferEngine::new(tokio::runtime::Handle::current(), runner, 1);
        let _id1 = engine.enqueue_download("a1", "bkt", "k1", PathBuf::from("/tmp/a"), "k1");
        let id2 = engine.enqueue_download("a1", "bkt", "k2", PathBuf::from("/tmp/b"), "k2");
        wait_for(&engine, 2000, |tasks| {
            tasks.iter().any(|t| t.state == TransferState::Running)
        })
        .await;
        assert!(engine.pause(id2));
        gate.notify_one(); // 第一个完成，腾出槽位
        wait_for(&engine, 2000, |tasks| {
            tasks
                .first()
                .is_some_and(|t| t.state == TransferState::Completed)
        })
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        // 排队中被暂停的任务不被 pump 启动
        let task = &engine.snapshot()[1];
        assert_eq!(task.state, TransferState::Paused);
        assert!(log.lock().unwrap().contains(&"k1".to_string()));
        assert!(!log.lock().unwrap().contains(&"k2".to_string()));
    }

    #[test]
    fn state_labels_are_chinese_and_transition_helpers_agree() {
        assert_eq!(TransferState::Queued.label(), "排队中");
        assert_eq!(TransferState::Waiting.label(), "等待恢复");
        assert_eq!(TransferState::Failed.label(), "失败");
        for state in [
            TransferState::Queued,
            TransferState::Running,
            TransferState::Waiting,
            TransferState::Paused,
        ] {
            assert!(state.is_active());
            assert!(!state.is_finished());
        }
        for state in [
            TransferState::Completed,
            TransferState::Failed,
            TransferState::Cancelled,
        ] {
            assert!(!state.is_active());
            assert!(state.is_finished());
        }
    }

    #[test]
    fn download_kind_key_extraction() {
        let kind = TransferKind::Download {
            account_id: "a".into(),
            bucket: "b".into(),
            key: "photos/cat.jpg".into(),
            dest: PathBuf::from("/tmp/cat.jpg"),
        };
        assert_eq!(download_key(&kind), "photos/cat.jpg");
    }
}
