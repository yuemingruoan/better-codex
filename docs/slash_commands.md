# Slash 指令

关于 Codex CLI 的 slash 指令概览，请参考官方文档：
https://developers.openai.com/codex/cli/slash-commands

补充：`/lang` 用于切换界面语言（回车后弹出选择列表）。

## 本仓库新增命令

- `/preset`：打开 sub-agent 预设配置交互，按预设（`edit` / `read` / `grep` / `run` / `websearch`）设置模型与推理强度覆盖，并保存到配置。
- `/collab`（tui2）：打开协作模式选择弹窗，切换当前会话的协作预设。
- `/spec`：打开规范配置弹窗，仅支持 `Parallel Priority` 开关。
- `/spec` 交互提示：按 `Tab` 切换复选项，按 `Enter` 保存。
- `SDD Planning` 不在 `/spec` 菜单中切换；使用 `/sdd-develop` 或 `/sdd-develop-parallels` 时会自动注入。
- 开启后，Codex 会在每次请求时动态注入对应内置提示词（按当前语言选择中/英文）；关闭后后续请求不再携带。
- 以上配置均不会创建 `.codex/spec/AGENTS.md` 等外部文件。
