# Codex CLI 中文指南

## 项目简介
Codex CLI 是一套跨平台的开发辅助工具链，内含命令行界面、TUI UI、文件搜索、MCP 客户端等多个子组件。本仓库 `codex-rs` 提供 Rust 实现版本，适用于自动化执行、交互式开发以及与 AI 协作的场景。

## 目录约定
- `.codex/AGENTS.md`：AI 协作者在当前项目中的行为记录与说明
- `.codex/checkpoint.md`：Codex自动维护的开发日志，用于记录·每一步操作的结果、待办事项与风险提示。
- `.codex/PROMPT.md`:执行init时额外给予AI的提示词，包含想要实现的需求，注意事项，甚至项目结构

## 环境准备
1. 安装 Git ≥ 2.23（建议）。
2. 系统要求：macOS 12+/Linux（Ubuntu 20.04+/Debian 10+）/Windows 11（需 WSL2）。
3. 内存至少 4 GB（推荐 8 GB）。
4. 若需参与开发，可酌情安装 Rust 工具链与 `just`、`rg` 等辅助工具；仅使用发布版二进制时无需这些依赖。

## 安装 / 更新指南

### 1. 获取最新版本
- 打开本仓库的[发布页面](https://github.com/yuemingruoan/better-codex/releases)，Release 标题通常为 `v1.3.0`、`v1.2.1` 这类语义化版本号。
- 在最新 Release 的 **Assets** 区域中，根据平台下载对应压缩包或可执行文件（Windows、macOS、Linux 皆会提供，各文件名已在说明中标注）。
- 解压得到的二进制文件后：
  - Windows 重命名为 `codex.exe`；
  - macOS / Linux 重命名为 `codex`。
- 后续按平台指南替换旧版本即可完成升级。

### 2. Windows（搭配 npm 全局安装）
```powershell
npm install -g @openai/codex-cli
```
- npm 全局目录通常位于 `%APPDATA%\npm`。
- 将下载得到的 `codex.exe` 放入 `%APPDATA%\npm` 目录，并替换与 `codex.cmd` 同级的旧版本（务必保留 `.cmd` 启动脚本）。

### 3. macOS / Linux
1. 先安装 CLI（可选择 npm、Homebrew 或其他渠道）：
   ```bash
   npm install -g @openai/codex-cli
   # 或
   brew install openai/codex/codex-cli
   ```
2. 查找当前 `codex` 可执行文件位置：
   ```bash
   which codex
   ```
3. 若输出是符号链接（Homebrew 与部分 npm 发行版常见），请使用 `readlink` 获取真实路径：
   ```bash
   realpath "$(which codex)"            # GNU 系统
   readlink -f "$(which codex)"         # Linux
   readlink "$(which codex)"            # macOS 需逐级解析
   ```
4. 使用从发布页下载的最新 `codex` 可执行文件替换目标路径中的旧版本，并保持原有权限位即可完成升级。
5. 如果你偏好通过 Homebrew 图形化安装，可执行：
   ```bash
   brew install --cask codex
   ```
   安装完成后终端直接运行 `codex` 即可启动。若升级过程中遇到 Homebrew 缓存或权限问题，可参考 [FAQ 关于 `brew upgrade codex` 的章节](./docs/faq.md#brew-upgrade-codex-isnt-upgrading-me)。

<details>
<summary>也可以直接前往 <a href="https://github.com/yuemingruoan/better-codex/releases/latest">最新 GitHub Release</a> 下载与你平台匹配的二进制文件</summary>

每个 Release 都会附带多份可执行文件，常用条目如下：

- macOS
  - Apple Silicon/arm64: `codex-aarch64-apple-darwin.tar.gz`
  - x86_64（较老的 Intel Mac）: `codex-x86_64-apple-darwin.tar.gz`
- Linux
  - x86_64: `codex-x86_64-unknown-linux-musl.tar.gz`
  - arm64: `codex-aarch64-unknown-linux-musl.tar.gz`

归档包内只包含一个可执行文件，名称带有平台后缀（例如 `codex-x86_64-unknown-linux-musl`），解压后通常需要重命名为 `codex` 以便加入 PATH。

</details>

### 通过 ChatGPT 订阅计划使用 Codex

<p align="center">
  <img src="./.github/codex-cli-login.png" alt="Codex CLI 登录示意" width="80%" />
  </p>

直接运行 `codex`，在登录界面选择 **Sign in with ChatGPT**，即可将 Codex 纳入 ChatGPT Plus / Pro / Team / Edu / Enterprise 订阅额度。[了解你的 ChatGPT 套餐包含哪些 Codex 功能](https://help.openai.com/en/articles/11369540-codex-in-chatgpt)。

Codex 也支持 API Key 登录，但需要 [额外配置](./docs/authentication.md#usage-based-billing-alternative-use-an-openai-api-key)。如果你此前已经使用 API Key 进行按量计费，请根据[迁移指引](./docs/authentication.md#migrating-from-usage-based-billing-api-key)完成切换。若登录遇到困难，可在 [该 issue](https://github.com/openai/codex/issues/1243) 反馈。

### Model Context Protocol（MCP）

Codex 能够访问 MCP 服务器，配置示例见 [配置文档的 MCP 章节](./docs/config.md#mcp_servers)。

### 配置

Codex CLI 的偏好设置保存在 `~/.codex/config.toml`。完整配置项请查阅 [Configuration 文档](./docs/config.md)。

### Execpolicy

请参考 [Execpolicy 快速上手](./docs/execpolicy.md) 来设置 Codex 可执行命令的规则。

### 文档 & FAQ 索引

- [**快速上手**](./docs/getting-started.md)
  - [CLI 使用方式](./docs/getting-started.md#cli-usage)
  - [斜杠指令](./docs/slash_commands.md)
  - [以 Prompt 作为输入运行](./docs/getting-started.md#running-with-a-prompt-as-input)
  - [示例 Prompt](./docs/getting-started.md#example-prompts)
  - [自定义 Prompt](./docs/prompts.md)
  - [AGENTS.md 记忆机制](./docs/getting-started.md#memory-with-agentsmd)
- [**配置**](./docs/config.md)
  - [配置示例](./docs/example-config.md)
- [**沙箱与审批**](./docs/sandbox.md)
- [**Execpolicy 快速上手**](./docs/execpolicy.md)
- [**认证方式**](./docs/authentication.md)
  - [强制指定认证方式](./docs/authentication.md#forcing-a-specific-auth-method-advanced)
  - [无头设备登录](./docs/authentication.md#connecting-on-a-headless-machine)
- **自动化 Codex**
  - [GitHub Action](https://github.com/openai/codex-action)
  - [TypeScript SDK](./sdk/typescript/README.md)
  - [非交互模式（`codex exec`）](./docs/exec.md)
- [**进阶主题**](./docs/advanced.md)
  - [Tracing / 详细日志](./docs/advanced.md#tracing--verbose-logging)
  - [Model Context Protocol（MCP）](./docs/advanced.md#model-context-protocol-mcp)
- [**零数据保留（ZDR）**](./docs/zdr.md)
- [**贡献指南**](./docs/contributing.md)
- [**安装与构建**](./docs/install.md)
  - [系统要求](./docs/install.md#system-requirements)
  - [DotSlash](./docs/install.md#dotslash)
  - [源码构建](./docs/install.md#build-from-source)
- [**FAQ**](./docs/faq.md)
- [**开源基金**](./docs/open-source-fund.md)

---

## 许可证

本仓库遵循 [Apache-2.0 License](LICENSE)。
