## 2026-01-12 12:34:34 CST
- 新增剪贴板图片失效问题的任务规划文档 `.codex/task.md`。
- 记录拟定的修复方向（剪贴板图片 data URL 附件）与验证路径。

- 当前暂无待办

## 2026-01-12 13:03:27 CST
- 合并 sdd/ai-ctrl-v-codex-clipboard-xi3ak 到 develop-main（merge commit）。
- 清理本次 SDD 任务计划文件 `.codex/task.md`。
- 保留 TUI2 剪贴板图片修复记录与测试结果。

- 当前暂无待办

## 2026-01-12 12:55:39 CST
- 为 TUI2 剪贴板图片新增 data URL 附件链路与日志，避免依赖本地路径读取。
- 扩展附件类型以支持 data URL，并补充相关单测。
- 更新 TUI2 状态快照以匹配当前版本号，完成测试验证。

- 当前暂无待办
## 2026-01-12 15:08:26 CST
- 本地合并 sdd/clean-clipboard-cache 到 develop-main（merge commit）。
- 新增 `/clean` 用于清理 `.codex` 下剪贴板图片缓存，并补充单测。
- 修复 slash 补全排序，确保 `/compact` 等内建命令保持展示顺序。
- 已运行 `cargo test -p codex-tui2`。

- 当前待办：
  - 运行 `just fix -p codex-tui2`（需用户确认）。
  - 推送 develop-main 到远端（当前 HTTPS/SSH 连接不可用）。
## 2026-01-12 15:30:47 CST
- 已运行 `just fix -p codex-tui2`（无代码变更，仅告警）。

- 当前待办：
  - 推送 develop-main 到远端（HTTPS/SSH 连接问题待解决）。
## 2026-01-15 12:01:31 CST
- 新增上游同步任务规划 `.codex/task.md`，覆盖基线确认、Release 选择、合并与冲突汇报、验证步骤。
- 当前处于规划阶段，等待用户确认或提出异议。

- 当前待办：
  - 等待用户确认计划与 Release 选择假设。

## 2026-01-16 01:06:17 CST
- 解决 TUI2 相关冲突：审批弹窗/Windows 沙盒引导、模型占位显示、示例提示词、聊天/页脚快照等。
- 完成 TUI2 冲突文件的合并与暂存，更新 .codex/task.md 进度。

- 当前待办：
  - 运行 `just fmt`（codex-rs）。
  - 询问用户是否执行 `just fix -p codex-tui2`，并按需运行测试（`cargo test -p codex-tui2` 等）。
## 2026-01-16 03:05:10 CST
- 运行 `cargo test --all-features`（codex-rs）；`tools::handlers::grep_files` 相关测试超过 60s 未完成，已中断。
- 记录到的告警：`codex-core` SDD git 相关 dead_code/unused，以及 TUI/TUI2/CLI 的未使用项（编译通过）。
- 未发现待处理的 `.snap.new` 快照文件。

- 当前待办：
  - 决定是否跳过/单独处理 `grep_files` 相关测试或排查 `rg` 性能问题。
  - T6 测试验证未完成（等待处理方式后补跑/确认）。

## 2026-01-16 13:41:31 CST
- 已运行 `just fix -p codex-mcp-server`（无代码变更，保留既有未使用项告警）。
- 已运行 `cargo test --all-features`（全部通过；存在已知的 dead_code/unused 警告与少量测试自带提示）。

- 当前待办：
  - 无

## 2026-01-17 20:13:11 CST
- 新增 `/model` 弹窗中英文重复显示问题的任务规划文档 `.codex/task.md`。
- 记录拟定修复方向：避免 header 与 title/subtitle 双重渲染，统一按当前语言显示。

- 当前待办：
  - 等待用户确认计划或补充需求。

## 2026-01-17 20:16:10 CST
- 按用户反馈更新任务规划：范围扩展至 TUI2，同步修复 `/model` 弹窗中英文重复显示问题。
- 更新测试与快照步骤，覆盖 `codex-tui2`。

- 当前待办：
  - 等待用户确认更新后的计划或补充需求。

## 2026-01-17 21:03:10 CST
- 通过 PR #10 以 merge commit 合并 `fix/model-menu-i18n` 到 `develop-main`。
- 完成 `/model` 弹窗中英文重复显示修复（TUI/TUI2），并更新相关快照。
- 已同步 `develop-main` 到最新合并结果。

- 当前待办：
  - 无

