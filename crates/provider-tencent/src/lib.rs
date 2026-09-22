//! 腾讯云 COS Provider
//!
//! 签名：COS 签名 v5（`Authorization: q-sign-algorithm=sha1&...`，见 `sign` 模块）。
//! 列举空间走全局入口 `GET https://service.cos.myqcloud.com/`（返回每个桶的
//! `<Location>` 即地域）；对象操作走 virtual-hosted
//! `https://{bucket}.cos.{region}.myqcloud.com/{key}`。
//!
//! 两个与其它服务商不同、且必须显式处理的事实：
//!
//! - **Bucket 名必须带 APPID 后缀**（`examplebucket-1250000000`）。COS 的桶标识由
//!   `<名称>-<APPID>` 组成，只写名称会得到 DNS 解析失败或 404，报错完全看不出原因，
//!   所以这里在上网前就用 [`bucket_name_error`] 挡住。
//! - **对象操作必须有地域**。地域来自 ListBuckets 的 `Location`；缺失时回退去
//!   service 端点查一次（见 [`TencentProvider::resolve_region`]），仍拿不到才报错——
//!   蒙一个地域会返回极具误导性的 `SignatureDoesNotMatch`。
//!
//! 测试可用 [`TencentProvider::with_endpoint`] 指向本地 mock（切 path-style，
//! 并跳过 bucket 名校验与地域查询）。

mod sign;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use futures_util::StreamExt;
use object_storage_core::{ByteProgress, StorageError, StorageProvider};
use object_storage_domain::{
    Bucket, CloudObject, ListObjectsRequest, ListingEntry, ObjectPage, ProviderKind,
};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub use sign::TencentCredential;

/// 列举空间用全局入口：`GET /` 返回账号下**全部地域**的桶，且带 `Location`
const DEFAULT_SERVICE_ENDPOINT: &str = "https://service.cos.myqcloud.com";

/// 单次 ListBuckets / ListObjects 的条数上限（COS 定义即 1000，也是默认值）
const MAX_KEYS: u32 = 1000;

/// PUT Object（简单上传）的对象大小上限：5 GB，超过必须走分块上传
/// （本项目明确不做分块上传，故这里 Fail Fast 而不是默默失败）
const MAX_SIMPLE_UPLOAD_BYTES: u64 = 5 * 1024 * 1024 * 1024;

/// 请求头签名的有效期。签名只在请求发出时校验一次，30 分钟足够宽松；
/// 服务端另外容忍 ±15 分钟时钟偏移（`RequestTimeTooSkewed`）。
const REQUEST_SIGNATURE_TTL: Duration = Duration::from_secs(1800);

/// 测试模式下的占位地域（path-style 请求不真正使用它）
const TEST_REGION: &str = "test-region";

pub struct TencentProvider {
    http: reqwest::Client,
    download_http: reqwest::Client,
    upload_http: reqwest::Client,
    cred: TencentCredential,
    service_base: reqwest::Url,
    /// bucket → 地域。UI 正常情况下每次都从空间列表带上地域，这里只兜底
    /// 「手填 Bucket 名」那条路径，避免每次都去查 service 端点。
    region_cache: Mutex<HashMap<String, String>>,
    /// 测试：所有请求打到 service_base（path-style），跳过 bucket 名校验与地域查询
    test_mode: bool,
}

impl TencentProvider {
    pub fn new(cred: TencentCredential) -> Self {
        Self::with_endpoint(cred, DEFAULT_SERVICE_ENDPOINT).expect("内置端点 URL 必然合法")
    }

