//! 按文件类型区分的行图标（Lucide 图标集，仅此一家——agents.md §7 禁止混用）。
//!
//! 素材在 `assets/icons/*.svg`（Lucide 官方源码，stroke=currentColor，
//! 与 gpui-component 自带图标同规格），经 `AppAssets`（组合 AssetSource）
//! 在运行时供给 gpui SVG 渲染器：先查本 crate 图标，未命中回退
//! gpui-component-assets（IconName 全集仍然可用）。
//!
//! 决策函数 `file_type_icon` 是纯函数（扩展名 → 图标路径 + 语义色），单测锁死。

use gpui::{AssetSource, SharedString, Styled as _};
use gpui_component::Icon;
use std::borrow::Cow;

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
    /// 该类型的 Lucide SVG 路径（AppAssets 资产命名空间）。
    pub fn svg_path(self) -> &'static str {
        match self {
            FileIconKind::Generic => "icons/file.svg", // gpui-component 自带
            FileIconKind::Text => "icons/file-type-text.svg",
            FileIconKind::Code => "icons/file-type-code.svg",
            FileIconKind::Image => "icons/file-type-image.svg",
            FileIconKind::Video => "icons/file-type-video.svg",
            FileIconKind::Audio => "icons/file-type-audio.svg",
            FileIconKind::Archive => "icons/file-type-archive.svg",
            FileIconKind::Spreadsheet => "icons/file-type-spreadsheet.svg",
            FileIconKind::Font => "icons/file-type-font.svg",
        }
    }

    /// 语义色（hsla）。低饱和基调：hue 210 为主色系（agents.md §7 低饱和
    /// Accent），文本/代码走 muted，图片/视频/音频给轻微类型区分度。
    pub fn color(self, muted: gpui::Hsla, accent: gpui::Hsla) -> gpui::Hsla {
        match self {
            FileIconKind::Generic | FileIconKind::Text | FileIconKind::Code => muted,
            // 其余类型用 accent 色相但保持低饱和：直接取主题 accent
            // （本身已低饱和青蓝 hue 210），不硬编码 hex/hsla。
            _ => accent,
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

/// 应用资产源：本 crate 的图标（assets/icons/）优先，未命中回退
/// gpui-component-assets（它的 IconName 全集照常工作）。
/// `Application::with_assets` 只接受一个 AssetSource，故在此组合。
pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        // 本 crate 图标：构建期嵌入（include_bytes!），零运行时 IO。
        if let Some(bytes) = embedded_icon(path) {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        gpui_component_assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        let mut items = gpui_component_assets::Assets.list(path)?;
        items.extend(
            FILE_TYPE_ICONS
                .iter()
                .filter(|p| p.starts_with(path))
                .map(|p| SharedString::from(*p)),
        );
        Ok(items)
    }
}

/// 构建期嵌入的类型图标（路径 → bytes）。新增图标时在此登记。
static FILE_TYPE_ICONS: &[&str] = &[
    "icons/file-type-text.svg",
    "icons/file-type-code.svg",
    "icons/file-type-image.svg",
    "icons/file-type-video.svg",
    "icons/file-type-audio.svg",
    "icons/file-type-archive.svg",
    "icons/file-type-spreadsheet.svg",
    "icons/file-type-font.svg",
];

fn embedded_icon(path: &str) -> Option<&'static [u8]> {
    match path {
        "icons/file-type-text.svg" => Some(include_bytes!("../assets/icons/lucide-file-text.svg")),
        "icons/file-type-code.svg" => Some(include_bytes!("../assets/icons/lucide-file-code.svg")),
        "icons/file-type-image.svg" => {
            Some(include_bytes!("../assets/icons/lucide-file-image.svg"))
        }
        "icons/file-type-video.svg" => Some(include_bytes!("../assets/icons/lucide-film.svg")),
        "icons/file-type-audio.svg" => Some(include_bytes!("../assets/icons/lucide-music-2.svg")),
        "icons/file-type-archive.svg" => {
            Some(include_bytes!("../assets/icons/lucide-file-archive.svg"))
        }
        "icons/file-type-spreadsheet.svg" => Some(include_bytes!(
            "../assets/icons/lucide-file-spreadsheet.svg"
        )),
        "icons/file-type-font.svg" => Some(include_bytes!("../assets/icons/lucide-file-type.svg")),
        _ => None,
    }
}

/// 行图标渲染：按 Object Key 的扩展名选 Lucide SVG，muted/accent 语义着色。
/// 目录行不经过这里（目录恒用 IconName::Folder）。
pub(crate) fn file_type_icon(key: &str, muted: gpui::Hsla, accent: gpui::Hsla) -> Icon {
    let kind = file_icon_kind(key);
    Icon::path(Icon::empty(), kind.svg_path()).text_color(kind.color(muted, accent))
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

    #[test]
    fn file_icon_paths_are_unique_and_embedded() {
        // 每个分组的 SVG 路径唯一，且都能从嵌入表取到 bytes（Generic 除外，
        // 它回退 gpui-component-assets 的 file.svg）
        let kinds = [
            FileIconKind::Text,
            FileIconKind::Code,
            FileIconKind::Image,
            FileIconKind::Video,
            FileIconKind::Audio,
            FileIconKind::Archive,
            FileIconKind::Spreadsheet,
            FileIconKind::Font,
        ];
        let mut paths: Vec<&str> = kinds.iter().map(|k| k.svg_path()).collect();
        let count = paths.len();
        paths.sort_unstable();
        paths.dedup();
        assert_eq!(paths.len(), count, "各类型 svg 路径不得重复");
        for path in paths {
            assert!(
                embedded_icon(path).is_some(),
                "{path} 必须在 embedded_icon 登记过"
            );
        }
        assert!(embedded_icon("icons/file.svg").is_none());
    }
}