## 2026-01-17 21:15:36 CST
- 新增 `/sdd-develop` 继续开发无响应问题的任务规划 `.codex/task.md`。
- 记录定位结论：core 未处理 `Op::SddGitAction`，导致 Git 动作未触发。

- 当前待办：
  - 等待用户确认计划或补充需求。

## 2026-01-17 21:58:13 CST
- 在 core 补齐 `Op::SddGitAction` 调度逻辑，恢复 SDD 分支创建流程。
- 新增 SDD Git 创建分支集成测试并纳入 `tests/suite`。
- 运行 `cargo test -p codex-core sdd_git_action_create_branch_dispatches` 与 `cargo test -p codex-core`，均通过。
- 运行 `just fmt` 与 `just fix -p codex-core`。

- 当前待办：
  - 确认是否执行 `cargo test --all-features`。

## 2026-01-17 22:03:23 CST
- 合并 `sdd/fix-sdd-git-action-dispatch` 到 `develop-main`（merge commit）。
- 清理本次 SDD 计划文件 `.codex/task.md`。
- 保留已完成的测试与格式化记录。

- 当前待办：
  - 无

## 2026-01-17 22:08:09 CST
- 合并 `develop-main` 到 `main`，PR 标题为 “patch”，内容为 “fix sdd-develop workflow”。
- `main` 已包含 SDD 修复与对应测试变更。

- 当前待办：
  - 无
## 2026-02-01 13:44:30 CST
- 新增 `/sdd-develop` 基线分支改为当前分支的任务规划 `.codex/task.md`。
- 记录范围：UI 记录基线、core SDD Git 动态基线、测试与 i18n 同步。

- 当前待办：
  - 等待用户确认计划或补充需求。

## 2026-02-01 15:09:37 CST
- 通过 merge commit 将 `sdd/sdd-develop-sdd-develop` 合并到 `develop-main`。
- 清理本次 SDD 任务文件 `.codex/task.md`，删除本地开发分支。
- 更新 SDD 基线分支逻辑与 i18n，并同步 TUI/TUI2 状态快照。
- 运行 `cargo test -p codex-core`、`cargo test -p codex-tui`、`cargo test -p codex-tui2`；全量 `cargo test --all-features` 因用户说明 `rg` 问题未再重跑。

- 当前待办：
  - 如需远端清理，删除远端分支 `sdd/sdd-develop-sdd-develop`（按团队流程）。

## 2026-02-01 15:25:33 CST
- 清理项目 `.codex` 目录，仅保留 `checkpoint.md`，删除与代码无引用的 i18n/检查输出与临时文件。
- 保持现有引用逻辑不变（未发现代码中引用这些 `.codex` 文件名）。

- 当前待办：
  - 无

## 2026-02-01 15:52:27 CST
- 新增批量读取文件工具规划文档 `.codex/task.md`。
- 记录需求要点：多文件读取、通配符、结构化 JSON、20 文件与 50,000 行上限、默认全模型启用。

- 当前待办：
  - 等待用户确认计划或补充需求。

## 2026-02-01 16:04:27 CST
- 根据反馈更新任务规划：工具名改为 `batches_read_file`，并明确 JSON 参数输入。

- 当前待办：
  - 等待用户确认更新后的计划或补充需求。

## 2026-02-01 16:06:00 CST
- 更新任务规划中的建议分支名以匹配新工具名。

- 当前待办：
  - 等待用户确认更新后的计划或补充需求。
## 2026-02-09 01:55:10 CST
- 实现 `/sdd-develop-parallels`（tui/tui2）：新增命令入口、`SddWorkflow` 分流、collab experimental 门禁、并行流程计划/执行/合并提示词接入。
- parallels 路径去硬编码 Git 动作：计划批准后不再触发 `CreateBranch`，合并阶段不再触发 `FinalizeMerge`（tui2）；改为提示词引导主 Agent 编排。
- 新增中英文 i18n：`prompt.sdd_*_parallels`、`chatwidget.sdd.*parallels`、`slash_command.description.sdd_develop_parallels`，并增强现有 `sdd_execute` 协作说明。
- 补充回归测试（tui/tui2）：collab 门禁、计划批准不触发 `SddGitAction::CreateBranch`、parallels 合并不走硬编码 Git 路径。
- 同步提示词镜像文档：更新 `prompt_for_sdd_execute*.md` 协作原则，新增 plan/execute/merge parallels 中英文文档（tui 与 tui2）。
- 已执行：`just fmt`、`just fix -p codex-tui`、`just fix -p codex-tui2`、`cargo test -p codex-core i18n::tests::catalogs_share_keys`、`cargo test -p codex-tui --lib`、`cargo test -p codex-tui2 --lib`；并接受 2 处 slash popup 快照更新。