    pub fn with_endpoint(
        cred: TencentCredential,
        service_base: &str,
    ) -> Result<Self, StorageError> {
        let service_base = reqwest::Url::parse(service_base)
            .map_err(|e| StorageError::InvalidInput(format!("COS 端点不合法: {e}")))?;
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
        // 下载/上传共用一条长连接客户端：不设总超时（大对象传输耗时不可预估）
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
            region_cache: Mutex::new(HashMap::new()),
            test_mode,
        })
    }

    /// service 端点用于签名的 host（带端口；reqwest 发的 Host 头也带端口）
    fn service_host(&self) -> Result<String, StorageError> {
        let host = self
            .service_base
            .host_str()
            .ok_or_else(|| StorageError::InvalidInput("COS 端点缺少 host".into()))?;
        Ok(match self.service_base.port() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_string(),
        })
    }

    /// 对象请求的 Host：virtual-hosted `{bucket}.cos.{region}.myqcloud.com`
    /// （测试模式回落 service 端点，走 path-style）
    fn bucket_host(&self, bucket: &str, region: &str) -> Result<String, StorageError> {
        if self.test_mode {
            return self.service_host();
        }
        Ok(format!("{bucket}.cos.{region}.myqcloud.com"))
    }

    /// 对象 URL。**签名用的路径与它不同**：签名要用解码后的路径（见 [`Self::signing_path`]）。
    fn object_url(
        &self,
        bucket: &str,
        region: &str,
        key: &str,
    ) -> Result<reqwest::Url, StorageError> {
        let mut url = if self.test_mode {
            self.service_base.clone()
        } else {
            reqwest::Url::parse(&format!("https://{bucket}.cos.{region}.myqcloud.com"))
                .map_err(|e| StorageError::InvalidInput(format!("对象 URL 不合法: {e}")))?
        };
        let encoded = sign::percent_encode_path(key);
        let path = if key.is_empty() {
            format!("/{bucket}")
        } else if self.test_mode {
            format!("/{bucket}/{encoded}")
        } else {
            format!("/{encoded}")
        };
        url.set_path(&path);
        url.set_query(None);
        Ok(url)
    }

    /// 参与签名的 UriPathname：**解码后的原始 UTF-8 路径**，不含 bucket。
    /// 用线上那个百分号编码形式去签会得到一个服务端算不出的 HttpString。
    fn signing_path(key: &str) -> String {
        if key.is_empty() {
            "/".to_string()
        } else {
            format!("/{key}")
        }
    }

    /// 组装某次请求的 `Authorization` 头值
    fn sign_request(
        &self,
        method: &str,
        host: &str,
        path: &str,
        params: &[(String, String)],
        extra_headers: &[(&str, &str)],
    ) -> Result<String, StorageError> {
        let key_time = sign::key_time(sign::unix_now()?, REQUEST_SIGNATURE_TTL);
        let mut headers = sign::host_header(host);
        headers.extend(
            extra_headers
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string())),
        );
        Ok(sign::authorization(
            &self.cred, method, path, params, &headers, &key_time,
        ))
    }

    /// 解析 Bucket 的地域。
    ///
    /// 优先级：调用方带的地域 → 进程内缓存 → 查一次 service 端点（ListBuckets
    /// 返回每个桶的 `Location`）→ 报可操作的错。**不蒙默认地域**：地域错了服务端
    /// 只会回 `SignatureDoesNotMatch`，用户完全无从判断。
    async fn resolve_region(
        &self,
        bucket: &str,
        provided: Option<&str>,
    ) -> Result<String, StorageError> {
        if let Some(region) = provided.filter(|s| !s.is_empty()) {
            self.cache_region(bucket, region);
            return Ok(region.to_string());
        }
        if self.test_mode {
            return Ok(TEST_REGION.to_string());
        }
        if let Some(cached) = self.cached_region(bucket) {
            return Ok(cached);
        }
        let found = self
            .list_buckets()
            .await?
            .into_iter()
            .find(|b| b.name == bucket)
            .and_then(|b| b.region.filter(|r| !r.is_empty()));
        match found {
            Some(region) => {
                self.cache_region(bucket, &region);
                Ok(region)
            }
            None => Err(StorageError::InvalidInput(format!(
                "无法确定 Bucket `{bucket}` 的地域。请在侧栏重新加载空间列表后再试；\
                 若账号无 cos:GetService 权限，可在「输入 Bucket 名称」时一并填写地域\
                 （如 ap-beijing）。腾讯云 COS 的对象操作需要地域来构造访问域名。"
            ))),
        }
    }

    fn cached_region(&self, bucket: &str) -> Option<String> {
        // 锁毒化（持锁线程 panic）直接响报，不静默（agents.md §5.5）
        self.region_cache
            .lock()
            .expect("region_cache 锁毒化")
            .get(bucket)
            .cloned()
    }

    fn cache_region(&self, bucket: &str, region: &str) {
        self.region_cache
            .lock()
            .expect("region_cache 锁毒化")
            .insert(bucket.to_string(), region.to_string());
    }

    /// 会话级地域回填：AppServices 每次操作都新建 provider 实例，实例内缓存
    /// 活不过一次操作；列表阶段拿到的地域由它带给新实例（否则每个下载/上传/
    /// 删除都要再查一次 service 端点，无 `cos:GetService` 权限的账号直接卡死）。
    pub fn seed_region(&mut self, bucket: &str, region: &str) {
        self.cache_region(bucket, region);
    }

    /// 某个 bucket 当前缓存的地域（观测/测试用）。
    pub fn region_of(&self, bucket: &str) -> Option<String> {
        self.cached_region(bucket)
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
            "[tencent] {context} HTTP {code} body={}",
            truncate(&body, 800)
        );
        let cos_code = xml_first(&body, "Code").unwrap_or_default();
        let message = xml_first(&body, "Message")
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                if code == 403 && body.contains("403 Forbidden") {
                    return "COS 拒绝访问，通常是存储桶权限、地域或账号授权不匹配".into();
                }
                if cos_code.is_empty() {
                    truncate(&body, 300)
                } else {
                    cos_code.clone()
                }
            });
        if cos_code == "SignatureDoesNotMatch" || cos_code == "InvalidAccessKeyId" {
            // RequestId 是排查 COS 签名问题时唯一能拿给服务端的凭据
            let mut detail = format!("{context}: [{cos_code}] {message}");
            if let Some(rid) = xml_first(&body, "RequestId") {
                detail.push_str(&format!("；RequestId={rid}"));
            }
            return Err(StorageError::Auth(detail));
        }
        if cos_code == "NoSuchBucket" {
            return Err(StorageError::Api {
                status: code,
                message: format!("{context}: 存储桶不存在，或不属于当前账号（{message}）"),
            });
        }
        if cos_code == "NoSuchKey" {
            return Err(StorageError::Api {
                status: code,
                message: format!("{context}: 对象不存在（{message}）"),
            });
        }
        if cos_code == "AccessDenied" || code == 403 {
            if context == "list_buckets" {
                return Err(StorageError::InvalidInput(
                    "无法自动列举空间（当前账号无 cos:GetService 权限）。请填写有权限的存储桶名称"
                        .into(),
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
        // COS 核心对象 API 的限流是 503 SlowDown，**不是** 429
        if code == 503 || code == 429 || cos_code == "SlowDown" {
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

    /// Bucket 名的形状校验（`<名称>-<APPID>`）。
    /// 测试模式的 mock 用 `b1` 这类短名，故跳过——与「测试模式跳过地域查询」同理；
    /// 规则本身由 `bucket_name_requires_appid_suffix` 直接钉在纯函数上。
    fn require_bucket_shape(&self, bucket: &str) -> Result<(), StorageError> {
        if self.test_mode {
            return Ok(());
        }
        match bucket_name_error(bucket) {
            Some(message) => Err(StorageError::InvalidInput(message)),
            None => Ok(()),
        }
    }

    /// 空 bucket / 空 key 一律先于网络拒绝（下载/上传/删除/签名共用）
    fn require_bucket_and_key(&self, bucket: &str, key: &str) -> Result<(), StorageError> {
        if bucket.is_empty() {
            return Err(StorageError::InvalidInput("bucket 不能为空".into()));
        }
        if key.is_empty() {
            return Err(StorageError::InvalidInput("key 不能为空".into()));
        }
        self.require_bucket_shape(bucket)
    }
}

impl std::fmt::Debug for TencentProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TencentProvider")
            .field("cred", &self.cred)
            .field("service_base", &self.service_base.as_str())
            .field("test_mode", &self.test_mode)
            .finish()
    }
}

/// COS 的桶标识是 `<BucketName>-<APPID>`（如 `examplebucket-1250000000`）。
/// 名称段只允许小写字母/数字/中划线，APPID 段必须是纯数字。
///
/// 只写名称（`mybucket`）时服务端返回的是 DNS 解析失败或 404，看不出原因，
/// 所以这里在上网前就给出可操作的报错。
pub(crate) fn bucket_name_error(bucket: &str) -> Option<String> {
    let valid = match bucket.rsplit_once('-') {
        Some((name, appid)) => {
            !name.is_empty()
                && !appid.is_empty()
                && appid.bytes().all(|b| b.is_ascii_digit())
                && name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        }
        None => false,
    };
    (!valid).then(|| {
        format!(
            "存储桶名 `{bucket}` 不合法：腾讯云 COS 的存储桶标识是 `<名称>-<APPID>`\
             （例如 examplebucket-1250000000），请填写带 APPID 后缀的完整名称"
        )
    })
}

impl StorageProvider for TencentProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Tencent
    }

    async fn list_buckets(&self) -> Result<Vec<Bucket>, StorageError> {
        let host = self.service_host()?;
        let mut buckets = Vec::new();
        let mut marker: Option<String> = None;
        // COS 的 ListBuckets 也分页（Marker/NextMarker/IsTruncated），需取完为止
        loop {
            let mut params: Vec<(String, String)> = Vec::new();
            if let Some(m) = marker.as_ref() {
                params.push(("marker".into(), m.clone()));
            }
            params.push(("max-keys".into(), MAX_KEYS.to_string()));
            let auth = self.sign_request("GET", &host, "/", &params, &[])?;
            let mut url = self.service_base.clone();
            url.set_path("/");
            url.set_query(Some(&encode_query(&params)));
            let resp = self
                .http
                .get(url)
                .header(AUTHORIZATION, auth)
                .send()
                .await
                .map_err(|e| StorageError::Network(format!("list_buckets: {e}")))?;
            let resp = self.check_status(resp, "list_buckets").await?;
            let text = text_or_invalid(resp, "list_buckets").await?;
            for block in xml_blocks(&text, "Bucket") {
                let Some(name) = xml_first(&block, "Name") else {
                    continue;
                };
                buckets.push(Bucket {
                    name,
                    kind: ProviderKind::Tencent,
                    region: xml_first(&block, "Location").filter(|s| !s.is_empty()),
                });
            }
            let truncated =
                xml_first(&text, "IsTruncated").is_some_and(|v| v.eq_ignore_ascii_case("true"));
            let next = xml_first(&text, "NextMarker").filter(|s| !s.is_empty());
            match next {
                // 截断但没给新 marker，或 marker 没能前进：停下，避免死循环
                Some(next) if truncated && Some(&next) != marker.as_ref() => marker = Some(next),
                _ => break,
            }
        }
        Ok(buckets)
    }

    async fn list_objects(&self, request: ListObjectsRequest) -> Result<ObjectPage, StorageError> {
        if request.bucket.is_empty() {
            return Err(StorageError::InvalidInput("bucket 不能为空".into()));
        }
        if request.limit == 0 || request.limit > MAX_KEYS {
            return Err(StorageError::InvalidInput(format!(
                "limit 必须在 1..={MAX_KEYS} 之间"
            )));
        }
        self.require_bucket_shape(&request.bucket)?;
        let region = self
            .resolve_region(&request.bucket, request.region.as_deref())
            .await?;
        let host = self.bucket_host(&request.bucket, &region)?;
        let mut url = self.object_url(&request.bucket, &region, "")?;

        let mut params: Vec<(String, String)> = Vec::new();
        if let Some(prefix) = request.prefix.as_ref().filter(|s| !s.is_empty()) {
            params.push(("prefix".into(), prefix.clone()));
        }
        if let Some(delimiter) = request.delimiter.as_ref().filter(|s| !s.is_empty()) {
            params.push(("delimiter".into(), delimiter.clone()));
        }
        if let Some(marker) = request.marker.as_ref().filter(|s| !s.is_empty()) {
            params.push(("marker".into(), marker.clone()));
        }
        params.push(("max-keys".into(), request.limit.to_string()));
        url.set_query(Some(&encode_query(&params)));

        let auth = self.sign_request("GET", &host, &Self::signing_path(""), &params, &[])?;
        let resp = self
            .http
            .get(url)
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
        self.require_bucket_and_key(bucket, key)?;
        let region = self.resolve_region(bucket, None).await?;
        let host = self.bucket_host(bucket, &region)?;
        let url = self.object_url(bucket, &region, key)?;
        let auth = self.sign_request("GET", &host, &Self::signing_path(key), &[], &[])?;
        let resp = self
            .download_http
            .get(url)
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
        self.require_bucket_and_key(bucket, key)?;
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
        if file_len > MAX_SIMPLE_UPLOAD_BYTES {
            return Err(StorageError::InvalidInput(format!(
                "upload_object: 文件 {:.1} GB 超过腾讯云 COS 简单上传的 5 GB 上限，\
                 本项目暂不支持分块上传",
                file_len as f64 / (1024.0 * 1024.0 * 1024.0)
            )));
        }
        let file = tokio::fs::File::open(source).await.map_err(|e| {
            StorageError::Io(format!(
                "upload_object: 打开本地文件 {} 失败: {e}",
                source.display()
            ))
        })?;
        let region = self.resolve_region(bucket, None).await?;
        let host = self.bucket_host(bucket, &region)?;
        let url = self.object_url(bucket, &region, key)?;
        let content_type = "application/octet-stream";
        // 只签 host + content-type：签得越少越不容易因值与实际发送不一致而炸
        let auth = self.sign_request(
            "PUT",
            &host,
            &Self::signing_path(key),
            &[],
            &[("content-type", content_type)],
        )?;
        if let Some(cb) = &progress {
            cb(0, Some(file_len));
        }
        // COS 允许分块传输（Transfer-Encoding: chunked），故无需预先声明长度
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
        self.require_bucket_and_key(bucket, key)?;
        let region = self.resolve_region(bucket, None).await?;
        let host = self.bucket_host(bucket, &region)?;
        let url = self.object_url(bucket, &region, key)?;
        let auth = self.sign_request("DELETE", &host, &Self::signing_path(key), &[], &[])?;
        let resp = self
            .http
            .delete(url)
            .header(AUTHORIZATION, auth)
            .send()
            .await
            .map_err(|e| StorageError::Network(format!("delete_object: {e}")))?;
        // 删除不存在的对象 COS 返回 204（幂等），无需特判
        let _resp = self.check_status(resp, "delete_object").await?;
        Ok(())
    }

    async fn signed_get_url(
        &self,
        bucket: &str,
        key: &str,
        ttl_secs: u64,
    ) -> Result<String, StorageError> {
        self.require_bucket_and_key(bucket, key)?;
        if ttl_secs == 0 {
            return Err(StorageError::InvalidInput("ttl_secs 必须大于 0".into()));
        }
        let region = self.resolve_region(bucket, None).await?;
        let host = self.bucket_host(bucket, &region)?;
        let start = sign::unix_now()?;
        let key_time = sign::key_time(start, Duration::from_secs(ttl_secs));
        // 预签名：同一套签名算法，只是把七个 q-* 参数从 Authorization 头挪到 query
        let authorization = sign::authorization(
            &self.cred,
            "GET",
            &Self::signing_path(key),
            &[],
            &sign::host_header(&host),
            &key_time,
        );
        let mut url = self.object_url(bucket, &region, key)?;
        url.set_query(Some(&sign::encode_authorization_query(&authorization)));
        Ok(url.to_string())
    }
}

