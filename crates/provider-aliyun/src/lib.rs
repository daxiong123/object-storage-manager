//! Aliyun OSS Provider
//!
//! 签名：OSS Signature V1（Header `Authorization: OSS AK:sig`）。
//! 列举空间走 `GET /`（GetService）；对象操作走 virtual-hosted
//! `https://{bucket}.{location}.aliyuncs.com/{key}`。
//! 测试可用 [`AliyunProvider::with_endpoint`] 指向本地 mock（path-style）。

mod sign;

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::StreamExt;
use object_storage_core::{ByteProgress, StorageError, StorageProvider};
use object_storage_domain::{
    Bucket, CloudObject, ListObjectsRequest, ListingEntry, ObjectPage, ProviderKind,
};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, DATE, HeaderName};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub use sign::AliyunCredential;

/// 列举空间用地域入口（GetService 在各 region 均可列出账号下全部 Bucket）
const DEFAULT_SERVICE_ENDPOINT: &str = "https://oss-cn-hangzhou.aliyuncs.com";
const DEFAULT_LOCATION: &str = "oss-cn-hangzhou";

pub struct AliyunProvider {
    http: reqwest::Client,
    download_http: reqwest::Client,
    upload_http: reqwest::Client,
    cred: AliyunCredential,
    service_base: reqwest::Url,
    /// 测试：所有请求打到 service_base（path-style），跳过 GetBucketLocation
    test_mode: bool,
}

impl AliyunProvider {
    pub fn new(cred: AliyunCredential) -> Self {
        Self::with_endpoint(cred, DEFAULT_SERVICE_ENDPOINT).expect("内置端点 URL 必然合法")
    }

