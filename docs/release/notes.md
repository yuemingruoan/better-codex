# Release Notes / 发布说明

## v1.7.3 (changes since 1.7.2)

### English

#### Release Scope
- Merged all updates from `develop-main` into `main` for this release window.
- Integrated `6` commits since `v1.7.2` (`49 files changed`, `+2,035 / -180`).
- Bumped release version to `1.7.3` across workspace and package artifacts.

#### Collaboration Presets & TUI2 Workflow
- Restored TUI2 `/collab` entry and aligned its behavior with collab experimental gating.
- Added TUI2 `/preset` for sub-agent presets (`edit`, `read`, `grep`, `run`, `websearch`) with runtime update + config persistence.
- Added planning-mode integration for `/sdd-develop` and `/sdd-develop-parallels`, including automatic plan/default mode transitions.
- Updated `/spec` interaction to support tabular checkbox-style selection (`[ ]`, `Tab`, `Enter`) and expanded multi-agent guidance.

#### Config, Injection & Prompt Behavior
- Added `sub_agent_presets` and `spec.sdd_planning` config surfaces, with persistent edit APIs and schema updates.
- Extended collab tool schema/adapter to support preset-aware sub-agent spawning while preserving existing agent types.
- Refined spec-instruction injection scope: `parallel_priority` / `sdd_planning` prompts are injected only for user-prompt sampling requests, not follow-up tool/report loops.

#### i18n, Docs & Structure
- Added/updated bilingual (English/Chinese) prompt and UI copy for presets, SDD planning guidance, and sub-agent naming conventions.
- Updated `docs/config.md` and `docs/slash_commands.md` for `/preset`, `/collab`, and new spec controls.
- Removed legacy release-note file `docs/release-notes/1.6.5.md` as part of docs structure cleanup.

---

### 中文

#### 发布范围
- 本次发布将 `develop-main` 上的更新完整并入 `main`。
- 纳入自 `v1.7.2` 以来 `6` 个提交（`49` 个文件变更，`+2,035 / -180`）。
- workspace 与相关包版本统一升级至 `1.7.3`。

#### 协作预设与 TUI2 工作流
- 恢复 TUI2 `/collab` 入口，并与 collab experimental 门禁保持一致。
- 新增 TUI2 `/preset`，支持子 Agent 预设（`edit`、`read`、`grep`、`run`、`websearch`）的运行时调整与配置持久化。
- 为 `/sdd-develop` 与 `/sdd-develop-parallels` 增加 planning 阶段模式切换，自动在 plan/default 模式间转换。
- 更新 `/spec` 交互为表格式复选选择（`[ ]`、`Tab`、`Enter`），并扩展多 Agent 指导文案。

#### 配置、注入与提示词行为
- 增加 `sub_agent_presets` 与 `spec.sdd_planning` 配置能力，并补齐持久化编辑 API 与 schema。
- 扩展 collab 工具 schema/adapter，支持按 preset 生成子 Agent 参数，同时保持既有 agent type 不变。
- 收敛规范注入范围：`parallel_priority` / `sdd_planning` 提示词仅在用户提示请求时注入，不再在工具 follow-up/汇报循环中重复附加。

#### i18n、文档与结构
- 新增/更新中英文提示词与 UI 文案，覆盖预设说明、SDD planning 规范、按职责命名子 Agent 指引。
- 更新 `docs/config.md` 与 `docs/slash_commands.md`，补充 `/preset`、`/collab` 与新 spec 开关说明。
- 清理遗留发布说明文件 `docs/release-notes/1.6.5.md`，完成文档结构收口。

---

## v1.7.2 (changes since 1.7.1)

### English

#### Release Scope
- Merged all updates from `develop-main` into `main` for this release window.
- Synced fork with upstream `rust-v0.84.0..rust-v0.99.0` and integrated `233` commits since `v1.7.1` (`651 files changed`, `+46,517 / -11,749`).
- Kept fork merge policies intact: root `README.md` unchanged, Chinese prompt assets retained, and only release-critical GitHub workflows preserved.
- Bumped release version to `1.7.2` across workspace and package artifacts.

