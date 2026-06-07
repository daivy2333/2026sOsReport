# 周总结：StarryOS 异步串口（2026-W21）

> **项目**：StarryOS（`asyncuart-dev` 分支）
> **主题**：kernel 层独立异步串口栈完成、手动 QA 通过、性能基线建立

---

## 一、本周概览

本周完成异步串口栈从 spike 到性能基线的**全链路交付**：Q0–Q4 实现 kernel 层独立异步串口（不修改 axplat/axhal/axtask），Q5/Q5.1 性能优化落地，Q5.2 完成 12 项手动 QA 场景验证，建立 Console vs Async 的统一量纲性能基线。

> **量化**：Q0–Q5.2 全部完成、12 项 QA 场景 PASS、CPU 效率提升 14.5×、write 延迟降低 2.2–7.5×、6 份新分析文档产出。

---

## 二、背景与历程

### 2.1 起点：Console 同步阻塞

`axhal::console` 路径以 Polling TX 实现同步阻塞串口，TX 期间 CPU 100% 空转，是高性能异步通信的主要瓶颈。

### 2.2 早期方向试错（已归档）

> 渐进式集成（方向 A，M0–M2 通过、M3 因 IRQ 风暴失败）与完全剔除（方向 B，P0–P4 完成）两个方向已归档至 `archive.md`。关键教训：**集成前 MUST dump IIR/MCR/LSR 验证硬件状态**；**`stride=4` 越界是 MMIO 误诊的真因**（详见 `learned/spec.md` L78/L80/L122）。

最终收敛至**方向 C：kernel 层独立实现异步串口**，不修改任何外部 crate。

---

## 三、实施路径（Q0 → Q5.2）

| 阶段 | 目标 | 关键产出 |
|------|------|----------|
| **Q0** | Spike | `stride=1` MMIO 验证 + 寄存器访问 + ISR 注册 + `axmm::iomap` |
| **Q1** | 驱动架构 | Ring Buffer + ISR + RX/TX copier + `critical-section` |
| **Q2** | VFS 集成 | `DeviceOps` + `/dev/async_uart` + 与 Console 共存 |
| **Q3** | RX 接管 | `Tty<AsyncUartReader, ConsoleWriter>` → Shell stdin |
| **Q4** | 全异步 RX+TX | TX copier 接管，Shell 双向异步 |
| **Q5** | 性能优化 | IER 缓存 + ISR 合并 + batch I/O + waker skip + rx/tx 独立锁 |
| **Q5.1** | 性能优化续 | NAPI 中断合并（连续成功 ≥16 次切轮询）+ 批量 API + TX interleave 修复 |
| **Q5.2** | 测试补全 | 12 项手动 QA + 性能基线 + 非阻塞模式 |

---

## 四、当前架构

### 4.1 数据流

```
┌──────────────────────────────────────────────────────────────┐
│ 用户态                                                        │
│   Shell  ──write()──►  Tty<AsyncUartReader, AsyncUartWriter>  │
└──────────────────────────────────────────────────────────────┘
                          │                ▲
                          ▼                │
┌──────────────────────────────────────────────────────────────┐
│ kernel 层（drivers/）                                          │
│                                                                │
│  AsyncUartWriter  ──push──►  RingBufTx  ◄──pop──  TX Copier   │
│                                                  │            │
│                                                  ▼            │
│                                          enable_tx_intr       │
│                                                                │
│  AsyncUartReader  ◄──pop───  RingBufRx  ──push──  RX Copier   │
│        ▲                                                    ▲  │
│        │                                                    │  │
│        │                                          enable_rx_intr
│        │                                                    │  │
│  DRAIN_WAKER  ◄────────  ISR  ◄────  UART (NS16550)         │
└──────────────────────────────────────────────────────────────┘
                          │
                          ▼
                   内核日志（earlycon，axhal::console Polling TX，Panic 安全）
```

