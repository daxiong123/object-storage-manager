//! 按文件类型区分的行图标（Lucide 图标集，仅此一家——agents.md §7 禁止混用）。
//!
//! 素材在 `assets/icons/*.svg`（Lucide 官方源码，stroke=currentColor，
//! 与 gpui-component 自带图标同规格），构建期嵌入并由 `Icon::data` 直接渲染。
//!
//! 纯函数 `file_icon_kind` 负责扩展名分组，单测锁死。

use gpui::Styled as _;
use gpui_component::{Icon, IconName};

/// 文件类型图标语义分组。颜色只表达类型区分（muted 基调 + 少量低饱和
/// 语义色），不表达状态——status 色只用于真实反馈（agents.md §7）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileIconKind {
    /// 通用文件（无法识别扩展名）
    Generic,
    /// 文本 / Markdown
    Text,
    /// 代码 / 配置（json/yaml/toml/rs/js/ts/xml/html/css…）
    Code,
    /// 图片
    Image,
    /// 视频
    Video,
    /// 音频
    Audio,
    /// 压缩包
    Archive,
    /// 表格（csv/xls/xlsx）
    Spreadsheet,
    /// 字体（ttf/otf/woff…）
    Font,
}

impl FileIconKind {
    fn icon(self) -> Icon {
        match self {
            FileIconKind::Generic => Icon::new(IconName::File),
            FileIconKind::Text => {
                Icon::default().data(include_bytes!("../assets/icons/lucide-file-text.svg"))
            }
            FileIconKind::Code => {
                Icon::default().data(include_bytes!("../assets/icons/lucide-file-code.svg"))
            }
            FileIconKind::Image => {
                Icon::default().data(include_bytes!("../assets/icons/lucide-file-image.svg"))
            }
            FileIconKind::Video => {
                Icon::default().data(include_bytes!("../assets/icons/lucide-film.svg"))
            }
            FileIconKind::Audio => {
                Icon::default().data(include_bytes!("../assets/icons/lucide-music-2.svg"))
            }
            FileIconKind::Archive => {
                Icon::default().data(include_bytes!("../assets/icons/lucide-file-archive.svg"))
            }
            FileIconKind::Spreadsheet => Icon::default().data(include_bytes!(
                "../assets/icons/lucide-file-spreadsheet.svg"
            )),
            FileIconKind::Font => {
                Icon::default().data(include_bytes!("../assets/icons/lucide-file-type.svg"))
            }
        }
    }
}

/// 扩展名 → 文件类型分组。`key` 是 Cloud Object Key（`/` 分隔，取最后段
/// 的扩展名）；无扩展名 = Generic。
pub(crate) fn file_icon_kind(key: &str) -> FileIconKind {
    let name = key.rsplit('/').next().unwrap_or(key);
    // 最后一段可能就是扩展名（.gitignore 形态 rsplit 得到 "gitignore"，
    // 属可识别集合则正常归类；不属于则 Generic——与系统行为一致）。
    let Some(ext) = name.rsplit('.').next().map(str::to_ascii_lowercase) else {
        return FileIconKind::Generic;
    };
    if ext == name.to_ascii_lowercase() {
        // 无扩展名（rsplit('.') 与原名相同：名字里没有点）
        return FileIconKind::Generic;
    }
    match ext.as_str() {
        // 文本 / Markdown
        "txt" | "md" | "markdown" | "log" | "rtf" => FileIconKind::Text,
        // 代码 / 配置
        "json" | "yaml" | "yml" | "toml" | "xml" | "html" | "htm" | "css" | "scss" | "js"
        | "mjs" | "cjs" | "ts" | "tsx" | "jsx" | "rs" | "py" | "go" | "java" | "kt" | "swift"
        | "c" | "h" | "cpp" | "hpp" | "sh" | "bash" | "zsh" | "sql" | "php" | "rb" | "ini"
        | "conf" | "env" | "properties" | "gradle" | "gitignore" | "dockerfile" => {
            FileIconKind::Code
        }
        // 图片
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "ico" | "tiff" | "tif"
        | "heic" | "avif" => FileIconKind::Image,
        // 视频
        "mp4" | "mov" | "avi" | "mkv" | "webm" | "flv" | "wmv" | "m4v" | "mpeg" | "mpg" => {
            FileIconKind::Video
        }
        // 音频
        "mp3" | "wav" | "flac" | "aac" | "ogg" | "m4a" | "wma" | "opus" | "aiff" => {
            FileIconKind::Audio
        }
        // 压缩包
        "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "7z" | "rar" | "dmg" | "iso" => {
            FileIconKind::Archive
        }
        // 表格
        "csv" | "tsv" | "xls" | "xlsx" | "ods" => FileIconKind::Spreadsheet,
        // 字体
        "ttf" | "otf" | "woff" | "woff2" | "eot" => FileIconKind::Font,
        _ => FileIconKind::Generic,
    }
}

/// 行图标渲染：按 Object Key 的扩展名选 Lucide SVG，muted/accent 语义着色。
/// 目录行不经过这里（目录恒用 IconName::Folder）。
/// 文件图标：**所有类型同一个颜色**（取自 `theme::file_icon_color`），
/// 只靠字形区分类型——与参照实现一致（实测它每行图标的最饱和像素都是同一个金色）。
///
/// 之前是按类型分色（文本/代码取 muted、其余取 accent），那样在 Linear 风格下非文本
/// 图标用的是近白的 `accent`，实际几乎看不见；统一取色后这个既有缺陷也一并消失。
pub(crate) fn file_type_icon(key: &str, mode: gpui_component::ThemeMode) -> Icon {
    file_icon_kind(key)
        .icon()
        .text_color(crate::theme::file_icon_color(mode))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_icon_kind_classifies_common_extensions() {
        assert_eq!(file_icon_kind("a/b/readme.md"), FileIconKind::Text);
        assert_eq!(file_icon_kind("config.json"), FileIconKind::Code);
        assert_eq!(file_icon_kind("photo.PNG"), FileIconKind::Image);
        assert_eq!(file_icon_kind("movie.mp4"), FileIconKind::Video);
        assert_eq!(file_icon_kind("song.flac"), FileIconKind::Audio);
        assert_eq!(file_icon_kind("backup.tar.gz"), FileIconKind::Archive);
        assert_eq!(file_icon_kind("data.csv"), FileIconKind::Spreadsheet);
        assert_eq!(file_icon_kind("fonts/Inter.ttf"), FileIconKind::Font);
    }

    #[test]
    fn file_icon_kind_unknown_or_extensionless_is_generic() {
        assert_eq!(file_icon_kind("docs/report.xyz"), FileIconKind::Generic);
        assert_eq!(file_icon_kind("no-extension"), FileIconKind::Generic);
        assert_eq!(file_icon_kind(""), FileIconKind::Generic);
        // .gitignore：rsplit 取得 "gitignore"，在代码/配置集合内
        assert_eq!(file_icon_kind(".gitignore"), FileIconKind::Code);
        // 目录前缀不影响归类
        assert_eq!(file_icon_kind("a/b/c/logo.svg"), FileIconKind::Image);
    }
}
