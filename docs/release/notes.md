# Release Notes / 发布说明

## Unreleased / 未发布

### English
- The update script now supports macOS Apple Silicon (arm64) only.
- The script exits early in unsupported runtime environments.
- Release workflow now verifies the macOS artifact is arm64-only before publishing.

### 中文
- 更新脚本改为仅支持 macOS Apple Silicon（arm64）。
- 在不支持的运行环境中，脚本会提前退出。
- 发布流程新增 macOS 产物架构校验，发布前确保仅包含 arm64。

## v1.7.5 (changes since 1.7.4)

### English

#### Release Scope
- Merged all updates from `develop-main` into `main` for this release window.
- Bumped release version to `1.7.5` across workspace and package artifacts.

#### Repository Migration Notice
- Repository name is now `yuemingruoan/better-codex` (previously `yuemingruoan/better-chinese-codex`).
- Release and update links were migrated to the new repository.
- This release explicitly marks the repository migration to avoid stale update targets.

#### Update Check & Upgrade Prompt Fixes
- Updated TUI/TUI2 release page links and GitHub API update-check endpoint to `yuemingruoan/better-codex`.
- Updated release-note links shown in update prompts to point to the new repository.
- Relaxed TUI tag parsing to accept `rust-vX.Y.Z`, `vX.Y.Z`, and `X.Y.Z`, so update detection works with fork tags.

#### Core Runtime Fix
- Fixed a Windows regression where `features.tui2=true` could not activate TUI2 due to CLI-side gating.
- Added/updated tests around frontend selection logic to ensure Windows respects TUI2 configuration.

---

### 中文

#### 发布范围
- 本次发布将 `develop-main` 上的更新完整并入 `main`。
- workspace 与相关包版本统一升级至 `1.7.5`。

#### 仓库迁移说明
- 仓库名已迁移为 `yuemingruoan/better-codex`（原为 `yuemingruoan/better-chinese-codex`）。
- 发布页与更新检查链接已统一迁移到新仓库。
- 本次版本在发布说明中明确记录仓库迁移，避免旧地址导致的更新失败。

#### 更新检查与升级提示修复
- 将 TUI/TUI2 的 Release 页面链接与 GitHub API 更新检查地址切换为 `yuemingruoan/better-codex`。
- 更新提示中的 Release Notes 链接改为新仓库地址。
- 放宽 TUI 标签解析规则：支持 `rust-vX.Y.Z`、`vX.Y.Z`、`X.Y.Z`，适配 fork 的版本标签格式。

#### 核心运行时修复
- 修复 Windows 下 `features.tui2=true` 仍无法启用 TUI2 的 CLI 入口门禁问题。
- 补充并更新前端选择逻辑测试，确保 Windows 平台按配置正确切换到 TUI2。
