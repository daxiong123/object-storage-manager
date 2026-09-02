//! Provider abstraction：对象存储 Provider trait 与公共错误类型
//!
//! 抽象原则（agents.md §3 / spec §63）：为真实需求抽象——七牛与阿里云两个真实
//! Provider 需要统一接口，而不是为不存在的平台做抽象。因此本 crate 只有：
//! - `StorageProvider` trait（async，按服务商枚举分发，不需要 dyn）
//! - `StorageError` 统一错误
//!
//! trait 方法返回 `impl Future + Send`：调用方可以把 Future 交给 tokio 或
//! gpui 后台执行器 spawn（这正是 clippy `async_fn_in_trait` 警告的真实关切；
//! 不用裸 `async fn` 是为了让 Send 义务显式化，避免将来破坏 API）。

use object_storage_domain::{Bucket, ListObjectsRequest, ObjectPage, ProviderKind};
use thiserror::Error;

use std::future::Future;
use std::sync::Arc;

/// 字节进度回调：`(已完成, 总大小)`。总大小在上传时已知；下载可能没有 Content-Length。
/// 调用方（传输引擎）负责节流通知 UI，本层每块都可回调。
pub type ByteProgress = Arc<dyn Fn(u64, Option<u64>) + Send + Sync>;

/// 存储层统一错误
///
/// Provider 实现负责把自己的传输层错误（reqwest 等）映射到这里的语义分类，
/// 上层 UI/Transfer 只依赖本枚举。
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("网络错误: {0}")]
    Network(String),

    #[error("认证失败（AccessKey/SecretKey 错误或无权限）: {0}")]
    Auth(String),

    #[error("请求被限流: {0}")]
    RateLimited(String),

    #[error("API 错误 (HTTP {status}): {message}")]
    Api { status: u16, message: String },

    #[error("响应解析失败: {0}")]
    InvalidResponse(String),

    #[error("无效输入: {0}")]
    InvalidInput(String),

    /// 本地文件系统 IO 错误（下载落盘/上传读盘；非远端 API 错误）
    #[error("本地 IO 错误: {0}")]
    Io(String),
}

/// 对象存储 Provider 统一接口
///
/// 实现要求：
/// - 可跨线程共享（`Send + Sync`），内部不可变，用 `reqwest::Client` 复用连接
/// - 所有 IO 走 async；分页由调用方通过 `ObjectPage::next_marker` 驱动
pub trait StorageProvider: Send + Sync {
    /// 服务商类型
    fn kind(&self) -> ProviderKind;

    /// 列举账号下所有 Bucket
    fn list_buckets(&self) -> impl Future<Output = Result<Vec<Bucket>, StorageError>> + Send;

    /// 列举 Bucket 内对象（单页）
    ///
    /// 翻页：把返回页的 `next_marker` 填进下一次请求的 `marker`，
    /// 直到 `has_more()` 为 false。
    fn list_objects(
        &self,
        request: ListObjectsRequest,
    ) -> impl Future<Output = Result<ObjectPage, StorageError>> + Send;

    /// 流式下载对象到本地文件，返回写入的字节数。
    ///
    /// 内存红线（agents.md §2）：分块写盘，**绝不把整个对象读进内存**。
    /// 实现方按需覆盖本地文件（`dest` 已存在时截断重写）。
    fn download_object_to_file(
        &self,
        bucket: &str,
        key: &str,
        dest: &std::path::Path,
        progress: Option<ByteProgress>,
    ) -> impl Future<Output = Result<u64, StorageError>> + Send;

    /// 流式上传本地文件到对象，返回上传的字节数。
    ///
    /// 内存红线：分块读盘，**绝不把整个文件读进 `Vec<u8>`**。
    fn upload_object_from_file(
        &self,
        bucket: &str,
        key: &str,
        source: &std::path::Path,
        progress: Option<ByteProgress>,
    ) -> impl Future<Output = Result<u64, StorageError>> + Send;

    /// 删除远端对象。成功即对象已不存在；对象本就不存在时由实现方报错（不静默）。
    fn delete_object(
        &self,
        bucket: &str,
        key: &str,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_error_displays_chinese() {
        let e = StorageError::Api {
            status: 631,
            message: "no such bucket".into(),
        };
        assert_eq!(e.to_string(), "API 错误 (HTTP 631): no such bucket");
    }
}
