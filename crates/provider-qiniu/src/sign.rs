//! 七牛签名算法
//!
//! 算法逐字节核对自官方 SDK 源码（qiniu-credential 0.2.4 `src/lib.rs`、
//! qiniu-http-client 0.2.4 `src/client/authorization.rs`），并内置官方测试向量：
//!
//! - HMAC-SHA1（SecretKey 为 key），URL 安全 Base64（**带 padding**，
//!   官方 `qiniu-utils::base64::urlsafe` = `URL_SAFE`，见官方向量尾部 `=`）
//! - V2 请求签名：sign 数据为 `"METHOD path[?query]\nHost: host[:port]\n"`，
//!   加上 `Content-Type` 行（如有）以及规范化为 `X-Qiniu-Xxx` Title-Case
//!   后按名称排序的 `X-Qiniu-*` 行，最后跟一个空行
//! - Authorization 头为 `Qiniu AK:sig`（老版 V1 `QBox` 凭证在需要时
//!   基于 `sign_token` 两行拼出，hello/world 官方向量已覆盖 HMAC 原语）
//!
//! 注意：签名必须使用**实际发送的**原始 query 串（与官方 SDK 行为一致），
//! Provider 层在设置 URL query 之后才取 `Url::query()` 参与签名。

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE as BASE64_URL_SAFE;
use hmac::{Hmac, Mac};
use object_storage_core::StorageError;
use sha1::Sha1;

type HmacSha1 = Hmac<Sha1>;

/// 七牛密钥对
///
/// `Debug` 永远不输出 SecretKey（防止日志泄露）。
#[derive(Clone, PartialEq, Eq)]
pub struct QiniuCredential {
    access_key: String,
    secret_key: String,
}

impl QiniuCredential {
    pub fn new(
        access_key: impl Into<String>,
        secret_key: impl Into<String>,
    ) -> Result<Self, StorageError> {
        let access_key = access_key.into();
        let secret_key = secret_key.into();
        if access_key.trim().is_empty() || secret_key.trim().is_empty() {
            return Err(StorageError::InvalidInput(
                "Qiniu AccessKey/SecretKey 不能为空".into(),
            ));
        }
        Ok(Self {
            access_key,
            secret_key,
        })
    }

    pub fn access_key(&self) -> &str {
        &self.access_key
    }
}

impl std::fmt::Debug for QiniuCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QiniuCredential")
            .field("access_key", &self.access_key)
            .field("secret_key", &"***")
            .finish()
    }
}

/// HMAC-SHA1 → `AK:sig`
fn sign_token(cred: &QiniuCredential, sign_data: &[u8]) -> String {
    let mut mac =
        HmacSha1::new_from_slice(cred.secret_key.as_bytes()).expect("HMAC-SHA1 接受任意长度 key");
    mac.update(sign_data);
    let sig = BASE64_URL_SAFE.encode(mac.finalize().into_bytes());
    format!("{}:{}", cred.access_key, sig)
}

/// 请求签名 V2：`Authorization: Qiniu AK:sig`
/// （老版 V1 `QBox` 管理凭证在需要时基于 `sign_token` 两行拼出即可）
pub(crate) fn authorization_v2(cred: &QiniuCredential, sign_data: &[u8]) -> String {
    format!("Qiniu {}", sign_token(cred, sign_data))
}

/// `X-Qiniu-` 头名称规范化：`x-qiniu-aaa` → `X-Qiniu-Aaa`
/// （对应官方 `make_header_name`：每个 `-` 后首字母大写，其余小写）
fn canonicalize_header_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut upper = true;
    for ch in name.chars() {
        if upper && ch.is_ascii_lowercase() {
            out.push(ch.to_ascii_uppercase());
        } else if !upper && ch.is_ascii_uppercase() {
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
        upper = ch == '-';
    }
    out
}

/// V2 sign 数据（对应官方 `_sign_request_v2_without_body` + Content-Type 分支）：
///
/// ```text
/// METHOD path[?query]\n
/// Host: host[:port]\n
/// [Content-Type: value\n]
/// [X-Qiniu-Name: value\n]*   ← 规范化后排序
/// \n
/// ```
pub(crate) fn build_v2_sign_data(
    method: &str,
    path: &str,
    query: Option<&str>,
    host: &str,
    port: Option<u16>,
    content_type: Option<&str>,
    x_qiniu_headers: &[(&str, &str)],
) -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(method.as_bytes());
    data.push(b' ');
    data.extend_from_slice(path.as_bytes());
    if let Some(q) = query.filter(|q| !q.is_empty()) {
        data.push(b'?');
        data.extend_from_slice(q.as_bytes());
    }
    data.extend_from_slice(b"\nHost: ");
    data.extend_from_slice(host.as_bytes());
    if let Some(port) = port {
        data.push(b':');
        data.extend_from_slice(port.to_string().as_bytes());
    }
    data.push(b'\n');
    if let Some(ct) = content_type {
        data.extend_from_slice(b"Content-Type: ");
        data.extend_from_slice(ct.as_bytes());
        data.push(b'\n');
    }
    let mut named: Vec<(String, &str)> = x_qiniu_headers
        .iter()
        .map(|(k, v)| (canonicalize_header_name(k), *v))
        .filter(|(k, _)| k.len() > "X-Qiniu-".len() && k.starts_with("X-Qiniu-"))
        .collect();
    named.sort_unstable();
    for (k, v) in named {
        data.extend_from_slice(format!("{k}: {v}\n").as_bytes());
    }
    data.push(b'\n');
    data
}

