//! 本模块的单元测试（拆分自原 workspace_view.rs 的 `mod tests`）。
//!
//! 纯逻辑（选择语义、排序/过滤、路径与校验、格式化）在这里锁死；
//! 渲染与交互仍需人肉验收（见 agents.md「UI 冒烟边界」）。

use super::copy_move::*;
use super::delete::*;
use super::download::*;
use super::folder::*;
use super::menus::*;
use super::sidebar::*;
use super::titlebar::*;
use super::upload::*;
use super::*;

#[test]
fn format_size_human_readable() {
    assert_eq!(format_size(0), "0 B");
    assert_eq!(format_size(1023), "1023 B");
    assert_eq!(format_size(1024), "1.0 KB");
    assert_eq!(format_size(1536), "1.5 KB");
    assert_eq!(format_size(1024 * 1024), "1.0 MB");
    assert_eq!(format_size(5 * 1024 * 1024 * 1024), "5.0 GB");
    // 超出 TB 封顶：不再升单位
    let tb = 1024.0_f64.powi(4);
    assert_eq!(format_size((tb * 2048.0) as u64), "2048.0 TB");
}

#[test]
fn format_integer_grouped_adds_commas() {
    assert_eq!(format_integer_grouped(0), "0");
    assert_eq!(format_integer_grouped(999), "999");
    assert_eq!(format_integer_grouped(36_648), "36,648");
    assert_eq!(format_integer_grouped(1_234_567), "1,234,567");
}

#[test]
fn display_name_takes_last_segment() {
    assert_eq!(display_name("a/b/c.txt"), "c.txt");
    assert_eq!(display_name("c.txt"), "c.txt");
    assert_eq!(display_name("a/b/"), "b");
    assert_eq!(display_name(""), "");
}

#[test]
fn parent_prefix_walks_up() {
    assert_eq!(parent_prefix("a/"), None);
    assert_eq!(parent_prefix("a/b/"), Some("a/"));
    assert_eq!(parent_prefix("a/b/c/"), Some("a/b/"));
}

#[test]
fn format_time_shapes_local_datetime() {
    // epoch 0 → 本地时区的 "YYYY-MM-DD HH:MM"（16 字符）；跨时区只验证形状
    let s = format_time(0);
    assert_eq!(s.len(), 16, "实际输出：{s}");
    assert_eq!(&s[4..5], "-");
    assert_eq!(&s[7..8], "-");
    assert_eq!(&s[10..11], " ");
    assert_eq!(&s[13..14], ":");
    // 非法时间戳：原样输出数字，不静默美化
    assert_eq!(format_time(i64::MIN), i64::MIN.to_string());
}

