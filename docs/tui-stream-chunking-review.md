# TUI2 流式分块（Stream Chunking）

本文说明 TUI2 中流式分块的工作方式及其设计原因。

## 问题

流式输出可能比“一帧一行”的动画更快。如果提交速度固定而到达速度突然升高，队列会堆积，界面显示会落后于实际接收。

## 设计目标

- 在正常负载下保持既有基线行为。
- 当积压增加时减少显示延迟。
- 保持输出顺序稳定。
- 避免突兀的单帧刷空导致的跳动感。
- 策略与传输无关，仅依赖队列状态。

## 非目标

- 策略不负责调度动画 tick。
- 策略不依赖上游来源身份。
- 策略不会重排序队列输出。

## 逻辑所在位置

- `codex-rs/tui2/src/streaming/chunking.rs`
  - 自适应策略、模式切换与 drain 计划选择。
- `codex-rs/tui2/src/streaming/commit_tick.rs`
  - 每次提交 tick 的编排：快照、决策、drain、追踪。
- `codex-rs/tui2/src/streaming/controller.rs`
  - commit-tick 编排使用的队列/drain 基元。
- `codex-rs/tui2/src/chatwidget.rs`
  - 集成点：触发 commit-tick 编排并处理 UI 生命周期事件。

## 运行流程

每个 commit tick 执行：

1. 构建跨 controller 的队列快照。
   - `queued_lines`：总排队行数。
   - `oldest_age`：各 controller 中最老队列行的最大年龄。
2. 询问自适应策略决策。
   - 输出：当前模式与 drain 计划。
3. 将 drain 计划应用到每个 controller。
4. 输出被 drain 的 `HistoryCell` 供调用方插入。
5. 输出追踪日志以便观测。

在 `CatchUpOnly` 范围内，策略状态仍会推进，但除非当前模式是 `CatchUp`，否则跳过 draining。

## 模式与切换

两种模式：

- `Smooth`
  - 基线行为：每个基线提交 tick drain 一行。
  - 基线 tick 间隔当前来自 `tui/src/app.rs:COMMIT_ANIMATION_TICK`（约 8.3ms，约 120fps）。
- `CatchUp`
  - 每个 tick 通过 `Batch(queued_lines)` drain 当前积压队列。

进入与退出使用滞回：

- 当队列深度或队列年龄超过进入阈值时进入 `CatchUp`。
- 退出需要深度与年龄同时低于退出阈值，并持续一个保持窗口（`EXIT_HOLD`）。

这可避免负载在阈值附近振荡导致频繁切换。

## 当前实验性调参值

以下是 `streaming/chunking.rs` 及 `tui/src/app.rs` 中的当前数值，属实验性，可能会随追踪数据调整。

- 基线 commit tick：`~8.3ms`（`app.rs` 中的 `COMMIT_ANIMATION_TICK`）
- 进入 catch-up：
  - `queued_lines >= 8` 或 `oldest_age >= 120ms`
- 退出 catch-up 条件：
  - `queued_lines <= 2` 且 `oldest_age <= 40ms`
- 退出保持（`CatchUp -> Smooth`）：`250ms`
- 退出后再进入保持：`250ms`
- 严重积压阈值：
  - `queued_lines >= 64` 或 `oldest_age >= 300ms`

## Drain 规划

在 `Smooth` 模式下，计划始终为 `Single`。

在 `CatchUp` 模式下，计划为 `Batch(queued_lines)`，即 drain 当前积压队列以快速收敛。

## 设计原因

该设计既保持了正常动画语义，又让积压行为更具自适应性：

- 正常负载下，行为熟悉且稳定。
- 压力升高时，队列年龄快速下降且不牺牲顺序。
- 滞回避免频繁模式抖动。

## 不变量

- 队列顺序保持不变。
- 队列清空时，策略重置回 `Smooth`。
- `CatchUp` 只有在持续低压后才退出。
- 在 `CatchUp` 中 drain 会立即生效。

## 可观测性

commit-tick 编排会输出 trace 事件：

- `stream chunking commit tick`
  - `mode`、`queued_lines`、`oldest_queued_age_ms`、`drain_plan`、`has_controller`、`all_idle`
- `stream chunking mode transition`
  - `prior_mode`、`new_mode`、`queued_lines`、`oldest_queued_age_ms`、`entered_catch_up`

这些事件旨在解释显示延迟：通过展示队列压力、选定的 drain 行为和模式切换随时间的变化。
