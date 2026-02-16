# Checkpoint

## 2026-02-16 计划阶段
- 任务：Windows 下 `tui2` 配置启用无效问题诊断与修复
- 任务单：`.codex/task.md`
- 目标分支：`sdd/windows-tui2-bug`
- 当前状态：规划已落盘，待执行阶段按 `T0` 门禁检查后推进
- 阻塞项：`collab` experimental 门禁状态待执行时确认

## 2026-02-16 执行阶段
- 门禁检查：`~/.codex/config.toml` 中 `[features].collab = true`，允许进入 parallels 执行
- Sub-A 交付：`codex-rs/cli/src/main.rs` 完成 TDD（先失败后通过），新增 `select_tui_frontend` 回归测试
- Sub-B 交付：`codex-rs/cli/Cargo.toml` 依赖修复，移除 `cfg(not(windows))` 对 `codex-tui2` 与 `codex-utils-absolute-path` 的限制
- 里程碑提交：`b24b8b59` `fix(cli): enable tui2 frontend selection on windows`

## 2026-02-16 集成阶段
- 主集成验证：`cd codex-rs && just fmt`（通过）
- 主集成验证：`cd codex-rs && cargo test -p codex-cli`（通过）
- 残留风险：当前 macOS 环境交叉 `--target x86_64-pc-windows-msvc` 仍报 target/core 解析异常，`T5` 待 Windows 实机复验
