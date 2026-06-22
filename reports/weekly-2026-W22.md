# 周总结：StarryOS 异步串口

> **项目**：StarryOS（`asyncuart-dev` 分支）
> **范围**：kernel 层独立异步串口栈 + OpenSpec 文档体系

---

## 一、本周概览

本周沿两条主线推进：（一）**Q7 用户态性能缺陷修复**——四项子任务全部落地，CPU 效率基线重测；（二）**OpenSpec 文档体系迁移**——P0 里程碑完成，4 个 spec 域与 rules 章节完成结构化归档。期间同步沉淀远期规划（Q8/Q9）与 Embassy 选型纪律（OE1~OE5）。

> **量化**：7 次 git 提交、5 份规范文档迁移、4 项 Q7 优化生效、CPU 效率提升 14.3×。

---

## 二、做了什么

### 2.1 Q7：用户态性能修复（4 项子任务全部完成）

| 编号 | 修复项 | 关键变更 | 量化效果 |
|------|--------|----------|----------|
| **O42** | 三重 yield storm | `ProcessMode::Manual` → `External`，引入独立 tty-reader 任务 | 无数据时 CPU 占用归零 |
| **O43** | FIONBIO 传播 | `Tty` 添加 `AtomicBool` 字段，open/fcntl/ioctl 三入口统一传播 | nonblocking 标志贯穿 TTY/ldisc |
| **O44** | benchmark 失真 | TX 改测 `/dev/console + tcdrain()`，RX 改 raw mode | 测量真实 UART 吞吐/延迟 |
| **O45** | tcdrain 真异步化 | 新增 `DRAIN_WAKER` + PollSet 三段式等待 | 64B 切换 9→6 次，延迟 ~300µs → ~200µs |

### 2.2 P0：OpenSpec 文档体系建立

- **5 份 Markdown 源文件**（`.claude/docs/{architecture,learned,references,optimization,rules}.md`）**整体迁移**至 `openspec/specs/`，源文件以 `.bak` 留底。
- **rules 域**二次整合至 `CLAUDE.md` 规则章节（7 大节，原 17 Requirements），原文归档至墓碑目录 `openspec/changes/archive/rules-domain-2026-06-03/`。
- **CodeGraph 代码索引**建立（119 文件 / 2,174 节点 / 5,781 边 / 4.98 MB），探索模式由 `Read+Grep` 切换为 `codegraph_explore`。
- **形式化校验**：`openspec validate --specs` 全部通过。

### 2.3 远期规划与经验沉淀

- **O46 / O47**：pipe/signalfd/pidfd 改造（AtomicWaker 静态分发）与 embassy-time 集成的可行性分析，登记至 `optimization/spec.md`。
- **OE1~OE5**：Embassy 反模式（Channel/Mutex/Watch/Semaphore/select!）归档至"已排除优化"区，明确禁用边界。
- **L81~L84**：Embassy 选型边界（项目仅用 `embassy_sync::AtomicWaker`）写入 `learned/spec.md` 新 Requirement。

---

## 三、怎么做的

1. **诊断-修复-验证三步法**。基于 2026-06-01 两份根因分析文档（`user-async-perf-analysis.md` 与 `nonblocking-mode-analysis.md`），逐项修复并以 e2e benchmark 验证。性能数据以 102,400 字节统一量纲采集，避免量纲失真。
2. **规范驱动的文档迁移**。所有 spec 变更经 `/opsx:propose` 生成 proposal，`openspec validate` 形式化校验，确保文档结构与 schema 一致。
3. **工具链加固**。CodeGraph 索引替代 `Read+Grep` 探索，token 消耗与查询时间显著降低；MCP 不可用时**不静默降级**，按规范流程上报修复。

---

## 四、达到什么程度

### 4.1 Milestone 状态

```
Q0  Q1  Q2  Q3  Q4  Q5  Q5.1  Q5.2  Q7  P0     Q6     Q8 / Q9
✅  ✅  ✅  ✅  ✅  ✅  ✅    ✅     ✅  ✅     ⏳     📋
──── kernel 层异步串口栈已完成 ────   OpenSpec   硬件   计划
```

### 4.2 性能基线（统一 102,400 字节数据量）

| 指标 | Console | Async | 倍率 |
|------|---------|-------|------|
| CPU 效率 | 3,835 cycles/byte | 268 cycles/byte | **14.3×** |
| 延迟 P50 | 17.5 µs | 6.5 µs | **2.7×** |
| 端到端预测（4096B 真板） | — | 97.7% 线速 | 软件开销 < 2.3% |

### 4.3 架构收敛态

```
Shell stdin :  UART → ISR → RX_WAKER → copier → ring buffer → AsyncUartReader → Tty → Shell
Shell stdout:  Shell → Tty → AsyncUartWriter → ring buffer → TX_WAKER → copier → UART
tcdrain     :  PollSet 等 copier → DRAIN_WAKER 等 UART → 返回
kernel 日志 :  ax_println! → Console polling TX（与 Async 共存）
```

