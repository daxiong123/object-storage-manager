//! SQLite 持久化（~/Library/Application Support/），永不存 Secret
//!
//! TODO: 按 agents.md 的职责边界逐步实现。
//! 红线：SQLite 永远不保存 Secret，Secret 只进 macOS Keychain（agents.md §5/§6）。
