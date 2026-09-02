//! Domain Models：Account/Bucket/Object 等纯领域模型
//!
//! 原则（agents.md §5）：
//! - 本地路径一律 `PathBuf`（本 crate 目前无本地路径模型）
//! - Cloud Object Key 一律 `String` + `/`，两者严格区分
//!
//! 本 crate 零依赖：只放纯数据模型，不做 IO、不依赖 reqwest/tokio。

/// 存储服务商类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderKind {
    /// 七牛 Kodo
    Qiniu,
    /// 阿里云 OSS
    Aliyun,
}

impl ProviderKind {
    pub fn display_name(self) -> &'static str {
        match self {
            ProviderKind::Qiniu => "七牛 Kodo",
            ProviderKind::Aliyun => "阿里云 OSS",
        }
    }
}

/// 存储空间（Bucket）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bucket {
    /// Bucket 名称（云侧标识）
    pub name: String,
    /// 所属服务商
    pub kind: ProviderKind,
    /// 区域（如七牛 z0 / OSS cn-hangzhou），列表 API 可能不返回，故为 Option
    pub region: Option<String>,
}

/// 云端对象
///
/// `key` 是 Cloud Object Key（String + `/`），**不是**本地路径。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudObject {
    /// 对象 Key（String + `/`）
    pub key: String,
    /// 字节数
    pub size: u64,
    /// MIME 类型
    pub mime_type: Option<String>,
    /// ETag（七牛为 hash 字段）
    pub etag: Option<String>,
    /// 上传时间（Unix epoch 毫秒）
    ///
    /// 七牛 putTime 原始单位是 100ns，Provider 层已统一换算为毫秒。
    pub put_time_millis: i64,
}

/// 目录列举结果中的一项：真实对象，或模拟目录的公共前缀
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListingEntry {
    Object(CloudObject),
    /// 模拟目录（以 `/` 结尾的前缀，来自 delimiter 列举的 commonPrefixes）
    CommonPrefix(String),
}

/// 一次目录列举请求
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListObjectsRequest {
    /// Bucket 名称
    pub bucket: String,
    /// 前缀过滤（None = 不过滤）
    pub prefix: Option<String>,
    /// 目录分隔符（通常为 "/"，用于模拟目录；None = 平铺列举）
    pub delimiter: Option<String>,
    /// 翻页标记（来自上一页 `ObjectPage::next_marker`）
    pub marker: Option<String>,
    /// 单页条数上限（Provider 负责校验合法范围）
    pub limit: u32,
}

impl ListObjectsRequest {
    /// 默认单页 100 条
    pub fn new(bucket: impl Into<String>) -> Self {
        Self {
            bucket: bucket.into(),
            prefix: None,
            delimiter: None,
            marker: None,
            limit: 100,
        }
    }
}

/// 一次目录列举的返回页
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ObjectPage {
    pub entries: Vec<ListingEntry>,
    /// 下一页标记；None 或空表示没有更多数据
    pub next_marker: Option<String>,
}

impl ObjectPage {
    pub fn has_more(&self) -> bool {
        self.next_marker.as_ref().is_some_and(|m| !m.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_page_has_more_semantics() {
        let empty = ObjectPage::default();
        assert!(!empty.has_more());

        let blank = ObjectPage {
            entries: vec![],
            next_marker: Some(String::new()),
        };
        // 七牛用空字符串 marker 表示列举结束
        assert!(!blank.has_more());

        let more = ObjectPage {
            entries: vec![],
            next_marker: Some("abc".into()),
        };
        assert!(more.has_more());
    }

    #[test]
    fn list_request_defaults() {
        let req = ListObjectsRequest::new("demo");
        assert_eq!(req.bucket, "demo");
        assert_eq!(req.limit, 100);
        assert_eq!(req.prefix, None);
    }
}