---

## 五、问题与解决方案

| 问题 | 根因 | 解决方案 |
|------|------|----------|
| 三重 yield storm | 3 层 `block_on(poll_io)` + `Manual.wake_by_ref()` 循环 | 改 `External` 模式 + 独立 tty-reader 任务 |
| FIONBIO 失效 | `Tty::read_at` 与 `ldisc.read` 硬编码 `false` | Tty 增加 `AtomicBool`，三入口传播 |
| benchmark 失真 | TX 测 `/dev/null` 绕过 UART | 改测 `/dev/console + tcdrain()` |
| tcdrain 同步等待 | 现有实现为 PollSet spin | 引入专用 `DRAIN_WAKER`，TX 中断联动唤醒 |
| 文档膨胀 | 5 份 `.md` 分散于 `.claude/docs/` | 迁移至 `openspec/specs/` 统一管理 |

**经验**：跨层状态传播必须穷举所有入口（`open/fcntl/ioctl`），一处遗漏即功能不完整；详见 `learned` L140 FIONBIO 教训。

---

## 六、代码更改索引

### Q7：用户态性能修复

| 编号 | 变更说明 | 文件 |
|------|----------|------|
| O42 | yield storm 修复：Manual→External + 独立 tty-reader | [`ldisc.rs`](https://github.com/daivy2333/StarryOS/blob/asyncuart-dev/kernel/src/pseudofs/dev/tty/terminal/ldisc.rs#L65) · [`ntty_async.rs`](https://github.com/daivy2333/StarryOS/blob/asyncuart-dev/kernel/src/drivers/ntty_async.rs) |
| O43 | FIONBIO 三入口传播：Tty 添加 AtomicBool | [`tty/mod.rs`](https://github.com/daivy2333/StarryOS/blob/asyncuart-dev/kernel/src/pseudofs/dev/tty/mod.rs#L43) · [`ldisc.rs`](https://github.com/daivy2333/StarryOS/blob/asyncuart-dev/kernel/src/pseudofs/dev/tty/terminal/ldisc.rs#L220) |
| O44 | benchmark 修正：TX 改测 /dev/console + tcdrain | [`benchmark.c`](https://github.com/daivy2333/StarryOS/blob/asyncuart-dev/tests/benchmark.c#L36) |
| O45 | DRAIN_WAKER + 三段式 tcdrain | [`isr.rs`](https://github.com/daivy2333/StarryOS/blob/asyncuart-dev/kernel/src/drivers/isr.rs#L9) |

### P0：OpenSpec 文档体系

| 变更说明 | 路径 |
|----------|------|
| 5 份源文件迁移（.bak 留底） | [`.claude/docs/*.md.bak`](https://github.com/daivy2333/StarryOS/tree/asyncuart-dev/.claude/docs) |
| 4 个 spec 域 | [`openspec/specs/`](https://github.com/daivy2333/StarryOS/tree/asyncuart-dev/openspec/specs) |
| rules 整合至 CLAUDE.md | [`CLAUDE.md`](https://github.com/daivy2333/StarryOS/blob/asyncuart-dev/CLAUDE.md) |
| rules 墓碑归档 | [`archive/rules-domain-2026-06-03/`](https://github.com/daivy2333/StarryOS/tree/asyncuart-dev/openspec/changes/archive/rules-domain-2026-06-03) |

---

## 七、下周计划

1. **Q6 真板验证**（跟踪）：VisionFive2 UART 时钟适配、真实 FIFO 深度验证、DMA 通道发现、高速波特率支持——受硬件到位时间约束。
2. **Q8 启动准备**（O46.1~O46.4）：pipe 改造为 AtomicWaker 静态分发，预期唤醒延迟 ~200ns → ~50ns。
3. **Q9 评估触发**（O47）：Q6.3（DMA 失败路径）完成后评估 embassy-time 集成的 ROI，负 ROI 即归档至"已排除"。
4. **文档与经验沉淀**：评估 O46/O47 实施方案的可移植性，更新 Embassy 选型纪律。

---

## 八、风险与展望

- **硬件依赖**：Q6 完全受制于 VisionFive2 板卡到位时间，是 Q9 启动的硬性门控。
- **理论性能上限**：115200 bps ≈ 11.52 KB/s，Async 与 Console 在吞吐量上无法拉开差距，CPU 效率是核心差异化指标。
- **Embassy 选型纪律**：仅引入 `embassy_sync::AtomicWaker`，严格规避 OE1~OE5 反模式（Channel/Mutex/Watch/Semaphore/select!）侵蚀代码质量。

---

*文件位置：`.claude/docs/weekly-2026-W22.md`*
*生成时间：2026-06-07*