- 当前待办：
  - `cargo test -p codex-core` 全量仍有既有环境/二进制依赖失败（`codex` / `test_stdio_server` 缺失等），需按项目测试环境补齐后二次确认。
  - `cargo test -p codex-tui` 与 `cargo test -p codex-tui2` 的 integration 测试会因缺少 `codex` 二进制失败；lib 测试已全部通过。

## 2026-02-09 02:05:54 CST
- 完成收口复检：`just fmt`、`cargo test -p codex-tui`、`cargo test -p codex-tui2` 均通过（含 `tests/all.rs`）。
- 为 core 集成测试补齐依赖：执行 `cargo build -p codex-rmcp-client --bin test_stdio_server` 后，`rmcp_client/truncation` 相关失败已消除。
- `cargo test -p codex-core` 仍剩 2 个失败：`suite::model_tools::model_selects_expected_tools`、`suite::prompt_caching::prompt_tools_are_consistent_across_requests`（均为工具列表期望未包含 `batches_read_file`）。
- 已确认无未处理快照：`.snap.new` 文件为 0；`.codex/task.md` 的 T9 保持未完成并记录阻塞。

- 当前待办：
  - 评估并修复 `codex-core` 两个既有断言（工具列表期望）后，复跑 `cargo test -p codex-core` 并更新 T9 状态。

## 2026-02-09 02:35:34 CST
- 修复 `codex-core` 断言：更新 `core/tests/suite/model_tools.rs` 与 `core/tests/suite/prompt_caching.rs`，为默认工具列表补齐 `batches_read_file` 期望。
- 执行 `just fmt` 后复跑 `cargo test -p codex-core`，全量通过（单元 + `tests/all.rs` + 其余测试集）。
- 已完成 SDD 收尾：`.codex/task.md` 中 T9 由 `[ ]` 更新为 `[x]`；并确认无 `.snap.new` 残留。

- 当前待办：
  - 无

## 2026-02-09 12:29:45 CST
- 阅读并核对 `/sdd-develop` 相关实现（`tui`/`tui2` chatwidget、`core` 的 `sdd_git` 与 i18n 提示词），确认当前分支创建与合并清理时机。
- 按新需求重写 `.codex/task.md`：将工作流目标调整为“创建 task.md 前签出固定分支 `sdd-develop`，合并后删除该分支”，并补齐双端实现、测试、回滚与汇报计划。
- 保持任务状态初始化为 `[ ]`，等待执行阶段逐项勾选。

- 当前待办：
  - 等待用户确认新的任务计划或提出异议。

## 2026-02-11 18:31:00 CST
- 完成 `/spec`（`Parallel Priority`）需求相关代码与文档摸底，确认现有 AGENTS 加载与请求注入链路。
- 覆盖写入新的规划文档 `.codex/task.md`，替换旧的多 Agent 历史任务内容。
- 明确实现方向：内置提示词、弹窗开关、启用携带/禁用不携带、按当前语言选择提示词。

- 当前待办：
  - 等待用户确认 `.codex/task.md` 规划内容或提出调整意见。

## 2026-02-13 00:51:55 CST
- 采用多 sub-agent 并行完成 T18/T19/T25 收口，提交 `97cca18bc`：修复 TUI/TUI2 编译兼容、补齐 request_user_input 中断渲染链路并同步快照。
- 新增 Claude 风格工具迁移（T2-T11），提交 `a6747d9ed`：落地 `Write/Edit/Glob/NotebookEdit` 新 handler，扩展 `AskUserQuestion/Bash/Read/Grep/TodoWrite/EnterPlanMode/ExitPlanMode` 适配与 spec/注册。
- 已完成验证：`just fmt`、`cargo test -p codex-tui --quiet`、`cargo test -p codex-tui2 --quiet`、`cargo test -p codex-core tools::handlers::claude_`、`cargo test -p codex-core tools::spec::`、`cargo test -p codex-core --quiet`、`just fix -p codex-core`。

- 当前待办：
  - T17（WebFetch/WebSearch 适配）待确认是否按“网络工具不重构”策略保持现状。
  - T20（整体收尾与文档矩阵）待最终验收后勾选。

