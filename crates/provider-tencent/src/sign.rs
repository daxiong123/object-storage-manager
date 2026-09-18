//! 腾讯云 COS 请求签名 v5（`q-sign-algorithm=sha1`）
//!
//! 派生链（官方文档 product/436/7778，逐字节核对自五个官方 SDK；腾讯云**没有**
//! 官方 Rust SDK，故只能照规范实现）：
//!
//! ```text
//! KeyTime     = [StartUnix];[EndUnix]
//! SignKey     = hex(HMAC-SHA1(SecretKey, KeyTime))
//! HttpString  = [method 小写]\n[UriPathname]\n[HttpParameters]\n[HttpHeaders]\n
//! StringToSign= "sha1"\n[KeyTime]\nhex(SHA1(HttpString))\n
//! Signature   = hex(HMAC-SHA1(SignKey, StringToSign))
//! Authorization: q-sign-algorithm=sha1&q-ak=..&q-sign-time=..&q-key-time=..
//!                &q-header-list=..&q-url-param-list=..&q-signature=..
//! ```
//!
//! 四个必须按原样复现、否则静默签错的点：
//!
//! 1. **SignKey 的十六进制字符串是当「文本」用的**，不是当原始字节——它是下一层
//!    HMAC 的 key（`sign_key.as_bytes()`）。官方文档原文写明「SignKey as the key
//!    (in string form, not raw binary)」，五个 SDK 一致。
//! 2. **`UriPathname` 是解码后的原始 UTF-8 路径**，不是线上的百分号编码形式。
//!    文档例 1 的请求行是 `/exampleobject(%E8%85%BE%E8%AE%AF%E4%BA%91)`，但只有用
//!    解码后的 `腾讯云` 才能算出文档公布的那个 `SHA1(HttpString)`——本模块的
//!    `http_string_uses_decoded_path` 把这条钉死。
//! 3. **`HttpString` 用 LF，且结尾的 `\n` 必须有**；空分量保留成空行
//!    （`get\n/obj\n\n\n` 是合法的，参数与头都为空时就是这样）。
//! 4. **签名用的头是「按需挑选的子集」**。我们只签 `host`（上传再加 `content-type`），
//!    不签 `date`/`content-length`/`content-md5`：签得越少，越不容易因为值与实际
//!    发送的不一致而炸；代价是这几个头失去防篡改保护，对本地客户端无实际影响。

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use object_storage_core::StorageError;
use sha1::{Digest, Sha1};

type HmacSha1 = Hmac<Sha1>;

/// 腾讯云密钥对（COS 侧称 SecretId / SecretKey）。
///
/// `Debug` 永远不输出 SecretKey（storage-core 红线：Secret 不进日志）。
#[derive(Clone, PartialEq, Eq)]
pub struct TencentCredential {
    secret_id: String,
    secret_key: String,
}

impl TencentCredential {
    pub fn new(
        secret_id: impl Into<String>,
        secret_key: impl Into<String>,
    ) -> Result<Self, StorageError> {
        let secret_id = secret_id.into().trim().to_string();
        let secret_key = secret_key.into().trim().to_string();
        if secret_id.is_empty() || secret_key.is_empty() {
            return Err(StorageError::InvalidInput(
                "腾讯云 SecretId/SecretKey 不能为空".into(),
            ));
        }
        Ok(Self {
            secret_id,
            secret_key,
        })
    }

    /// SecretId 即「Access Key」，不是 Secret，可持久化（存 SQLite）
    pub fn secret_id(&self) -> &str {
        &self.secret_id
    }
}

impl std::fmt::Debug for TencentCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TencentCredential")
            .field("secret_id", &self.secret_id)
            .field("secret_key", &"***")
            .finish()
    }
}

/// 当前 Unix 秒
pub(crate) fn unix_now() -> Result<u64, StorageError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| StorageError::InvalidInput(format!("系统时间异常: {e}")))
        .map(|d| d.as_secs())
}

/// `KeyTime = Start;End`（Unix 秒，无填充）
pub(crate) fn key_time(start: u64, ttl: Duration) -> String {
    format!("{start};{}", start + ttl.as_secs())
}