    pub fn with_endpoint(cred: AliyunCredential, service_base: &str) -> Result<Self, StorageError> {
        let service_base = reqwest::Url::parse(service_base)
            .map_err(|e| StorageError::InvalidInput(format!("OSS 端点不合法: {e}")))?;
        let test_mode = service_base.host_str() == Some("127.0.0.1")
            || service_base.host_str() == Some("localhost");
        let http = reqwest::Client::builder()
            .user_agent(concat!("CloudStorage/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .http1_only()
            .use_rustls_tls()
            .build()
            .map_err(|e| StorageError::Network(format!("HTTP Client 构建失败: {e}")))?;
        let long_http = reqwest::Client::builder()
            .user_agent(concat!("CloudStorage/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .http1_only()
            .use_rustls_tls()
            .build()
            .map_err(|e| StorageError::Network(format!("HTTP Client 构建失败: {e}")))?;
        Ok(Self {
            http,
            download_http: long_http.clone(),
            upload_http: long_http,
            cred,
            service_base,
            test_mode,
        })
    }

    /// 同时发送 `Date` 与 `x-oss-date`（同一 GMT 字符串）。
    /// 实测 OSS 服务端 StringToSign 的 Date 行仍填该时间，并且把 `x-oss-date`
    /// 列入 CanonicalizedOSSHeaders；Date 留空会 SignatureDoesNotMatch。
    fn signed_headers(
        &self,
        method: &str,
        content_type: &str,
        resource: &str,
    ) -> Result<(String, String), StorageError> {
        let date = sign::http_gmt(SystemTime::now())?;
        let oss_headers = format!("x-oss-date:{date}\n");
        let sts = sign::string_to_sign(method, "", content_type, &date, &oss_headers, resource);
        let auth = sign::authorization(&self.cred, &sts);
        Ok((date, auth))
    }

    async fn bucket_location(&self, bucket: &str) -> Result<String, StorageError> {
        if self.test_mode {
            return Ok(DEFAULT_LOCATION.to_string());
        }
        // path-style：Host 用地域标准入口（oss-cn-xxx.aliyuncs.com），
        // 不要用 `{bucket}.oss.aliyuncs.com`——OSS 会 403「Your host is invalid」。
        let mut url = self.service_base.clone();
        url.set_path(&format!("/{bucket}"));
        url.set_query(Some("location"));
        let resource = sign::canonicalized_resource(bucket, None, &[("location", "")]);
        let (date, auth) = self.signed_headers("GET", "", &resource)?;
        let resp = self
            .http
            .get(url)
            .header(DATE, date.clone())
            .header(HeaderName::from_static("x-oss-date"), date.clone())
            .header(AUTHORIZATION, auth)
            .send()
            .await
            .map_err(|e| StorageError::Network(format!("bucket_location: {e}")))?;
        let resp = self.check_status(resp, "bucket_location").await?;
        let text = text_or_invalid(resp, "bucket_location").await?;
        let loc = xml_first(&text, "LocationConstraint").unwrap_or_default();
        if loc.is_empty() {
            Ok(DEFAULT_LOCATION.to_string())
        } else if loc.starts_with("oss-") {
            Ok(loc)
        } else {
            Ok(format!("oss-{loc}"))
        }
    }

    fn object_url(
        &self,
        bucket: &str,
        location: &str,
        key: &str,
    ) -> Result<reqwest::Url, StorageError> {
        if self.test_mode {
            let mut url = self.service_base.clone();
            let path = if key.is_empty() {
                format!("/{bucket}")
            } else {
                format!("/{bucket}/{}", sign::percent_encode_path(key))
            };
            url.set_path(&path);
            url.set_query(None);
            return Ok(url);
        }
        let location = if location.is_empty() {
            DEFAULT_LOCATION
        } else {
            location
        };
        let host = format!("{bucket}.{location}.aliyuncs.com");
        let mut url = reqwest::Url::parse(&format!("https://{host}"))
            .map_err(|e| StorageError::InvalidInput(format!("对象 URL 不合法: {e}")))?;
        url.set_path(&format!("/{}", sign::percent_encode_path(key)));
        Ok(url)
    }

    async fn check_status(
        &self,
        resp: reqwest::Response,
        context: &str,
    ) -> Result<reqwest::Response, StorageError> {
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let code = status.as_u16();
        let body = resp.text().await.unwrap_or_default();
        eprintln!(
            "[aliyun] {context} HTTP {code} body={}",
            truncate(&body, 800)
        );
        let oss_code = xml_first(&body, "Code").unwrap_or_default();
        let message = xml_first(&body, "Message")
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                if oss_code.is_empty() {
                    truncate(&body, 300)
                } else {
                    oss_code.clone()
                }
            });
        if oss_code == "SignatureDoesNotMatch" || oss_code == "InvalidAccessKeyId" {
            let mut detail = format!("{context}: [{oss_code}] {message}");
            if let Some(sts) = xml_first(&body, "StringToSign") {
                detail.push_str(&format!("；服务端 StringToSign=[{sts}]"));
            }
            return Err(StorageError::Auth(detail));
        }
        if oss_code == "AccessDenied" || code == 403 {
            if message.contains("ListBuckets")
                || xml_first(&body, "AuthAction").as_deref() == Some("oss:ListBuckets")
            {
                return Err(StorageError::InvalidInput(
                    "无法自动列举空间（RAM 无 oss:ListBuckets）。请填写有权限的 Bucket 名称".into(),
                ));
            }
            return Err(StorageError::Api {
                status: code,
                message: format!("{context}: {message}"),
            });
        }
        if code == 401 {
            return Err(StorageError::Auth(format!("{context}: {message}")));
        }
        if code == 503 || code == 429 {
            return Err(StorageError::RateLimited(format!("{context}: {message}")));
        }
        Err(StorageError::Api {
            status: code,
            message: format!("{context}: {message}"),
        })
    }

    async fn download_cleanup_failed(dest: &Path, cause: String) -> StorageError {
        match tokio::fs::remove_file(dest).await {
            Ok(()) => StorageError::Io(format!("download_object: {cause}；已清理半成品文件")),
            Err(rm) => StorageError::Io(format!(
                "download_object: {cause}；且清理半成品文件失败: {rm}（残留：{}）",
                dest.display()
            )),
        }
    }
}

impl std::fmt::Debug for AliyunProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AliyunProvider")
            .field("cred", &self.cred)
            .field("service_base", &self.service_base.as_str())
            .field("test_mode", &self.test_mode)
            .finish()
    }
}