#### Tool System Refactor & CLI Behavior
- Completed the staged tool-system refactor and Claude-style tool protocol alignment across core, CLI, TUI, and TUI2.
- Unified tool naming and parameter semantics; standardized `agent_id`/sub-agent-related call shapes across collab flows.
- Added `search_tool` support and improved tool capability gating behavior.
- Improved `view_image` handling with model-modality gating and text-safe output sanitization for incompatible models.
- Hardened execution policy validation (including empty-command list guards) and reduced ambiguity in approval matching logic.
- Split command parsing/safety concerns into dedicated crates and removed obsolete feature toggles/legacy paths.

#### Collaboration & Multi-Agent Workflow
- Refined sub-agent lifecycle controls across `spawn_agent`, `send_input`, `wait`, `wait_agents`, and close flows.
- Landed `resume_agent` support and improved interrupt/result rendering behavior in interactive clients.
- Unified sub-agent naming semantics (migrating UI/protocol surface from `label` semantics to `name`-oriented behavior where applicable).
- Improved agent lifecycle governance details and status synchronization to reduce stuck/hidden state edge cases.
- Added/updated parallel tool execution capability annotations and related runtime plumbing.

#### Memory, Backfill & State Reliability
- Delivered memory v2 staged rollout (PR1-PR6 + consolidation work), including extraction/prompt pipeline foundations.
- Improved backfill stability: resumed workflows, deduplication safeguards, and stronger readiness ordering.
- Strengthened compaction accounting and failure-state handling for post-response item tracking.
- Added DB repair safeguards and improved thread metadata utilization for richer history/summary behavior.

#### Network, Proxy, Sandbox & Transport
- Expanded websocket transport support: preconnect behavior, compression support, and HTTP fallback when upgrade is not available.
- Preferred websocket transport when model/provider configuration opts in.
- Added proxy-aware network routing inside sandboxed execution paths and improved SOCKS default behavior.
- Reserved loopback ephemeral listeners for managed proxy coordination and improved structured blocked-error reporting.
- Added richer runtime/network context to telemetry and environment surfaces (including sandbox/network metadata propagation).
- Improved no-network sandbox hardening (including `io_uring` syscall restrictions) and fixed related Linux flake paths.

#### TUI / TUI2 Experience & i18n
- Added `/spec` workflow interactions and synchronized plan/execute/merge prompt behavior in TUI and TUI2.
- Added `/statusline` command capabilities and completed statusline prompt localization (Chinese/English).
- Improved `request_user_input` rendering for long option labels and directional hints.
- Improved draft rehydration and placeholder restoration for image and resume-related flows.
- Fixed multiple UX edge cases: history recall cursor position, steer-mode tab submit behavior, and unified-exec working-line summary retention.

#### App-Server, Apps & API Surface
- Expanded app-server capability set, including turn/steer APIs and websocket transport iteration.
- Added optional event suppression and experimental-feature listing surfaces where applicable.
- Improved app loading/installation behavior and app feature-check request paths.
- Updated auth-related app-server flow for external auth compatibility.
- Continued alignment of protocol and requirement surfaces (including experimental network/config exposure).

#### Models, Packaging, Build & Test Stability
- Updated `models.json` and related model/tool gating logic to match upstream behavior while preserving fork policy defaults.
- Removed offline fallback model paths and tightened provider/model resolution logic.
- Migrated platform binary publishing behavior to `@openai/codex` dist-tag strategy and continued npm package split alignment.
- Landed broad CI/test deflake work across Linux/Windows/Bazel/nextest scenarios.
- Improved cross-platform reliability for shell timeouts, line ending normalization, and workspace resource resolution.

#### Documentation & Process
- Updated release/process documentation and migration checkpoints for upstream sync and tool-system rollout.
- Kept docs aligned to fork policy: release workflows preserved and release notes maintained in bilingual format.

---

### 中文