#[test]
fn collect_folder_uploads_nested_and_skips_junk() {
    let dir = std::env::temp_dir().join(format!(
        "cloudstorage-folder-up-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let nested = dir.join("photos").join("a");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(dir.join("photos").join("root.jpg"), b"r").unwrap();
    std::fs::write(nested.join("cat.jpg"), b"c").unwrap();
    std::fs::write(dir.join("photos").join(".DS_Store"), b"x").unwrap();
    std::fs::write(dir.join("photos").join(".localized"), b"x").unwrap();
    std::fs::write(dir.join("photos").join("._hidden"), b"x").unwrap();
    std::fs::create_dir_all(dir.join("photos").join("empty")).unwrap();

    let mut files = collect_folder_uploads(&dir.join("photos")).unwrap();
    files.sort_by(|a, b| a.relative_key.cmp(&b.relative_key));
    let keys: Vec<_> = files.iter().map(|f| f.relative_key.as_str()).collect();
    assert_eq!(keys, ["photos/a/cat.jpg", "photos/root.jpg"]);
    assert_eq!(files[0].display_name, "cat.jpg");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn collect_folder_uploads_rejects_file() {
    let path = std::env::temp_dir().join(format!("cloudstorage-not-dir-{}", std::process::id()));
    std::fs::write(&path, b"x").unwrap();
    let err = collect_folder_uploads(&path).unwrap_err();
    assert!(err.contains("不是目录"), "实际 {err}");
    std::fs::remove_file(&path).unwrap();
}

#[test]
fn is_directory_object_matches_trailing_slash_only() {
    // 七牛目录占位对象 key 以 / 结尾；普通文件（含无扩展名）不算目录
    assert!(is_directory_object("config/"));
    assert!(is_directory_object("a/b/c/"));
    assert!(!is_directory_object("config"));
    assert!(!is_directory_object("a/b/file.txt"));
    assert!(!is_directory_object("README"));
}

#[test]
fn preview_kind_classifies_extensions() {
    assert_eq!(preview_kind("logo.png"), PreviewKind::Image);
    assert_eq!(preview_kind("notes.txt"), PreviewKind::Text);
    assert_eq!(preview_kind("config.json"), PreviewKind::Text);
    assert_eq!(preview_kind("specs.pdf"), PreviewKind::System);
    assert_eq!(preview_kind("movie.mp4"), PreviewKind::System);
}

#[test]
fn object_menu_items_keep_product_order() {
    assert_eq!(
        object_menu_items(),
        vec![
            ObjectMenuItem::Details,
            ObjectMenuItem::CopyUrl,
            ObjectMenuItem::Download,
            ObjectMenuItem::Rename,
            ObjectMenuItem::CopyTo,
            ObjectMenuItem::MoveTo,
            ObjectMenuItem::Delete,
        ]
    );
}

#[test]
fn row_activation_requires_double_click() {
    // 单击只选中、不打开（对象→预览 / 目录→进入）：这条一旦放松成 >=1，
    // 单击就变成「选中 + 跳走」，与 Finder 语义和选中语义都冲突。
    assert!(!row_activation(1, false), "单击不得激活行");
    assert!(row_activation(2, false), "双击应激活行");
    assert!(row_activation(3, false), "三击同样是激活（平台连击计数）");
    // 行内重命名尚未收尾（校验失败残留 editor）时不激活，避免跳走丢编辑态
    assert!(!row_activation(2, true), "重命名中不得激活行");
}

#[test]
fn top_more_menu_items_keep_product_order() {
    // 「上传文件夹」已归到工具栏的「上传 ▾」，不在这里重复；
    // 剩下的是「新建类 → 对象类」的顺序。
    assert_eq!(
        top_more_menu_items(),
        vec![
            TopMoreMenuItem::CreateFolder,
            TopMoreMenuItem::CopyTo,
            TopMoreMenuItem::MoveTo,
            TopMoreMenuItem::Delete,
        ]
    );
}

#[test]
fn upload_menu_lists_files_before_folder() {
    assert_eq!(
        upload_menu_items(),
        vec![UploadMenuItem::Files, UploadMenuItem::Folder]
    );
    assert_eq!(upload_menu_item_label(UploadMenuItem::Files), "上传文件…");
    assert_eq!(
        upload_menu_item_label(UploadMenuItem::Folder),
        "上传文件夹…"
    );
}

#[test]
fn copy_move_target_prefix_normalizes_and_rejects_invalid_paths() {
    assert_eq!(normalize_copy_move_target_prefix("").unwrap(), "");
    assert_eq!(
        normalize_copy_move_target_prefix("backup").unwrap(),
        "backup/"
    );
    assert_eq!(
        normalize_copy_move_target_prefix(" backup/2026/ ").unwrap(),
        "backup/2026/"
    );
    assert_eq!(
        normalize_copy_move_target_prefix("/absolute").unwrap_err(),
        "目标目录不能以 / 开头"
    );
    assert_eq!(
        normalize_copy_move_target_prefix("a/../b").unwrap_err(),
        "目标目录不能包含 .."
    );
}

#[test]
fn copy_move_target_keys_keep_file_names_and_reject_same_target() {
    let keys = vec!["a/avatar.jpg".to_string(), "b/config.json".to_string()];
    assert_eq!(
        copy_move_target_keys(&keys, "backup").unwrap(),
        vec![
            ("a/avatar.jpg".to_string(), "backup/avatar.jpg".to_string()),
            (
                "b/config.json".to_string(),
                "backup/config.json".to_string()
            ),
        ]
    );
    assert!(copy_move_target_keys(&["avatar.jpg".to_string()], "").is_err());
    assert!(
        copy_move_target_keys(&keys, "backup/flat/")
            .expect("different display names are safe")
            .iter()
            .all(|(_, target)| target.starts_with("backup/flat/"))
    );
    assert!(
        copy_move_target_keys(
            &["a/avatar.jpg".to_string(), "b/avatar.jpg".to_string()],
            "backup/"
        )
        .is_err()
    );
}

#[test]
fn copy_move_overlay_navigation_keeps_workspace_prefix_and_enter_blocks_while_loading() {
    let workspace_prefix = Some("main/current/".to_string());
    let mut overlay_prefix = "main/current/".to_string();
    let mut overlay_entries = vec![
        ListingEntry::CommonPrefix("main/current/photos/".into()),
        entry_object("main/current/readme.txt"),
    ];
    let mut overlay_state = AsyncState::Idle;

    prepare_copy_move_directory_load(
        &mut overlay_prefix,
        &mut overlay_entries,
        &mut overlay_state,
        "main/current/photos/".into(),
    );

    assert_eq!(workspace_prefix.as_deref(), Some("main/current/"));
    assert_eq!(overlay_prefix, "main/current/photos/");
    assert!(overlay_entries.is_empty(), "目录切换必须丢弃旧目录项");
    assert_eq!(overlay_state, AsyncState::Loading);
    assert!(
        !can_commit_copy_move(false, &overlay_state, None),
        "Input PressEnter 走 commit_copy_move；目录加载中必须和按钮一样禁止提交"
    );

    overlay_state = AsyncState::Idle;
    overlay_entries = vec![ListingEntry::CommonPrefix(
        "main/current/photos/raw/".into(),
    )];
    let parent = parent_prefix(&overlay_prefix)
        .map(str::to_string)
        .unwrap_or_default();
    prepare_copy_move_directory_load(
        &mut overlay_prefix,
        &mut overlay_entries,
        &mut overlay_state,
        parent,
    );

    assert_eq!(workspace_prefix.as_deref(), Some("main/current/"));
    assert_eq!(overlay_prefix, "main/current/");
    assert!(overlay_entries.is_empty(), "上一级也必须触发重新加载");
    assert_eq!(overlay_state, AsyncState::Loading);

    overlay_state = AsyncState::Idle;
    assert!(can_commit_copy_move(false, &overlay_state, None));
    assert!(!can_commit_copy_move(true, &overlay_state, None));
    assert!(!can_commit_copy_move(
        false,
        &overlay_state,
        Some("目标对象已存在")
    ));
}

#[test]
fn provider_url_scheme_matches_cloud_provider() {
    assert_eq!(provider_url_scheme(ProviderKind::Aliyun), "oss");
    assert_eq!(provider_url_scheme(ProviderKind::Qiniu), "kodo");
}

#[test]
fn effective_default_download_dir_requires_existing_directory() {
    let dir = std::env::temp_dir().join(format!(
        "cloudstorage-dl-dir-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    // 未设置：None
    assert_eq!(effective_default_download_dir(None), None);
    // 目录不存在：None（面板退回 HOME）
    assert_eq!(effective_default_download_dir(Some(dir.as_path())), None);
    // 目录存在：原样返回
    std::fs::create_dir_all(&dir).unwrap();
    assert_eq!(
        effective_default_download_dir(Some(dir.as_path())),
        Some(dir.clone())
    );
    // 指向文件：None
    let file = dir.join("not-a-dir.txt");
    std::fs::write(&file, b"x").unwrap();
    assert_eq!(effective_default_download_dir(Some(file.as_path())), None);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn download_dest_path_takes_last_segment() {
    let dest_dir = PathBuf::from("/tmp/cloudstorage-test-dest");
    assert_eq!(
        download_dest_path(&dest_dir, "a/b/c/report.pdf"),
        dest_dir.join("report.pdf")
    );
    // 嵌套同名文件不互相覆盖路径的前缀部分：目录名作为整体拼接
    assert_eq!(
        download_dest_path(&dest_dir, "x/avatar.jpg"),
        PathBuf::from("/tmp/cloudstorage-test-dest/avatar.jpg")
    );
    // 扁平化语义：不同目录下同名文件落在同一目标路径。
    // 这是既有行为（传输引擎 File::create 覆盖写），测试锁死以免无意变更。
    assert_eq!(
        download_dest_path(&dest_dir, "a/avatar.jpg"),
        download_dest_path(&dest_dir, "b/avatar.jpg"),
        "同名展平 = 同目标路径（引擎覆盖写语义）"
    );
}

#[test]
fn single_download_confirm_texts_match_batch_structure() {
    let dir = PathBuf::from("/Users/demo/Downloads");
    let (title, detail) = single_download_confirm_texts("a/b/report.pdf", &dir);
    assert_eq!(title, "将「report.pdf」下载到默认目录。");
    assert!(detail.contains("/Users/demo/Downloads"));
    assert!(detail.contains("另存为"));

    let (batch_title, batch_detail) = batch_download_confirm_texts(3, &dir);
    assert_eq!(batch_title, "将 3 个对象下载到默认目录。");
    assert!(batch_detail.contains("/Users/demo/Downloads"));
    assert!(batch_detail.contains("不支持设置初始目录"));
}

#[test]
fn copy_move_summary_lists_partial_failures() {
    assert_eq!(
        copy_move_summary(CopyMoveMode::Copy, 2, &[]),
        "已复制 2 个对象"
    );
    assert_eq!(
        copy_move_summary(
            CopyMoveMode::Move,
            1,
            &[("a/b.txt".to_string(), "无权限".to_string())]
        ),
        "移动完成 1 个，失败 1 个：b.txt：无权限"
    );
}

#[test]
fn selection_pure_logic_plain_click_moves_current_row_and_leaves_checkboxes_alone() {
    let keys = vec!["a".to_string(), "b".to_string(), "c".to_string()];
    // 勾选集合里已有 b（用户自己勾的）
    let current: indexmap::IndexSet<String> = ["b".to_string()].into_iter().collect();
    let intent = ObjectSelectionIntent {
        command: false,
        shift: false,
        select_all: false,
        clicked_index: Some(2),
    };
    let (next, anchor, preview) = apply_object_selection(
        intent,
        &keys,
        &current,
        Some(1),
        ClickedEntry::Object("c".into()),
    );
    // 不带动词的点击**不动勾选集合**：点行看一眼不该把复选框勾上——曾经会替换集合，
    // 于是「点几行看一眼，再按删除」把它们一起删了。「当前行」由调用方按
    // `plain_click()` 取点击项（`handle_object_row_click`）。
    assert_eq!(next, current, "普通点击不得改动勾选集合");
    assert_eq!(anchor, Some(2), "锚点跟到点击项，随后的 ⇧ 范围才有起点");
    assert!(preview, "普通点击应触发预览");
}

#[test]
fn selection_pure_logic_command_click_toggles() {
    let keys = vec!["a".to_string(), "b".to_string(), "c".to_string()];
    let current: indexmap::IndexSet<String> =
        ["a".to_string(), "b".to_string()].into_iter().collect();
    // ⌘Click 已选中的 b → 取消
    let intent = ObjectSelectionIntent {
        command: true,
        shift: false,
        select_all: false,
        clicked_index: Some(1),
    };
    let (next, anchor, preview) = apply_object_selection(
        intent,
        &keys,
        &current,
        Some(1),
        ClickedEntry::Object("b".into()),
    );
    assert_eq!(next.len(), 1);
    assert!(next.contains("a"));
    assert_eq!(anchor, Some(1));
    assert!(!preview);

    // ⌘Click 未选中的 c → 追加
    let intent = ObjectSelectionIntent {
        command: true,
        shift: false,
        select_all: false,
        clicked_index: Some(2),
    };
    let (next, _, preview) = apply_object_selection(
        intent,
        &keys,
        &current,
        Some(0),
        ClickedEntry::Object("c".into()),
    );
    assert_eq!(next.len(), 3);
    assert!(!preview);
}

#[test]
fn selection_pure_logic_shift_click_range_selects() {
    let keys = (0..5).map(|i| i.to_string()).collect::<Vec<_>>();
    let current: indexmap::IndexSet<String> = ["1".to_string()].into_iter().collect();
    // 锚点 1，⇧Click 3 → 选 1..=3
    let intent = ObjectSelectionIntent {
        command: false,
        shift: true,
        select_all: false,
        clicked_index: Some(3),
    };
    let (next, anchor, preview) = apply_object_selection(
        intent,
        &keys,
        &current,
        Some(1),
        ClickedEntry::Object("3".into()),
    );
    assert_eq!(next.len(), 3);
    assert!(next.contains("1") && next.contains("2") && next.contains("3"));
    assert_eq!(anchor, Some(1), "⇧Click 不改变锚点");
    assert!(!preview);

    // ⌘⇧Click：增量（追加范围，不清空原选择）
    let intent = ObjectSelectionIntent {
        command: true,
        shift: true,
        select_all: false,
        clicked_index: Some(4),
    };
    let (next, _, _) = apply_object_selection(
        intent,
        &keys,
        &current,
        Some(1),
        ClickedEntry::Object("4".into()),
    );
    assert_eq!(next.len(), 4);
    assert!(next.contains("1"), "原选中保留");
}

#[test]
fn selection_pure_logic_select_all_and_no_hit() {
    let keys = vec!["a".to_string(), "b".to_string()];
    let current: indexmap::IndexSet<String> = ["a".to_string()].into_iter().collect();
    // ⌘A 全选
    let intent = ObjectSelectionIntent {
        command: false,
        shift: false,
        select_all: true,
        clicked_index: None,
    };
    let (next, _, preview) =
        apply_object_selection(intent, &keys, &current, None, ClickedEntry::None);
    assert_eq!(next.len(), 2);
    assert!(!preview);

    // 未命中任何条目（ClickedEntry::None）：清空集合与锚点。
    // 列表空白点击不走这里——容器直接调 clear_object_selection，
    // 它还要一并关掉行内菜单与详情弹层（纯函数表达不了）。
    let intent = ObjectSelectionIntent {
        command: false,
        shift: false,
        select_all: false,
        clicked_index: None,
    };
    let (next, anchor, _) =
        apply_object_selection(intent, &keys, &current, Some(0), ClickedEntry::None);
    assert!(next.is_empty());
    assert_eq!(anchor, None);
}

#[test]
fn selection_pure_logic_prefix_click_keeps_selection() {
    let keys = vec!["a".to_string()];
    let current: indexmap::IndexSet<String> = ["a".to_string()].into_iter().collect();
    let intent = ObjectSelectionIntent {
        command: false,
        shift: false,
        select_all: false,
        clicked_index: Some(0),
    };
    // 点目录前缀：不改变选择、不触发预览（目录点击是下钻）
    let (next, anchor, preview) = apply_object_selection(
        intent,
        &keys,
        &current,
        Some(0),
        ClickedEntry::CommonPrefix("dir/".into()),
    );
    assert_eq!(next, current);
    assert_eq!(anchor, Some(0));
    assert!(!preview);
}

#[test]
fn delete_summary_lists_names_for_multiple() {
    assert_eq!(delete_summary(&["a/b.txt".into()]), "");
    assert_eq!(
        delete_summary(&["a/1.txt".into(), "b/2.txt".into()]),
        "（1.txt、2.txt）"
    );
    let many: Vec<String> = ["1", "2", "3", "4", "5"]
        .iter()
        .map(|s| format!("x/{s}.txt"))
        .collect();
    assert_eq!(delete_summary(&many), "（1.txt、2.txt、3.txt 等 5 个）");
}

#[test]
fn delete_summary_handles_missing_extensions() {
    assert_eq!(delete_summary(&["a/b".into(), "c".into()]), "（b、c）");
}

#[test]
fn preview_download_error_message_hides_html_and_lists_checks() {
    let html = r#"API 错误 (HTTP 403): download_object: <html>
<head><title>403 Forbidden</title></head>
<body><center><h1>403 Forbidden</h1></center></body>
</html>"#;
    let message = preview_download_error_message("private-bucket", "report/a.pdf", html);

    assert!(message.contains("无法预览"));
    assert!(message.contains("远端拒绝访问"));
    assert!(message.contains("Bucket `private-bucket`"));
    assert!(message.contains("a.pdf"));
    assert!(message.contains("七牛"));
    assert!(message.contains("下载域名"));
    assert!(message.contains("阿里云 OSS"));
    assert!(message.contains("Endpoint/区域"));
    assert!(message.contains("RAM 权限"));
    assert!(!message.contains("<html>"), "不应暴露原始 HTML");
    assert!(!message.contains("<title>"), "不应暴露原始 HTML");
}

#[test]
fn sanitize_remote_error_truncates_long_plain_text() {
    let error = "x".repeat(400);
    let sanitized = sanitize_remote_error(&error);
    assert!(sanitized.ends_with('…'));
    assert!(sanitized.chars().count() <= 301);
}

#[test]
fn rename_target_key_replaces_last_segment_only() {
    assert_eq!(
        rename_target_key("a/b/c.txt", "d.txt").unwrap(),
        "a/b/d.txt"
    );
    assert_eq!(rename_target_key("top.txt", "new.txt").unwrap(), "new.txt");
    // 无扩展名：替换最后一段
    assert_eq!(rename_target_key("a/b", "c").unwrap(), "a/c");
    // 目录前缀保持（含中文）
    assert_eq!(
        rename_target_key("报告/2024/summary.pdf", "2025.pdf").unwrap(),
        "报告/2024/2025.pdf"
    );
}

#[test]
fn rename_target_key_rejects_invalid_names() {
    assert!(rename_target_key("a/b", "").is_err(), "空名");
    assert!(rename_target_key("a/b", "  ").is_err(), "纯空白");
    assert!(
        rename_target_key("a/b", "x/y").is_err(),
        "含 / 等于移动目录，禁止"
    );
    assert!(rename_target_key("a/b", ".").is_err());
    assert!(rename_target_key("a/b", "..").is_err());
}

#[test]
fn rename_target_key_allows_dotfiles_and_inner_dots() {
    // .gitignore 这类点开头的名字合法；名字中间的点也合法
    assert_eq!(
        rename_target_key("a/b", ".gitignore").unwrap(),
        "a/.gitignore"
    );
    assert_eq!(rename_target_key("a/b.tar.gz", "c.zip").unwrap(), "a/c.zip");
}

#[test]
fn rename_validation_message_reports_modal_errors() {
    let entries = vec![entry_object("dir/existing.jpg")];
    assert_eq!(
        rename_validation_message("dir/avatar.jpg", "avatar.jpg", &entries).as_deref(),
        Some("请输入一个不同的新名称")
    );
    assert_eq!(
        rename_validation_message("dir/avatar.jpg", "bad/name.jpg", &entries).as_deref(),
        Some("名称不能包含 /")
    );
    assert_eq!(
        rename_validation_message("dir/avatar.jpg", "existing.jpg", &entries).as_deref(),
        Some("目标名称已存在：existing.jpg，请换一个名字")
    );
    assert!(rename_validation_message("dir/avatar.jpg", "next.jpg", &entries).is_none());
}

#[test]
fn create_folder_target_key_keeps_current_prefix() {
    assert_eq!(create_folder_target_key(None, "photos").unwrap(), "photos/");
    assert_eq!(
        create_folder_target_key(Some("reports/2026/"), "q1").unwrap(),
        "reports/2026/q1/"
    );
    assert_eq!(
        create_folder_target_key(None, "  photos  ").unwrap(),
        "photos/"
    );
}

#[test]
fn breadcrumb_prefixes_build_click_targets() {
    assert!(breadcrumb_prefixes(None).is_empty());
    assert_eq!(
        breadcrumb_prefixes(Some("firmware/cp-es-c101/")),
        vec![
            ("firmware".to_string(), "firmware/".to_string()),
            ("cp-es-c101".to_string(), "firmware/cp-es-c101/".to_string()),
        ]
    );
    // 多余的 `/` 与重复段照实保留（服务端返回什么就展示什么）
    assert_eq!(
        breadcrumb_prefixes(Some("a//b/")),
        vec![
            ("a".to_string(), "a/".to_string()),
            ("b".to_string(), "a/b/".to_string()),
        ]
    );
    // 无尾斜杠（异常数据防御）：仍能生成段
    assert_eq!(
        breadcrumb_prefixes(Some("orphan")),
        vec![("orphan".to_string(), "orphan/".to_string())]
    );
    // Unicode 目录名按 `/` 正确切分
    assert_eq!(
        breadcrumb_prefixes(Some("报告/2026/")),
        vec![
            ("报告".to_string(), "报告/".to_string()),
            ("2026".to_string(), "报告/2026/".to_string()),
        ]
    );
}

#[test]
fn collapse_breadcrumb_short_paths_keep_all_segments() {
    let segments = breadcrumb_prefixes(Some("a/b/c/"));
    assert!(collapse_breadcrumb(&segments).is_none());
}

#[test]
fn collapse_breadcrumb_long_paths_hide_middle_keep_head_and_tail() {
    let segments = breadcrumb_prefixes(Some("l1/l2/l3/l4/l5/l6/"));
    let (collapsed_prefix, tail) = collapse_breadcrumb(&segments).expect("6 段应触发折叠");
    // 点击 `…` 直达被收起的最深一层（l2/l3/l4 被收起，l4 最深）
    assert_eq!(collapsed_prefix, "l1/l2/l3/l4/");
    // 首段 + 最后两段保留
    assert_eq!(
        tail,
        vec![
            ("l5".to_string(), "l1/l2/l3/l4/l5/".to_string()),
            ("l6".to_string(), "l1/l2/l3/l4/l5/l6/".to_string()),
        ]
    );
}

#[test]
fn create_folder_validation_message_reports_errors() {
    let entries = vec![
        ListingEntry::CommonPrefix("photos/".into()),
        entry_object("reports/"),
    ];
    assert_eq!(
        create_folder_validation_message(None, "", &entries).as_deref(),
        Some("目录名不能为空")
    );
    assert_eq!(
        create_folder_validation_message(None, "a/b", &entries).as_deref(),
        Some("目录名不能包含 /")
    );
    assert_eq!(
        create_folder_validation_message(None, "photos", &entries).as_deref(),
        Some("目录已存在：photos")
    );
    assert_eq!(
        create_folder_validation_message(None, "reports", &entries).as_deref(),
        Some("目录已存在：reports")
    );
    assert!(create_folder_validation_message(None, "next", &entries).is_none());
}

fn entry_object(key: &str) -> ListingEntry {
    ListingEntry::Object(CloudObject {
        key: key.into(),
        size: 1,
        mime_type: None,
        etag: None,
        put_time_millis: 0,
    })
}

#[test]
fn menu_targets_follow_the_entry_point() {
    // 右键入口（keep_selection = true）：open_object_menu 已按 Finder 语义把该行纳入
    // 选择，所以目标就是选择集。
    // ⋯ 入口（keep_selection = false）：**不改选择**，目标是这一行；若该行本就在多选里，
    // 则整批仍是目标（批量操作语义不变）。
    let selection: indexmap::IndexSet<String> =
        ["a.txt", "b.txt"].iter().map(|s| s.to_string()).collect();

    assert_eq!(
        menu_targets_for("b.txt", &selection, true),
        vec!["a.txt".to_string(), "b.txt".to_string()]
    );

    // ⋯ 点在「已在选择里」的行 → 整批（顺序保持）
    assert_eq!(
        menu_targets_for("b.txt", &selection, false),
        vec!["a.txt".to_string(), "b.txt".to_string()]
    );

    // ⋯ 点在「没选中」的行 → 只有这一行。**这条是安全关键**：
    // 若回落成选择集，菜单里的删除/下载会去打别的行。
    assert_eq!(
        menu_targets_for("c.txt", &selection, false),
        vec!["c.txt".to_string()]
    );

    // 空选择 + ⋯ → 只有该行
    let empty = indexmap::IndexSet::new();
    assert_eq!(
        menu_targets_for("c.txt", &empty, false),
        vec!["c.txt".to_string()]
    );
}

#[test]
fn normalize_prefix_query_trims_and_drops_leading_slash() {
    // 前缀搜索的输入规范化：空串 = 回桶根；前导 `/` 不进 key；
    // **不补尾斜杠**——参照实现是「文件前缀搜索」，输入 2025 要能列出 2025… 的对象，
    // 补成 2025/ 就只剩目录了。
    assert_eq!(
        normalize_prefix_query("  photos/2024  "),
        Some("photos/2024".into())
    );
    assert_eq!(normalize_prefix_query("/photos/"), Some("photos/".into()));
    // 前后都有斜杠与空白时一并处理
    assert_eq!(normalize_prefix_query(" /logs "), Some("logs".into()));
    assert_eq!(normalize_prefix_query("2025"), Some("2025".into()));
    // 空/纯空白 = 回到桶根
    assert_eq!(normalize_prefix_query(""), None);
    assert_eq!(normalize_prefix_query("   "), None);
    assert_eq!(normalize_prefix_query("/"), None);
}

#[test]
fn display_slot_accounts_for_directory_prefixes() {
    // 虚拟列表的 item 下标 = 显示顺序里的槽位；目录前缀也占槽位。
    // 这条守着键盘导航「把选中行滚进视野」不会滚错位置。
    let entries = vec![
        ListingEntry::CommonPrefix("photos/".into()),
        entry_object("a.txt"),
        ListingEntry::CommonPrefix("reports/".into()),
        entry_object("b.txt"),
    ];
    let natural: Vec<usize> = (0..entries.len()).collect();
    assert_eq!(display_slot_of_key(&entries, &natural, "a.txt"), Some(1));
    assert_eq!(display_slot_of_key(&entries, &natural, "b.txt"), Some(3));

    // 反序（模拟「名称降序」排序）后槽位随之改变
    let reversed = vec![3, 2, 1, 0];
    assert_eq!(display_slot_of_key(&entries, &reversed, "b.txt"), Some(0));
    assert_eq!(display_slot_of_key(&entries, &reversed, "a.txt"), Some(2));

    // 被过滤掉（不在显示顺序里）或不存在 → 没有槽位，不滚
    assert_eq!(display_slot_of_key(&entries, &[2, 0], "b.txt"), None);
    assert_eq!(display_slot_of_key(&entries, &natural, "missing.txt"), None);
}

#[test]
fn object_selection_ix_ignores_directory_prefixes() {
    let entries = vec![
        ListingEntry::CommonPrefix("photos/".into()),
        entry_object("a.txt"),
        ListingEntry::CommonPrefix("reports/".into()),
        entry_object("b.txt"),
    ];
    assert_eq!(object_selection_ix(&entries, "a.txt"), Some(0));
    assert_eq!(object_selection_ix(&entries, "b.txt"), Some(1));
    assert_eq!(object_selection_ix(&entries, "missing.txt"), None);
}

#[test]
fn directory_mixed_keyboard_multi_select_blocks_rename_then_single_rename_keeps_prefix() {
    let entries = vec![
        ListingEntry::CommonPrefix("photos/".into()),
        entry_object("photos/a.jpg"),
        ListingEntry::CommonPrefix("reports/".into()),
        entry_object("reports/b.pdf"),
        entry_object("reports/c.pdf"),
    ];
    let keys = object_keys(&entries);
    assert_eq!(keys, ["photos/a.jpg", "reports/b.pdf", "reports/c.pdf"]);

    // Keyboard path: ⌘A selects objects only; directory prefixes are ignored.
    let select_all = ObjectSelectionIntent {
        command: false,
        shift: false,
        select_all: true,
        clicked_index: None,
    };
    let empty = indexmap::IndexSet::new();
    let (multi, anchor, preview) =
        apply_object_selection(select_all, &keys, &empty, None, ClickedEntry::None);
    assert_eq!(multi.len(), 3);
    assert!(multi.contains("photos/a.jpg"));
    assert!(multi.contains("reports/b.pdf"));
    assert!(multi.contains("reports/c.pdf"));
    assert_eq!(anchor, None);
    assert!(!preview);
    assert!(
        multi.len() > 1,
        "Return rename must be blocked for multi-select"
    );

    // Pointer/keyboard recovery path: select one object from the mixed table.
    let b_ix = object_selection_ix(&entries, "reports/b.pdf").expect("b.pdf object index");
    let single_click = ObjectSelectionIntent {
        command: false,
        shift: false,
        select_all: false,
        clicked_index: Some(b_ix),
    };
    // 行级作用域（行级按钮 / ⋯ 菜单的行为）：清空勾选集合，把「当前行」指向这一行。
    // 纯函数这一层等价于「空集合 + 普通点击」——主选由调用方按 plain_click() 取点击项。
    let (scoped, scope_anchor, preview) = apply_object_selection(
        single_click,
        &keys,
        &indexmap::IndexSet::new(),
        anchor,
        ClickedEntry::Object("reports/b.pdf".into()),
    );
    assert!(scoped.is_empty(), "行级作用域不再往勾选集合里塞东西");
    assert_eq!(scope_anchor, Some(1));
    assert!(preview);
    assert!(scoped.len() <= 1, "行级作用域下单行重命名不被多选挡住");

    // Rename changes only the last segment and detects visible conflicts.
    let renamed = rename_target_key("reports/b.pdf", "renamed.pdf").unwrap();
    assert_eq!(renamed, "reports/renamed.pdf");
    assert!(!object_key_exists(&entries, &renamed));
    let conflict = rename_target_key("reports/b.pdf", "c.pdf").unwrap();
    assert_eq!(conflict, "reports/c.pdf");
    assert!(object_key_exists(&entries, &conflict));
}

#[test]
fn directory_mixed_shift_range_and_command_toggle_use_object_indexes() {
    let entries = vec![
        ListingEntry::CommonPrefix("photos/".into()),
        entry_object("photos/a.jpg"),
        ListingEntry::CommonPrefix("reports/".into()),
        entry_object("reports/b.pdf"),
        ListingEntry::CommonPrefix("archive/".into()),
        entry_object("archive/c.txt"),
        entry_object("archive/d.txt"),
    ];
    let keys = object_keys(&entries);
    assert_eq!(
        keys,
        [
            "photos/a.jpg",
            "reports/b.pdf",
            "archive/c.txt",
            "archive/d.txt"
        ]
    );

    // 普通点击第二个对象建立锚点：entries 下标是 3，对象序号必须是 1。
    let b_ix = object_selection_ix(&entries, "reports/b.pdf").expect("b.pdf object index");
    let single_click = ObjectSelectionIntent {
        command: false,
        shift: false,
        select_all: false,
        clicked_index: Some(b_ix),
    };
    let empty = indexmap::IndexSet::new();
    let (selected, anchor, preview) = apply_object_selection(
        single_click,
        &keys,
        &empty,
        None,
        ClickedEntry::Object("reports/b.pdf".into()),
    );
    assert!(selected.is_empty(), "普通点击不勾选任何东西（只设当前行）");
    assert_eq!(anchor, Some(1));
    assert!(preview);

    // ⇧Click 第四个对象：跨过目录前缀，只选对象序号 1..=3。
    let d_ix = object_selection_ix(&entries, "archive/d.txt").expect("d.txt object index");
    let shift_click = ObjectSelectionIntent {
        command: false,
        shift: true,
        select_all: false,
        clicked_index: Some(d_ix),
    };
    let (range, anchor, preview) = apply_object_selection(
        shift_click,
        &keys,
        &selected,
        anchor,
        ClickedEntry::Object("archive/d.txt".into()),
    );
    assert_eq!(range.len(), 3);
    assert!(!range.contains("photos/a.jpg"));
    assert!(range.contains("reports/b.pdf"));
    assert!(range.contains("archive/c.txt"));
    assert!(range.contains("archive/d.txt"));
    assert_eq!(anchor, Some(1), "⇧Click 不改变锚点");
    assert!(!preview);

    // ⌘Click 取消中间对象：集合移除该对象，锚点更新为它的对象序号 2。
    let c_ix = object_selection_ix(&entries, "archive/c.txt").expect("c.txt object index");
    let command_click = ObjectSelectionIntent {
        command: true,
        shift: false,
        select_all: false,
        clicked_index: Some(c_ix),
    };
    let (toggled, anchor, preview) = apply_object_selection(
        command_click,
        &keys,
        &range,
        anchor,
        ClickedEntry::Object("archive/c.txt".into()),
    );
    assert_eq!(toggled.len(), 2);
    assert!(toggled.contains("reports/b.pdf"));
    assert!(!toggled.contains("archive/c.txt"));
    assert!(toggled.contains("archive/d.txt"));
    assert_eq!(anchor, Some(2));
    assert!(!preview);
}

#[test]
fn directory_mixed_reverse_shift_range_uses_object_indexes() {
    let entries = vec![
        ListingEntry::CommonPrefix("photos/".into()),
        entry_object("photos/a.jpg"),
        ListingEntry::CommonPrefix("reports/".into()),
        entry_object("reports/b.pdf"),
        ListingEntry::CommonPrefix("archive/".into()),
        entry_object("archive/c.txt"),
        entry_object("archive/d.txt"),
    ];
    let keys = object_keys(&entries);

    // 普通点击最后一个对象建立锚点：entries 下标是 6，对象序号必须是 3。
    let d_ix = object_selection_ix(&entries, "archive/d.txt").expect("d.txt object index");
    let single_click = ObjectSelectionIntent {
        command: false,
        shift: false,
        select_all: false,
        clicked_index: Some(d_ix),
    };
    let empty = indexmap::IndexSet::new();
    let (selected, anchor, preview) = apply_object_selection(
        single_click,
        &keys,
        &empty,
        None,
        ClickedEntry::Object("archive/d.txt".into()),
    );
    assert!(selected.is_empty(), "普通点击不勾选任何东西（只设当前行）");
    assert_eq!(anchor, Some(3));
    assert!(preview);

    // ⇧Click 靠前对象：从锚点 3 反向选到对象序号 1，目录前缀不参与范围。
    let b_ix = object_selection_ix(&entries, "reports/b.pdf").expect("b.pdf object index");
    let reverse_shift = ObjectSelectionIntent {
        command: false,
        shift: true,
        select_all: false,
        clicked_index: Some(b_ix),
    };
    let (range, anchor, preview) = apply_object_selection(
        reverse_shift,
        &keys,
        &selected,
        anchor,
        ClickedEntry::Object("reports/b.pdf".into()),
    );
    assert_eq!(range.len(), 3);
    assert!(!range.contains("photos/a.jpg"));
    assert!(range.contains("reports/b.pdf"));
    assert!(range.contains("archive/c.txt"));
    assert!(range.contains("archive/d.txt"));
    assert_eq!(anchor, Some(3), "反向 ⇧Click 也不改变锚点");
    assert!(!preview);
}

fn entry_object_sized(key: &str, size: u64, time: i64) -> ListingEntry {
    ListingEntry::Object(CloudObject {
        key: key.into(),
        size,
        mime_type: None,
        etag: None,
        put_time_millis: time,
    })
}

#[test]
fn sort_entries_natural_keeps_listing_order_within_groups_but_puts_dirs_first() {
    // `Natural` 的「原序」只保证**组内**不动：目录仍然恒排在对象前（Finder 语义，
    // 参照实现亦然）。这条曾经写成「恒等映射」，把「前缀夹在中间不动」当成了预期，
    // 于是默认视图里目录会被 provider 的返回顺序推到整张表最后。
    let entries = vec![
        entry_object("z.txt"),
        ListingEntry::CommonPrefix("a/".into()),
        entry_object("m.txt"),
        ListingEntry::CommonPrefix("b/".into()),
    ];
    assert_eq!(
        sort_entries(&entries, ObjectSort::Natural),
        vec![1, 3, 0, 2]
    );
}

#[test]
fn sort_entries_name_asc_puts_dirs_first_case_insensitive() {
    let entries = vec![
        entry_object("Zebra.txt"),
        ListingEntry::CommonPrefix("Apple/".into()),
        entry_object("apple.txt"),
    ];
    let order = sort_entries(&entries, ObjectSort::NameAsc);
    let names: Vec<String> = order
        .iter()
        .map(|&i| match &entries[i] {
            ListingEntry::CommonPrefix(p) => p.clone(),
            ListingEntry::Object(o) => o.key.clone(),
        })
        .collect();
    // 目录恒在对象前；对象大小写不敏感字典序
    assert_eq!(names, vec!["Apple/", "apple.txt", "Zebra.txt"]);
}

#[test]
fn sort_entries_name_desc_reverses_objects_keeps_dirs_first() {
    let entries = vec![
        entry_object("a.txt"),
        entry_object("b.txt"),
        ListingEntry::CommonPrefix("dir/".into()),
    ];
    let order = sort_entries(&entries, ObjectSort::NameDesc);
    let names: Vec<String> = order
        .iter()
        .map(|&i| match &entries[i] {
            ListingEntry::CommonPrefix(p) => p.clone(),
            ListingEntry::Object(o) => o.key.clone(),
        })
        .collect();
    assert_eq!(names, vec!["dir/", "b.txt", "a.txt"]);
}

#[test]
fn sort_entries_size_and_time_desc() {
    let entries = vec![
        entry_object_sized("small", 1, 100),
        entry_object_sized("big", 999, 50),
        entry_object_sized("newest", 10, 300),
    ];
    let by_size = sort_entries(&entries, ObjectSort::SizeDesc);
    let keys: Vec<&str> = by_size
        .iter()
        .map(|&i| match &entries[i] {
            ListingEntry::Object(o) => o.key.as_str(),
            _ => "",
        })
        .collect();
    assert_eq!(keys, vec!["big", "newest", "small"]);

    let by_time = sort_entries(&entries, ObjectSort::TimeDesc);
    let keys: Vec<&str> = by_time
        .iter()
        .map(|&i| match &entries[i] {
            ListingEntry::Object(o) => o.key.as_str(),
            _ => "",
        })
        .collect();
    assert_eq!(keys, vec!["newest", "small", "big"]);
}

#[test]
fn object_sort_column_headers_cycle_independently() {
    assert_eq!(ObjectSort::Natural.cycle_name(), ObjectSort::NameAsc);
    assert_eq!(ObjectSort::NameAsc.cycle_name(), ObjectSort::NameDesc);
    assert_eq!(ObjectSort::NameDesc.cycle_name(), ObjectSort::Natural);
    assert_eq!(ObjectSort::SizeDesc.cycle_name(), ObjectSort::NameAsc);

    assert_eq!(ObjectSort::Natural.cycle_size(), ObjectSort::SizeDesc);
    assert_eq!(ObjectSort::SizeDesc.cycle_size(), ObjectSort::Natural);
    assert_eq!(ObjectSort::NameAsc.cycle_size(), ObjectSort::SizeDesc);

    assert_eq!(ObjectSort::Natural.cycle_time(), ObjectSort::TimeDesc);
    assert_eq!(ObjectSort::TimeDesc.cycle_time(), ObjectSort::Natural);

    assert_eq!(ObjectSort::NameAsc.name_mark(), " ↑");
    assert_eq!(ObjectSort::NameDesc.name_mark(), " ↓");
    assert_eq!(ObjectSort::Natural.name_mark(), "");
    assert_eq!(ObjectSort::SizeDesc.size_mark(), " ↓");
    assert_eq!(ObjectSort::TimeDesc.time_mark(), " ↓");
    assert_eq!(ObjectSort::NameAsc.label(), Some("名称 A→Z"));
    assert_eq!(ObjectSort::Natural.label(), None);
}

#[test]
fn visible_object_keys_skip_prefixes_and_follow_display_order() {
    let entries = vec![
        ListingEntry::CommonPrefix("photos/".into()),
        entry_object("b.txt"),
        entry_object("a.txt"),
    ];
    assert_eq!(
        visible_object_keys(&entries, &[0, 1, 2]),
        vec!["b.txt".to_string(), "a.txt".to_string()]
    );
    assert_eq!(
        visible_object_keys(&entries, &[2, 0, 1]),
        vec!["a.txt".to_string(), "b.txt".to_string()]
    );
}

#[test]
fn keyboard_nav_moves_primary_and_stops_at_ends() {
    let keys = vec!["a".into(), "b".into(), "c".into()];
    let selection: indexmap::IndexSet<String> = ["a".into()].into_iter().collect();
    let (next, anchor, target) = apply_object_keyboard_nav(
        &keys,
        &selection,
        Some("a"),
        Some(0),
        ObjectNavDirection::Next,
        false,
    );
    assert_eq!(
        next, selection,
        "不带动词的方向键只移动当前行，不动勾选集合"
    );
    assert_eq!(anchor, Some(1));
    assert_eq!(target, 1, "主选 = keys[target]，由调用方取");

    let (end, end_anchor, end_target) = apply_object_keyboard_nav(
        &keys,
        &next,
        Some("b"),
        Some(1),
        ObjectNavDirection::Next,
        false,
    );
    let (stay, stay_anchor, stay_target) = apply_object_keyboard_nav(
        &keys,
        &end,
        Some("c"),
        end_anchor,
        ObjectNavDirection::Next,
        false,
    );
    assert_eq!(stay, end, "到末尾再按 ↓：集合仍不动");
    assert_eq!(stay_anchor, Some(2), "到末尾再按 ↓ 停住");
    assert_eq!(end_target, 2);
    assert_eq!(stay_target, 2);
}

#[test]
fn keyboard_nav_without_selection_picks_first_or_last() {
    let keys = vec!["a".into(), "b".into(), "c".into()];
    let empty = indexmap::IndexSet::new();
    let (next, anchor, target) =
        apply_object_keyboard_nav(&keys, &empty, None, None, ObjectNavDirection::Next, false);
    assert!(next.is_empty(), "方向键不勾选任何东西（只移动当前行）");
    assert_eq!(anchor, Some(0));
    assert_eq!(target, 0);

    let (prev, prev_anchor, prev_target) =
        apply_object_keyboard_nav(&keys, &empty, None, None, ObjectNavDirection::Prev, false);
    assert!(prev.is_empty(), "方向键不勾选任何东西（只移动当前行）");
    assert_eq!(prev_anchor, Some(2));
    assert_eq!(prev_target, 2);
}

#[test]
fn keyboard_nav_shift_extends_from_anchor() {
    let keys = vec!["a".into(), "b".into(), "c".into(), "d".into()];
    let selection: indexmap::IndexSet<String> = ["b".into()].into_iter().collect();
    let (next, anchor, target) = apply_object_keyboard_nav(
        &keys,
        &selection,
        Some("b"),
        Some(1),
        ObjectNavDirection::Next,
        true,
    );
    assert_eq!(
        next.iter().cloned().collect::<Vec<_>>(),
        vec!["b".to_string(), "c".to_string()]
    );
    assert_eq!(anchor, Some(1), "⇧ 不改锚点");
    assert_eq!(target, 2);

    let (next, anchor, target) = apply_object_keyboard_nav(
        &keys,
        &next,
        Some("c"),
        anchor,
        ObjectNavDirection::Next,
        true,
    );
    assert_eq!(
        next.iter().cloned().collect::<Vec<_>>(),
        vec!["b".to_string(), "c".to_string(), "d".to_string()]
    );
    assert_eq!(anchor, Some(1));
    assert_eq!(target, 3);
}

#[test]
fn keyboard_nav_shift_prev_uses_primary_not_set_last() {
    let keys = vec!["a".into(), "b".into(), "c".into()];
    let selection: indexmap::IndexSet<String> =
        ["a".into(), "b".into(), "c".into()].into_iter().collect();
    // 主选在 a（集合最后一项是 c）：⇧↑ 应从 a 再往前，停在 a
    let (next, anchor, target) = apply_object_keyboard_nav(
        &keys,
        &selection,
        Some("a"),
        Some(0),
        ObjectNavDirection::Prev,
        true,
    );
    assert_eq!(target, 0);
    assert_eq!(anchor, Some(0));
    assert_eq!(
        next.iter().cloned().collect::<Vec<_>>(),
        vec!["a".to_string()]
    );
}

#[test]
fn keyboard_nav_empty_keys_clears() {
    let selection: indexmap::IndexSet<String> = ["ghost".into()].into_iter().collect();
    let (next, anchor, target) = apply_object_keyboard_nav(
        &[],
        &selection,
        Some("ghost"),
        Some(0),
        ObjectNavDirection::Next,
        false,
    );
    assert!(next.is_empty());
    assert_eq!(anchor, None);
    assert_eq!(target, 0);
}

#[test]
fn provider_icon_distinguishes_vendors() {
    assert!(matches!(
        provider_icon(ProviderKind::Qiniu),
        IconName::Globe
    ));
    assert!(matches!(
        provider_icon(ProviderKind::Aliyun),
        IconName::Building2
    ));
}

#[test]
fn cached_copy_matches_requires_suffix_and_prefix() {
    // 缓存文件名 = {nanos}-{display_name}；后缀命中且 nanos 前缀非空
    assert!(cached_copy_matches(
        Some(std::path::Path::new("/tmp/preview/123-a b.png")),
        "x/a b.png"
    ));
    // key 不同 → 不复用
    assert!(!cached_copy_matches(
        Some(std::path::Path::new("/tmp/preview/123-a b.png")),
        "x/other.png"
    ));
    // 纯名字相等（没有 nanos 前缀）不算命中——防止 /tmp/report.pdf 这种
    // 巧合路径被误判为 report.pdf 的缓存
    assert!(!cached_copy_matches(
        Some(std::path::Path::new("/tmp/report.pdf")),
        "report.pdf"
    ));
    // 无缓存 / 空 key
    assert!(!cached_copy_matches(None, "a"));
    assert!(!cached_copy_matches(
        Some(std::path::Path::new("/tmp/1-a")),
        ""
    ));
}

#[test]
fn copy_object_url_request_requires_selected_object() {
    let err = copy_object_url_request(Some("account-1"), Some("bucket-1"), None, 3600).unwrap_err();
    assert_eq!(
        err,
        DownloadMessage {
            is_error: true,
            text: "请先选中一个对象再复制链接".into(),
        }
    );
}

#[test]
fn copy_object_url_request_uses_current_selection_and_configured_ttl() {
    let object = CloudObject {
        key: "report/a b.pdf".into(),
        size: 42,
        mime_type: Some("application/pdf".into()),
        etag: Some("etag".into()),
        put_time_millis: 1,
    };
    // TTL 来自设置（⌘, 可改），不再取编译期常量
    let request =
        copy_object_url_request(Some("account-1"), Some("bucket-1"), Some(&object), 600).unwrap();
    assert_eq!(
        request,
        CopyObjectUrlRequest {
            account_id: "account-1".into(),
            bucket: "bucket-1".into(),
            key: "report/a b.pdf".into(),
            ttl_secs: 600,
        }
    );
}