/// COS 的 UrlEncode：等价于 JS `encodeURIComponent` 再额外编码 `! ' ( ) *`，
/// 百分号用大写十六进制，空格 → `%20`（**绝不** `+`），保留 `A-Za-z0-9-_.~`。
///
/// 用于 query 参数值与 header 值（`/` 会被编码）。
pub(crate) fn sign_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for &b in value.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char);
            }
            _ => push_percent(&mut out, b),
        }
    }
    out
}

/// 对象 Key 的 URL path 编码：保留 `/` 作为路径分隔（其余同 [`sign_encode`]）。
pub(crate) fn percent_encode_path(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for &b in value.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(b as char);
            }
            _ => push_percent(&mut out, b),
        }
    }
    out
}

/// 预签名 URL 的 query 形态：把整条 Authorization 百分号编码，但 **`&` 与 `=` 保持
/// 字面量**（否则 query 就解析不出七个参数了）。`;` 因此会变成 `%3B`。
pub(crate) fn encode_authorization_query(authorization: &str) -> String {
    let mut out = String::with_capacity(authorization.len());
    for &b in authorization.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'&' | b'=' => {
                out.push(b as char);
            }
            _ => push_percent(&mut out, b),
        }
    }
    out
}

fn push_percent(out: &mut String, byte: u8) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    out.push('%');
    out.push(HEX[(byte >> 4) as usize] as char);
    out.push(HEX[(byte & 0x0F) as usize] as char);
}

/// 十六进制小写（COS 的 HMAC/SHA1 结果一律小写十六进制）
pub(crate) fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn hmac_sha1(key: &[u8], message: &[u8]) -> [u8; 20] {
    let mut mac = HmacSha1::new_from_slice(key).expect("HMAC-SHA1 接受任意长度 key");
    mac.update(message);
    let mut out = [0u8; 20];
    out.copy_from_slice(&mac.finalize().into_bytes());
    out
}

pub(crate) fn sha1_hex(data: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(data);
    to_hex(&hasher.finalize())
}

/// 供测试断言中间量：`SignKey = hex(HMAC-SHA1(SecretKey, KeyTime))`
pub(crate) fn sign_key(cred: &TencentCredential, key_time: &str) -> String {
    to_hex(&hmac_sha1(cred.secret_key.as_bytes(), key_time.as_bytes()))
}

/// 供测试断言中间量：`StringToSign = "sha1"\nKeyTime\nhex(SHA1(HttpString))\n`
pub(crate) fn string_to_sign(key_time: &str, http_string: &str) -> String {
    format!("sha1\n{key_time}\n{}\n", sha1_hex(http_string.as_bytes()))
}

/// `Signature = hex(HMAC-SHA1(SignKey, StringToSign))`——注意 SignKey 以文本形态当 key
pub(crate) fn signature(sign_key_hex: &str, string_to_sign: &str) -> String {
    to_hex(&hmac_sha1(
        sign_key_hex.as_bytes(),
        string_to_sign.as_bytes(),
    ))
}

/// `HttpString = method\nUriPathname\nHttpParameters\nHttpHeaders\n`
///
/// `method` 转小写；`path` 必须是**解码后**的路径；两个分量为空时保留空行。
pub(crate) fn http_string(
    method: &str,
    path: &str,
    http_parameters: &str,
    http_headers: &str,
) -> String {
    format!(
        "{}\n{}\n{}\n{}\n",
        method.to_lowercase(),
        path,
        http_parameters,
        http_headers
    )
}

/// 把 `(key, value)` 列表规范化成 `(列表, 拼接串)`：
/// key 先 UrlEncode 再转小写，value 只 UrlEncode，按 key 字典序排序；
/// 列表用 `;` 连接（`q-header-list` / `q-url-param-list`），
/// 拼接串用 `&` 连接 `key=value`（`HttpHeaders` / `HttpParameters`）。
fn canonicalize(items: &[(String, String)]) -> (String, String) {
    let mut pairs: Vec<(String, String)> = items
        .iter()
        .map(|(k, v)| (sign_encode(k).to_lowercase(), sign_encode(v)))
        .collect();
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    let list = pairs
        .iter()
        .map(|(k, _)| k.clone())
        .collect::<Vec<_>>()
        .join(";");
    let joined = pairs
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");
    (list, joined)
}

