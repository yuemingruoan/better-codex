# Release Notes / 发布说明

## v1.7.4 (changes since 1.7.3)

### English

#### Release Scope
- Merged all updates from `develop-main` into `main` for this release window.
- Integrated `4` commits since `v1.7.3` (`13 files changed`, `+675 / -366`).
- Bumped release version to `1.7.4` across workspace and package artifacts.

#### Collaboration Gate & Preset Flow
- Moved collab gating into `/collab` with three explicit options: `Plan`, `Proxy`, `Close`.
- Selecting `Plan` or `Proxy` now automatically enables collab; selecting `Close` disables collab.
- Added `/spec` guard so `Parallel Priority` can only be enabled after collab is enabled.

#### Presets, Models, and i18n
- Removed preset clear actions for model/reasoning overrides from `/preset`.
- Set all built-in sub-agent preset defaults to `gpt-5.3-codex` + `low` reasoning.
- Restored `gpt-5.3-codex` visibility in `/model` picker.
- Completed bilingual i18n updates for `/collab`, `/preset`, and related status/persistence messages.

#### SDD Workflow & Docs
- Updated `/sdd-develop` so branch creation happens before task planning prompt emission.
- Updated `/sdd-develop-parallels` planning/execution prompts with explicit task.md chapter requirements and function-based sub-agent guidance.
- Updated `docs/config.md` and `docs/slash_commands.md` to match current behavior.

---

### 中文

#### 发布范围
- 本次发布将 `develop-main` 上的更新完整并入 `main`。
- 纳入自 `v1.7.3` 以来 `4` 个提交（`13` 个文件变更，`+675 / -366`）。
- workspace 与相关包版本统一升级至 `1.7.4`。

#### Collab 门禁与预设流程
- 将 collab 门禁下沉到 `/collab`，提供 `Plan`、`Proxy`、`Close` 三个明确选项。
- 选择 `Plan` 或 `Proxy` 时自动启用 collab；选择 `Close` 时关闭 collab。
- 为 `/spec` 增加门禁：仅在已启用 collab 后才允许开启 `Parallel Priority`。

#### 预设、模型与 i18n
- 在 `/preset` 中移除“清除模型覆盖/清除推理覆盖”操作项。
- 将内置 5 个 sub-agent 预设默认值统一为 `gpt-5.3-codex + low`。
- 恢复 `gpt-5.3-codex` 在 `/model` 选择器中的可见性。
- 补齐 `/collab`、`/preset` 及相关状态/持久化提示的中英文 i18n 文案。

#### SDD 工作流与文档
- 调整 `/sdd-develop`：在发送 task 规划提示前先创建分支。
- 调整 `/sdd-develop-parallels` 规划/执行提示词：显式 task.md 章节要求，并强调按功能拆分子 Agent。
- 同步更新 `docs/config.md` 与 `docs/slash_commands.md` 以匹配当前行为。