impl StorageProvider for AliyunProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Aliyun
    }

    async fn list_buckets(&self) -> Result<Vec<Bucket>, StorageError> {
        let resource = sign::canonicalized_resource("", None, &[]);
        let (date, auth) = self.signed_headers("GET", "", &resource)?;
        let mut url = self.service_base.clone();
        url.set_path("/");
        url.set_query(None);
        let resp = self
            .http
            .get(url)
            .header(DATE, date.clone())
            .header(HeaderName::from_static("x-oss-date"), date.clone())
            .header(AUTHORIZATION, auth)
            .send()
            .await
            .map_err(|e| StorageError::Network(format!("list_buckets: {e}")))?;
        let resp = self.check_status(resp, "list_buckets").await?;
        let text = text_or_invalid(resp, "list_buckets").await?;
        let mut buckets = Vec::new();
        for block in xml_blocks(&text, "Bucket") {
            let Some(name) = xml_first(&block, "Name") else {
                continue;
            };
            let region = xml_first(&block, "Location");
            buckets.push(Bucket {
                name,
                kind: ProviderKind::Aliyun,
                region,
            });
        }
        Ok(buckets)
    }

    async fn list_objects(&self, request: ListObjectsRequest) -> Result<ObjectPage, StorageError> {
        if request.bucket.is_empty() {
            return Err(StorageError::InvalidInput("bucket 不能为空".into()));
        }
        if request.limit == 0 {
            return Err(StorageError::InvalidInput("limit 必须大于 0".into()));
        }
        let location = self.bucket_location(&request.bucket).await?;
        let mut url = self.object_url(&request.bucket, &location, "")?;
        let mut sub: Vec<(String, String)> = Vec::new();
        if let Some(prefix) = &request.prefix {
            sub.push(("prefix".into(), prefix.clone()));
        }
        if let Some(delimiter) = &request.delimiter {
            sub.push(("delimiter".into(), delimiter.clone()));
        }
        if let Some(marker) = &request.marker {
            sub.push(("marker".into(), marker.clone()));
        }
        sub.push(("max-keys".into(), request.limit.to_string()));
        let refs: Vec<(&str, &str)> = sub.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let resource = sign::canonicalized_resource(&request.bucket, None, &refs);
        let mut pairs: Vec<(&str, String)> = sub
            .iter()
            .map(|(k, v)| (k.as_str(), sign::percent_encode_path(v)))
            .collect();
        pairs.sort_by(|a, b| a.0.cmp(b.0));
        let qs = pairs
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("&");
        url.set_query(Some(&qs));
        let (date, auth) = self.signed_headers("GET", "", &resource)?;
        let resp = self
            .http
            .get(url)
            .header(DATE, date.clone())
            .header(HeaderName::from_static("x-oss-date"), date.clone())
            .header(AUTHORIZATION, auth)
            .send()
            .await
            .map_err(|e| StorageError::Network(format!("list_objects: {e}")))?;
        let resp = self.check_status(resp, "list_objects").await?;
        let text = text_or_invalid(resp, "list_objects").await?;
        parse_list_bucket(&text)
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
        let location = self.bucket_location(bucket).await?;
        let url = self.object_url(bucket, &location, key)?;
        let resource = sign::canonicalized_resource(bucket, Some(key), &[]);
        let (date, auth) = self.signed_headers("GET", "", &resource)?;
        let resp = self
            .download_http
            .get(url)
            .header(DATE, date.clone())
            .header(HeaderName::from_static("x-oss-date"), date.clone())
            .header(AUTHORIZATION, auth)
            .send()
            .await
            .map_err(|e| StorageError::Network(format!("download_object: {e}")))?;
        let resp = self.check_status(resp, "download_object").await?;
        let content_len = resp.content_length();
        let mut file = tokio::fs::File::create(dest).await.map_err(|e| {
            StorageError::Io(format!(
                "download_object: 创建本地文件 {} 失败: {e}",
                dest.display()
            ))
        })?;
        let mut total: u64 = 0;
        let mut stream = resp.bytes_stream();
        if let Some(cb) = &progress {
            cb(0, content_len);
        }
        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(c) => c,
                Err(e) => {
                    return Err(
                        Self::download_cleanup_failed(dest, format!("接收数据中断: {e}")).await,
                    );
                }
            };
            if let Err(e) = file.write_all(&chunk).await {
                return Err(Self::download_cleanup_failed(dest, format!("写入失败: {e}")).await);
            }
            total += chunk.len() as u64;
            if let Some(cb) = &progress {
                cb(total, content_len);
            }
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
        let location = self.bucket_location(bucket).await?;
        let url = self.object_url(bucket, &location, key)?;
        let resource = sign::canonicalized_resource(bucket, Some(key), &[]);
        let content_type = "application/octet-stream";
        let (date, auth) = self.signed_headers("PUT", content_type, &resource)?;
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
        let resp = self
            .upload_http
            .put(url)
            .header(DATE, date.clone())
            .header(HeaderName::from_static("x-oss-date"), date.clone())
            .header(CONTENT_TYPE, content_type)
            .header(AUTHORIZATION, auth)
            .body(reqwest::Body::wrap_stream(stream))
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
        let location = self.bucket_location(bucket).await?;
        let url = self.object_url(bucket, &location, key)?;
        let resource = sign::canonicalized_resource(bucket, Some(key), &[]);
        let (date, auth) = self.signed_headers("DELETE", "", &resource)?;
        let resp = self
            .http
            .delete(url)
            .header(DATE, date.clone())
            .header(HeaderName::from_static("x-oss-date"), date.clone())
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
        if bucket.is_empty() {
            return Err(StorageError::InvalidInput("bucket 不能为空".into()));
        }
        if key.is_empty() {
            return Err(StorageError::InvalidInput("key 不能为空".into()));
        }
        if ttl_secs == 0 {
            return Err(StorageError::InvalidInput("ttl_secs 必须大于 0".into()));
        }
        let location = self.bucket_location(bucket).await?;
        let mut url = self.object_url(bucket, &location, key)?;
        let expires = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| StorageError::InvalidInput(format!("signed_get_url: 系统时间异常: {e}")))?
            .as_secs()
            + ttl_secs;
        let resource = sign::canonicalized_resource(bucket, Some(key), &[]);
        let sts = sign::string_to_sign("GET", "", "", &expires.to_string(), "", &resource);
        let sig = sign::signature_only(&self.cred, &sts);
        let query = format!(
            "OSSAccessKeyId={}&Expires={expires}&Signature={}",
            sign::percent_encode_query(self.cred.access_key()),
            sign::percent_encode_query(&sig)
        );
        url.set_query(Some(&query));
        Ok(url.to_string())
    }
}