### 4.2 模块组成（kernel/src/drivers/，~500 行）

| 模块 | 行数 | 职责 |
|------|------|------|
| `uart_init.rs` | 164 | UART 硬件初始化 + IER 缓存（`AtomicU8`） |
| `isr.rs` | 24 | ISR 分发：`ISR` 寄存器读类型 → 禁中断 → `AtomicWaker::wake` |
| `ring_buffer.rs` | 58 | `RingBufRx` / `RingBufTx` + `PollSet` |
| `async_driver.rs` | 90 | `AsyncUartDriver` + RX/TX copier 任务 |
| `device_ops.rs` | 25 | `AsyncUartReader` / `AsyncUartWriter` + `TtyRead` / `TtyWrite` trait |
| `ntty_async.rs` | 31 | `AsyncTty = Tty<AsyncUartReader, AsyncUartWriter>` + `lazy_static` |

### 4.3 关键设计约束

- **ISR 极简**：仅做「读 ISR 寄存器判断类型 → 禁中断 → `AtomicWaker::wake` → 返回」，禁止数据搬运与锁操作。
- **MMIO 封装**：所有寄存器操作走 `uart_16550::Uart16550<MmioBackend>` 安全 API，禁止裸地址 `write_volatile`。
- **`stride=1`**：NS16550 寄存器仅 8 字节，stride=4 越界导致 LoadFault（**L122 铁律**）。
- **不修改 axplat/axhal/axtask**：kernel 层独立实现，保留上游可升级性。

---

## 五、手动 QA 验证（Q5.2 · 12 场景全部 PASS）

测试矩阵见 `docs/manual-qa-report.md`，覆盖功能正确性、并发稳定性与端到端性能。

| 编号 | 场景 | 验证点 | 结论 |
|------|------|--------|------|
| T1 | 基础 Shell | `ls` / `cd` / `pwd` | PASS |
| T2 | TX 小数据 | `echo` 4 B / 16 B | 即时回显无丢失 |
| T3 | TX 中数据 | `dd bs=64 count=10` | 640 B 完整 |
| T4 | TX 大数据 | `dd bs=4096 count=10` | 20 KB 完整 |
| T5 | RX 回显完整性 | `cat /etc/passwd` | 完整输出 |
| T6 | 并发 TX+RX | `dd … &` + 交互命令 | Shell 不卡 |
| T7 | Shell 输入 | `read x && echo …` | 输入正确接收 |
| T8 | 管道 TX | `ls -laR / \| cat > /dev/console` | 递归输出完整 |
| T9 | 混合压力 | 多 dd 并发 + 交互 | 无 crash |
| T10 | FIONBIO (e2e) | `O_NONBLOCK` open + `ioctl(FIONBIO)` | EAGAIN 双 PASS |
| T11 | 端到端延迟 | `./benchmark` | avg 150.7 µs，P99 252.9 µs |
| T12 | 端到端吞吐量 | `./benchmark` | 4096 B 真板预测效率 97.9% |

**核心结论**：T1–T9 验证功能与并发稳定性，T10–T12 验证非阻塞与端到端性能，**12 场景全部 PASS**。

---

## 六、性能对比（Console vs Async）

> **前提**：QEMU NS16550 不仿真串口线延迟（86.8 µs/byte @ 115200 bps），QEMU 上用户态吞吐量不可反映真板性能；本对比聚焦**内核态速度、CPU 效率、write() 延迟**等可可信维度。完整报告见 `docs/uart-performance-comparison.md`。

### 6.1 内核态 CPU 效率

| 指标 | Console | Async | 倍率 |
|------|---------|-------|------|
| **TX CPU cycles/byte** | 3,835 | **265** | **14.5×** ↑ |

### 6.2 用户态 write() 延迟