## 2026-02-12 20:39:22 CST
- 新增 Claude Code 风格工具集重构的任务规划 `.codex/task.md`，覆盖范围、里程碑、测试与风险。
- 明确新工具集清单与 WebFetch/WebSearch 不重构的范围说明。

- 当前待办：
  - 等待用户确认本次工具系统重构计划或提出调整意见。

## 2026-02-12 20:45:28 CST
- 按用户要求补充 `.codex/task.md`，新增每个工具的具体移植方案与映射关系。
- 明确各工具对应的现有 handler/模块与预期改造点。

- 当前待办：
  - 等待用户确认每个工具的移植方案或提出调整意见。

## 2026-02-12 20:47:17 CST
- 按反馈补充 TUI/TUI2 渲染适配要求，重点覆盖 TodoWrite/AskUserQuestion 相关输出。
- 更新验收与测试计划，明确 TUI/TUI2 需要可渲染且信息不丢失。

- 当前待办：
  - 等待用户确认渲染适配要求或提出调整意见。

## 2026-02-12 20:50:13 CST
- 按用户要求更新规划：明确 sub_agent 相关工具无需修改内核实现，仅通过新工具层做兼容映射。
- 在 `.codex/task.md` 补充 sub_agent 兼容范围、非范围、风险、验收与专项测试项。

- 当前待办：
  - 等待用户确认 sub_agent 兼容策略或提出进一步约束。

## 2026-02-12 20:57:20 CST
- 根据最新约束重写 `.codex/task.md`，补充工具系统重构的技术细节与实现路径。
- 明确 sub_agent 策略为“可统一入参格式，但功能语义不变”，并加入 Task/TaskOutput/TaskStop 兼容专项。
- 增补 TUI/TUI2 渲染落地细节与测试矩阵（重点覆盖 TodoWrite、AskUserQuestion）。

- 当前待办：
  - 等待用户确认详细技术方案或提出需调整的工具级设计。

## 2026-02-12 21:00:14 CST
- 按用户要求将任务规划改为“sub_agent 单元化并行执行”结构，重写 `.codex/task.md`。
- 新增并行波次、sub_agent 分工矩阵、主 Agent 冲突裁决职责与 Task/TaskOutput/TaskStop 协同策略。
- 保留“可统一入参但 sub_agent 功能语义不变”边界，并落实到任务与验收项。

- 当前待办：
  - 等待用户确认 sub_agent 分工与并行节奏是否需要进一步调整。

## 2026-02-12 21:03:11 CST
- 按用户要求将任务拆分方式改为“按功能全链路分配到具体 sub_agent（例如 Write 全链路、Read 全链路）”。
- 重写 `.codex/task.md` 的工作项与里程碑，改为工具级 Agent 拆分（T2-T19）+ 主 Agent 集成裁决（T20）。
- 补充每个功能 Agent 的交付边界、并行波次与冲突处理规则。

- 当前待办：
  - 等待用户确认功能型 Agent 列表与并行波次是否还需微调。

## 2026-02-12 21:07:57 CST
- 基于代码审计补充并发系统问题清单，并将修复项并入 `.codex/task.md`（T21-T25）。
- 新增修复范围：工具并行锁放大、wait 语义分叉、close 状态不准确与错误吞掉、TaskOutput 非阻塞语义、request_user_input 中断渲染缺口。
- 更新里程碑，新增并发稳定性修复波次（M4）与专项测试项。

- 当前待办：
  - 等待用户确认并发修复优先级与是否与工具重构同批实施。

## 2026-02-11 20:25:14 CST
- 已将分支 `sdd/spec-ai-parallel-priority-codex` 推送为远端 `sdd-spec-ai-parallel-priority-codex`（因远端存在 `sdd` 分支名冲突，无法使用斜杠分支名）。
- 已创建并合并 PR #15（Merge commit）：https://github.com/yuemingruoan/better-chinese-codex/pull/15，合并提交 `5e1bfdf9126c6ef00a45da5ad154392044d70db2`，目标分支 `develop-main`。
- 已完成收尾清理：删除 `.codex/task.md` 与临时 PR 草稿文件，拉取最新 `develop-main` 并删除本地开发分支 `sdd/spec-ai-parallel-priority-codex`，远端特性分支已删除。
- 测试记录：任务计划内定向测试均通过；补充执行 `cargo test --all-features` 时 `codex-network-proxy` 有 4 个既有失败，但单独执行 `cargo test -p codex-network-proxy --all-features --lib` 通过。

