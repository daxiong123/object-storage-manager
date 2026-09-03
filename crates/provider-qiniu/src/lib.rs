//! Qiniu Kodo Provider
//!
//! API 核对自官方 SDK 生成代码（qiniu-apis 0.2.4）：
//! - 列举空间：UC 服务 `GET /buckets`，V2 签名，返回 bucket 名字符串数组
//! - 列举文件：RSF 服务 `GET /list?bucket=&marker=&limit=&prefix=&delimiter=`，
//!   V2 签名，返回 `{marker, items[key,hash,fsize,mimeType,putTime,type,status],
//!   commonPrefixes}`；`putTime` 单位为 100ns，这里统一换算为毫秒
//! - limit 合法范围 1–1000
//!
//! 默认端点（官方文档）：UC `https://uc.qiniuapi.com`、RSF `https://rsf.qbox.me`。
//! 测试可用 [`QiniuProvider::with_endpoints`] 指向本地 mock。

mod sign;

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::StreamExt;
use object_storage_core::{ByteProgress, StorageError, StorageProvider};
use object_storage_domain::{
    Bucket, CloudObject, ListObjectsRequest, ListingEntry, ObjectPage, ProviderKind,
};
use reqwest::header::AUTHORIZATION;
use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub use sign::QiniuCredential;

/// 官方 UC 服务端点（空间管理）
const DEFAULT_UC_ENDPOINT: &str = "https://uc.qiniuapi.com";
/// 官方 RSF 服务端点（列举文件）
const DEFAULT_RSF_ENDPOINT: &str = "https://rsf.qbox.me";
/// 官方表单上传入口（华东/智能；区域专用域名按 UC /v4/query 解析并缓存，
/// 查询失败回退到本入口）
const DEFAULT_UP_ENDPOINT: &str = "https://upload.qiniup.com";
/// 默认上传入口的 host（回退时替换 up host 用）
const DEFAULT_UP_ENDPOINT_HOST: &str = "upload.qiniup.com";

/// UC /v4/query 响应体（官方 Rust SDK `structs.rs` 同构；
/// `hosts` 为新字段名，`regions` 为旧别名——serde alias 处理）。
#[derive(Debug, serde::Deserialize)]
struct RegionQueryBody {
    #[serde(default, alias = "regions")]
    hosts: Vec<RegionQueryHost>,
}

#[derive(Debug, serde::Deserialize)]
struct RegionQueryHost {
    /// 上传服务域名组（preferred 在前）
    up: RegionQueryDomains,
    /// 该 host 的缓存有效期（秒）；缺省 86400（官方 default_ttl）。
    /// ttl 在 host 级别而非响应顶层（官方 RegionResponseBody 同构）。
    #[serde(default = "default_region_ttl")]
    ttl: u64,
}

#[derive(Debug, serde::Deserialize)]
struct RegionQueryDomains {
    #[serde(default)]
    domains: Vec<String>,
}

fn default_region_ttl() -> u64 {
    86_400
}
/// 官方 RS 服务端点（对象管理：删除/stat/move）
const DEFAULT_RS_ENDPOINT: &str = "https://rs.qbox.me";

pub struct QiniuProvider {
    http: reqwest::Client,
    /// 下载专用客户端：不带总超时（30s 总超时会据断大文件下载）；
    /// 连接超时仍在。挂死风险由后续传输引擎接管。
    download_http: reqwest::Client,
    /// 上传同样不设总超时（大文件可能数十分钟）
    upload_http: reqwest::Client,
    cred: QiniuCredential,
    uc_base: reqwest::Url,
    rsf_base: reqwest::Url,
    rs_base: reqwest::Url,
    up_base: reqwest::Url,
    /// 下载域名协议（生产恒为 https；测试指向本地 mock 时切 http）
    download_use_https: bool,
    /// 区域缓存（bucket → 上传端点）：UC /v4/query 结果 + 到期时刻。
    /// 只缓存 up 域名（本 App 用到的区域能力只有上传；下载走空间绑定域名，
    /// 删除/列举走全局 RS/RSF，均可路由）。
    region_cache: std::sync::Mutex<std::collections::HashMap<String, CachedUpEndpoint>>,
}

/// 区域上传端点缓存条目。
struct CachedUpEndpoint {
    /// 上传域名（https 前缀在用时装上，避免缓存里混协议）
    host: String,
    /// 缓存到期时刻（UC 响应 ttl；到期后重新查询）
    expires_at: std::time::Instant,
}

impl QiniuProvider {
    pub fn new(cred: QiniuCredential) -> Self {
        Self::with_endpoints(cred, DEFAULT_UC_ENDPOINT, DEFAULT_RSF_ENDPOINT)
            .expect("内置端点 URL 必然合法")
    }

    /// 指定服务端点（测试用；生产走 [`QiniuProvider::new`]）
    pub fn with_endpoints(
        cred: QiniuCredential,
        uc_base: &str,
        rsf_base: &str,
    ) -> Result<Self, StorageError> {
        Self::with_all_endpoints(
            cred,
            uc_base,
            rsf_base,
            DEFAULT_UP_ENDPOINT,
            DEFAULT_RS_ENDPOINT,
        )
    }

