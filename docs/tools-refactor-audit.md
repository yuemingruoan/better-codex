# 工具系统重构审计手册（2026-02）

本文用于记录本轮工具系统重构后的**工具名称、参数契约、返回值契约**，方便后续审计与修改。

适用范围：
- `codex-rs/core/src/tools/spec.rs` 中静态注册的工具（含 Claude 风格别名）。
- `codex-rs/core/src/tools/handlers/*.rs` 中对应处理器返回结构。
- 不含外部 MCP 服务自定义工具的完整枚举（仅说明接入方式与边界）。

## 1. 命名统一约定（本轮收口结论）

- Agent 标识统一：
  - 单个：`agent_id`
  - 多个：`agent_ids`
- 创建者过滤标识统一：`creator_agent_id`（输出字段）
- 等待完成列表统一：`completed_agent_ids`（输出字段）
- 已移除兼容入口（不再建议使用）：`id` / `ids` / `task_id` / `shell_id` / `creator_id`（对应相关工具）

---

## 2. 协作基础工具（CollabHandler / CollabBatchHandler）

> 入口定义：`codex-rs/core/src/tools/spec.rs`  
> 处理实现：`codex-rs/core/src/tools/handlers/collab.rs`、`codex-rs/core/src/tools/handlers/collab_batch.rs`

| 工具名 | 参数（`*` 必填） | 返回值（FunctionCall 文本体中的 JSON） |
| --- | --- | --- |
| `spawn_agent` | `items*`, `agent_type`, `name`, `acceptance_criteria`, `test_commands`, `allow_nested_agents`, `model`, `reasoning_effort`, `reasoning_summary`, `approval_policy`, `sandbox_mode` | `{ "agent_id": string }` |
| `send_input` | `agent_id*`, `items*`, `interrupt` | `{ "submission_id": string }` |
| `task_batch` | `operations*[]`（元素：`id`, `params`=`spawn_agent` 参数），`fail_fast` | `{ "results": [{ "id"?, "success": bool, "agent_id"?, "error"? }], "summary": { "total": u64, "succeeded": u64, "failed": u64 } }` |
| `task_send_batch` | `operations*[]`（元素：`id`, `params`=`send_input` 参数），`fail_fast` | `{ "results": [{ "id"?, "success": bool, "agent_id"?, "submission_id"?, "error"? }], "summary": { "total": u64, "succeeded": u64, "failed": u64 } }` |
| `resume_agent` | `agent_id*` | `{ "status": AgentStatus }` |
| `wait` | `agent_ids*`, `timeout_ms` | `{ "status": { "<agent_id>": AgentStatus }, "timed_out": bool, "wakeup_reason": "any_completed\|all_completed\|timeout\|no_targets" }` |
| `wait_agents` | `agent_ids`, `mode`(`any`/`all`), `timeout_ms` | `{ "statuses": [{ "agent_id": string, "status": AgentStatus }], "completed_agent_ids": string[], "timed_out": bool, "wakeup_reason": "any_completed\|all_completed\|timeout\|no_targets" }` |
| `list_agents` | `agent_id`（创建者过滤）、`statuses[]`, `include_closed` | `{ "agents": [{ "agent_id": string, "creator_agent_id"?, "name"?, "goal": string, "acceptance_criteria": string[], "test_commands": string[], "allow_nested_agents": bool, "created_at_ms": i64, "updated_at_ms": i64, "status": AgentStatus, "closed": bool }] }` |
| `rename_agent` | `agent_id*`, `name*` | `{ "agent_id": string, "name": string }` |
| `close_agent` | `agent_id*` | `{ "status": AgentStatus }` |
| `close_agents` | `agent_ids*`, `ignore_missing` | `{ "results": [{ "agent_id": string, "status": AgentStatus, "closed": bool, "error"? }] }` |

---

## 3. Claude 风格工具（统一入口层）

> 入口定义：`codex-rs/core/src/tools/spec.rs`  
> 参数映射实现：`codex-rs/core/src/tools/handlers/claude_tool_adapter.rs`  
> 专用实现：`claude_write.rs` / `claude_edit.rs` / `claude_glob.rs` / `claude_notebook_edit.rs`

