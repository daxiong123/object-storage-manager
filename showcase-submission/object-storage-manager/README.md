# Object Storage Manager

A native, high-performance macOS workspace for Qiniu Kodo and Aliyun OSS object storage. Unlike a web console in a shell, Object Storage Manager is a real desktop application: fast to launch, low on memory, keyboard-first, and deeply integrated with the system.

## Overview

Object Storage Manager brings Finder-grade interaction habits to cloud object storage, combined with the look, feel, and responsiveness of a modern native app. It is built entirely with Rust and the GPUI framework — no Electron, Chromium, or WebView.

## Key features

- **Account management** — Multiple accounts for Qiniu Kodo and Aliyun OSS. Account metadata lives in SQLite while secrets are stored in the macOS Keychain, with compensation on failed creation and idempotent removal.
- **Bucket and object browsing** — Drill into buckets and prefixes with a breadcrumb trail, refresh, filter, multi-select, range selection, and select all. Pick up where a mouse or a keyboard starts: the same actions power the command palette, menus, shortcuts, and the context menu.
- **Object operations** — Upload files and folders, drag and drop from Finder, download in bulk, delete with confirmation, inline rename, create directories, and copy or move objects within the current bucket.
- **Preview and open** — Preview images in-app, view and edit text files with save-back, and hand PDFs, Office documents, and videos to Quick Look. Open with, and reveal in Finder are included.
- **Resilient transfer engine** — A queue with a state machine, configurable concurrency, byte-level progress, and UI throttling. Transfers pause when the machine sleeps or the network drops and resume afterwards instead of being mislabeled as failed. An active queue is persisted so quitting restores it on next launch.
- **macOS integration** — System Keychain for credentials, Quick Look for previews, clipboard handling for signed URLs, notifications, system appearance, and native file panels throughout.
- **Theme system** — Semantic design tokens with Light, Dark, and System modes, with selection colors separated from primary colors.

## Workflows

- **Browse and transfer**: connect to a provider, drill into a bucket, filter the object list, then upload, download, or move items in bulk with live progress.
- **Preview without downloading**: open an image or text file directly, or press Space for a quick peek, and navigate between objects with the arrow keys.
- **Secure links**: generate signed URLs with a configurable time-to-live, with an option to clear the clipboard afterwards.
- **Safe exits**: quit while transfers are active and pick up exactly where the queue left off on the next launch.

## Use cases

- Developers and operators who manage assets, backups, and deliverables across Qiniu Kodo and Aliyun OSS from one native app.
- Teams that prefer keyboard-driven desktop workflows over browser consoles.
- Anyone who needs secure, streaming transfers of large files without loading them into memory.

## Platform

Approved platforms: **macOS 14 or later**, Apple Silicon preferred. The project is under active development; the core browsing and transfer paths are already usable.