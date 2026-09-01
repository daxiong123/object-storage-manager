//! Transfer Engine：上传/下载队列、Streaming、Sleep/Wake 恢复（P0）
//!
//! TODO: 按 agents.md 的职责边界逐步实现。
//! 硬约束：Sleep/断网后 Transfer 状态为 Waiting/Paused 并恢复，不得误标 Failed；
//! 全链路 Streaming，禁止整个大文件进 Vec<u8>（agents.md §5/§6）。
