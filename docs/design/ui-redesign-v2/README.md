# CloudStorage UI 重设计 — 最终定稿「Blob0 式平铺」（v3）

> **状态**：Phase 1+2 已落代码（2026-10-20）  
> **参照**：Blob0（https://blob0.com/zh/ ，原图分析见 `blob0-reference.md`）  
> **范围**：主窗口壳层 + 侧栏 + 对象工作区 + 文件图标  
> **约束**：对齐 `agents.md` §5.6/§5.7/§7（已同步更新）

## 最终方向（2026-10-20 拍板）

用户从 Workbench v2（Zed 式卡片 + 冷灰）改为 **Blob0 式 Finder 文件管理器语言**：
平铺白底、实心蓝选中、彩色类型图标、主色上传按钮。**网格视图与账号卡移底部暂缓**。

## 已落地

| 设计 | 实现 | 备注 |
|------|------|------|
| 平铺壳层 | `theme.rs` 色板 v3：亮色 chrome `#F5F6F8`/内容白；暗色 chrome `#17191C`/内容 `#0E0F11`；1px 分隔线 | 撤掉了 Phase 1 的内容卡片（用户改选 Blob0 方向） |
| 侧栏实心蓝 pill | `sidebar_row` / rail：选中 = `theme.primary` 底 + `primary_foreground` 字 | 「选中≠主色」唯一例外，agents.md §5.7/§7 已注明；弹层内选中仍淡染 |
| 上传主色分裂按钮 | `render_object_toolbar`：`toolbar-upload` 改 `.primary()`；新建目录改描边默认按钮 | 「主色只给对话框提交」翻转，agents.md §5.6 已注明 |
| 彩色类型图标 | `theme::file_type_icon_color(kind, mode)` + `file_folder_color(mode)`；`file_type.rs` 与各目录调用点接入 | 媒体粉/视频紫/音频薄荷/代码青/压缩包琥珀/表格草绿/字体品红/文档浅蓝；未知=中性灰 |
| 标题栏面包屑凹陷槽 | Phase 1 落地，保留（与 Blob0 的 inset 搜索框同语言） | `inset_background(mode)` 色值随 v3 色板重调 |

## 验证

`cargo fmt --check` / `clippy --all-targets`（无告警）/ `cargo test --workspace`
（246 passed / 2 ignored，比基线多 1 条类型图标测试）+ 20 条色板纪律测试全绿。
视觉验收走 §5.8 离屏预览截图。

## 设计稿 HTML（历史对照，部分已被代码超越）

- [`workbench-light.html`](./workbench-light.html) / [`workbench-dark.html`](./workbench-dark.html) — Workbench v2 卡片版（已弃用方向，仅留档）
- [`components.html`](./components.html) — 按钮/菜单/弹层/空态/分段控件

## 暂缓（另立项）

- 网格视图（Blob0 大图标网格：新布局 + 图标资产 + 视图切换）
- 账号卡移侧栏底部（IA 调整）
- 预览浮层底部元数据条（B8，小，可随手做）
- 内容区 bucket 标题块 + 已加载统计（B3；统计只能按已加载条目算，需诚实标注）

在浏览器打开同目录下的 HTML（无需构建）：

- [`workbench-light.html`](./workbench-light.html) — 亮色主界面
- [`workbench-dark.html`](./workbench-dark.html) — 暗色主界面
- [`components.html`](./components.html) — 按钮、菜单、弹层、空态
