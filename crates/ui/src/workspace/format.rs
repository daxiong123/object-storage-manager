//! 通用格式化与键名辅助（大小/时间/显示名等纯函数）。

use super::*;

pub(super) fn object_key(entry: &ListingEntry) -> Option<&str> {
    match entry {
        ListingEntry::Object(object) => Some(object.key.as_str()),
        ListingEntry::CommonPrefix(_) => None,
    }
}

/// 字节数人性化：B 整数展示，KB/MB/GB/TB 保留 1 位小数。
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

pub(super) fn format_integer_grouped(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// 云端 Key 的末段展示名（目录前缀先去掉结尾 `/`）。
pub fn display_name(key: &str) -> &str {
    let trimmed = key.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(i) => &trimmed[i + 1..],
        None => trimmed,
    }
}

/// epoch 毫秒 → 本地时间 "YYYY-MM-DD HH:MM"。非法时间戳原样输出数字（不静默美化）。
pub fn format_time(millis: i64) -> String {
    chrono::DateTime::from_timestamp_millis(millis)
        .map(|utc| {
            utc.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| millis.to_string())
}