    /// 指定 UC / RSF / 上传 / RS 端点（测试用）。
    pub fn with_all_endpoints(
        cred: QiniuCredential,
        uc_base: &str,
        rsf_base: &str,
        up_base: &str,
        rs_base: &str,
    ) -> Result<Self, StorageError> {
        let uc_base = reqwest::Url::parse(uc_base)
            .map_err(|e| StorageError::InvalidInput(format!("UC 端点不合法: {e}")))?;
        let rsf_base = reqwest::Url::parse(rsf_base)
            .map_err(|e| StorageError::InvalidInput(format!("RSF 端点不合法: {e}")))?;
        let up_base = reqwest::Url::parse(up_base)
            .map_err(|e| StorageError::InvalidInput(format!("上传端点不合法: {e}")))?;
        let rs_base = reqwest::Url::parse(rs_base)
            .map_err(|e| StorageError::InvalidInput(format!("RS 端点不合法: {e}")))?;
        let http = reqwest::Client::builder()
            .user_agent(concat!("CloudStorage/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .use_rustls_tls()
            .build()
            .map_err(|e| StorageError::Network(format!("HTTP Client 构建失败: {e}")))?;
        let long_http = reqwest::Client::builder()
            .user_agent(concat!("CloudStorage/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            // 注意：不设总超时——上下载可能持续数十分钟；读阻塞由传输引擎处理
            .use_rustls_tls()
            .build()
            .map_err(|e| StorageError::Network(format!("HTTP Client 构建失败: {e}")))?;
        Ok(Self {
            http,
            download_http: long_http.clone(),
            upload_http: long_http,
            cred,
            uc_base,
            rsf_base,
            rs_base,
            up_base,
            download_use_https: true,
            region_cache: std::sync::Mutex::new(std::collections::HashMap::new()),
        })
    }

    fn uc_url(&self, path: &str) -> Result<reqwest::Url, StorageError> {
        self.uc_base
            .join(path)
            .map_err(|e| StorageError::InvalidInput(format!("URL 拼接失败: {e}")))
    }

    fn rsf_url(&self, path: &str) -> Result<reqwest::Url, StorageError> {
        self.rsf_base
            .join(path)
            .map_err(|e| StorageError::InvalidInput(format!("URL 拼接失败: {e}")))
    }

    fn rs_url(&self, path: &str) -> Result<reqwest::Url, StorageError> {
        self.rs_base
            .join(path)
            .map_err(|e| StorageError::InvalidInput(format!("URL 拼接失败: {e}")))
    }

    /// 测试专用：下载域名走 http（本地 mock 无 TLS）
    #[cfg(test)]
    fn set_download_use_https(&mut self, use_https: bool) {
        self.download_use_https = use_https;
    }

    fn download_scheme(&self) -> &'static str {
        if self.download_use_https {
            "https://"
        } else {
            "http://"
        }
    }

    /// 查询空间绑定的下载域名。UC `GET /v2/domains?tbl=<bucket>`，V2 签名，
    /// 响应为域名字符串数组（逐字节核对官方 qiniu-apis 0.2.4；旧版
    /// `v6/domain/list` 已废弃，勿用）。
    async fn bucket_domains(&self, bucket: &str) -> Result<Vec<String>, StorageError> {
        let mut url = self.uc_url("v2/domains")?;
        url.set_query(Some(&format!(
            "tbl={}",
            sign::percent_encode_query_value(bucket)
        )));
        let auth = sign::authorization_v2_for_url(&self.cred, "GET", &url);
        let resp = self
            .http
            .get(url)
            .header(AUTHORIZATION, auth)
            .send()
            .await
            .map_err(|e| StorageError::Network(format!("bucket_domains: {e}")))?;
        let resp = self.check_status(resp, "bucket_domains").await?;
        let text = text_or_invalid(resp, "bucket_domains").await?;
        let domains: Vec<String> = serde_json::from_str(&text).map_err(|e| {
            StorageError::InvalidResponse(format!(
                "bucket_domains: {e}; body={}",
                truncate(&text, 500)
            ))
        })?;
        Ok(domains)
    }

    /// 查询空间所在区域的上传域名。UC `GET /v4/query?ak=&bucket=`
    /// （公开接口，无需签名——官方 Rust SDK `BucketRegionsQueryer::do_sync_query`
    /// 同样不带 Authorization）。响应含 `hosts[]`（兼容旧字段名 `regions[]`），
    /// 每项 `up.domains[]` 是该区域上传域名（preferred 在前）。
    ///
    /// 结果按 bucket 缓存（TTL 取响应 `ttl` 秒，缺省 86400）；查询失败回退
    /// 默认智能上传入口 `upload.qiniup.com`（可路由任意区域，慢但可用）。
    async fn bucket_up_host(&self, bucket: &str) -> String {
        // 1) 缓存命中（未过期）直接用
        if let Some(host) = self.cached_up_host(bucket) {
            return host;
        }
        // 2) UC 查询
        match self.query_up_host(bucket).await {
            Ok((host, ttl)) => {
                self.cache_up_host(bucket, &host, ttl);
                host
            }
            Err(error) => {
                eprintln!(
                    "[qiniu] region query failed (bucket={bucket}), fallback to {DEFAULT_UP_ENDPOINT_HOST}: {error}"
                );
                DEFAULT_UP_ENDPOINT_HOST.to_string()
            }
        }
    }

    fn cached_up_host(&self, bucket: &str) -> Option<String> {
        let cache = self.region_cache.lock().expect("区域缓存锁毒化");
        cache
            .get(bucket)
            .filter(|c| c.expires_at > std::time::Instant::now())
            .map(|c| c.host.clone())
    }

    /// 本地 mock 测试模式：UC 端点指向 127.0.0.1/localhost 时，上传也直接
    /// 打到 `up_base`（测试用端点），不做真实区域解析。
    fn is_test_mode(&self) -> bool {
        matches!(
            self.uc_base.host_str(),
            Some("127.0.0.1") | Some("localhost")
        )
    }

    fn cache_up_host(&self, bucket: &str, host: &str, ttl_secs: u64) {
        let mut cache = self.region_cache.lock().expect("区域缓存锁毒化");
        cache.insert(
            bucket.to_string(),
            CachedUpEndpoint {
                host: host.to_string(),
                expires_at: std::time::Instant::now() + std::time::Duration::from_secs(ttl_secs),
            },
        );
    }

    async fn query_up_host(&self, bucket: &str) -> Result<(String, u64), StorageError> {
        let mut url = self.uc_url("v4/query")?;
        url.set_query(Some(&format!(
            "ak={}&bucket={}",
            sign::percent_encode_query_value(self.cred.access_key()),
            sign::percent_encode_query_value(bucket)
        )));
        let resp = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|e| StorageError::Network(format!("bucket_region: {e}")))?;
        let resp = self.check_status(resp, "bucket_region").await?;
        let text = text_or_invalid(resp, "bucket_region").await?;
        let body: RegionQueryBody = serde_json::from_str(&text).map_err(|e| {
            StorageError::InvalidResponse(format!(
                "bucket_region: {e}; body={}",
                truncate(&text, 500)
            ))
        })?;
        let Some(first) = body.hosts.first() else {
            return Err(StorageError::InvalidResponse(format!(
                "bucket_region: 空间 {bucket} 区域查询返回空 hosts"
            )));
        };
        if first.up.domains.is_empty() {
            return Err(StorageError::InvalidResponse(format!(
                "bucket_region: 空间 {bucket} 区域查询无 up 域名"
            )));
        }
        Ok((first.up.domains[0].clone(), first.ttl))
    }

    /// 生成带签名的下载 URL。token 含 SK 派生签名，调用方不得写入日志。
    async fn signed_download_url(
        &self,
        bucket: &str,
        key: &str,
        ttl_secs: u64,
    ) -> Result<reqwest::Url, StorageError> {
        if bucket.is_empty() {
            return Err(StorageError::InvalidInput("bucket 不能为空".into()));
        }
        if key.is_empty() {
            return Err(StorageError::InvalidInput("key 不能为空".into()));
        }
        if ttl_secs == 0 {
            return Err(StorageError::InvalidInput("ttl_secs 必须大于 0".into()));
        }
        let domains = self.bucket_domains(bucket).await?;
        let Some(domain) = domains.first().filter(|d| !d.is_empty()) else {
            return Err(StorageError::InvalidResponse(format!(
                "signed_get_url: 空间 {bucket} 未绑定可用下载域名（请在七牛控制台绑定 CDN/测试域名）"
            )));
        };
        let deadline = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| StorageError::InvalidInput(format!("signed_get_url: 系统时间异常: {e}")))?
            .as_secs()
            + ttl_secs;
        let url: reqwest::Url = format!(
            "{}{}/{}?e={deadline}",
            self.download_scheme(),
            domain,
            sign::percent_encode_path_value(key)
        )
        .parse()
        .map_err(|e| StorageError::InvalidInput(format!("signed_get_url: 下载 URL 不合法: {e}")))?;
        let token = sign::sign_token(&self.cred, url.as_str().as_bytes());
        let mut signed_url = url;
        signed_url.set_query(Some(&format!("e={deadline}&token={token}")));
        Ok(signed_url)
    }

    /// 下载失败时删除半成品文件；删除也失败则并入错误信息（不静默吞掉）
    async fn download_cleanup_failed(dest: &Path, cause: String) -> StorageError {
        match tokio::fs::remove_file(dest).await {
            Ok(()) => StorageError::Io(format!("download_object: {cause}；已清理半成品文件")),
            Err(rm) => StorageError::Io(format!(
                "download_object: {cause}；且清理半成品文件失败: {rm}（残留：{}）",
                dest.display()
            )),
        }
    }

    /// 统一的非 2xx 错误映射（七牛错误响应体为 `{"error": "..."}`，
    /// 且会使用 631/612 等非标准 HTTP 状态码）
    async fn check_status(
        &self,
        resp: reqwest::Response,
        context: &str,
    ) -> Result<reqwest::Response, StorageError> {
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let body = resp.text().await.unwrap_or_default();
        #[derive(Deserialize)]
        struct ErrorBody {
            #[serde(default)]
            error: Option<String>,
        }
        let message = serde_json::from_str::<ErrorBody>(&body)
            .ok()
            .and_then(|e| e.error)
            .unwrap_or_else(|| truncate(&body, 500));
        match status.as_u16() {
            401 => Err(StorageError::Auth(format!("{context}: {message}"))),
            429 | 573 | 579 => Err(StorageError::RateLimited(format!(
                "{context} (HTTP {status}): {message}"
            ))),
            _ => Err(StorageError::Api {
                status: status.as_u16(),
                message: format!("{context}: {message}"),
            }),
        }
    }
}

async fn text_or_invalid(resp: reqwest::Response, context: &str) -> Result<String, StorageError> {
    resp.text()
        .await
        .map_err(|e| StorageError::Network(format!("{context}: 读取响应体失败: {e}")))
}

impl std::fmt::Debug for QiniuProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QiniuProvider")
            .field("cred", &self.cred)
            .field("uc_base", &self.uc_base.as_str())
            .field("rsf_base", &self.rsf_base.as_str())
            .field("rs_base", &self.rs_base.as_str())
            .field("up_base", &self.up_base.as_str())
            .finish()
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max).collect();
        format!("{cut}…")
    }
}