- 当前待办：
  - 跟踪 `cargo test --all-features` 下 `codex-network-proxy` 的 4 个不稳定用例（非本次 `/spec` 变更范围）。

## 2026-02-12 13:17:38 CST
- 已阅读并核对项目上下文（`AGENTS.md`、`docs/release/notes.md`、`.github/workflows`、当前 git 分支/远端状态），确认当前同步基线为 `rust-v0.98.0`。
- 已按“合并 OpenAI 官方仓库至 `rust-v0.99.0`”需求重写 `.codex/task.md`，覆盖范围/非范围、任务拆分、冲突裁决关卡、测试与回滚方案。
- 已将功能冲突处理流程明确为“你逐项裁决后再落地”，并在任务表中加入强制决策节点与汇报节奏。

- 当前待办：
  - 等待你确认 `.codex/task.md` 规划内容或提出调整意见。

## 2026-02-12 16:31:04 CST
- 已完成 11 个冲突文件的人工收敛并清除 unmerged 状态，`git diff --name-only --diff-filter=U` 结果为空。
- 已按你的裁决落地 collab `items` 语义：`spawn_agent` / `send_input` 转为必填 `items`，并同步更新对应单测与工具 schema。
- 已执行 `just fmt`、`just write-config-schema`，并补齐本轮编译阻塞修复（`tasks/sdd_git.rs` 的 `ExecRequest` 适配、`codex.rs` 会话配置字段补齐）。
- 定向验证通过：`spawn_agent_requires_items`、`send_input_requires_items`、`test_build_specs_collab_tools_enabled`、`config_schema_matches_fixture`、`suite::client::parallel_priority_spec_injected_when_enabled_and_removed_after_override`。
- `cargo test -p codex-core` 全量仍失败（20 项），主要阻塞集中在 `models.json` 新 schema 兼容（`shell_type` 缺失）及其连锁用例。

- 当前待办：
  - 继续清理 `models.json` / `models_manager` 相关失败，收敛 `codex-core` 全量回归。
  - 进入 T5/T6：补齐冲突裁决记录与剩余实现落地。

## 2026-02-12 16:57:30 CST
- 已完成 `core` 测试收敛：同步 `models.json` 到上游 `rust-v0.99.0`，更新 `tools/spec` 与相关 suite 断言（`exp-5.1` 用例移除、`apply_patch`/`batches_read_file` 顺序对齐、`exp-5.1` 不再要求 `apply_patch`）。
- 已修复 `grep_files` 集成测试前置条件：在 `grep_files` suite 内挂载远端模型并显式开启 `experimental_supported_tools=["grep_files"]`，避免 fallback 模型导致 `unsupported call: grep_files`。
- 已完成验证：`just fmt`、`cargo test -p codex-core grep_files_tool_`、`cargo test -p codex-core` 全量通过（unit + `tests/all.rs` + `responses_headers.rs`）。
- 已提交：`953ab7716`（`完成 core v0.99 模型与工具测试收敛`）。
- 已更新任务状态：`.codex/task.md` 中 `T7` 勾选为 `[x]`。

- 当前待办：
  - 进入 `T5/T6`：补齐“冲突点-候选方案-影响-推荐”的裁决记录并收口剩余落地项。
  - 进入 `T9`：更新 `docs/release/notes.md`（中英文）与版本策略复核。

## 2026-02-12 17:00:39 CST
- 已完成 `T5/T6/T9` 收口：将已裁决冲突（collab `items` 语义、network 字段共存、models.json 合并策略）写入双语发布说明并同步任务勾选。
- 已更新 `docs/release/notes.md`，新增 “Unreleased (rust-v0.99.0 upstream sync)” 中英文小节，覆盖同步范围、冲突处理与验证记录。
- 已复核版本策略：`codex-rs/Cargo.toml`、`codex-cli/package.json`、`sdk/typescript/package.json`、`shell-tool-mcp/package.json`、`codex-rs/responses-api-proxy/npm/package.json` 均保持 `1.7.1`（未对齐上游版本号）。
- 已补充验证：`cargo test -p codex-core i18n::tests::catalogs_share_keys` 通过。

- 当前待办：
  - `T8`：如需执行 `cargo test --all-features`，等待你明确授权后再跑。
  - `T10`：最终阶段汇总与交付清单确认。

## 2026-02-12 17:19:15 CST
- 已收到你的明确决策：不执行 `cargo test --all-features`，仅保留针对性测试。
- 已按豁免策略完成任务收尾：`T8` 标记为“用户确认跳过全量回归”，`T10` 标记完成并同步阶段记录。
- 本轮最终测试基线维持为针对性验证（core 相关与 i18n 一致性），不新增全量回归执行记录。