/// 对一个完整的请求 URL 做 V2 签名（GET 无 body 场景）
///
/// 必须在 URL query 最终确定后调用：签名使用 `Url::query()` 的原始串，
/// 与 reqwest 实际发送的内容保持一致。
pub(crate) fn authorization_v2_for_url(
    cred: &QiniuCredential,
    method: &str,
    url: &reqwest::Url,
) -> String {
    let sign_data = build_v2_sign_data(
        method,
        url.path(),
        url.query(),
        url.host_str().unwrap_or_default(),
        url.port(),
        None,
        &[],
    );
    authorization_v2(cred, &sign_data)
}

/// RFC 3986 query 值编码：保留 `A-Za-z0-9-._~`，其余按 `%XX` 大写十六进制编码。
/// （不用 form-urlencoded 的 `+` 空格，避免任何服务端解码歧义。）
pub(crate) fn percent_encode_query_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for &b in value.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 官方向量 1（qiniu-credential 0.2.4 `Credential::sign` doc-test）
    #[test]
    fn sign_token_matches_official_vector_hello() {
        let cred = QiniuCredential::new("abcdefghklmnopq", "1234567890").unwrap();
        assert_eq!(
            sign_token(&cred, b"hello"),
            "abcdefghklmnopq:b84KVc-LroDiz0ebUANfdzSRxa0="
        );
    }

    /// 官方向量 2（qiniu-credential 0.2.4 `Credential::sign_reader` doc-test）
    #[test]
    fn sign_token_matches_official_vector_world() {
        let cred = QiniuCredential::new("abcdefghklmnopq", "1234567890").unwrap();
        assert_eq!(
            sign_token(&cred, b"world"),
            "abcdefghklmnopq:VjgXt0P_nCxHuaTfiFz-UjDJ1AQ="
        );
    }

    /// 官方向量 3（qiniu-http-client 0.2.4 `test_credential_authorition_v2`）：
    /// X-Qiniu-* 头规范化 + 排序必须与官方逐字节一致
    #[test]
    fn sign_v2_matches_official_vector_with_x_qiniu_headers() {
        let cred = QiniuCredential::new("ak", "sk").unwrap();
        let sign_data = build_v2_sign_data(
            "GET",
            "/mkfile/sdf.jpg",
            None,
            "upload.qiniup.com",
            None,
            None,
            // 故意乱序传入，验证排序；小写传入，验证规范化
            &[("x-qiniu-bbb", "AAA"), ("x-qiniu-aaa", "CCC")],
        );
        assert_eq!(
            authorization_v2(&cred, &sign_data),
            "Qiniu ak:arPKqUn6T6DrnHhygbFS40PGBgY="
        );
    }

    #[test]
    fn v2_sign_data_shape_without_headers() {
        let data = build_v2_sign_data("GET", "/buckets", None, "uc.qiniuapi.com", None, None, &[]);
        assert_eq!(
            String::from_utf8(data).unwrap(),
            "GET /buckets\nHost: uc.qiniuapi.com\n\n"
        );
    }

    #[test]
    fn v2_sign_data_shape_with_port_and_query() {
        let data = build_v2_sign_data(
            "GET",
            "/list",
            Some("bucket=demo&limit=100"),
            "127.0.0.1",
            Some(8080),
            None,
            &[],
        );
        assert_eq!(
            String::from_utf8(data).unwrap(),
            "GET /list?bucket=demo&limit=100\nHost: 127.0.0.1:8080\n\n"
        );
    }

    #[test]
    fn bare_x_qiniu_header_is_excluded() {
        // 官方实现：规范化后 name 长度必须 > "X-Qiniu-".len()，裸 "X-Qiniu-" 不参与签名
        let with_bare = build_v2_sign_data(
            "GET",
            "/p",
            None,
            "h",
            None,
            None,
            &[("X-Qiniu-", "a"), ("x-qiniu-b", "B")],
        );
        let without_bare =
            build_v2_sign_data("GET", "/p", None, "h", None, None, &[("x-qiniu-b", "B")]);
        assert_eq!(with_bare, without_bare);
    }

    #[test]
    fn percent_encoding_rfc3986() {
        assert_eq!(percent_encode_query_value("photos/"), "photos%2F");
        assert_eq!(percent_encode_query_value("a b"), "a%20b");
        assert_eq!(percent_encode_query_value("旅行"), "%E6%97%85%E8%A1%8C");
        assert_eq!(percent_encode_query_value("a+b~c_d.e-f"), "a%2Bb~c_d.e-f");
    }

    #[test]
    fn credential_rejects_blank_and_redacts_debug() {
        assert!(QiniuCredential::new("", "sk").is_err());
        assert!(QiniuCredential::new("ak", "  ").is_err());
        let cred = QiniuCredential::new("my-ak", "my-sk").unwrap();
        let debug = format!("{cred:?}");
        assert!(debug.contains("my-ak"));
        assert!(!debug.contains("my-sk"), "Debug 不得泄露 SecretKey");
    }
}