| 工具名 | 参数（`*` 必填） | 真实执行目标 | 返回值契约 |
| --- | --- | --- | --- |
| `Task` | `description*`, `prompt*`, `subagent_type*`, `name`, `model`, `max_turns`, `mode`, `resume`, `run_in_background`, `team_name` | 映射到 `spawn_agent` | 同 `spawn_agent`：`{ "agent_id": string }` |
| `TaskOutput` | `agent_id*`, `block`, `timeout` | 映射到 `wait` | 同 `wait` 返回结构 |
| `TaskStop` | `agent_id*` | 映射到 `close_agent` | 同 `close_agent`：`{ "status": AgentStatus }` |
| `Skill` | `skill*`, `args` | 映射到 `spawn_agent`（`skill://` 输入项） | 同 `spawn_agent` |
| `ToolSearch` | `query*`, `max_results` | 映射到 `search_tool_bm25` | 同 `search_tool_bm25`（见下节） |
| `AskUserQuestion` | `questions*[]`（每题含 `header*`, `question*`, `options*[]`, `id`, `multiSelect`） | 映射到 `request_user_input` | `RequestUserInputResponse`：`{ "answers": { "<question_id>": { "answers": string[] } } }` |
| `Bash` | `command*`, `timeout`, `description`, `run_in_background` | 映射到 `exec_command` | 文本输出（非 JSON，含 `Chunk ID`/`Wall time`/`Output` 等段） |
| `Read` | `file_path*`, `offset`, `limit`, `mode`, `indentation`, `pages` | 映射到 `read_file` | 文本输出（文件内容、行号格式或对应文件类型渲染） |
| `Grep` | `pattern*`, `path`, `glob`, `output_mode`, `head_limit`, `offset` | 映射到 `grep_files` | 文本输出（匹配列表或 `No matches found.`） |
| `TodoWrite` | `todos*[]`（每项含 `status*`, `content`, `activeForm`） | 映射到 `update_plan` | 固定文本：`Plan updated` |
| `EnterPlanMode` | 无 | 会话模式切换 | `{ "changed": bool, "previous_mode": string, "current_mode": string }` |
| `ExitPlanMode` | `allowedPrompts`（透传） | 会话模式切换 | `{ "changed": bool, "previous_mode": string, "current_mode": string }` |
| `Write` | `file_path*`, `content*` | `ClaudeWriteHandler` | `{ "ok": true, "file_path": string, "bytes_written": usize }` |
| `Edit` | `file_path*`, `old_string*`, `new_string*`, `replace_all` | `ClaudeEditHandler` | `{ "ok": true, "file_path": string, "replacements": usize, "replace_all": bool }` |
| `Glob` | `pattern*`, `path` | `ClaudeGlobHandler` | `{ "ok": true, "base_path": string, "pattern": string, "matches": string[], "count": usize }` |
| `NotebookEdit` | `notebook_path*`, `new_source*`, `edit_mode`, `cell_id`, `cell_number`, `cell_type` | `ClaudeNotebookEditHandler` | `{ "ok": true, "notebook_path": string, "edit_mode": "replace\|insert\|delete", "cell_index": usize\|null, "cell_count": usize }` |

---

## 4. 别名依赖的基础工具（Native）

> 这些工具是别名层的实际执行目标，修改返回值会直接影响上层别名契约。

| 工具名 | 参数（`*` 必填） | 返回值 |
| --- | --- | --- |
| `exec_command` | `cmd*`, `workdir`, `shell`, `login`, `tty`, `yield_time_ms`, `max_output_tokens`, `sandbox_permissions`, `justification`, `prefix_rule` | 文本输出（执行摘要 + 命令输出） |
| `write_stdin` | `session_id*`, `chars`, `yield_time_ms`, `max_output_tokens` | 文本输出（执行摘要 + 命令输出） |
| `search_tool_bm25` | `query*`, `limit` | `{ "query": string, "total_tools": usize, "active_selected_tools": string[], "tools": [{ "name": string, "server": string, "title"?, "description"?, "connector_id"?, "connector_name"?, "input_keys": string[], "score": number }] }` |
| `request_user_input` | `questions*[]` | `RequestUserInputResponse` |
| `update_plan` | `plan*[]`, `explanation` | 固定文本：`Plan updated` |
| `read_file` | `file_path*`, `offset`, `limit`, `mode`, `indentation`, `pages` | 文本输出 |
| `grep_files` | `pattern*`, `path`, `glob`, `output_mode`, `head_limit`, `offset`, 其他 grep 选项 | 文本输出 |

---

## 5. 网络相关工具边界（本轮未重构网络内核）

| 工具名 | 状态 | 参数与返回值说明 |
| --- | --- | --- |
| `web_search` | Core 内置工具（`ToolSpec::WebSearch`） | 由 `web_search_mode` 控制可用与 `external_web_access`，不走本地函数参数 schema；结果以 provider 的 web search item 形式进入响应流。 |
| `WebFetch` | 非 Core 静态注册；通常由外部 MCP 提供 | 当外部 MCP 暴露该工具时，Codex 通过 MCP 动态转换接入；参数与返回值由外部 MCP schema 决定，Core 仅做转发。 |

---

## 6. 返回值附录（关键类型）

### 6.1 `AgentStatus`

`pending_init` | `running` | `completed(string|null)` | `errored(string)` | `shutdown` | `not_found`

### 6.2 `WaitWakeupReason`

`any_completed` | `all_completed` | `timeout` | `no_targets`

---

## 7. 修改入口索引（建议）

- 工具名称/参数 schema：
  - `codex-rs/core/src/tools/spec.rs`
- Collab 工具行为与返回值：
  - `codex-rs/core/src/tools/handlers/collab.rs`
  - `codex-rs/core/src/tools/handlers/collab_batch.rs`
- Claude 别名映射：
  - `codex-rs/core/src/tools/handlers/claude_tool_adapter.rs`
- Claude 专用文件类工具：
  - `codex-rs/core/src/tools/handlers/claude_write.rs`
  - `codex-rs/core/src/tools/handlers/claude_edit.rs`
  - `codex-rs/core/src/tools/handlers/claude_glob.rs`
  - `codex-rs/core/src/tools/handlers/claude_notebook_edit.rs`

如需继续演进命名规范，建议先统一更新本文件，再同步改 `spec -> handler -> 测试` 三层，避免契约漂移。
z