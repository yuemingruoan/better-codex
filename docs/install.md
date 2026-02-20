## 安装与构建

### 系统要求

| 要求 | 说明 |
| --- | --- |
| 操作系统 | macOS 12+、Ubuntu 20.04+/Debian 10+，或 Windows 11 **通过 WSL2** |
| Git（可选，推荐） | 2.23+，用于内置 PR 辅助功能 |
| 内存 | 最低 4 GB（推荐 8 GB） |

### DotSlash

GitHub Release 中还包含一个名为 `codex` 的 [DotSlash](https://dotslash-cli.com/) 文件。使用 DotSlash 可以在源码仓库中轻量固定一个可执行版本，确保无论开发平台如何，所有贡献者都使用同一版本。

### 从源码构建

```bash
# 克隆仓库并进入 Cargo 工作区根目录。
git clone https://github.com/openai/codex.git
cd codex/codex-rs

# 如有需要，安装 Rust 工具链。
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
rustup component add rustfmt
rustup component add clippy
# 安装工作区 justfile 依赖的辅助工具：
cargo install just
# 可选：安装 nextest 以使用 `just test`
cargo install cargo-nextest

# 构建 Codex。
cargo build

# 用示例提示词启动 TUI。
cargo run --bin codex -- "explain this codebase to me"

# 修改代码后使用根目录 justfile 辅助命令（默认指向 codex-rs）：
just fmt
just fix -p <crate-you-touched>

# 运行相关测试（项目级最快），例如：
cargo test -p codex-tui2
# 若安装了 cargo-nextest，`just test` 会通过 nextest 运行测试：
just test
# 如需完整的 `--all-features` 矩阵，请使用：
cargo test --all-features
```

## 追踪 / 详细日志

Codex 使用 Rust 编写，可通过 `RUST_LOG` 环境变量配置日志行为。

TUI2 默认可使用 `RUST_LOG=codex_core=info,codex_rmcp_client=info`，日志默认写入 `~/.codex/log/codex-tui.log`。如需单次运行重定向日志目录，可使用 `-c log_dir=...`（例如 `-c log_dir=./.codex-log`）。

```bash
tail -F ~/.codex/log/codex-tui.log
```

相比之下，非交互模式（`codex exec`）默认 `RUST_LOG=error`，日志会直接输出到标准输出，无需单独查看文件。

更多 `RUST_LOG` 配置选项请参考 Rust 文档：
https://docs.rs/env_logger/latest/env_logger/#enabling-logging
