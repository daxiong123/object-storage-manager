//! Provider abstraction：对象存储 Provider trait 与公共错误类型
//!
//! 抽象原则（agents.md §3 / spec §63）：为真实需求抽象——七牛 Kodo、阿里云 OSS、
//! 腾讯云 COS 三个真实 Provider 需要统一接口，而不是为不存在的平台做抽象。因此本 crate 只有：
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

/// 下载落盘守卫：数据先写到**本次尝试专属**的同目录临时文件，成功后原子 rename 到目标。
///
/// - 暂停/取消/挂起时传输引擎靠 `JoinHandle::abort()` 丢弃 future，守卫的 Drop 只删自己的
///   临时文件，半截数据不会留在目标路径上；
/// - 临时名带进程内唯一序号：两个下载共用同一目标路径时，取消较旧的一个不会误删较新一个
///   已完成（或进行中）的产物，目标路径上的既有文件在成功前也不会被截断。
pub struct PartialFileGuard {
    dest: std::path::PathBuf,
    temp: std::path::PathBuf,
    armed: bool,
}

impl PartialFileGuard {
    pub fn new(dest: &std::path::Path) -> Self {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // 临时名只含进程号 + 序号，与目标文件名长度无关：目标名接近文件系统单段上限
        // （255 字节）时，在其后追加后缀会让原本可创建的下载在写入前就 ENAMETOOLONG。
        let temp = dest.with_file_name(format!(".osm-{}-{seq}.part", std::process::id()));
        Self {
            dest: dest.to_path_buf(),
            temp,
            armed: true,
        }
    }

    /// 本次尝试写入的临时文件路径。
    pub fn temp_path(&self) -> &std::path::Path {
        &self.temp
    }

    /// 下载完整成功：把临时文件原子地换到目标路径。
    pub fn commit(mut self) -> std::io::Result<()> {
        std::fs::rename(&self.temp, &self.dest)?;
        self.armed = false;
        Ok(())
    }
}

impl Drop for PartialFileGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(&self.temp);
        }
    }
}

/// 含独立 `.` / `..` 路径段的 key 无法被 HTTP 客户端按字面寻址：URL 点段归一化会把
/// `a/../b` 改写成 `/b`，请求（以及签名）打到别的对象上。联网前拒绝，三家共用。
pub fn dot_segment_error(key: &str) -> Option<String> {
    key.split('/')
        .any(|segment| segment == "." || segment == "..")
        .then(|| {
            format!(
                "对象 key `{key}` 含独立的 `.` / `..` 路径段。HTTP 客户端无法按字面寻址这类对象\
                 （URL 点段归一化会改变请求路径），本应用拒绝操作以免误伤其它对象；\
                 如需处理请在云厂商控制台操作"
            )
        })
}

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

    /// 生成带签名的 GET URL（私有空间必须签名；公有空间同样签名，行为与官方 SDK 一致）。
    /// `ttl_secs` 是链接有效期。返回值含 token，调用方不得写入日志。
    fn signed_get_url(
        &self,
        bucket: &str,
        key: &str,
        ttl_secs: u64,
    ) -> impl Future<Output = Result<String, StorageError>> + Send;
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

#[cfg(test)]
mod guard_tests {
    use super::PartialFileGuard;

    #[test]
    fn dot_segments_are_rejected_only_as_whole_segments() {
        assert!(super::dot_segment_error("a/../b").is_some());
        assert!(super::dot_segment_error("./x").is_some());
        assert!(super::dot_segment_error("a/b/..").is_some());
        assert!(super::dot_segment_error("a/..b/c.d/.hidden").is_none());
    }

    #[test]
    fn guard_writes_to_private_temp_and_commit_publishes_atomically() {
        let dir = std::env::temp_dir().join(format!("osm-guard-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("a.bin");
        std::fs::write(&dest, b"old").unwrap();

        // 被丢弃（abort）的尝试：只删自己的临时文件，目标路径上的既有文件原样保留
        let aborted = PartialFileGuard::new(&dest);
        std::fs::write(aborted.temp_path(), b"partial").unwrap();
        let aborted_temp = aborted.temp_path().to_path_buf();
        // 另一个并发尝试：临时名必须不同
        let other = PartialFileGuard::new(&dest);
        assert_ne!(aborted.temp_path(), other.temp_path());
        std::fs::write(other.temp_path(), b"new").unwrap();
        drop(aborted);
        assert!(!aborted_temp.exists());
        assert_eq!(std::fs::read(&dest).unwrap(), b"old");

        // 成功的尝试：commit 后目标内容为新内容，临时文件消失
        let other_temp = other.temp_path().to_path_buf();
        other.commit().unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"new");
        assert!(!other_temp.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn guard_temp_name_does_not_depend_on_destination_length() {
        let long = "a".repeat(255);
        let dest = std::path::Path::new("/tmp").join(long);
        let guard = PartialFileGuard::new(&dest);
        let name = guard.temp_path().file_name().unwrap().to_string_lossy();
        assert!(name.len() < 64, "临时名过长：{name}");
        assert_eq!(guard.temp_path().parent(), dest.parent());
    }
}
