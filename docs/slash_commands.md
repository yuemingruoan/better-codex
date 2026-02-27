# Slash 指令

关于 Codex CLI 的 slash 指令概览，请参考官方文档：
https://developers.openai.com/codex/cli/slash-commands

补充：`/lang` 用于切换界面语言（回车后弹出选择列表）。

## 本仓库新增命令

- `/agent`：统一 Agent 控制中心（唯一入口），聚合以下能力：
  - 协作模式：`Plan` / `Proxy` / `Close`。
  - sub-agent 预设：`edit/read/grep/run/websearch` 的模型与推理覆盖。
  - 请求规范：`Parallel Priority` 开关。
  - SDD 工作流路由：标准与并行模式。
- 原独立入口 `/collab`、`/preset`、`/spec`、`/sdd-develop`、`/sdd-develop-parallels` 已移除。
- 开启后，Codex 会在每次请求时动态注入对应内置提示词（按当前语言选择中/英文）；关闭后后续请求不再携带。
- 以上配置均不会创建 `.codex/spec/AGENTS.md` 等外部文件。
