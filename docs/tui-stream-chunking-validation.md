# TUI2 流式分块验证流程

本文记录用于验证自适应流式分块与防抖行为的流程。

## 范围

目标是在运行时 trace 中验证两项性质：

- 队列压力上升时显示延迟下降。
- 模式切换保持稳定，避免快速抖动。

## Trace 目标

分块观测由以下 trace 输出：

- `streaming::commit_tick`（TUI2 提交节奏追踪目标）

使用两条 trace 消息：

- `stream chunking commit tick`
- `stream chunking mode transition`

## 运行命令

启用 chunking trace 运行 Codex：

```bash
RUST_LOG='streaming::commit_tick=trace,codex_core=info,codex_rmcp_client=info' \
  just codex --enable=responses_websockets
```

## 日志采集流程

提示：单次测量可用 `-c log_dir=...` 指向新目录，避免与历史会话混淆。

1. 记录 `~/.codex/log/codex-tui.log` 当前大小作为起始偏移。
2. 运行产生持续流式输出的交互提示词。
3. 结束运行。
4. 仅解析起始偏移之后写入的日志字节。

这样可避免混入早期会话数据。

## 评估指标

对每个测量窗口统计：

- `commit_ticks`
- `mode_transitions`
- `smooth_ticks`
- `catchup_ticks`
- drain 计划分布（`Single`、`Batch(n)`）
- 队列深度（`max`、`p95`、`p99`）
- 最老队列年龄（`max`、`p95`、`p99`）
- 快速再进入计数：
  - 在 1 秒内出现 `Smooth -> CatchUp` 且紧随 `CatchUp -> Smooth` 的次数

## 结果解读

- 健康行为：
  - 积压被 drain 时队列年龄保持受控
  - 模式切换次数相对总 tick 数较低
  - 快速再进入事件少且集中在突发边界
- 回归表现：
  - 长时间窗口内频繁短间隔模式切换
  - smooth 模式下队列年龄持续增长
  - 长时间 catch-up 但积压未减少

## 实验历史

本节记录主要调参历程，便于后续在既有基础上继续演进。

- 基线
  - 50ms 提交 tick + smooth 模式单行 drain。
  - 保留了熟悉节奏，但持续积压时感觉迟滞。
- Pass 1：即时 catch-up，基线 tick 不变
  - 保持 smooth 语义，但在 catch-up 中每个 tick drain 全部积压。
  - 结果：队列延迟下降更快，但 smooth 频率较低导致观感仍有台阶。
- Pass 2：更快的基线 tick（25ms）
  - 提升 smooth 频率，减轻台阶感。
  - 结果：更好，但仍未与绘制节奏对齐。
- Pass 3：帧对齐基线 tick（约 16.7ms）
  - 将基线提交节奏设为约 60fps。
  - 结果：更平滑，同时保留滞回与快速收敛。
- Pass 4：更高帧对齐基线 tick（约 8.3ms）
  - 将基线提交节奏设为约 120fps。
  - 结果：进一步减少 smooth 模式台阶，同时保持相同的自适应策略形状。

当前状态包含：

- `CatchUp` 中即时 drain
- 模式进入/退出的滞回稳定性
- 帧对齐的 smooth 模式提交节奏（约 8.3ms）

## 备注

- 验证与上游来源无关，不依赖具体 provider 名称。
- 该流程刻意保留既有 smooth 基线行为，聚焦突发/积压处理表现。