- 当前待办：
  - 无（等待你选择后续动作：继续新需求或发起合并流程）。

## 2026-02-12 17:19:15 CST
- 已通过 PR 流程完成合并：PR #16（Merge commit）`https://github.com/yuemingruoan/better-chinese-codex/pull/16`，目标分支 `develop-main`，合并提交 `3ea5285d588b58a4a945fbfaf3311571b6441456`。
- 已确认测试基线沿用本轮针对测试（按用户指令豁免 `--all-features`）：`just fmt`、`cargo test -p codex-core grep_files_tool_`、`cargo test -p codex-core`、`cargo test -p codex-core i18n::tests::catalogs_share_keys`。
- 已完成收尾清理：删除 `.codex/task.md`，保留并追加 `.codex/checkpoint.md` 阶段记录。

- 当前待办：
  - 删除合并完成后的本地/远端特性分支（按团队分支清理流程执行）。

## 2026-02-12 17:27:21 CST
- 已完成分支清理：远端 `sdd-openai-rust-v0-99-agents-md` 已删除，本地 `sdd/openai-rust-v0-99-agents-md` 已删除。
- 当前工作分支为 `develop-main`，并已同步到远端最新提交 `63c81370f`（包含 task 清理与 checkpoint 记录）。

- 当前待办：
  - 无。

## 2026-02-12 21:12:45 CST
- 已完成 Claude Code 风格工具系统重构规划收口：`.codex/task.md` 覆盖 T1-T26、M0-M6、工具全链路分工与冲突裁决规则。
- 已纳入并发修复与批量协作能力设计：P1-P6 问题、`TaskBatch`/`TaskSendBatch` 参数与返回语义、TUI/TUI2 批量摘要渲染要求。
- 已明确边界：WebFetch/WebSearch 仅做接口适配，不重构网络内核；sub_agent 既有语义保持不变，仅做兼容映射层统一。

- 当前待办：
  - 等待用户确认 `.codex/task.md`，确认后进入 T1（基线冻结与裁决规则）执行阶段。

## 2026-02-12 22:50:02 CST
- 按多 Agent 并行模式完成本轮实现收口：落地 `Task/TaskOutput/TaskStop/ToolSearch/Skill` Claude 风格 alias，新增 `task_batch`/`task_send_batch`（含 `fail_fast` 与部分成功语义）。
- 完成并发/稳定性修复：`wait`/`wait_agents` 统一 `wakeup_reason`，支持 `timeout_ms=0` 非阻塞快照；`close_agent` 返回与事件改为关闭后最终状态；`shutdown_agent_with_descendants` 不再吞掉关闭错误。
- 完成渲染链路补洞：TUI request_user_input 中断结果持久化；TUI2 新增 `RequestUserInput` 事件接线、pending 队列与中断后可见状态渲染（最小闭环）。
- 已更新 `.codex/task.md` 进度：勾选 `T12/T13/T14/T15/T16/T22/T23/T24/T26` 为完成，保留 `T18/T19/T25` 待最终 UI 回归验证。
- 验证记录：
  - 通过：`just fmt`、`cargo test -p codex-core`、`cargo test -p codex-core tools::spec::`、`cargo test -p codex-core tools::handlers::claude_tool_adapter::`、`cargo test -p codex-core collab_batch::tests::`、`cargo test -p codex-core wait_`、`cargo test -p codex-core close_agent_`、`cargo test -p codex-core shutdown_agent_reports_close_errors`。
  - 阻塞：`cargo test -p codex-tui request_user_input` 在当前分支存在既有编译错误（大量与本次改动无关的 i18n/签名不匹配）；`cargo test --manifest-path codex-rs/tui2/Cargo.toml chatwidget` 因 workspace 缺少 `mcp-types` 依赖映射无法启动。

- 当前待办：
  - 收敛 `codex-tui` 既有编译错误后，复跑 T18/T25 相关测试并决定是否勾选。
  - 处理 `codex-rs/tui2/Cargo.toml` workspace 依赖缺口后，复跑 T19/T25 相关测试并决定是否勾选。