#[derive(Deserialize)]
struct ListResponse {
    #[serde(default)]
    marker: Option<String>,
    #[serde(default)]
    items: Vec<ListItem>,
    #[serde(default, rename = "commonPrefixes")]
    common_prefixes: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct ListItem {
    key: String,
    #[serde(default)]
    hash: Option<String>,
    fsize: u64,
    #[serde(rename = "mimeType", default)]
    mime_type: Option<String>,
    /// 七牛 putTime：Unix epoch 起 100ns 计数
    #[serde(rename = "putTime")]
    put_time_100ns: u64,
}

impl StorageProvider for QiniuProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Qiniu
    }

    async fn list_buckets(&self) -> Result<Vec<Bucket>, StorageError> {
        let url = self.uc_url("buckets")?;
        let auth = sign::authorization_v2_for_url(&self.cred, "GET", &url);
        let resp = self
            .http
            .get(url)
            .header(AUTHORIZATION, auth)
            .send()
            .await
            .map_err(|e| StorageError::Network(format!("list_buckets: {e}")))?;
        let resp = self.check_status(resp, "list_buckets").await?;
        let text = text_or_invalid(resp, "list_buckets").await?;
        let names: Vec<String> = serde_json::from_str(&text).map_err(|e| {
            StorageError::InvalidResponse(format!(
                "list_buckets: {e}; body={}",
                truncate(&text, 500)
            ))
        })?;
        Ok(names
            .into_iter()
            .map(|name| Bucket {
                name,
                kind: ProviderKind::Qiniu,
                region: None,
            })
            .collect())
    }

    async fn list_objects(&self, request: ListObjectsRequest) -> Result<ObjectPage, StorageError> {
        if request.bucket.is_empty() {
            return Err(StorageError::InvalidInput("bucket 不能为空".into()));
        }
        if !(1..=1000).contains(&request.limit) {
            return Err(StorageError::InvalidInput(format!(
                "limit 必须在 1–1000，实际 {}",
                request.limit
            )));
        }

        // 组装 query（值做 RFC 3986 编码；顺序固定，保证可复现）
        let mut pairs: Vec<(String, String)> = vec![("bucket".into(), request.bucket)];
        if let Some(p) = request.prefix.as_ref().filter(|p| !p.is_empty()) {
            pairs.push(("prefix".into(), p.clone()));
        }
        if let Some(d) = request.delimiter.as_ref().filter(|d| !d.is_empty()) {
            pairs.push(("delimiter".into(), d.clone()));
        }
        if let Some(m) = request.marker.as_ref().filter(|m| !m.is_empty()) {
            pairs.push(("marker".into(), m.clone()));
        }
        pairs.push(("limit".into(), request.limit.to_string()));
        let query = pairs
            .iter()
            .map(|(k, v)| format!("{k}={}", sign::percent_encode_query_value(v)))
            .collect::<Vec<_>>()
            .join("&");

        let mut url = self.rsf_url("list")?;
        url.set_query(Some(&query));
        // 关键：query 最终确定后再签名，保证签名串 == 实际发送串
        let auth = sign::authorization_v2_for_url(&self.cred, "GET", &url);

        let resp = self
            .http
            .get(url)
            .header(AUTHORIZATION, auth)
            .send()
            .await
            .map_err(|e| StorageError::Network(format!("list_objects: {e}")))?;
        let resp = self.check_status(resp, "list_objects").await?;
        let text = text_or_invalid(resp, "list_objects").await?;
        let parsed: ListResponse = serde_json::from_str(&text).map_err(|e| {
            StorageError::InvalidResponse(format!(
                "list_objects: {e}; body={}",
                truncate(&text, 500)
            ))
        })?;

        let mut entries: Vec<ListingEntry> = parsed
            .items
            .into_iter()
            .map(|item| {
                ListingEntry::Object(CloudObject {
                    key: item.key,
                    size: item.fsize,
                    mime_type: item.mime_type,
                    etag: item.hash,
                    // 100ns → 毫秒
                    put_time_millis: (item.put_time_100ns / 10_000) as i64,
                })
            })
            .collect();
        if let Some(prefixes) = parsed.common_prefixes {
            entries.extend(prefixes.into_iter().map(ListingEntry::CommonPrefix));
        }

        Ok(ObjectPage {
            next_marker: parsed.marker.filter(|m| !m.is_empty()),
            entries,
        })
    }

    async fn download_object_to_file(
        &self,
        bucket: &str,
        key: &str,
        dest: &Path,
        progress: Option<ByteProgress>,
    ) -> Result<u64, StorageError> {
        if bucket.is_empty() {
            return Err(StorageError::InvalidInput("bucket 不能为空".into()));
        }
        if key.is_empty() {
            return Err(StorageError::InvalidInput("key 不能为空".into()));
        }

        let signed_url = self.signed_download_url(bucket, key, 3600).await?;

        // 下载专用客户端不带总超时（大文件）
        let resp = self
            .download_http
            .get(signed_url)
            .send()
            .await
            .map_err(|e| StorageError::Network(format!("download_object: {e}")))?;
        let resp = self.check_status(resp, "download_object").await?;
        let content_len = resp.content_length();

        // 流式落盘：分块写，不驻留整文件（内存红线，agents.md §2）。
        // 中途失败则清理半成品文件后响报（错误永不静默）。
        let mut file = tokio::fs::File::create(dest).await.map_err(|e| {
            StorageError::Io(format!(
                "download_object: 创建本地文件 {} 失败: {e}",
                dest.display()
            ))
        })?;
        let mut total: u64 = 0;
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| {
                StorageError::Network(format!("download_object: 接收数据中断: {e}"))
            })?;
            if let Err(e) = file.write_all(&chunk).await {
                return Err(
                    Self::download_cleanup_failed(dest, format!("写入本地文件失败: {e}")).await,
                );
            }
            total += chunk.len() as u64;
            if let Some(cb) = &progress {
                cb(total, content_len);
            }
        }
        if let Err(e) = file.flush().await {
            return Err(
                Self::download_cleanup_failed(dest, format!("写入本地文件失败: {e}")).await,
            );
        }
        Ok(total)
    }

    async fn upload_object_from_file(
        &self,
        bucket: &str,
        key: &str,
        source: &Path,
        progress: Option<ByteProgress>,
    ) -> Result<u64, StorageError> {
        if bucket.is_empty() {
            return Err(StorageError::InvalidInput("bucket 不能为空".into()));
        }
        if key.is_empty() {
            return Err(StorageError::InvalidInput("key 不能为空".into()));
        }
        let meta = tokio::fs::metadata(source).await.map_err(|e| {
            StorageError::Io(format!(
                "upload_object: 读取本地文件 {} 失败: {e}",
                source.display()
            ))
        })?;
        if meta.is_dir() {
            return Err(StorageError::InvalidInput(format!(
                "upload_object: {} 是目录，本里程碑只上传文件",
                source.display()
            )));
        }
        let file_len = meta.len();
        let file = tokio::fs::File::open(source).await.map_err(|e| {
            StorageError::Io(format!(
                "upload_object: 打开本地文件 {} 失败: {e}",
                source.display()
            ))
        })?;

        let deadline = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| StorageError::InvalidInput(format!("upload_object: 系统时间异常: {e}")))?
            .as_secs()
            + 3600;
        let token = sign::upload_token(&self.cred, bucket, key, deadline);
        // 区域上传域名：UC /v4/query 解析（带缓存）；非华东 bucket 上传到
        // 华东域名会被慢路由甚至拒绝，因此必须按 bucket 解析。
        // 测试模式（本地 mock）直接用注入的 up_base。
        let up_url = if self.is_test_mode() {
            self.up_base.clone()
        } else {
            let up_host = self.bucket_up_host(bucket).await;
            format!("https://{up_host}/")
                .parse()
                .map_err(|e| StorageError::InvalidInput(format!("上传端点不合法: {e}")))?
        };
        let file_name = source
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file")
            .to_string();

        // 64KiB 分块读盘，wrap_stream 喂给 multipart；已知长度让 reqwest 算 Content-Length。
        // 进度按「已交给 HTTP body 的字节」计（与实际上传进度同阶）。
        if let Some(cb) = &progress {
            cb(0, Some(file_len));
        }
        let stream = futures_util::stream::unfold(
            (file, 0u64, progress, file_len),
            |(mut file, done, progress, file_len)| async move {
                let mut buf = vec![0u8; 64 * 1024];
                match file.read(&mut buf).await {
                    Ok(0) => None,
                    Ok(n) => {
                        buf.truncate(n);
                        let done = done + n as u64;
                        if let Some(cb) = &progress {
                            cb(done, Some(file_len));
                        }
                        Some((
                            Ok::<_, std::io::Error>(buf),
                            (file, done, progress, file_len),
                        ))
                    }
                    Err(e) => Some((Err(e), (file, done, progress, file_len))),
                }
            },
        );
        let part = reqwest::multipart::Part::stream_with_length(
            reqwest::Body::wrap_stream(stream),
            file_len,
        )
        .file_name(file_name)
        .mime_str("application/octet-stream")
        .map_err(|e| StorageError::InvalidInput(format!("upload_object: MIME 设置失败: {e}")))?;
        let form = reqwest::multipart::Form::new()
            .text("token", token)
            .text("key", key.to_string())
            .part("file", part);

        let resp = self
            .upload_http
            .post(up_url)
            .multipart(form)
            .send()
            .await
            .map_err(|e| StorageError::Network(format!("upload_object: {e}")))?;
        let _resp = self.check_status(resp, "upload_object").await?;
        Ok(file_len)
    }

    async fn delete_object(&self, bucket: &str, key: &str) -> Result<(), StorageError> {
        if bucket.is_empty() {
            return Err(StorageError::InvalidInput("bucket 不能为空".into()));
        }
        if key.is_empty() {
            return Err(StorageError::InvalidInput("key 不能为空".into()));
        }
        // RS `POST /delete/<urlsafe_base64(bucket:key)>`，V2 签名，无 body
        // （官方 qiniu-apis delete_object PathParams::set_entry_as_str）
        let entry = sign::encoded_entry(bucket, key);
        let url = self.rs_url(&format!("delete/{entry}"))?;
        let auth = sign::authorization_v2_for_url(&self.cred, "POST", &url);
        let resp = self
            .http
            .post(url)
            .header(AUTHORIZATION, auth)
            .send()
            .await
            .map_err(|e| StorageError::Network(format!("delete_object: {e}")))?;
        let _resp = self.check_status(resp, "delete_object").await?;
        Ok(())
    }

    async fn signed_get_url(
        &self,
        bucket: &str,
        key: &str,
        ttl_secs: u64,
    ) -> Result<String, StorageError> {
        Ok(self
            .signed_download_url(bucket, key, ttl_secs)
            .await?
            .to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpListener};
    use std::sync::{Arc, Mutex};
    use std::thread;

    struct CapturedRequest {
        request_line: String,
        headers: Vec<(String, String)>,
    }

    impl CapturedRequest {
        fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.as_str())
        }
    }

    /// 启动一次性本地 mock：读入请求并捕获，然后返回固定响应
    fn spawn_mock(
        status: u16,
        body: &'static str,
    ) -> (SocketAddr, Arc<Mutex<Option<CapturedRequest>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock");
        let addr = listener.local_addr().expect("mock addr");
        let captured: Arc<Mutex<Option<CapturedRequest>>> = Arc::new(Mutex::new(None));
        let captured_clone = captured.clone();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buf = Vec::new();
            let mut chunk = [0u8; 1024];
            loop {
                let n = stream.read(&mut chunk).expect("read request");
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let text = String::from_utf8_lossy(&buf);
            let mut lines = text.split("\r\n");
            let request_line = lines.next().unwrap_or_default().to_string();
            let headers: Vec<(String, String)> = lines
                .by_ref()
                .take_while(|l| !l.is_empty())
                .filter_map(|l| {
                    let (k, v) = l.split_once(": ")?;
                    Some((k.to_string(), v.to_string()))
                })
                .collect();
            // POST multipart 会在头之后继续推 body；不读完客户端可能还在写。
            if let Some(len) = headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("Content-Length"))
                .and_then(|(_, v)| v.parse::<usize>().ok())
            {
                let header_end = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap_or(0) + 4;
                while buf.len().saturating_sub(header_end) < len {
                    let n = stream.read(&mut chunk).expect("read body");
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }
            }
            *captured_clone.lock().unwrap() = Some(CapturedRequest {
                request_line,
                headers,
            });
            let resp = format!(
                "HTTP/1.1 {status} MOCK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(resp.as_bytes()).expect("write response");
            stream.flush().expect("flush");
        });
        (addr, captured)
    }

    fn test_provider(addr: SocketAddr) -> QiniuProvider {
        let cred = QiniuCredential::new("test-ak", "test-sk").unwrap();
        let base = format!("http://{addr}");
        QiniuProvider::with_all_endpoints(cred, &base, &base, &base, &base).unwrap()
    }

    fn tokio() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn list_buckets_request_and_parsing() {
        let (addr, captured) = spawn_mock(200, r#"["bucket-a","bucket-b"]"#);
        let provider = test_provider(addr);
        let buckets = tokio().block_on(provider.list_buckets()).unwrap();

        let req = captured.lock().unwrap().take().unwrap();
        assert!(
            req.request_line.starts_with("GET /buckets HTTP/1.1"),
            "request_line={}",
            req.request_line
        );
        assert_eq!(req.header("Host"), Some(addr.to_string().as_str()));

        // 签名数据构造必须与实际请求逐字节一致（HMAC 原语正确性由官方向量测试保证）
        let expected_sign_data = sign::build_v2_sign_data(
            "GET",
            "/buckets",
            None,
            "127.0.0.1",
            Some(addr.port()),
            None,
            &[],
        );
        let cred = QiniuCredential::new("test-ak", "test-sk").unwrap();
        assert_eq!(
            req.header("Authorization"),
            Some(sign::authorization_v2(&cred, &expected_sign_data).as_str())
        );

        assert_eq!(buckets.len(), 2);
        assert_eq!(buckets[0].name, "bucket-a");
        assert_eq!(buckets[0].kind, ProviderKind::Qiniu);
    }

    #[test]
    fn list_objects_request_encoding_signing_and_parsing() {
        let (addr, captured) = spawn_mock(
            200,
            r#"{"marker":"m2","items":[{"key":"photos/a.jpg","hash":"FrUEPCjaNnQ2fBlHnHuoBQfMgJZf","fsize":123,"mimeType":"image/jpeg","putTime":15910878169036950,"type":0,"status":0}],"commonPrefixes":["photos/2024/"]}"#,
        );
        let provider = test_provider(addr);
        let request = ListObjectsRequest {
            bucket: "demo".into(),
            prefix: Some("photos/".into()),
            delimiter: None,
            marker: None,
            limit: 100,
            region: None,
        };
        let page = tokio().block_on(provider.list_objects(request)).unwrap();

        let req = captured.lock().unwrap().take().unwrap();
        // query 值必须被编码（"/" → %2F），且签名使用同一原始串
        let expected_query = "bucket=demo&prefix=photos%2F&limit=100";
        assert!(
            req.request_line
                .starts_with(&format!("GET /list?{expected_query} HTTP/1.1")),
            "request_line={}",
            req.request_line
        );
        let expected_sign_data = sign::build_v2_sign_data(
            "GET",
            "/list",
            Some(expected_query),
            "127.0.0.1",
            Some(addr.port()),
            None,
            &[],
        );
        let cred = QiniuCredential::new("test-ak", "test-sk").unwrap();
        assert_eq!(
            req.header("Authorization"),
            Some(sign::authorization_v2(&cred, &expected_sign_data).as_str())
        );

        // 响应解析：对象 + 公共前缀 + putTime 换算（100ns → ms）
        assert_eq!(page.entries.len(), 2);
        match &page.entries[0] {
            ListingEntry::Object(o) => {
                assert_eq!(o.key, "photos/a.jpg");
                assert_eq!(o.size, 123);
                assert_eq!(o.mime_type.as_deref(), Some("image/jpeg"));
                assert_eq!(o.etag.as_deref(), Some("FrUEPCjaNnQ2fBlHnHuoBQfMgJZf"));
                assert_eq!(o.put_time_millis, 1_591_087_816_903);
            }
            other => panic!("第一项应为对象，实际 {other:?}"),
        }
        assert_eq!(
            page.entries[1],
            ListingEntry::CommonPrefix("photos/2024/".into())
        );
        assert_eq!(page.next_marker.as_deref(), Some("m2"));
        assert!(page.has_more());
    }

    #[test]
    fn list_objects_validates_input() {
        let (addr, _captured) = spawn_mock(200, "[]");
        let provider = test_provider(addr);
        let rt = tokio();

        let empty_bucket = rt.block_on(provider.list_objects(ListObjectsRequest::new("")));
        assert!(matches!(empty_bucket, Err(StorageError::InvalidInput(_))));

        let bad_limit = rt.block_on(provider.list_objects(ListObjectsRequest {
            bucket: "demo".into(),
            limit: 1001,
            ..ListObjectsRequest::new("demo")
        }));
        assert!(matches!(bad_limit, Err(StorageError::InvalidInput(_))));
    }

    #[test]
    fn error_mapping_401_and_631() {
        // 401 → Auth
        let (addr, _c) = spawn_mock(401, r#"{"error":"bad token"}"#);
        let provider = test_provider(addr);
        let err = tokio().block_on(provider.list_buckets()).unwrap_err();
        assert!(
            matches!(err, StorageError::Auth(ref m) if m.contains("bad token")),
            "err={err}"
        );

        // 631（七牛非标准状态码）→ Api
        let (addr, _c) = spawn_mock(631, r#"{"error":"no such bucket"}"#);
        let provider = test_provider(addr);
        let err = tokio().block_on(provider.list_buckets()).unwrap_err();
        assert!(
            matches!(err, StorageError::Api { status: 631, ref message } if message.contains("no such bucket")),
            "err={err}"
        );
    }

    /// 真实凭证冒烟测试：
    /// `QINIU_ACCESS_KEY=xxx QINIU_SECRET_KEY=yyy cargo test -p object-storage-qiniu -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "需要真实凭证：环境变量 QINIU_ACCESS_KEY / QINIU_SECRET_KEY"]
    async fn live_list_buckets_and_objects() {
        let access_key = std::env::var("QINIU_ACCESS_KEY").expect("未设置 QINIU_ACCESS_KEY");
        let secret_key = std::env::var("QINIU_SECRET_KEY").expect("未设置 QINIU_SECRET_KEY");
        let cred = QiniuCredential::new(access_key, secret_key).unwrap();
        let provider = QiniuProvider::new(cred);

        let buckets = provider.list_buckets().await.expect("list_buckets");
        println!(
            "buckets: {:?}",
            buckets.iter().map(|b| &b.name).collect::<Vec<_>>()
        );

        if let Some(first) = buckets.first() {
            let page = provider
                .list_objects(ListObjectsRequest::new(first.name.clone()))
                .await
                .expect("list_objects");
            println!(
                "entries: {}, next_marker: {:?}",
                page.entries.len(),
                page.next_marker
            );
            for entry in page.entries.iter().take(5) {
                println!("  {entry:?}");
            }
        }
    }

    /// 下载全链路（mock）：域名接口返回下载服务器地址 → 下载 URL path 转义、
    /// e/token 参数齐全且无 Authorization 头 → 分块落盘字节数与内容一致。
    #[test]
    fn download_to_file_streams_and_signs_url() {
        let body: &str = "hello 下载内容 body";
        // 先绑下载源服务器，才能把它写进域名接口的响应体（'static 约束用 Box::leak）
        let (addr_b, captured_b) = spawn_mock(200, body);
        let domains_body: &str =
            Box::leak(format!("[\"127.0.0.1:{}\"]", addr_b.port()).into_boxed_str());
        let (addr_a, captured_a) = spawn_mock(200, domains_body);

        let uc = format!("http://{addr_a}");
        let rsf = format!("http://{addr_b}");
        let cred = QiniuCredential::new("test-ak", "test-sk").unwrap();
        let mut provider = QiniuProvider::with_endpoints(cred, &uc, &rsf).unwrap();
        provider.set_download_use_https(false);

        let dir = std::env::temp_dir().join(format!(
            "cloudstorage-dl-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("b c.txt");

        let (total, dl_req) = tokio().block_on(async {
            let total = provider
                .download_object_to_file("b1", "a/b c.txt", &dest, None)
                .await
                .unwrap();
            (total, captured_b.lock().unwrap().take().unwrap())
        });

        assert_eq!(total, body.len() as u64);
        assert_eq!(std::fs::read(&dest).unwrap(), body.as_bytes());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let e_value: u64 = dl_req
            .request_line
            .split("?e=")
            .nth(1)
            .and_then(|rest| rest.split('&').next())
            .and_then(|v| v.parse().ok())
            .expect("e 参数必须存在且为数字");
        assert!(
            e_value > now && e_value <= now + 7200,
            "e 应在 [now, now+3600] 附近，实际 {e_value} vs {now}"
        );

        assert!(
            dl_req.request_line.starts_with("GET /a/b%20c.txt?e="),
            "path 必须按 path 规则转义且保留 /，实际: {}",
            dl_req.request_line
        );
        assert!(
            dl_req.request_line.contains("&token=test-ak:"),
            "token 必须在 URL 上（下载签名不走 Authorization 头），实际: {}",
            dl_req.request_line
        );
        assert!(
            dl_req.header("Authorization").is_none(),
            "下载请求不应携带 Authorization 头"
        );

        // 域名请求：UC V2 签名走 Authorization 头
        let req_a = captured_a.lock().unwrap().take().unwrap();
        assert!(req_a.request_line.starts_with("GET /v2/domains?tbl=b1"));
        assert!(
            req_a
                .header("Authorization")
                .expect("域名请求必须 V2 签名")
                .starts_with("Qiniu test-ak:")
        );

        std::fs::remove_dir_all(dir).unwrap();
    }

    /// 对象不存在（612）：先于落盘报错，不产生半成品文件
    #[test]
    fn download_missing_object_fails_without_partial_file() {
        let (addr_b, _captured_b) = spawn_mock(612, r#"{"error":"no such file"}"#);
        let domains_body: &str =
            Box::leak(format!("[\"127.0.0.1:{}\"]", addr_b.port()).into_boxed_str());
        let (addr_a, _captured_a) = spawn_mock(200, domains_body);

        let uc = format!("http://{addr_a}");
        let rsf = format!("http://{addr_b}");
        let cred = QiniuCredential::new("test-ak", "test-sk").unwrap();
        let mut provider = QiniuProvider::with_endpoints(cred, &uc, &rsf).unwrap();
        provider.set_download_use_https(false);

        let dir = std::env::temp_dir().join(format!(
            "cloudstorage-dl404-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("out.bin");

        let err = tokio()
            .block_on(provider.download_object_to_file("b1", "k", &dest, None))
            .unwrap_err();
        match &err {
            StorageError::Api { status, message } => {
                assert_eq!(*status, 612);
                assert!(message.contains("no such file"), "实际: {message}");
            }
            other => panic!("应报 Api 错误，实际 {other:?}"),
        }
        assert!(!dest.exists(), "612 时不应创建本地文件");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn download_rejects_empty_key_before_network() {
        let provider = test_provider("127.0.0.1:9".parse().unwrap());
        let err = tokio()
            .block_on(provider.download_object_to_file(
                "b1",
                "",
                &std::env::temp_dir().join("nope"),
                None,
            ))
            .unwrap_err();
        assert!(matches!(err, StorageError::InvalidInput(_)), "实际 {err:?}");
    }

    #[test]
    fn upload_form_posts_multipart_without_authorization_header() {
        let (addr, captured) = spawn_mock(200, r#"{"hash":"etag","key":"dir/a.bin"}"#);
        let provider = test_provider(addr);
        let dir = std::env::temp_dir().join(format!(
            "cloudstorage-up-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("a.bin");
        std::fs::write(&src, b"hello-upload").unwrap();

        let n = tokio()
            .block_on(provider.upload_object_from_file("b1", "dir/a.bin", &src, None))
            .unwrap();
        assert_eq!(n, 12);

        let req = captured.lock().unwrap().take().unwrap();
        assert!(
            req.request_line.starts_with("POST / HTTP/1.1")
                || req.request_line.starts_with("POST / HTTP/1.0"),
            "request_line={}",
            req.request_line
        );
        let ct = req.header("Content-Type").unwrap_or("");
        assert!(
            ct.starts_with("multipart/form-data"),
            "Content-Type 应为 multipart，实际 {ct}"
        );
        assert!(
            req.header("Authorization").is_none(),
            "表单上传凭证在 body 的 token 字段，不应再带 Authorization"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn upload_rejects_empty_key_before_network() {
        let provider = test_provider("127.0.0.1:9".parse().unwrap());
        let err = tokio()
            .block_on(provider.upload_object_from_file("b1", "", Path::new("/tmp/x"), None))
            .unwrap_err();
        assert!(matches!(err, StorageError::InvalidInput(_)), "实际 {err:?}");
    }

    #[test]
    fn delete_object_posts_encoded_entry_with_v2_auth() {
        let (addr, captured) = spawn_mock(200, r#"{}"#);
        let provider = test_provider(addr);
        tokio()
            .block_on(provider.delete_object("b1", "a/b"))
            .unwrap();

        let req = captured.lock().unwrap().take().unwrap();
        let entry = sign::encoded_entry("b1", "a/b");
        assert!(
            req.request_line
                .starts_with(&format!("POST /delete/{entry} ")),
            "request_line={}",
            req.request_line
        );
        let auth = req.header("Authorization").expect("删除必须 V2 签名");
        assert!(auth.starts_with("Qiniu test-ak:"), "实际 {auth}");
        let expected = sign::authorization_v2_for_url(
            &QiniuCredential::new("test-ak", "test-sk").unwrap(),
            "POST",
            &format!("http://{addr}/delete/{entry}").parse().unwrap(),
        );
        assert_eq!(auth, expected);
    }

    #[test]
    fn delete_rejects_empty_key_before_network() {
        let provider = test_provider("127.0.0.1:9".parse().unwrap());
        let err = tokio()
            .block_on(provider.delete_object("b1", ""))
            .unwrap_err();
        assert!(matches!(err, StorageError::InvalidInput(_)), "实际 {err:?}");
    }

    #[test]
    fn region_query_parses_up_domains_and_caches() {
        // UC mock 返回官方 /v4/query 结构（新字段名 hosts + preferred domains）
        let body = r#"{
            "hosts": [
                {
                    "region": "z1",
                    "ttl": 3600,
                    "up": {"domains": ["up-z1.qiniup.com", "upload-z1.qiniup.com"]},
                    "io": {"domains": ["iovip-z1.qiniuio.com"]},
                    "uc": {"domains": ["ucqbox.com"]},
                    "rs": {"domains": ["rs-z1.qbox.me"]},
                    "rsf": {"domains": ["rsf-z1.qbox.me"]},
                    "api": {"domains": ["api-z1.qiniuapi.com"]},
                    "s3": {"domains": ["s3-z1.qiniuapi.com"]}
                }
            ]
        }"#;
        let (addr, captured) = spawn_mock(200, body);
        let provider = test_provider(addr);

        // 测试模式 is_test_mode 挡住真实解析；这里直接调 query_up_host 验证解析
        let (host, ttl) = tokio().block_on(provider.query_up_host("b1")).unwrap();
        assert_eq!(
            host, "up-z1.qiniup.com",
            "取 up.domains 第一个（preferred）"
        );
        assert_eq!(ttl, 3600);
        let req = captured.lock().unwrap().take().unwrap();
        assert!(
            req.request_line.starts_with("GET /v4/query?"),
            "request_line={}",
            req.request_line
        );
        assert!(
            req.request_line.contains("bucket=b1"),
            "请求必须带 bucket 参数"
        );
        assert!(
            req.request_line.contains("ak=test-ak"),
            "请求必须带 ak 参数"
        );
        assert!(
            req.header("Authorization").is_none(),
            "/v4/query 是公开接口，不带 Authorization"
        );

        // 缓存写入后命中（不再发请求）
        provider.cache_up_host("b1", "cached.example.com", 600);
        assert_eq!(
            provider.cached_up_host("b1").as_deref(),
            Some("cached.example.com")
        );
    }

    #[test]
    fn region_query_accepts_legacy_regions_field_and_default_ttl() {
        // 旧字段名 regions + 无 ttl（官方 serde alias + default_ttl 语义）
        let body = r#"{"regions": [{"up": {"domains": ["up-z0.qiniup.com"]}}]}"#;
        let (addr, _captured) = spawn_mock(200, body);
        let provider = test_provider(addr);
        let (host, ttl) = tokio().block_on(provider.query_up_host("b1")).unwrap();
        assert_eq!(host, "up-z0.qiniup.com");
        assert_eq!(ttl, 86_400);
    }

    #[test]
    fn region_query_error_response_fails_cleanly() {
        // 631 空间不存在：check_status 报 Api 错误（不静默吞掉）
        let (addr, _captured) = spawn_mock(631, r#"{"error":"no such bucket"}"#);
        let provider = test_provider(addr);
        let err = tokio().block_on(provider.query_up_host("b1")).unwrap_err();
        assert!(matches!(err, StorageError::Api { .. }), "实际 {err:?}");
    }

    #[test]
    fn signed_get_url_queries_domains_and_embeds_token() {
        let (addr, captured) = spawn_mock(200, r#"["cdn.example.com"]"#);
        let provider = test_provider(addr);
        let url = tokio()
            .block_on(provider.signed_get_url("b1", "a b/中文.png", 3600))
            .unwrap();
        assert!(
            url.starts_with("https://cdn.example.com/a%20b/%E4%B8%AD%E6%96%87.png?e="),
            "url={url}"
        );
        assert!(url.contains("&token=test-ak:"), "url={url}");
        assert!(!url.contains(' '), "url={url}");
        assert!(!url.contains("中文"), "url={url}");
        let req = captured.lock().unwrap().take().unwrap();
        assert!(
            req.request_line.starts_with("GET /v2/domains?tbl=b1 "),
            "request_line={}",
            req.request_line
        );
    }

    #[test]
    fn signed_get_url_rejects_zero_ttl() {
        let provider = test_provider("127.0.0.1:9".parse().unwrap());
        let err = tokio()
            .block_on(provider.signed_get_url("b1", "a", 0))
            .unwrap_err();
        assert!(matches!(err, StorageError::InvalidInput(_)), "实际 {err:?}");
    }
}