fn text_or_invalid(
    resp: reqwest::Response,
    context: &str,
) -> impl std::future::Future<Output = Result<String, StorageError>> {
    async move {
        resp.text()
            .await
            .map_err(|e| StorageError::InvalidResponse(format!("{context}: 读取响应失败: {e}")))
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n).collect::<String>())
    }
}

fn xml_unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

fn xml_first(hay: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = hay.find(&open)? + open.len();
    let end = hay[start..].find(&close)? + start;
    Some(xml_unescape(&hay[start..end]))
}

fn xml_blocks(hay: &str, tag: &str) -> Vec<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let mut out = Vec::new();
    let mut rest = hay;
    while let Some(i) = rest.find(&open) {
        let start = i + open.len();
        let Some(rel_end) = rest[start..].find(&close) else {
            break;
        };
        out.push(rest[start..start + rel_end].to_string());
        rest = &rest[start + rel_end + close.len()..];
    }
    out
}

fn parse_rfc3339_millis(value: &str) -> i64 {
    // 2024-06-04T16:29:00.000Z / 2024-06-04T16:29:00Z
    let trimmed = value.trim();
    let bytes = trimmed.as_bytes();
    if bytes.len() < 19 {
        return 0;
    }
    let parse_u = |s: &str| s.parse::<i64>().unwrap_or(0);
    let year = parse_u(&trimmed[0..4]);
    let month = parse_u(&trimmed[5..7]);
    let day = parse_u(&trimmed[8..10]);
    let hour = parse_u(&trimmed[11..13]);
    let min = parse_u(&trimmed[14..16]);
    let sec = parse_u(&trimmed[17..19]);
    days_from_civil(year, month, day) * 86_400_000 + (hour * 3600 + min * 60 + sec) * 1000
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn parse_list_bucket(text: &str) -> Result<ObjectPage, StorageError> {
    let truncated = xml_first(text, "IsTruncated")
        .map(|v| v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let next_marker = xml_first(text, "NextMarker").filter(|s| !s.is_empty());
    let mut entries = Vec::new();
    for block in xml_blocks(text, "Contents") {
        let Some(key) = xml_first(&block, "Key") else {
            continue;
        };
        let size = xml_first(&block, "Size")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let etag = xml_first(&block, "ETag").map(|s| s.trim_matches('"').to_string());
        let put_time_millis = xml_first(&block, "LastModified")
            .map(|s| parse_rfc3339_millis(&s))
            .unwrap_or(0);
        entries.push(ListingEntry::Object(CloudObject {
            key,
            size,
            mime_type: xml_first(&block, "Type"),
            etag,
            put_time_millis,
        }));
    }
    for prefix in xml_blocks(text, "CommonPrefixes") {
        if let Some(p) = xml_first(&prefix, "Prefix") {
            entries.push(ListingEntry::CommonPrefix(p));
        }
    }
    let next_marker = if truncated {
        next_marker.or_else(|| {
            entries.iter().rev().find_map(|e| match e {
                ListingEntry::Object(o) => Some(o.key.clone()),
                ListingEntry::CommonPrefix(p) => Some(p.clone()),
            })
        })
    } else {
        None
    };
    Ok(ObjectPage {
        entries,
        next_marker,
    })
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
                "HTTP/1.1 {status} MOCK\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(resp.as_bytes()).expect("write response");
            stream.flush().expect("flush");
        });
        (addr, captured)
    }

    fn test_provider(addr: SocketAddr) -> AliyunProvider {
        let cred = AliyunCredential::new("test-ak", "test-sk").unwrap();
        let base = format!("http://{addr}");
        AliyunProvider::with_endpoint(cred, &base).unwrap()
    }

    fn tokio() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn list_buckets_parses_xml_and_signs_v1() {
        let body = r#"<ListAllMyBucketsResult><Buckets><Bucket><Name>assets-prod</Name><Location>oss-cn-shanghai</Location></Bucket></Buckets></ListAllMyBucketsResult>"#;
        let (addr, captured) = spawn_mock(200, body);
        let provider = test_provider(addr);
        let buckets = tokio().block_on(provider.list_buckets()).unwrap();
        assert_eq!(buckets.len(), 1);
        assert_eq!(buckets[0].name, "assets-prod");
        assert_eq!(buckets[0].kind, ProviderKind::Aliyun);
        assert_eq!(buckets[0].region.as_deref(), Some("oss-cn-shanghai"));
        let req = captured.lock().unwrap().take().unwrap();
        assert!(req.request_line.starts_with("GET / "));
        let auth = req.header("Authorization").expect("必须带 OSS 签名");
        assert!(auth.starts_with("OSS test-ak:"));
        let date = req.header("x-oss-date").expect("必须带 x-oss-date");
        let expected = sign::authorization(
            &AliyunCredential::new("test-ak", "test-sk").unwrap(),
            &sign::string_to_sign("GET", "", "", date, &format!("x-oss-date:{date}\n"), "/"),
        );
        assert_eq!(auth, expected);
    }

    #[test]
    fn list_objects_parses_contents_and_prefixes() {
        let body = r#"<ListBucketResult>
            <IsTruncated>false</IsTruncated>
            <Contents><Key>a.txt</Key><Size>12</Size><ETag>"abc"</ETag><LastModified>2024-06-04T16:29:00.000Z</LastModified></Contents>
            <CommonPrefixes><Prefix>dir/</Prefix></CommonPrefixes>
        </ListBucketResult>"#;
        let (addr, captured) = spawn_mock(200, body);
        let provider = test_provider(addr);
        let page = tokio()
            .block_on(provider.list_objects(ListObjectsRequest {
                bucket: "b1".into(),
                prefix: None,
                delimiter: Some("/".into()),
                marker: None,
                limit: 100,
            }))
            .unwrap();
        assert_eq!(page.entries.len(), 2);
        match &page.entries[0] {
            ListingEntry::Object(o) => {
                assert_eq!(o.key, "a.txt");
                assert_eq!(o.size, 12);
                assert_eq!(o.etag.as_deref(), Some("abc"));
                assert!(o.put_time_millis > 0);
            }
            other => panic!("期望对象，实际 {other:?}"),
        }
        assert!(matches!(&page.entries[1], ListingEntry::CommonPrefix(p) if p == "dir/"));
        let req = captured.lock().unwrap().take().unwrap();
        assert!(
            req.request_line.contains("GET /b1?"),
            "request_line={}",
            req.request_line
        );
        assert!(req.request_line.contains("delimiter=/"));
        assert!(req.request_line.contains("max-keys=100"));
        let auth = req.header("Authorization").unwrap();
        let date = req.header("x-oss-date").unwrap();
        let resource =
            sign::canonicalized_resource("b1", None, &[("delimiter", "/"), ("max-keys", "100")]);
        let expected = sign::authorization(
            &AliyunCredential::new("test-ak", "test-sk").unwrap(),
            &sign::string_to_sign(
                "GET",
                "",
                "",
                date,
                &format!("x-oss-date:{date}\n"),
                &resource,
            ),
        );
        assert_eq!(auth, expected);
    }

    #[test]
    fn delete_object_sends_signed_delete() {
        let (addr, captured) = spawn_mock(204, "");
        let provider = test_provider(addr);
        tokio()
            .block_on(provider.delete_object("b1", "a/b"))
            .unwrap();
        let req = captured.lock().unwrap().take().unwrap();
        assert!(
            req.request_line.starts_with("DELETE /b1/a/b "),
            "request_line={}",
            req.request_line
        );
        let auth = req.header("Authorization").unwrap();
        let date = req.header("x-oss-date").unwrap();
        let resource = sign::canonicalized_resource("b1", Some("a/b"), &[]);
        let expected = sign::authorization(
            &AliyunCredential::new("test-ak", "test-sk").unwrap(),
            &sign::string_to_sign(
                "DELETE",
                "",
                "",
                date,
                &format!("x-oss-date:{date}\n"),
                &resource,
            ),
        );
        assert_eq!(auth, expected);
    }

    #[test]
    fn signed_get_url_embeds_expires_and_signature() {
        let provider = test_provider("127.0.0.1:9".parse().unwrap());
        let url = tokio()
            .block_on(provider.signed_get_url("b1", "a/b.png", 3600))
            .unwrap();
        assert!(url.contains("/b1/a/b.png?"), "url={url}");
        assert!(url.contains("OSSAccessKeyId=test-ak"));
        assert!(url.contains("Expires="));
        assert!(url.contains("Signature="));
    }

    #[test]
    fn delete_rejects_empty_key() {
        let provider = test_provider("127.0.0.1:9".parse().unwrap());
        let err = tokio()
            .block_on(provider.delete_object("b1", ""))
            .unwrap_err();
        assert!(matches!(err, StorageError::InvalidInput(_)));
    }

    #[test]
    fn parse_rfc3339_known_instant() {
        // 2024-06-04T16:29:00Z
        let millis = parse_rfc3339_millis("2024-06-04T16:29:00.000Z");
        assert_eq!(millis, 1_717_518_540_000);
    }
}
