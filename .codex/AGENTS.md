# Repository Guidelines

## 项目结构与模块组织
- `codex-rs/`：Rust 工作区与核心实现（重点 crate：`core`、`cli`、`tui2`、`exec`、`mcp-server`、`exec-server`）。
- `codex-cli/`：历史 TypeScript CLI 包装层（npm 包 `@openai/codex`）。
- `sdk/typescript/`：TypeScript SDK（npm 包 `@openai/codex-sdk`）。
- `shell-tool-mcp/`：Shell Tool MCP 服务（npm 包 `@openai/codex-shell-tool-mcp`）。
- `codex-rs/responses-api-proxy/npm/`：Responses API Proxy 的 npm 打包目录。
- `docs/`：用户与开发文档；`.github/workflows/`：CI 与发布流程定义。
- `.codex/`：本地 Agent 记忆目录（如 `AGENTS.md`、`checkpoint.md`、`PROMPT.md`）；该目录默认被 `.gitignore` 忽略。

## 构建、测试与开发命令
- 环境建议与 CI 对齐：`node >= 22`、`pnpm >= 10.28`、Rust 工具链见 `codex-rs/rust-toolchain.toml`（当前 `1.93.0`）。
- 安装依赖（仓库根目录）：`pnpm install --frozen-lockfile`。
- Rust 常用命令（在仓库根目录执行，`justfile` 会自动切到 `codex-rs/`）：
  - `just fmt`：格式化 Rust 代码。
  - `just fix -p <crate>`：按 crate 运行 clippy 自动修复。
  - `just test`：通过 `cargo nextest` 跑测试。
  - `cargo test -p <crate>`：仅验证受影响 crate。
  - `cargo test --all-features --locked`：与 CI 核心测试矩阵对齐。
- JS/TS 常用命令：
  - `pnpm run format` / `pnpm run format:fix`：仓库级 Prettier 检查/修复。
  - `pnpm --filter @openai/codex-sdk run lint && pnpm --filter @openai/codex-sdk run test`。
  - `pnpm --filter @openai/codex-shell-tool-mcp run test`。
  - 更新脚本改动后运行：`pnpm run build:update && node scripts/update/better-codex-update.js --help`。

## 代码风格与命名规范
- Rust：
  - 使用 `rustfmt`（`imports_granularity = "Item"`）和工作区 clippy 规则。
  - 工作区显式禁止 `unwrap_used`、`expect_used`，请优先返回错误并补充上下文。
  - crate 命名使用 kebab-case（示例：`codex-file-search`）。
- TypeScript/JavaScript：
  - 使用 Prettier；根配置为 `tabWidth = 2`、`printWidth = 80`，`sdk/typescript` 局部配置为 `printWidth = 100`。
  - `sdk/typescript` 使用 ESLint（含 `@typescript-eslint`），未使用参数建议以 `_` 前缀命名。
- 文件与测试命名：
  - TS 测试文件遵循 `tests/**/*.test.ts`。
  - 分支建议：`feat/<topic>`、`fix/<topic>`、`chore/<topic>`。
- 特殊约束：如修改 `codex-rs/tui2/src/bottom_pane/`，需同步更新该目录下 `AGENTS.md` 要求的文档说明。

## 分支策略与工作流
- `main`：稳定发布分支，仅接受 `develop-main -> main` 的 PR，用于正式版本发布。
- `develop-main`：日常集成与发布前验证分支，功能与修复应优先合入该分支。
- `feat/<topic>`、`fix/<topic>`、`chore/<topic>`：短期开发分支，通常向 `develop-main` 发起 PR。
- 云端测试工作流：`.github/workflows/develop-main-tests.yml` 会在 `develop-main` 的 push/PR 上自动运行（含 Rust 测试与更新脚本冒烟测试）。

## 测试指引
- 任何功能改动或缺陷修复都应附带测试或更新现有测试。
- Rust：优先跑受影响 crate（`cargo test -p <crate>`），再跑全量（`just test` 或 `cargo test --all-features --locked`）。
- TypeScript 包：
  - SDK：`pnpm --filter @openai/codex-sdk run test`，必要时追加 `coverage`。
  - shell-tool-mcp：`pnpm --filter @openai/codex-shell-tool-mcp run test`。
- 涉及发布更新脚本时，至少执行一次 `node scripts/update/better-codex-update.js --help` 进行冒烟验证。
- 本地测试建议按改动范围执行最小必要验证；若本地环境受限，可依赖 GitHub Actions 云端 workflow 结果，不强制每次在本地跑全量测试。

## 提交与合并请求指南
- Commit message 建议遵循 Conventional Commits（仓库历史常见：`feat:`、`fix:`、`test:`、`ci:`、`chore:`、`doc:`）。
- 每次提交保持“单一主题 + 可构建 + 可测试”。
- PR 描述至少包含：`What`（改了什么）、`Why`（为什么改）、`How`（怎么改）、关联 issue/讨论链接。
- 建议优先在本地跑完受影响范围检查；若受环境限制，可先发起评审并以云端 workflow 验证结果为准。
- 贡献策略：`docs/contributing.md` 明确“外部代码贡献通常需受邀”；未受邀 PR 可能直接关闭。
- 分支守卫：面向 `main` 的 PR 仅允许 `develop-main -> main`，其他来源分支会被自动关闭（见 `.github/workflows/main-pr-guard.yml`）。

## 发布流程（固定：develop-main -> main）
1. **同步版本号（必须同时修改）**
   - `codex-rs/Cargo.toml`（`[workspace.package].version`）
   - `codex-cli/package.json`
   - `sdk/typescript/package.json`
   - `shell-tool-mcp/package.json`
   - `codex-rs/responses-api-proxy/npm/package.json`
2. **覆盖发布说明**：写入 `docs/release/notes.md`（仅记录“上一个版本 -> 当前版本”，需中英文，聚焦范围/核心功能/配置文档/验证结果）。
3. **合并流程**：在 `develop-main` 完成本次发布提交，发起 `develop-main -> main` PR，按策略合并并记录冲突处理结论。
4. **手动触发发布工作流**：`.github/workflows/release.yml`
   - `tag`: `v<semver>`（如 `v1.7.4`）
   - `target`: `main`
   - `notes_file`: `docs/release/notes.md`
   - `draft`: `false`
   - `prerelease`: `false`
5. **发布后核对**
   - Release 标签与标题一致（如 `v1.7.4`）。
   - Release 正文来源于 `docs/release/notes.md`。
   - `main` 已包含版本号与 notes 更新。

## 安全与配置提示
- 严禁提交密钥、令牌和 `.env*` 敏感信息。
- 若发现漏洞或负责任 AI 风险，请联系 `security@openai.com`。
- `.codex/` 主要用于本地 Agent 上下文，若要共享长期规则，请同步到可追踪文档（如 `README.md` 或 `docs/`）。