## 2026-02-13 10:10:42 CST
- 多 Agent 并行收口新增需求：落地子 Agent 命名/重命名能力（`spawn_agent.name` + `rename_agent` + `list_agents.name` 兼容输出）。
- 完成 schema 与 handler 链路扩展：`spec.rs` 注册 `rename_agent`，`collab` handler 增加 `rename_agent` 分支并保持旧 `label` 兼容。
- 按“网络能力不重构”策略完成 T17：保留原生 `web_search` 与 MCP `WebFetch` 供给路径，不改网络内核实现。
- 已同步 `.codex/task.md`：勾选 `T1/T17/T20/T27`，并补充 T27 设计、里程碑、风险与测试条目。
- 验证记录：
  - 通过：`just fmt`、`cargo test -p codex-core tools::handlers::collab::tests::spawn_agent_name_alias_is_visible_in_list_agents`、`cargo test -p codex-core tools::handlers::collab::tests::rename_agent_updates_name`、`cargo test -p codex-core tools::spec::tests::test_spawn_agent_tool_schema`、`cargo test -p codex-core tools::spec::tests::test_rename_agent_tool_schema`、`cargo test -p codex-core`。

- 当前待办：
  - 无。

## 2026-02-13 10:48:28 CST
- 按“统一命名规范、无需向前兼容”收口子 Agent 命名：`spawn_agent` 不再兼容 `label`，统一使用 `name`。
- 调整工具适配链路：`Task` 与 `Skill` alias 映射改为输出 `name`，不再向 `spawn_agent` 透传 `label`。
- 调整 `list_agents` 输出：仅保留 `name` 命名字段；移除兼容 `label` 输出。
- 同步更新 tool schema 与测试：`spawn_agent/task_batch` schema 移除 `label` 字段并补充 legacy 参数拒绝测试。
- 验证记录：
  - 通过：`just fmt`。
  - 通过（定向）：`cargo test -p codex-core tools::handlers::collab::tests::spawn_agent_rejects_legacy_label_parameter`、`cargo test -p codex-core tools::handlers::collab::tests::spawn_agent_name_is_visible_in_list_agents`、`cargo test -p codex-core tools::handlers::collab::tests::rename_agent_updates_name`、`cargo test -p codex-core tools::handlers::claude_tool_adapter::tests::task_maps_supported_agent_type_and_name`、`cargo test -p codex-core tools::handlers::claude_tool_adapter::tests::skill_maps_to_spawnable_skill_item`、`cargo test -p codex-core tools::spec::tests::test_spawn_agent_tool_schema`、`cargo test -p codex-core tools::spec::tests::test_task_batch_tool_schema`。
  - 通过（全量）：`cargo test -p codex-core`。

- 当前待办：
  - 无。

## 2026-02-13 12:52:11 CST
- 完成 T28“全工具命名统一审计收口”：collab/alias 入参统一为 `agent_id` / `agent_ids`，移除 `task_id` / `shell_id` / `id` / `ids` 的兼容入口。
- 统一 collab 输出命名：`id -> agent_id`、`creator_id -> creator_agent_id`、`completed_ids -> completed_agent_ids`，并同步测试断言。
- 修复批量发送链路命名漂移：`task_send_batch` 参数映射改为读取 `params.agent_id`，避免混合成功场景统计误判。
- 已更新 `.codex/task.md`：新增并勾选 `T28`，补充里程碑 `M5.8`、依赖与命名专项验收条目。
- 验证记录：
  - 通过：`just fmt`。
  - 通过：`cargo test -p codex-core`（含 `tests/all.rs` 与 `responses_headers.rs`）。
  - 通过：`cargo test -p codex-tui --quiet`。
  - 通过：`cargo test -p codex-tui2 --quiet`。

- 当前待办：
  - 无。

## 2026-02-15 11:00:10 CST
- 完成本轮 SDD 收尾：`/collab` 恢复、sub-agent 预设（`edit/read/grep/run/websearch`）、`/preset`、`spec.sdd_planning` 注入、`/spec` 表格复选交互（`Tab` 勾选/取消、`Enter` 保存）、`/sdd-develop*` 规划态 Plan mode 切换均已落地并通过回归。
- 已执行并通过：`just write-config-schema`、`cargo test -p codex-tui2`、`cargo insta pending-snapshots`（无 pending）、`cargo test -p codex-core`、`just fmt`、`just fix -p codex-core`、`just fix -p codex-tui2`、`cargo build -p codex-cli --release`。
- `cargo test -p codex-core` 首轮因缺少 `test_stdio_server` 失败；补齐 `cargo build -p codex-rmcp-client --bin test_stdio_server` 后复跑全量通过（含 `tests/all.rs` 与 `responses_headers.rs`）。
- 已将 `.codex/task.md` 对应 T1-T20 全部勾选为完成，作为本轮执行看板收口记录。

