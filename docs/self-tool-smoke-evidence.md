# 自身工具直连验证记录（禁用 Claude Code MCP）

- 验证日期：2026-02-13
- 验证分支：`develop-main`
- 验证前提：仅使用当前会话暴露的自身工具（`functions.*` + `web.*`），未调用 `mcp_router/*`。

## 验证矩阵（按 Claude 风格工具名对照）

| 目标工具名 | 本轮实际调用的自身工具 | 用例 | 结果 | 关键输出/备注 |
|---|---|---|---|---|
| AskUserQuestion | `request_user_input` | Default 模式直接调用 | 预期受限 | 返回 `request_user_input is unavailable in Default mode`（符合当前模式约束） |
| Bash | `exec_command` + `write_stdin` | `echo SELF_EXEC_OK`；启动 stdin 会话并写入 `PING_STDIN` | 通过 | 输出 `SELF_EXEC_OK`；`STDIN:PING_STDIN` |
| Edit | `apply_patch` | 修改 `demo.txt` 的 `line-b -> line-b-patched` | 通过 | `Success. Updated the following files` |
| EnterPlanMode | （当前会话未暴露独立工具） | 通过 `update_plan` 仅更新计划内容 | 受接口暴露限制 | 当前 runtime 无独立 `EnterPlanMode` 入口 |
| ExitPlanMode | （当前会话未暴露独立工具） | 同上 | 受接口暴露限制 | 当前 runtime 无独立 `ExitPlanMode` 入口 |
| Glob | `exec_command`（`find`） | `find /tmp/... -type f` | 通过（等价） | 输出目标文件绝对路径 |
| Grep | `exec_command`（`rg`） | `rg -n "needle" /tmp/...` | 通过（等价） | 输出 `.../a.txt:1:needle` |
| NotebookEdit | （当前会话未暴露独立工具） | 无法直连调用 | 受接口暴露限制 | 当前 runtime 未提供 notebook 编辑入口 |
| Read | `batches_read_file` | 普通读取 + `indentation` 模式读取 | 通过 | 返回 `L1/L2` 行文本；缩进块读取正常 |
| Skill | （当前会话未暴露独立工具） | 无法直连调用 | 受接口暴露限制 | 当前 runtime 未提供 `Skill` 入口 |
| Task | `spawn_agent` | 创建 `evidence-agent` | 通过 | 返回 `agent_id`，并收到 `AGENT_READY` |
| TaskOutput | `wait_agents` / `wait` | 等待子 Agent 产出 | 通过 | 返回 `completed` 内容与状态 |
| TaskStop | `close_agent` / `close_agents` | 关闭单个和批量 Agent | 通过 | 返回 `shutdown/closed=true` |
| TodoWrite | `update_plan` | 提交两步计划状态 | 通过 | 返回 `Plan updated` |
| ToolSearch | （当前会话未暴露独立工具） | 无法直连调用 | 受接口暴露限制 | 当前 runtime 未提供工具检索入口 |
| WebFetch | `web.open` | 打开 `https://openai.com` | 通过 | 成功返回页面内容（含标题 `OpenAI`） |
| WebSearch | `web.search_query` | 搜索 `OpenAI official website` | 通过 | 成功返回搜索结果集合 |
| Write | `apply_patch` / `exec_command` | 新建与覆盖文件 | 通过 | 文件写入后可被 `batches_read_file` 读取验证 |

## 子 Agent 并行能力抽检（自身工具）

- 并行创建两个 Agent（`parallel-a` / `parallel-b`）并用 `wait_agents(mode=any)` 先取先完成结果，随后 `mode=all` 汇总结果。
- 并行结果正常：先收到 `PAR_B`，最终汇总 `PAR_A` + `PAR_B`。
- 再次并行 `send_input` 后可继续得到 `AFTER_A` / `AFTER_B`，说明会话可复用。

## 结论

- 当前“自身工具”链路在本会话可见接口范围内整体可用。
- 需注意：部分 Claude 风格名称在当前 runtime 并非一一暴露为独立入口（如 `EnterPlanMode` / `NotebookEdit` / `Skill` / `ToolSearch`），需通过现有自身工具组合或在运行时额外暴露别名入口。