/// 把签名用的（解码后）参数编码成线上 query 串。
/// 顺序不必与签名一致（签名内部自己排序），但**编码必须一致**，
/// 否则服务端收到的值与 HttpString 里的对不上。
fn encode_query(params: &[(String, String)]) -> String {
    params
        .iter()
        .map(|(k, v)| format!("{}={}", sign::sign_encode(k), sign::sign_encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

async fn text_or_invalid(resp: reqwest::Response, context: &str) -> Result<String, StorageError> {
    resp.text()
        .await
        .map_err(|e| StorageError::InvalidResponse(format!("{context}: 读取响应失败: {e}")))
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

/// `2024-06-04T16:29:00.000Z` / `2024-06-04T16:29:00Z` → Unix 毫秒。
/// COS 的 `LastModified` 两种精度都出现过，秒与小数秒都要吃下。
fn parse_rfc3339_millis(value: &str) -> i64 {
    let trimmed = value.trim();
    if trimmed.len() < 19 {
        return 0;
    }
    let parse_u = |s: &str| s.parse::<i64>().unwrap_or(0);
    let year = parse_u(&trimmed[0..4]);
    let month = parse_u(&trimmed[5..7]);
    let day = parse_u(&trimmed[8..10]);
    let hour = parse_u(&trimmed[11..13]);
    let min = parse_u(&trimmed[14..16]);
    let sec = parse_u(&trimmed[17..19]);
    let millis = trimmed
        .get(19..)
        .and_then(|rest| rest.strip_prefix('.'))
        .map(|frac| parse_u(&frac.chars().take(3).collect::<String>()))
        .unwrap_or(0);
    days_from_civil(year, month, day) * 86_400_000 + (hour * 3600 + min * 60 + sec) * 1000 + millis
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
    let truncated = xml_first(text, "IsTruncated").is_some_and(|v| v.eq_ignore_ascii_case("true"));
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
            // ListObjects 不返回 Content-Type，由 UI 按扩展名推断
            mime_type: None,
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
        // COS 在截断时必定返回 NextMarker；万一缺失，从最后一项兜底合成
        xml_first(text, "NextMarker")
            .filter(|s| !s.is_empty())
            .or_else(|| {
                entries.last().map(|e| match e {
                    ListingEntry::Object(o) => o.key.clone(),
                    ListingEntry::CommonPrefix(p) => p.clone(),
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

    /// 按顺序服务 `responses` 里每个响应（每个一次连接，响应带 Connection: close，
    /// 故客户端下个请求会重新建连）。用于 ListBuckets 这类需要多轮的用例。
    fn spawn_mock_seq(
        responses: Vec<(u16, String)>,
    ) -> (SocketAddr, Arc<Mutex<Vec<CapturedRequest>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock");
        let addr = listener.local_addr().expect("mock addr");
        let captured: Arc<Mutex<Vec<CapturedRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let captured_clone = captured.clone();
        thread::spawn(move || {
            for (status, body) in responses {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
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
                let text = String::from_utf8_lossy(&buf).to_string();
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
                // 分块上传没有 Content-Length，要靠 Transfer-Encoding 判断体长；
                // 这里只需读到请求头，body 由响应直接关连接丢弃
                captured_clone.lock().unwrap().push(CapturedRequest {
                    request_line,
                    headers,
                });
                let resp = format!(
                    "HTTP/1.1 {status} MOCK\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(resp.as_bytes());
                let _ = stream.flush();
            }
        });
        (addr, captured)
    }

    fn spawn_mock(status: u16, body: &str) -> (SocketAddr, Arc<Mutex<Vec<CapturedRequest>>>) {
        spawn_mock_seq(vec![(status, body.to_string())])
    }

    fn test_provider(addr: SocketAddr) -> TencentProvider {
        let cred = TencentCredential::new("test-id", "test-key").unwrap();
        TencentProvider::with_endpoint(cred, &format!("http://{addr}")).unwrap()
    }

    fn tokio() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    fn object_request(bucket: &str) -> ListObjectsRequest {
        ListObjectsRequest {
            bucket: bucket.into(),
            prefix: None,
            delimiter: Some("/".into()),
            marker: None,
            limit: 100,
            region: None,
        }
    }

    #[test]
    fn list_buckets_parses_location_and_signs_v5() {
        let body = r#"<ListAllMyBucketsResult><Owner><ID>qcs::cam::uin/100000000001:uin/100000000001</ID></Owner><IsTruncated>false</IsTruncated><Buckets><Bucket><Name>assets-1250000000</Name><Location>ap-shanghai</Location><CreationDate>2019-05-24T10:56:40Z</CreationDate></Bucket></Buckets></ListAllMyBucketsResult>"#;
        let (addr, captured) = spawn_mock(200, body);
        let provider = test_provider(addr);
        let buckets = tokio().block_on(provider.list_buckets()).unwrap();
        assert_eq!(buckets.len(), 1);
        assert_eq!(buckets[0].name, "assets-1250000000");
        assert_eq!(buckets[0].kind, ProviderKind::Tencent);
        assert_eq!(buckets[0].region.as_deref(), Some("ap-shanghai"));

        let captured = captured.lock().unwrap();
        let req = &captured[0];
        assert_eq!(req.request_line, "GET /?max-keys=1000 HTTP/1.1");
        let auth = req.header("Authorization").expect("必须带 COS 签名");
        assert!(
            auth.starts_with("q-sign-algorithm=sha1&q-ak=test-id&"),
            "{auth}"
        );
        assert!(auth.contains("q-header-list=host&"), "{auth}");
        assert!(auth.contains("q-url-param-list=max-keys&"), "{auth}");
    }

    /// ListBuckets 分页：第一页截断 → 带 marker 再请求 → 合并两页
    #[test]
    fn list_buckets_follows_pagination() {
        let page1 = r#"<ListAllMyBucketsResult><IsTruncated>true</IsTruncated><NextMarker>page-2</NextMarker><Buckets><Bucket><Name>a-1250000000</Name><Location>ap-beijing</Location></Bucket></Buckets></ListAllMyBucketsResult>"#;
        let page2 = r#"<ListAllMyBucketsResult><IsTruncated>false</IsTruncated><Buckets><Bucket><Name>b-1250000000</Name><Location>ap-guangzhou</Location></Bucket></Buckets></ListAllMyBucketsResult>"#;
        let (addr, captured) =
            spawn_mock_seq(vec![(200, page1.to_string()), (200, page2.to_string())]);
        let provider = test_provider(addr);
        let buckets = tokio().block_on(provider.list_buckets()).unwrap();
        assert_eq!(buckets.len(), 2);
        assert_eq!(buckets[1].name, "b-1250000000");
        assert_eq!(buckets[1].region.as_deref(), Some("ap-guangzhou"));
        let captured = captured.lock().unwrap();
        assert!(
            captured[1].request_line.contains("marker=page-2"),
            "第二页必须带上 NextMarker：{}",
            captured[1].request_line
        );
    }

    #[test]
    fn list_objects_parses_contents_and_prefixes() {
        let body = r#"<ListBucketResult>
            <Name>assets-1250000000</Name>
            <Prefix/>
            <IsTruncated>false</IsTruncated>
            <Contents><Key>a.txt</Key><Size>12</Size><ETag>"abc"</ETag><LastModified>2024-06-04T16:29:00.000Z</LastModified></Contents>
            <CommonPrefixes><Prefix>dir/</Prefix></CommonPrefixes>
        </ListBucketResult>"#;
        let (addr, captured) = spawn_mock(200, body);
        let provider = test_provider(addr);
        let page = tokio()
            .block_on(provider.list_objects(object_request("b1")))
            .unwrap();
        assert_eq!(page.entries.len(), 2);
        match &page.entries[0] {
            ListingEntry::Object(o) => {
                assert_eq!(o.key, "a.txt");
                assert_eq!(o.size, 12);
                assert_eq!(o.etag.as_deref(), Some("abc"));
                assert_eq!(o.put_time_millis, 1_717_518_540_000);
            }
            other => panic!("期望对象，实际 {other:?}"),
        }
        assert!(matches!(&page.entries[1], ListingEntry::CommonPrefix(p) if p == "dir/"));
        assert!(!page.has_more());

        let captured = captured.lock().unwrap();
        let (line, req) = (captured[0].request_line.clone(), &captured[0]);
        assert!(line.contains("delimiter=%2F"), "line={line}");
        assert!(line.contains("max-keys=100"), "line={line}");
        assert!(line.contains("GET /b1?"), "line={line}");
        let auth = req.header("Authorization").unwrap();
        assert!(
            auth.contains("q-url-param-list=delimiter;max-keys&"),
            "{auth}"
        );
    }

    /// 前缀与 marker 原样进签名：中文前缀要按 UTF-8 逐字节编码
    #[test]
    fn list_objects_encodes_unicode_prefix() {
        let body = r#"<ListBucketResult><IsTruncated>false</IsTruncated></ListBucketResult>"#;
        let (addr, captured) = spawn_mock(200, body);
        let provider = test_provider(addr);
        let mut request = object_request("b1");
        request.prefix = Some("目录/".into());
        tokio().block_on(provider.list_objects(request)).unwrap();
        let captured = captured.lock().unwrap();
        assert!(
            captured[0]
                .request_line
                .contains("prefix=%E7%9B%AE%E5%BD%95%2F"),
            "line={}",
            captured[0].request_line
        );
    }

    #[test]
    fn list_objects_truncated_returns_next_marker() {
        let body = r#"<ListBucketResult><IsTruncated>true</IsTruncated><NextMarker>a.txt</NextMarker><Contents><Key>a.txt</Key><Size>1</Size></Contents></ListBucketResult>"#;
        let (addr, _captured) = spawn_mock(200, body);
        let provider = test_provider(addr);
        let page = tokio()
            .block_on(provider.list_objects(object_request("b1")))
            .unwrap();
        assert_eq!(page.next_marker.as_deref(), Some("a.txt"));
        assert!(page.has_more());
    }

    #[test]
    fn list_objects_validates_limit_and_bucket() {
        let provider = test_provider("127.0.0.1:9".parse().unwrap());
        for limit in [0, 1001] {
            let mut request = object_request("b1");
            request.limit = limit;
            let err = tokio()
                .block_on(provider.list_objects(request))
                .unwrap_err();
            assert!(
                matches!(err, StorageError::InvalidInput(_)),
                "limit={limit} 实际 {err:?}"
            );
        }
        let mut empty = object_request("");
        empty.limit = 10;
        let err = tokio().block_on(provider.list_objects(empty)).unwrap_err();
        assert!(matches!(err, StorageError::InvalidInput(_)), "实际 {err:?}");
    }

    #[test]
    fn upload_sends_signed_put_with_content_type() {
        let dir = std::env::temp_dir().join(format!("cos-upload-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("payload.bin");
        std::fs::write(&file, b"hello cos").unwrap();

        let (addr, captured) = spawn_mock(200, "");
        let provider = test_provider(addr);
        let bytes = tokio()
            .block_on(provider.upload_object_from_file("b1", "a/b.bin", &file, None))
            .unwrap();
        assert_eq!(bytes, 9);

        let captured = captured.lock().unwrap();
        let req = &captured[0];
        assert!(
            req.request_line.starts_with("PUT /b1/a/b.bin "),
            "request_line={}",
            req.request_line
        );
        assert_eq!(req.header("Content-Type"), Some("application/octet-stream"));
        // 只签 host + content-type，不签 date / content-length
        let auth = req.header("Authorization").unwrap();
        assert!(
            auth.contains("q-header-list=content-type;host&"),
            "auth={auth}"
        );
        assert!(!auth.contains("content-length"), "auth={auth}");
        // 签名必须与实际发送的 content-type 一致
        let expected = sign::authorization(
            &TencentCredential::new("test-id", "test-key").unwrap(),
            "PUT",
            "/a/b.bin",
            &[],
            &[
                ("host".to_string(), format!("127.0.0.1:{}", addr.port())),
                (
                    "content-type".to_string(),
                    "application/octet-stream".to_string(),
                ),
            ],
            key_time_of(auth),
        );
        assert_eq!(auth, expected, "签名内容对不上");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 从 `q-sign-time=` 里取回 KeyTime（断言里要按同一时间窗重算签名）
    fn key_time_of(authorization: &str) -> &str {
        let start =
            authorization.find("&q-sign-time=").expect("含 q-sign-time") + "&q-sign-time=".len();
        let rest = &authorization[start..];
        let end = start + rest.find("&q-key-time=").expect("含 q-key-time");
        &authorization[start..end]
    }

    #[test]
    fn upload_rejects_oversize_before_network() {
        let dir = std::env::temp_dir().join(format!("cos-oversize-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("big.bin");
        // 造一个 sparse 文件，只声明大小，不真写 5 GB
        let f = std::fs::File::create(&file).unwrap();
        f.set_len(MAX_SIMPLE_UPLOAD_BYTES + 1).unwrap();
        drop(f);

        let provider = test_provider("127.0.0.1:9".parse().unwrap());
        let err = tokio()
            .block_on(provider.upload_object_from_file("b1", "big.bin", &file, None))
            .unwrap_err();
        match err {
            StorageError::InvalidInput(m) => assert!(m.contains("5 GB"), "实际: {m}"),
            other => panic!("应报 InvalidInput，实际 {other:?}"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn download_streams_to_file() {
        let body = "file-content";
        let (addr, captured) = spawn_mock(200, body);
        let provider = test_provider(addr);
        let dest = std::env::temp_dir().join(format!("cos-download-{}", std::process::id()));
        let bytes = tokio()
            .block_on(provider.download_object_to_file("b1", "a/b.txt", &dest, None))
            .unwrap();
        assert_eq!(bytes, body.len() as u64);
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), body);

        let captured = captured.lock().unwrap();
        assert!(
            captured[0].request_line.starts_with("GET /b1/a/b.txt "),
            "request_line={}",
            captured[0].request_line
        );
        let auth = captured[0].header("Authorization").unwrap();
        assert!(auth.contains("q-header-list=host&"), "auth={auth}");
        std::fs::remove_file(&dest).ok();
    }

    #[test]
    fn download_missing_object_cleans_partial_file() {
        let body = r#"<Error><Code>NoSuchKey</Code><Message>The specified key does not exist.</Message><RequestId>rid-1</RequestId></Error>"#;
        let (addr, _captured) = spawn_mock(404, body);
        let provider = test_provider(addr);
        let dest = std::env::temp_dir().join(format!("cos-missing-{}", std::process::id()));
        let err = tokio()
            .block_on(provider.download_object_to_file("b1", "gone.txt", &dest, None))
            .unwrap_err();
        match err {
            StorageError::Api { status, message } => {
                assert_eq!(status, 404);
                assert!(message.contains("对象不存在"), "实际: {message}");
            }
            other => panic!("应报 Api 错误，实际 {other:?}"),
        }
        assert!(!dest.exists(), "失败后不应留下半成品文件");
    }

    #[test]
    fn access_denied_reports_actionable_message() {
        let body =
            "<html><head><title>403 Forbidden</title></head><body>403 Forbidden</body></html>";
        let (addr, _captured) = spawn_mock(403, body);
        let provider = test_provider(addr);
        let dest = std::env::temp_dir().join(format!("cos-forbidden-{}", std::process::id()));
        let err = tokio()
            .block_on(provider.download_object_to_file("b1", "k", &dest, None))
            .unwrap_err();
        match err {
            StorageError::Api { status, message } => {
                assert_eq!(status, 403);
                assert!(message.contains("COS 拒绝访问"), "实际: {message}");
                assert!(!message.contains("<html>"), "不应把 HTML 原样展示给用户");
            }
            other => panic!("应报 Api 错误，实际 {other:?}"),
        }
    }

    /// COS 的限流码是 503 SlowDown（不是 429）——映射必须落到 RateLimited
    #[test]
    fn slow_down_maps_to_rate_limited() {
        let body = r#"<Error><Code>SlowDown</Code><Message>Please reduce your request rate.</Message></Error>"#;
        let (addr, _captured) = spawn_mock(503, body);
        let provider = test_provider(addr);
        let err = tokio()
            .block_on(provider.delete_object("b1", "k"))
            .unwrap_err();
        assert!(matches!(err, StorageError::RateLimited(_)), "实际 {err:?}");
    }

    #[test]
    fn signature_mismatch_maps_to_auth_with_request_id() {
        let body = r#"<Error><Code>SignatureDoesNotMatch</Code><Message>The signature does not match.</Message><RequestId>rid-42</RequestId></Error>"#;
        let (addr, _captured) = spawn_mock(403, body);
        let provider = test_provider(addr);
        let err = tokio()
            .block_on(provider.delete_object("b1", "k"))
            .unwrap_err();
        match err {
            StorageError::Auth(message) => {
                assert!(message.contains("SignatureDoesNotMatch"), "实际: {message}");
                assert!(message.contains("rid-42"), "应带上 RequestId：{message}");
            }
            other => panic!("应报 Auth 错误，实际 {other:?}"),
        }
    }

    #[test]
    fn list_buckets_denied_suggests_manual_bucket() {
        let body = r#"<Error><Code>AccessDenied</Code><Message>Access Denied.</Message></Error>"#;
        let (addr, _captured) = spawn_mock(403, body);
        let provider = test_provider(addr);
        let err = tokio().block_on(provider.list_buckets()).unwrap_err();
        match err {
            StorageError::InvalidInput(message) => {
                assert!(message.contains("cos:GetService"), "实际: {message}");
            }
            other => panic!("应报 InvalidInput，实际 {other:?}"),
        }
    }

    #[test]
    fn delete_sends_signed_delete() {
        let (addr, captured) = spawn_mock(204, "");
        let provider = test_provider(addr);
        tokio()
            .block_on(provider.delete_object("b1", "a/b"))
            .unwrap();
        let captured = captured.lock().unwrap();
        assert!(
            captured[0].request_line.starts_with("DELETE /b1/a/b "),
            "request_line={}",
            captured[0].request_line
        );
        let auth = captured[0].header("Authorization").unwrap();
        assert!(auth.contains("q-header-list=host&"), "auth={auth}");
    }

    #[test]
    fn empty_bucket_or_key_rejected_before_network() {
        let provider = test_provider("127.0.0.1:9".parse().unwrap());
        assert!(matches!(
            tokio().block_on(provider.delete_object("b1", "")),
            Err(StorageError::InvalidInput(_))
        ));
        assert!(matches!(
            tokio().block_on(provider.delete_object("", "k")),
            Err(StorageError::InvalidInput(_))
        ));
    }

    #[test]
    fn signed_get_url_puts_signature_in_query() {
        let provider = test_provider("127.0.0.1:9".parse().unwrap());
        let url = tokio()
            .block_on(provider.signed_get_url("b1", "a b/中文.png", 3600))
            .unwrap();
        assert!(
            url.contains("/b1/a%20b/%E4%B8%AD%E6%96%87.png?"),
            "url={url}"
        );
        assert!(url.contains("q-sign-algorithm=sha1"), "url={url}");
        assert!(url.contains("q-ak=test-id"), "url={url}");
        assert!(url.contains("q-header-list=host&"), "url={url}");
        // KeyTime 的分号必须编码，否则 query 会被多切一刀
        assert!(url.contains("q-sign-time="), "url={url}");
        assert!(!url.contains(";"), "分号必须编码成 %3B：{url}");
        assert!(url.contains("%3B"), "分号必须编码成 %3B：{url}");
        assert!(!url.contains(' '), "url={url}");
    }

    #[test]
    fn signed_get_url_rejects_zero_ttl() {
        let provider = test_provider("127.0.0.1:9".parse().unwrap());
        let err = tokio()
            .block_on(provider.signed_get_url("b1", "a", 0))
            .unwrap_err();
        assert!(matches!(err, StorageError::InvalidInput(_)), "实际 {err:?}");
    }

    /// 会话级地域回填：seed_region 写入的缓存要在新实例构建对象请求时可用
    /// （不再查 service 端点）——resolve_region 的优先级里缓存先于 list_buckets。
    #[test]
    fn seed_region_populates_cache_used_by_resolve() {
        let provider = test_provider("127.0.0.1:9".parse().unwrap());
        assert!(provider.cached_region("b1").is_none());
        let mut provider = provider;
        provider.seed_region("b1", "ap-beijing");
        assert_eq!(provider.cached_region("b1").as_deref(), Some("ap-beijing"));
    }

    #[test]
    fn bucket_name_requires_appid_suffix() {
        assert!(bucket_name_error("examplebucket-1250000000").is_none());
        assert!(bucket_name_error("a-b-c-1250000000").is_none());
        for bad in [
            "examplebucket",
            "my-bucket",
            "MyBucket-125",
            "bucket-",
            "-125",
        ] {
            let message =
                bucket_name_error(bad).unwrap_or_else(|| panic!("`{bad}` 应当被判为不合法"));
            assert!(message.contains("APPID"), "`{bad}` 的提示: {message}");
        }
    }

    #[test]
    fn parse_rfc3339_known_instants() {
        assert_eq!(
            parse_rfc3339_millis("2024-06-04T16:29:00.000Z"),
            1_717_518_540_000
        );
        assert_eq!(
            parse_rfc3339_millis("2024-06-04T16:29:00Z"),
            1_717_518_540_000
        );
        assert_eq!(
            parse_rfc3339_millis("2019-05-24T10:56:40Z"),
            1_558_695_400_000
        );
        assert_eq!(parse_rfc3339_millis("bogus"), 0);
    }

    /// 需要真实凭证的联网用例：`TENCENT_SECRET_ID` / `TENCENT_SECRET_KEY`
    /// 用法见 README。默认 ignore（与七牛那条一致）。
    #[tokio::test]
    #[ignore]
    async fn live_list_buckets_and_objects() {
        let (Ok(id), Ok(key)) = (
            std::env::var("TENCENT_SECRET_ID"),
            std::env::var("TENCENT_SECRET_KEY"),
        ) else {
            panic!("需要 TENCENT_SECRET_ID / TENCENT_SECRET_KEY 环境变量");
        };
        let provider = TencentProvider::new(TencentCredential::new(id, key).unwrap());
        let buckets = provider.list_buckets().await.expect("list_buckets");
        for bucket in &buckets {
            println!("{} ({:?})", bucket.name, bucket.region);
        }
        let Some(first) = buckets.first() else {
            println!("账号下没有存储桶，跳过对象列举");
            return;
        };
        let mut request = ListObjectsRequest::new(&first.name);
        request.delimiter = Some("/".into());
        request.region = first.region.clone();
        let page = provider.list_objects(request).await.expect("list_objects");
        println!(
            "{} 条对象，has_more={}",
            page.entries.len(),
            page.has_more()
        );
    }
}
