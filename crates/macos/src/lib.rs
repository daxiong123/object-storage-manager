//! macOS native integration：Keychain / NSPanel / NSWorkspace / QuickLook / Pasteboard / 通知
//!
//! TODO: 按 agents.md 的职责边界逐步实现。
//! 依赖方向：objc2 / objc2-app-kit / objc2-foundation / objc2-security。
//! 原则：GPUI 负责主 UI，macOS Framework 负责系统级能力（agents.md §3/§5）。
