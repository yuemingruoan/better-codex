# 配置

基础配置说明请参考：
https://developers.openai.com/codex/config-basic

高级配置说明请参考：
https://developers.openai.com/codex/config-advanced

完整配置参考请参考：
https://developers.openai.com/codex/config-reference

## 连接 MCP 服务器

Codex 可以连接配置在 `~/.codex/config.toml` 中的 MCP 服务器。最新 MCP 配置项请参考配置参考文档：

- https://developers.openai.com/codex/config-reference

## Apps（连接器）

在输入框中使用 `$` 可插入 ChatGPT 连接器；弹出列表会显示可访问的应用。`/apps`
命令会列出可用与已安装的应用。已连接的应用会排在前面并标记为已连接，其他则标记为可安装。

## 通知

Codex 可以在代理完成一个回合后运行通知钩子。最新通知设置请参考配置参考文档：

- https://developers.openai.com/codex/config-reference

## 界面语言

在 `~/.codex/config.toml` 中可配置界面与提示语言：

```toml
# 可选值：en / zh-cn
language = "en"
```

当 `language` 缺失或无法识别时，默认使用英文。

## 内置规范（Spec）

可在 `~/.codex/config.toml` 中配置内置规范开关：

```toml
[spec]
parallel_priority = true
sdd_planning = true
```

- `parallel_priority = true`：在请求构建阶段动态注入内置 `Parallel Priority` 提示词。
- `parallel_priority = false`（默认）：不注入该提示词。
- `sdd_planning = true`：在请求构建阶段动态注入内置 `SDD Planning` 提示词（用于 SDD 规划流程）。
- `sdd_planning = false`（默认）：不注入该提示词。
- `/sdd-develop` 与 `/sdd-develop-parallels` 在流程内会自动启用并注入 `SDD Planning` 提示词，流程收尾后恢复原设置。
- 提示词文本由程序内置并按当前 `language` 选择中英文，不依赖 `.codex/spec/AGENTS.md` 外部文件。

## sub-agent 预设（subagent_presets）

可在 `~/.codex/config.toml` 中覆盖内置 sub-agent 预设（`edit` / `read` / `grep` / `run` / `websearch`）的模型与推理强度：

```toml
[subagent_presets.edit]
model = "gpt-5.1-codex"
reasoning_effort = "medium"

[subagent_presets.read]
model = "gpt-5.1"
reasoning_effort = "low"
```

- `model`：可选，覆盖该预设默认模型。
- `reasoning_effort`：可选，覆盖该预设默认推理强度。
- 不配置时使用内置默认值。

## JSON Schema

`config.toml` 对应的 JSON Schema 生成在 `codex-rs/core/config.schema.json`。

## 提示（Notices）

Codex 会在 `[notice]` 表中保存部分 UI 提示的“不要再提示”标记。

通过 Ctrl+C/Ctrl+D 退出时，会使用约 1 秒的双击提示（“再次按下 ctrl + c 退出”）。