- 当前待办：
  - 无。

## 2026-02-15 11:14:00 CST
- 已完成并行子任务成果主线集成：`sdd/sub-agent-edit-read-grep-run-we` 已通过本地 `--no-ff` 合并到 `develop-main`，合并提交 `fc6218d4`。
- 合并前后无代码漂移：`git diff --name-only 690415b6 fc6218d4` 为空；沿用已完成验证结论（`just write-config-schema`、`cargo test -p codex-core`、`cargo test -p codex-tui2`、`cargo insta pending-snapshots`、`just fmt`、`just fix -p codex-core`、`just fix -p codex-tui2`、`cargo build -p codex-cli --release`）。
- 冲突处理结论：无文本冲突（`ort` 自动合并）；测试阻塞曾来自 `test_stdio_server` 缺失，已通过 `cargo build -p codex-rmcp-client --bin test_stdio_server` 解决并复验通过。
- 清理执行：无临时 worktree；尝试删除本地特性分支被策略拦截（命令被 policy 拒绝），已记录为后续手工清理项。

- 当前待办：
  - 手工删除本地分支 `sdd/sub-agent-edit-read-grep-run-we`（当前会话策略拦截自动删除）。

## 2026-02-16 00:41:50 CST
- Finalized collab/preset/sdd integration on branch `sdd/1-collab-collab-collab-plan-prox` and synced `.codex/task.md` completion status (`T1`-`T14` marked done).
- Updated docs for new behaviors: `/collab` Plan/Proxy/Close gating, `/spec` collab dependency for Parallel Priority, `/preset` action changes, and `/sdd-develop` branch timing.
- Verified release build with `cd codex-rs && cargo build -p codex-cli --release` (success, warnings only).

- No pending items

## 2026-02-16 02:32:59 CST
- Completed final merge closure for `sdd/1-collab-collab-collab-plan-prox`: merged into `develop-main` with local `--no-ff` merge commit (`ort` strategy, no conflicts).
- Confirmed `.codex/task.md` task table is fully checked (`T1`-`T14`) and used as merge gate.
- Reused latest validated results from this wave: targeted `codex-core`/`codex-tui2` tests, `just fix -p codex-core`, `just fix -p codex-tui2`, `just fmt`, and `cargo build -p codex-cli --release`.
- Performed workspace cleanup audit: no temporary worktrees, no uncommitted files on `develop-main`.

- Pending items:
  - Local branch deletion for `sdd/1-collab-collab-collab-plan-prox` is still pending because `git branch -d` is blocked by session policy.

## 2026-02-16 执行阶段
- 门禁检查：`~/.codex/config.toml` 中 `[features].collab = true`，允许进入 parallels 执行
- Sub-A 交付：`codex-rs/cli/src/main.rs` 完成 TDD（先失败后通过），新增 `select_tui_frontend` 回归测试
- Sub-B 交付：`codex-rs/cli/Cargo.toml` 依赖修复，移除 `cfg(not(windows))` 对 `codex-tui2` 与 `codex-utils-absolute-path` 的限制
- 里程碑提交：`b24b8b59` `fix(cli): enable tui2 frontend selection on windows`

## 2026-02-16 集成阶段
- 主集成验证：`cd codex-rs && just fmt`（通过）
- 主集成验证：`cd codex-rs && cargo test -p codex-cli`（通过）
- 残留风险：当前 macOS 环境交叉 `--target x86_64-pc-windows-msvc` 仍报 target/core 解析异常，`T5` 待 Windows 实机复验

## 2026-02-16 合并收尾阶段
- 已将 `sdd/windows-tui2-bug` 通过本地 `--no-ff` 合并到 `develop-main`，生成合并提交：`12646347`（`ort`，无冲突）。
- 合并后再次验证：`cd codex-rs && just fmt`、`cd codex-rs && cargo test -p codex-cli` 均通过。
- Windows 交叉检查现状：`cd codex-rs && cargo check -p codex-cli --target x86_64-pc-windows-msvc` 在当前环境失败（`E0463: can't find crate for core/std`），需 Windows 实机/标准工具链环境复验 `T5`。
- 现场清理：无额外 worktree、无临时日志/过渡产物残留。
- 清理阻塞：尝试删除本地分支 `sdd/windows-tui2-bug` 被会话策略拦截（blocked by policy），需后续在允许策略下执行。
