# TUI 流式分块调参指南

本文说明如何在不改变底层策略形态的前提下，调整自适应流式分块常量。

## 范围

在调整 `codex-rs/tui2/src/streaming/chunking.rs` 中的队列压力阈值与滞回窗口，
以及 `codex-rs/tui2/src/app.rs` 中的基线提交节奏时，请使用本指南。

本指南关注“调参”，而不是重设计策略。

## 调参前准备

- 保持基线行为不变：
  - `Smooth` 模式每个基线 tick 仅 drain 一行。
  - `CatchUp` 模式会立即 drain 队列积压。
- 使用以下 trace 采集日志：
  - `codex_tui2::streaming::commit_tick`
- 在持续输出、突发输出以及混合输出的提示词上评估。

测量流程请参考 `docs/tui-stream-chunking-validation.md`。

## 调参目标

三项目标需要同时兼顾：

- 在突发输出下可见延迟低
- 模式切换抖动低（`Smooth <-> CatchUp` chatter）
- 在混合负载下进入/退出 catch-up 稳定

## 常量与控制含义

### 基线提交节奏

- `COMMIT_ANIMATION_TICK`（`tui2/src/app.rs`）
  - 数值更小：smooth 模式更新频率更高，稳态延迟更低。
  - 数值更大：平滑度更高，但可能提升感知延迟。
  - 一般在 chunking 阈值/保持区间稳定后再调整此项。

### 进入/退出阈值

- `ENTER_QUEUE_DEPTH_LINES`、`ENTER_OLDEST_AGE`
  - 数值更小：更早进入 catch-up（延迟更低，但切换风险更高）。
  - 数值更大：更晚进入（容忍更高延迟，但切换更少）。
- `EXIT_QUEUE_DEPTH_LINES`、`EXIT_OLDEST_AGE`
  - 数值更小：保持 catch-up 更久。
  - 数值更大：允许更早退出，可能增加再进入抖动。

### 滞回保持

- `EXIT_HOLD`
  - 延长可降低压力噪声导致的来回切换。
  - 过长会在压力消退后仍保持 catch-up。
- `REENTER_CATCH_UP_HOLD`
  - 延长可抑制退出后的快速再进入。
  - 过长会延迟近期开突发时的必要 catch-up。
  - 严重积压会按设计绕过该保持。

### 严重积压阈值

- `SEVERE_QUEUE_DEPTH_LINES`、`SEVERE_OLDEST_AGE`
  - 数值更小：更早绕过再进入保持。
  - 数值更大：只在极端压力下绕过保持。

## 推荐调参顺序

按以下顺序调参，便于因果清晰：

1. 进入/退出阈值（`ENTER_*`、`EXIT_*`）
2. 保持窗口（`EXIT_HOLD`、`REENTER_CATCH_UP_HOLD`）
3. 严重积压阈值（`SEVERE_*`）
4. 基线节奏（`COMMIT_ANIMATION_TICK`）

一次只改一个逻辑组，完成度量后再进入下一组。

## 症状驱动的调整建议

- catch-up 启动前延迟过大：
  - 下调 `ENTER_QUEUE_DEPTH_LINES` 和/或 `ENTER_OLDEST_AGE`
- 频繁出现 `Smooth -> CatchUp -> Smooth` 抖动：
  - 上调 `EXIT_HOLD`
  - 上调 `REENTER_CATCH_UP_HOLD`
  - 收紧退出阈值（下调 `EXIT_*`）
- 短突发时 catch-up 触发过多：
  - 上调 `ENTER_QUEUE_DEPTH_LINES` 和/或 `ENTER_OLDEST_AGE`
  - 上调 `REENTER_CATCH_UP_HOLD`
- catch-up 启动过晚：
  - 下调 `ENTER_QUEUE_DEPTH_LINES` 和/或 `ENTER_OLDEST_AGE`
  - 下调严重积压阈值（`SEVERE_*`），更早绕过再进入保持

## 每轮调参后的验证清单

- `cargo test -p codex-tui2` 通过。
- trace 窗口显示队列年龄受控。
- 模式切换未集中在短间隔重复周期。
- 进入 `CatchUp` 后能快速清空积压。