/// 组装 `Authorization` 头的值。
///
/// - `path`：**解码后**的 UriPathname（不含 bucket，`/` 开头；bucket 在 host 里）
/// - `params`：要参与签名的 query 参数（解码后的原值，本函数负责编码与排序）
/// - `headers`：要参与签名的头（**必须含 `host`**）
pub(crate) fn authorization(
    cred: &TencentCredential,
    method: &str,
    path: &str,
    params: &[(String, String)],
    headers: &[(String, String)],
    key_time: &str,
) -> String {
    let (header_list, http_headers) = canonicalize(headers);
    let (url_param_list, http_parameters) = canonicalize(params);
    let http_string = http_string(method, path, &http_parameters, &http_headers);
    let sts = string_to_sign(key_time, &http_string);
    let sig = signature(&sign_key(cred, key_time), &sts);
    format!(
        "q-sign-algorithm=sha1&q-ak={}&q-sign-time={key_time}&q-key-time={key_time}\
         &q-header-list={header_list}&q-url-param-list={url_param_list}&q-signature={sig}",
        cred.secret_id
    )
}

/// `host` 是唯一必须参与签名的头（缺了它服务端会算不出相同的 HttpString）。
pub(crate) fn host_header(host: &str) -> Vec<(String, String)> {
    vec![("host".to_string(), host.to_string())]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 官方文档例 1（上传）的 HttpString → SHA1。
    ///
    /// 这条向量**不需要 SecretKey**（文档里密钥被打码），因此能独立锁死
    /// HttpString 的构造：行序、LF、结尾换行、头名排序、值编码，
    /// 以及「UriPathname 用解码后的 UTF-8」这一条。
    #[test]
    fn http_string_uses_decoded_path() {
        let http = http_string(
            "put",
            "/exampleobject(腾讯云)",
            "",
            "content-length=13&content-md5=mQ%2FfVh815F3k6TAUm8m0eg%3D%3D&content-type=text%2Fplain&date=Thu%2C%2016%20May%202019%2006%3A45%3A51%20GMT&host=examplebucket-1250000000.cos.ap-beijing.myqcloud.com&x-cos-acl=private&x-cos-grant-read=uin%3D%22100000000011%22",
        );
        assert_eq!(
            sha1_hex(http.as_bytes()),
            "8b2751e77f43a0995d6e9eb9477f4b685cca4172"
        );
    }

    /// 官方文档例 2（下载 + 两个 response-* 参数）的 HttpString → SHA1。
    /// 锁死 query 参数的排序与值内 `=` 的编码（`max-age=600` → `max-age%3D600`）。
    #[test]
    fn http_string_sorts_url_params() {
        let http = http_string(
            "get",
            "/exampleobject(腾讯云)",
            "response-cache-control=max-age%3D600&response-content-type=application%2Foctet-stream",
            "date=Thu%2C%2016%20May%202019%2006%3A55%3A53%20GMT&host=examplebucket-1250000000.cos.ap-beijing.myqcloud.com",
        );
        assert_eq!(
            sha1_hex(http.as_bytes()),
            "54ecfe22f59d3514fdc764b87a32d8133ea611e6"
        );
    }

    /// 官方 Go SDK `cos-go-sdk-v5/auth_test.go` 的完整向量（末端签名可复现）。
    /// 这条覆盖空 `q-url-param-list`、`&` 拼接的 HttpHeaders、头名排序、
    /// 方法小写、以及「SignKey 十六进制串当文本当 key」。
    #[test]
    fn go_sdk_full_signature_vector() {
        let cred = TencentCredential::new(
            "QmFzZTY0IGlzIGEgZ2VuZXJp",
            "AKIDZfbOA78asKUYBcXFrJD0a1ICvR98JM",
        )
        .unwrap();
        let key_time = "1480932292;1481012292";
        let host = "testbucket-125000000.cos.ap-guangzhou.myqcloud.com";
        let headers = vec![
            ("host".to_string(), host.to_string()),
            (
                "x-cos-content-sha1".to_string(),
                "db8ac1c259eb89d4a131b253bacfca5f319d54f2".to_string(),
            ),
            // 注意：Go SDK 测试里的 "stroage" 是官方笔误，向量里就是它
            ("x-cos-stroage-class".to_string(), "nearline".to_string()),
        ];

        // 中间量逐项对齐，任一项错都能直接指出是哪一步
        let (header_list, http_headers) = canonicalize(&headers);
        assert_eq!(header_list, "host;x-cos-content-sha1;x-cos-stroage-class");
        assert_eq!(
            http_headers,
            "host=testbucket-125000000.cos.ap-guangzhou.myqcloud.com&\
             x-cos-content-sha1=db8ac1c259eb89d4a131b253bacfca5f319d54f2&\
             x-cos-stroage-class=nearline"
        );
        assert_eq!(
            sign_key(&cred, key_time),
            "95d110a8ead64cac52083100db75b7e3f369e72f"
        );

        let http = http_string("put", "/testfile2", "", &http_headers);
        assert_eq!(
            sha1_hex(http.as_bytes()),
            "113b22e92b74237531fafbfc8bb9e67be55645c7"
        );
        let sts = string_to_sign(key_time, &http);
        assert_eq!(
            sts,
            "sha1\n1480932292;1481012292\n113b22e92b74237531fafbfc8bb9e67be55645c7\n"
        );
        assert_eq!(
            signature(&sign_key(&cred, key_time), &sts),
            "ce4ac0ecbcdb30538b3fee0a97cc6389694ce53a"
        );

        // 整条 Authorization：七个键顺序固定，空 q-url-param-list 保持 `=`
        let auth = authorization(&cred, "PUT", "/testfile2", &[], &headers, key_time);
        assert_eq!(
            auth,
            "q-sign-algorithm=sha1&q-ak=QmFzZTY0IGlzIGEgZ2VuZXJp\
             &q-sign-time=1480932292;1481012292&q-key-time=1480932292;1481012292\
             &q-header-list=host;x-cos-content-sha1;x-cos-stroage-class\
             &q-url-param-list=&q-signature=ce4ac0ecbcdb30538b3fee0a97cc6389694ce53a"
        );
    }

    /// 空参数与空头都要保留空行（`get\n/obj\n\n\n`）
    #[test]
    fn empty_components_keep_their_blank_lines() {
        assert_eq!(http_string("GET", "/obj", "", ""), "get\n/obj\n\n\n");
        assert_eq!(
            http_string("GET", "/", "", "host=example.com"),
            "get\n/\n\nhost=example.com\n"
        );
    }

    #[test]
    fn sign_encode_matches_cos_urlencode_rules() {
        // 空格 → %20（不是 +）
        assert_eq!(sign_encode("a b"), "a%20b");
        // 这五个是 encodeURIComponent 不编、COS 要编的
        assert_eq!(sign_encode("!'()*"), "%21%27%28%29%2A");
        // 保留字符
        assert_eq!(sign_encode("-_.~"), "-_.~");
        // query 值与路径分隔符：sign_encode 编掉 `/`，percent_encode_path 保留
        assert_eq!(sign_encode("a/b"), "a%2Fb");
        assert_eq!(percent_encode_path("a/b c"), "a/b%20c");
        // 大写十六进制 + 多字节 UTF-8
        assert_eq!(sign_encode("中"), "%E4%B8%AD");
        // 预签名 query：`&`/`=` 字面量，`;` 编码
        assert_eq!(encode_authorization_query("a=1;2&b=3"), "a=1%3B2&b=3");
    }

    #[test]
    fn key_time_formats_start_and_end() {
        assert_eq!(
            key_time(1_480_932_292, Duration::from_secs(80_000)),
            "1480932292;1481012292"
        );
    }

    #[test]
    fn credential_rejects_blank_and_redacts_debug() {
        assert!(TencentCredential::new("", "sk").is_err());
        assert!(TencentCredential::new("id", "  ").is_err());
        let cred = TencentCredential::new("secret-id", "super-secret").unwrap();
        assert_eq!(cred.secret_id(), "secret-id");
        assert!(!format!("{cred:?}").contains("super-secret"));
    }

    /// 只签 host 时，HeaderList 恰为 `host`
    #[test]
    fn host_only_header_list() {
        let cred = TencentCredential::new("id", "key").unwrap();
        let auth = authorization(
            &cred,
            "GET",
            "/a b.png",
            &[],
            &host_header("b-1250000000.cos.ap-guangzhou.myqcloud.com"),
            "1;2",
        );
        assert!(auth.contains("q-header-list=host&"), "auth={auth}");
        assert!(auth.contains("q-url-param-list=&"), "auth={auth}");
    }
}