#### 发布范围
- 本次发布将 `develop-main` 上的更新完整并入 `main`。
- 完成上游 `rust-v0.84.0..rust-v0.99.0` 同步，并纳入自 `v1.7.1` 以来 `233` 个提交（`651` 个文件变更，`+46,517 / -11,749`）。
- 持续遵循 fork 合并策略：根目录 `README.md` 不覆盖、中文提示词资产保留、仅保留发布关键工作流。
- workspace 与相关包版本统一升级至 `1.7.2`。

#### 工具系统重构与 CLI 行为
- 完成分阶段工具系统重构，吸收并落地 Claude 风格工具协议对齐（覆盖 core/CLI/TUI/TUI2）。
- 统一工具命名与参数语义，收敛 `agent_id` 与子 Agent 相关调用契约。
- 新增 `search_tool` 能力，并完善工具能力门控路径。
- 改进 `view_image`：按模型模态能力门控，并对不兼容模型输出进行文本安全处理。
- 强化执行策略校验（含空命令列表防护）与审批匹配一致性。
- 拆分命令解析/安全职责到独立 crate，清理过时特性开关与历史兼容路径。

#### 协作与多 Agent 工作流
- 持续打磨 `spawn_agent`、`send_input`、`wait`、`wait_agents` 与关闭链路的生命周期控制。
- 增加 `resume_agent` 并改进中断/结果在交互端的渲染一致性。
- 统一子 Agent 命名语义，在相关链路中完成从 `label` 到 `name` 语义收敛。
- 完善 Agent 状态治理与生命周期同步，降低卡死态/幽灵态问题。
- 增补并行工具执行能力标注与运行时接线。

#### 记忆、回填与状态可靠性
- 落地 memory v2 分阶段能力（PR1-PR6 + consolidation），补齐提取与提示管线基础设施。
- 强化 backfill 稳定性：支持可恢复流程、重复回填防护与更稳健的就绪顺序。
- 完善 compact 统计与失败态处理，覆盖 response 后续 item 的计量一致性。
- 增加数据库修复保护，并利用状态库元数据改善线程摘要/历史体验。

#### 网络、代理、沙箱与传输
- 扩展 websocket 传输能力：预连接、压缩支持，以及升级失败时回退 HTTP。
- 在模型/提供方允许时优先使用 websocket 传输。
- 在沙箱路径中引入代理感知网络路由，并优化 SOCKS 默认行为。
- 为托管代理保留 loopback 临时监听端口，改进结构化阻断错误信息。
- 扩展遥测与环境上下文中的网络/沙箱元数据透出。
- 强化无网络沙箱策略（含 `io_uring` 限制）并修复相关 Linux 波动问题。

#### TUI / TUI2 体验与 i18n
- 新增 `/spec` 工作流交互，并在 TUI 与 TUI2 同步 plan/execute/merge 提示词行为。
- 增强 `/statusline` 指令能力，完成状态栏提示中英文 i18n。
- 优化 `request_user_input` 长选项换行与方向提示显示。
- 改进草稿重建、图片占位与 resume 相关恢复体验。
- 修复多项交互边界：历史回溯光标位置、steer 模式 Tab 提交、统一执行摘要保留。

#### App-Server、Apps 与 API
- 扩展 app-server 能力集（含 turn/steer API 与 websocket 传输迭代）。
- 增补可选事件抑制与实验特性枚举能力。
- 改进 app 加载/安装流程与能力检查请求路径。
- 优化外部鉴权场景下的 app-server 鉴权处理。
- 持续对齐协议与 requirements 面（含 experimental network/config 透出）。

#### 模型、打包、构建与测试稳定性
- 同步 `models.json` 与模型/工具门控逻辑，保持 fork 默认策略不变。
- 移除离线 fallback 路径，收紧 provider/model 解析策略。
- 平台二进制发布策略迁移至 `@openai/codex` dist-tag，并持续推进 npm 包拆分一致性。
- 大范围落地 Linux/Windows/Bazel/nextest 的 CI 与测试去抖修复。
- 提升跨平台稳定性（shell 超时、行尾归一化、工作区资源路径解析等）。

#### 文档与流程
- 更新上游同步与工具重构相关的发布文档和阶段记录。
- 持续遵循 fork 发布策略：保留关键工作流并维护中英文双语发布说明。

---
