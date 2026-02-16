# AGENTSS Release Flow / 固定发布流程

> 适用仓库：`better-codex`
> 默认分支策略：`develop-main` 开发，`main` 发布。

## 1. 版本号修改（必须同步）
将目标版本（例如 `1.7.4`）同步到以下位置：

- `codex-rs/Cargo.toml`（workspace `version`）
- `codex-cli/package.json`
- `sdk/typescript/package.json`
- `shell-tool-mcp/package.json`
- `codex-rs/responses-api-proxy/npm/package.json`

## 2. 发布说明
覆盖写入：`docs/release/notes.md`

要求：
- 只记录“上一个版本 -> 当前版本”的变更。
- 中英文都要覆盖。
- 内容聚焦：范围、核心功能变更、配置/文档变更、验证结果。

## 3. 合并流程（develop-main -> main）
- 在 `develop-main` 完成本次发布提交。
- 发起 PR：`develop-main` -> `main`。
- 按仓库策略合并（推荐 Merge commit），记录冲突处理结论。

## 4. 触发发布工作流
使用 `.github/workflows/release.yml` 手动触发：

- `tag`: `v<semver>`（例如 `v1.7.4`）
- `target`: `main`
- `notes_file`: `docs/release/notes.md`
- `draft`: `false`（常规正式发布）
- `prerelease`: `false`（常规正式发布）

## 5. 发布后核对
- 核对 Release 标签与标题一致（如 `v1.7.4`）。
- 核对 Release 正文来源于 `docs/release/notes.md`。
- 核对 `main` 分支包含版本号与 notes 更新。