| 分位 | Console | Async | 倍率 |
|------|---------|-------|------|
| **P50** | 17.5 µs | **7.9 µs** | 2.2× ↑ |
| **P95** | 32.8 µs | **12.2 µs** | 2.7× ↑ |
| **P99** | 62.5 µs | **8.3 µs** | **7.5×** ↑ |

### 6.3 RX 端

| 指标 | Console | Async |
|------|---------|-------|
| Ring Buffer | 无 | `ringbuf::HeapRb` 128 KB |
| RX 延迟 P50 | 不可测¹ | **600 ns** |

¹ Console 无 Ring Buffer，`read_bytes()` 为非阻塞 `try_receive()`，无可比 RX 延迟数据。

### 6.4 内存与吞吐

| 维度 | Console | Async |
|------|---------|-------|
| 内存 | 0 KB | 128 KB（RX/TX ring buffer）|
| 吞吐量（真板） | ~11.5 KB/s | ~11.5 KB/s（受 115200 bps 上限制约）|

**关键发现**：
- **CPU 效率是核心差异化指标**：Async 全中断驱动 + copier 模型，CPU 周期数下降一个数量级。
- **吞吐量受硬件上限制约**：115200 bps = 11.52 KB/s，Async 与 Console 在真板上**吞吐量持平**，延迟与 CPU 占用是主要差异维度。
- **真板预测效率 97.9%**：4096 B 端到端测试，软件开销 < 2.3%。

---

## 七、问题与解决方案

| 类别 | 问题 | 根因 | 解决方案 |
|------|------|------|----------|
| **硬件** | `stride=4` LoadFault（L122） | NS16550 仅 8 字节寄存器空间 | `stride=1` 铁律 |
| **硬件** | UART MMIO 误诊为权限问题 | 实为 stride 越界 | ISR 上下文验证证伪 |
| **架构** | M3 IRQ 风暴（方向 A） | Console 仅 RX 中断与 AsyncUart 不兼容 | 方向 C 独立实现 |
| **架构** | Console 与 AsyncUart 共用 FIFO 竞争 | 共享 UART 硬件 IER 单写者 | Console 退至 earlycon，TTY 泛型替换 |
| **性能** | TX 写满 FIFO 后 retry 无效 | 未 dump 寄存器状态 | 集成前 MUST 验证 IIR/MCR/LSR |
| **测试** | benchmark 不测真实 UART | TX 写 `/dev/null` 绕过 UART | 改 `/dev/console + tcdrain()` |

---

## 八、下周计划

1. **Q6 真板验证**（跟踪）：VisionFive2 UART 时钟适配、真实 FIFO 深度验证、DMA 通道发现、高速波特率支持。
2. **Q7 用户态性能修复**：基于性能分析中的 yield storm、FIONBIO 传播、benchmark 修正、tcdrain 真异步化四项优化。
3. **P0 OpenSpec 体系**：将 `.claude/docs/*.md` 规范迁移至 `openspec/specs/`，建立需求驱动的工作流。
4. **文档归档**：早期 M0–M2 决策归档为历史教训，保留 L78/L80/L122 等关键踩坑档案。

---

## 九、风险与展望

- **硬件依赖**：Q6 完全受制于 VisionFive2 板卡到位时间，是吞吐量真板数据采集的硬性门控。
- **吞吐量上限**：115200 bps 决定 Async 与 Console 在真板吞吐量持平，CPU 效率是核心价值锚点。
- **earlycon 残留**：内核日志仍走 `axhal::console` Polling TX，Q6 真板上评估是否替换为更轻量实现。
- **设计纪律延续**：ISR 极简、MMIO 封装、stride=1 三条铁律为后续 UART 集成的不变前提。

---

*文件位置：`.claude/docs/weekly-2026-W21.md`*
*配套文档：`docs/manual-qa-report.md` / `docs/uart-performance-comparison.md` / `docs/async-uart-architecture.md` / `docs/benchmark-report-{async,console}.md`*
*生成时间：2026-06-07*
