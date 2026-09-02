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

use std::time::Duration;

use object_storage_core::{StorageError, StorageProvider};
use object_storage_domain::{
    Bucket, CloudObject, ListObjectsRequest, ListingEntry, ObjectPage, ProviderKind,
};
use reqwest::header::AUTHORIZATION;
use serde::Deserialize;

use sign::QiniuCredential;

/// 官方 UC 服务端点（空间管理）
const DEFAULT_UC_ENDPOINT: &str = "https://uc.qiniuapi.com";
/// 官方 RSF 服务端点（列举文件）
const DEFAULT_RSF_ENDPOINT: &str = "https://rsf.qbox.me";

pub struct QiniuProvider {
    http: reqwest::Client,
    cred: QiniuCredential,
    uc_base: reqwest::Url,
    rsf_base: reqwest::Url,
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
        let uc_base = reqwest::Url::parse(uc_base)
            .map_err(|e| StorageError::InvalidInput(format!("UC 端点不合法: {e}")))?;
        let rsf_base = reqwest::Url::parse(rsf_base)
            .map_err(|e| StorageError::InvalidInput(format!("RSF 端点不合法: {e}")))?;
        let http = reqwest::Client::builder()
            .user_agent(concat!("CloudStorage/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .use_rustls_tls()
            .build()
            .map_err(|e| StorageError::Network(format!("HTTP Client 构建失败: {e}")))?;
        Ok(Self {
            http,
            cred,
            uc_base,
            rsf_base,
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
            let headers = lines
                .by_ref()
                .take_while(|l| !l.is_empty())
                .filter_map(|l| {
                    let (k, v) = l.split_once(": ")?;
                    Some((k.to_string(), v.to_string()))
                })
                .collect();
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
        QiniuProvider::with_endpoints(cred, &base, &base).unwrap()
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
}
