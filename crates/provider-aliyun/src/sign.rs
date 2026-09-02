//! 阿里云 OSS Signature Version 1
//!
//! 算法按官方文档「在 Header 中包含签名」构造：
//! `StringToSign = VERB\nContent-MD5\nContent-Type\nDate\nCanonicalizedOSSHeadersCanonicalizedResource`
//! `Signature = Base64(HMAC-SHA1(AccessKeySecret, StringToSign))`（**标准 Base64，带 padding**）
//! `Authorization: OSS AccessKeyId:Signature`
//!
//! CanonicalizedResource = `/{bucket}` 或 `/{bucket}/{key}`，加上按名字排序的
//! 子资源 query（`delimiter`/`marker`/`max-keys`/`prefix`/`location` 等）。
//! 签名必须使用即将发送的 Date / Content-Type 原值。

use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use hmac::{Hmac, Mac};
use object_storage_core::StorageError;
use sha1::Sha1;

type HmacSha1 = Hmac<Sha1>;

/// 阿里云密钥对。`Debug` 永远不输出 SecretKey。
#[derive(Clone, PartialEq, Eq)]
pub struct AliyunCredential {
    access_key: String,
    secret_key: String,
}

impl AliyunCredential {
    pub fn new(
        access_key: impl Into<String>,
        secret_key: impl Into<String>,
    ) -> Result<Self, StorageError> {
        let access_key = access_key.into();
        let secret_key = secret_key.into();
        if access_key.trim().is_empty() || secret_key.trim().is_empty() {
            return Err(StorageError::InvalidInput(
                "Aliyun AccessKey/SecretKey 不能为空".into(),
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

impl std::fmt::Debug for AliyunCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AliyunCredential")
            .field("access_key", &self.access_key)
            .field("secret_key", &"***")
            .finish()
    }
}

/// RFC 1123 HTTP-date，固定英文星期/月份（不走 locale）。
pub(crate) fn http_gmt(time: SystemTime) -> Result<String, StorageError> {
    let secs = time
        .duration_since(UNIX_EPOCH)
        .map_err(|e| StorageError::InvalidInput(format!("系统时间异常: {e}")))?
        .as_secs();
    // 手工换算，避免 chrono locale 把星期写成中文
    let (year, month, day, hour, min, sec, weekday) = civil_from_unix(secs);
    const WEEKDAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    Ok(format!(
        "{}, {:02} {} {:04} {:02}:{:02}:{:02} GMT",
        WEEKDAYS[weekday], day, MONTHS[month], year, hour, min, sec
    ))
}

/// Unix 秒 → (year, month0, day, hour, min, sec, weekday) ；weekday 0 = 周四（1970-01-01）
fn civil_from_unix(secs: u64) -> (i32, usize, u32, u32, u32, u32, usize) {
    let weekday = (secs / 86400) as usize % 7;
    let mut days = (secs / 86400) as i64;
    let tod = (secs % 86400) as u32;
    let hour = tod / 3600;
    let min = (tod % 3600) / 60;
    let sec = tod % 60;
    let mut year: i32 = 1970;
    loop {
        let diy = if is_leap(year) { 366 } else { 365 };
        if days < diy {
            break;
        }
        days -= diy;
        year += 1;
    }
    let mdays = month_days(year);
    let mut month = 0usize;
    while days >= mdays[month] as i64 {
        days -= mdays[month] as i64;
        month += 1;
    }
    (year, month, (days + 1) as u32, hour, min, sec, weekday)
}

fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn month_days(year: i32) -> [u32; 12] {
    let feb = if is_leap(year) { 29 } else { 28 };
    [31, feb, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
}

pub(crate) fn string_to_sign(
    method: &str,
    content_md5: &str,
    content_type: &str,
    date: &str,
    canonicalized_oss_headers: &str,
    canonicalized_resource: &str,
) -> String {
    format!(
        "{method}\n{content_md5}\n{content_type}\n{date}\n{canonicalized_oss_headers}{canonicalized_resource}"
    )
}

/// `canonicalized_oss_headers` 必须已含末尾 `\n`（无 x-oss-* 时为空串）。
pub(crate) fn authorization(cred: &AliyunCredential, string_to_sign: &str) -> String {
    let mut mac =
        HmacSha1::new_from_slice(cred.secret_key.as_bytes()).expect("HMAC-SHA1 接受任意长度 key");
    mac.update(string_to_sign.as_bytes());
    let sig = BASE64_STANDARD.encode(mac.finalize().into_bytes());
    format!("OSS {}:{}", cred.access_key, sig)
}

pub(crate) fn signature_only(cred: &AliyunCredential, string_to_sign: &str) -> String {
    let mut mac =
        HmacSha1::new_from_slice(cred.secret_key.as_bytes()).expect("HMAC-SHA1 接受任意长度 key");
    mac.update(string_to_sign.as_bytes());
    BASE64_STANDARD.encode(mac.finalize().into_bytes())
}

/// CanonicalizedResource：`/{bucket}` 或 `/{bucket}/{key}` + 排序后的子资源。
pub(crate) fn canonicalized_resource(
    bucket: &str,
    key: Option<&str>,
    subresources: &[(&str, &str)],
) -> String {
    let mut resource = if bucket.is_empty() {
        "/".to_string()
    } else if let Some(key) = key.filter(|k| !k.is_empty()) {
        format!("/{bucket}/{key}")
    } else {
        format!("/{bucket}")
    };
    if subresources.is_empty() {
        return resource;
    }
    let mut pairs: Vec<(&str, &str)> = subresources.to_vec();
    pairs.sort_by(|a, b| a.0.cmp(b.0));
    resource.push('?');
    resource.push_str(
        &pairs
            .iter()
            .map(|(k, v)| {
                if v.is_empty() {
                    (*k).to_string()
                } else {
                    format!("{k}={v}")
                }
            })
            .collect::<Vec<_>>()
            .join("&"),
    );
    resource
}

/// 对象 Key 的 URL path 编码：保留 `/`，其余按 RFC 3986 百分号编码。
pub(crate) fn percent_encode_path(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for &b in value.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub(crate) fn percent_encode_query(value: &str) -> String {
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
    use std::time::Duration;

    #[test]
    fn http_gmt_matches_known_timestamp() {
        // 2011-12-27 03:37:58 UTC = 官方文档示例 Date
        let t = UNIX_EPOCH + Duration::from_secs(1_324_957_078);
        assert_eq!(http_gmt(t).unwrap(), "Tue, 27 Dec 2011 03:37:58 GMT");
    }

    #[test]
    fn string_to_sign_get_service() {
        let s = string_to_sign("GET", "", "", "Tue, 27 Dec 2011 03:37:58 GMT", "", "/");
        assert_eq!(s, "GET\n\n\nTue, 27 Dec 2011 03:37:58 GMT\n/");
    }

    #[test]
    fn string_to_sign_put_with_headers() {
        let s = string_to_sign(
            "PUT",
            "eB5eJF1ptWaXm4bijSPyxw==",
            "application/pdf",
            "Tue, 27 Dec 2011 03:37:58 GMT",
            "x-oss-meta-author:alice\nx-oss-meta-magic:abracadabra\n",
            "/oss-example/oss-api.pdf",
        );
        assert_eq!(
            s,
            "PUT\neB5eJF1ptWaXm4bijSPyxw==\napplication/pdf\nTue, 27 Dec 2011 03:37:58 GMT\nx-oss-meta-author:alice\nx-oss-meta-magic:abracadabra\n/oss-example/oss-api.pdf"
        );
    }

    #[test]
    fn canonicalized_resource_sorts_subresources() {
        assert_eq!(
            canonicalized_resource(
                "b1",
                None,
                &[("prefix", "a/"), ("delimiter", "/"), ("max-keys", "100")]
            ),
            "/b1?delimiter=/&max-keys=100&prefix=a/"
        );
        assert_eq!(canonicalized_resource("", None, &[]), "/");
        assert_eq!(canonicalized_resource("b1", Some("a/b"), &[]), "/b1/a/b");
        assert_eq!(
            canonicalized_resource("b1", None, &[("location", "")]),
            "/b1?location"
        );
    }

    #[test]
    fn credential_rejects_blank_and_redacts_debug() {
        assert!(AliyunCredential::new("", "sk").is_err());
        assert!(AliyunCredential::new("ak", "  ").is_err());
        let cred = AliyunCredential::new("ak", "super-secret").unwrap();
        assert!(!format!("{cred:?}").contains("super-secret"));
    }

    #[test]
    fn authorization_uses_standard_base64() {
        let cred = AliyunCredential::new("ak", "sk").unwrap();
        let auth = authorization(&cred, "GET\n\n\nTue, 27 Dec 2011 03:37:58 GMT\n/");
        assert!(auth.starts_with("OSS ak:"));
        let sig = auth.trim_start_matches("OSS ak:");
        assert!(
            sig.contains('=') || sig.len() % 4 == 0,
            "标准 Base64 应带 padding，实际 {sig}"
        );
        assert!(!sig.contains('-') && !sig.contains('_'));
    }
}
