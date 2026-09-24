//! 按账号服务商分发的 Provider 句柄。
//!
//! StorageProvider 不是 dyn-safe（返回 `impl Future + Send`），上层用枚举分发，
//! 不为不存在的平台再加一层 trait。

use std::path::Path;

use object_storage_aliyun::AliyunProvider;
use object_storage_core::{ByteProgress, StorageError, StorageProvider};
use object_storage_domain::{Bucket, ListObjectsRequest, ObjectPage, ProviderKind};
use object_storage_qiniu::QiniuProvider;
use object_storage_tencent::TencentProvider;

#[derive(Debug)]
pub enum BuiltProvider {
    Qiniu(Box<QiniuProvider>),
    Aliyun(AliyunProvider),
    Tencent(TencentProvider),
}

impl BuiltProvider {
    pub fn kind(&self) -> ProviderKind {
        match self {
            Self::Qiniu(p) => p.kind(),
            Self::Aliyun(p) => p.kind(),
            Self::Tencent(p) => p.kind(),
        }
    }

    pub async fn list_buckets(&self) -> Result<Vec<Bucket>, StorageError> {
        match self {
            Self::Qiniu(p) => p.list_buckets().await,
            Self::Aliyun(p) => p.list_buckets().await,
            Self::Tencent(p) => p.list_buckets().await,
        }
    }

    pub async fn list_objects(
        &self,
        request: ListObjectsRequest,
    ) -> Result<ObjectPage, StorageError> {
        match self {
            Self::Qiniu(p) => p.list_objects(request).await,
            Self::Aliyun(p) => p.list_objects(request).await,
            Self::Tencent(p) => p.list_objects(request).await,
        }
    }

    pub async fn download_object_to_file(
        &self,
        bucket: &str,
        key: &str,
        dest: &Path,
        progress: Option<ByteProgress>,
    ) -> Result<u64, StorageError> {
        match self {
            Self::Qiniu(p) => p.download_object_to_file(bucket, key, dest, progress).await,
            Self::Aliyun(p) => p.download_object_to_file(bucket, key, dest, progress).await,
            Self::Tencent(p) => p.download_object_to_file(bucket, key, dest, progress).await,
        }
    }

    pub async fn upload_object_from_file(
        &self,
        bucket: &str,
        key: &str,
        source: &Path,
        progress: Option<ByteProgress>,
    ) -> Result<u64, StorageError> {
        match self {
            Self::Qiniu(p) => {
                p.upload_object_from_file(bucket, key, source, progress)
                    .await
            }
            Self::Aliyun(p) => {
                p.upload_object_from_file(bucket, key, source, progress)
                    .await
            }
            Self::Tencent(p) => {
                p.upload_object_from_file(bucket, key, source, progress)
                    .await
            }
        }
    }

    pub async fn delete_object(&self, bucket: &str, key: &str) -> Result<(), StorageError> {
        match self {
            Self::Qiniu(p) => p.delete_object(bucket, key).await,
            Self::Aliyun(p) => p.delete_object(bucket, key).await,
            Self::Tencent(p) => p.delete_object(bucket, key).await,
        }
    }

    pub async fn signed_get_url(
        &self,
        bucket: &str,
        key: &str,
        ttl_secs: u64,
    ) -> Result<String, StorageError> {
        match self {
            Self::Qiniu(p) => p.signed_get_url(bucket, key, ttl_secs).await,
            Self::Aliyun(p) => p.signed_get_url(bucket, key, ttl_secs).await,
            Self::Tencent(p) => p.signed_get_url(bucket, key, ttl_secs).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use object_storage_aliyun::AliyunCredential;
    use object_storage_qiniu::QiniuCredential;
    use object_storage_tencent::TencentCredential;

    /// 每个 BuiltProvider 变体都必须如实报出自己的 ProviderKind——
    /// 枚举分发最容易出的错就是复制粘贴时漏改一个臂。
    #[test]
    fn every_variant_reports_its_own_kind() {
        let cases = [
            (
                BuiltProvider::Qiniu(Box::new(QiniuProvider::new(
                    QiniuCredential::new("ak", "sk").unwrap(),
                ))),
                ProviderKind::Qiniu,
            ),
            (
                BuiltProvider::Aliyun(AliyunProvider::new(
                    AliyunCredential::new("ak", "sk").unwrap(),
                )),
                ProviderKind::Aliyun,
            ),
            (
                BuiltProvider::Tencent(TencentProvider::new(
                    TencentCredential::new("id", "key").unwrap(),
                )),
                ProviderKind::Tencent,
            ),
        ];
        for (provider, expected) in cases {
            assert_eq!(provider.kind(), expected);
        }
    }
}
