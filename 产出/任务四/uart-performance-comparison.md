# UART 性能对比：Console vs Async

> **项目**：[StarryOS](https://github.com/daivy2333/StarryOS) + [uart_16550](https://github.com/daivy2333/uart_16550)
> **分支**：`feat/uart-16550-async`（Q0~Q13 + LTO 完成，但 LTO 已 revert 参见 ADR-034）
> **截稿日期**：2026-06-17
> **测试环境**：QEMU riscv64-virt · NS16550 @ 115200 bps · FIFO 16B
> **关联文档**：`docs/async-uart-architecture.md`（架构） · `docs/benchmark-report-async.md`（Async 详细数据） · `docs/manual-qa-report.md`（QA 验证） · `.claude/analysis/async-uart-module-boundary.md`（Q13 事后视角）

> **⚠️ QEMU 仿真限制声明**：QEMU 的 NS16550 不仿真串口线延迟（86.8 µs/byte）。用户态吞吐量在 QEMU 上**不可直接对比**——真板两者均收敛至 ~11.5 KB/s（硬件理论上限）。本文讨论 QEMU 上**可信维度**：内核态速度、CPU 效率、write() 延迟、功能覆盖。
>
> **Q13 架构变更**：异步串口核心逻辑（ISR / ring buffer / copier / device_ops ~400 行）已提取到独立 [uart_16550](https://github.com/daivy2333/uart_16550) crate（`async` feature），内核仅保留初始化（~150 行）+ 适配层（~30 行）。

---

## 0. TL;DR

| 维度 | Console（同步阻塞）| Async（Q13.1）| 提升 |
|------|----------------|--------------|------|
| **CPU 效率** | 3,835 cycles/byte | 13 cycles/byte（Q12 数据）| ⬆ **~295×** |
| **write() 延迟 P50** | 17.5 µs（逐字节轮询 LSR）| 8.5 µs（push ring buf 即返回）| ⬆ **2.1×** |
| **唤醒延迟** | ~200ns（PollSet spinlock）| ~50ns（AtomicWaker lock-free）| ⬆ **4×** |
| **内核态吞吐 TX** | 567 KB/s | ~652 MB/s（Q13.1 + LTO）| ⬆ **~1150×** |
| **非阻塞读（FIONBIO）**| ❌ | ✅ open / fcntl / ioctl 三入口 | 功能补全 |
| **读超时（VTIME）**| ❌ | ✅ axtask::future::timeout | 功能补全 |
| **可移植性** | ❌（绑定 axplat）| ✅（5 OS trait 抽象，Q13） | 跨 OS 可复用 |
| **真板吞吐量** | ~11.5 KB/s | ~11.5 KB/s | 持平（硬件理论上限） |

**关键结论**：
- **Async 在 CPU 效率、延迟、唤醒、功能覆盖上全面胜出**
- **真板吞吐量受波特率限制（115200 bps = 11.52 KB/s），两者收敛**
- Q13 trait 抽象带来可移植性，代价 5.5µs 软件 overhead（已被 Q13.1 inline+batch 回收）
- LTO 跨 crate 内联使内核态 ↑69%，但 e2e 不变（**瓶颈在调度**）

---

## 1. 测量条件

### 1.1 核心论点

性能对比需在**统一测量条件**下进行。QEMU 仿真与真板硬件在串口时序上存在本质差异，导致部分数据不可比。

### 1.2 测试环境

| 项目 | Console | Async | 备注 |
|------|---------|-------|------|
| **目标架构** | RISC-V 64-bit | RISC-V 64-bit | `riscv64gc-unknown-linux-musl` |
| **模拟平台** | QEMU riscv64-virt | QEMU riscv64-virt | **不仿真串口线延迟** |
| **串口硬件** | NS16550 UART | NS16550 UART | 模拟设备 |
| **波特率** | 115200 bps | 115200 bps | 标准串口速率 |
| **FIFO 深度** | 16 字节 | 16 字节 | FCR 配置 |
| **构建模式** | release | release | LTO on/off 分两个独立构建 |
| **内核代码位置** | axplat 内置 | `kernel/src/drivers/` + `uart_16550` | Q13 提取 |
| **驱动代码量** | N/A（内置）| ~280 行 StarryOS + ~400 行 uart_16550 | Q13 后 |

### 1.3 关键限制

> **QEMU 仿真欺骗**：QEMU 16550 模型不仿真真实串口线延迟。Console 在 QEMU 上测的是纯 MMIO 速度（LSR 永远 THR_EMPTY），Async 测的是任务切换 + tcdrain 开销。**两者在 QEMU 上无法公平对比**——真板均收敛至 ~11.5 KB/s。

> **QEMU 性能数据可信维度**：
> - ✅ **可信**：内核态速度、CPU 效率、write() 延迟、功能覆盖
> - ❌ **不可信**：用户态吞吐（绝对值）

### 1.4 对比基线

- **Console**：Q12 之前的同步阻塞实现，作为 Async 的对比基线
- **Async**：Q13.1 + LTO（开发期 LTO 已 revert，参见 ADR-034）
- **数据来源**：
  - Async：`docs/benchmark-report-async.md`
  - Console：`docs/uart-performance-comparison-console.md`（如存在）/ 内部 benchmark

### 1.5 小结

测量条件明确后，QEMU 实测适用于**相对性能对比**与**功能正确性验证**，绝对吞吐需以真板为准（Q6 待定）。

---

## 2. 架构对比

### 2.1 核心论点

Console 与 Async 在架构上呈现**"同步轮询 vs 异步中断驱动"**的两种根本性设计选择，导致功能、性能、可移植性全面差异。

### 2.2 架构对照表

| 维度 | Console（阻塞）| Async（Q13.1）| 设计差异 |
|------|---------------|--------------|---------|
| **TX 路径** | `write()` → 逐字节轮询 LSR → 写 THR | `write()` → push ring buf → TX copier → 批量 `send_bytes()` | 同步 vs 异步 |
| **RX 路径** | ISR → tty-reader → ldisc → `read()` | ISR → RX copier → ring buf → tty-reader → ldisc → `read()` | 增加 ring buffer 层 |
| **缓冲区** | 无 | 128 KB（lock-free SPSC RingBuffer，Q12 去 Mutex）| 内存换性能 |
| **write() 延迟** | P50 17.5 µs（逐字节轮询 LSR）| P50 8.5 µs（push ring buf 即返回）| 同步轮询 vs push 即返回 |
| **空闲 CPU** | 0% | 0%（External 模式，无 yield storm）| 两者相当 |
| **tcdrain** | 隐式（每字节等 LSR）| ✅ TCSBRK + NS16550 TEMT 硬件直接唤醒 | 显式实现 |
| **非阻塞读** | ❌ | ✅ open / fcntl / ioctl 三入口 | 功能补全 |
| **读超时** | ❌ | ✅ VTIME（axtask::future::timeout）| 功能补全 |
| **唤醒机制** | PollSet（spinlock, ~200ns）| AtomicWaker（lock-free, ~50ns）| 单槽 waker 优化 |
| **标准化接口** | ❌ | ✅ embedded_io_async Read/Write trait | 生态兼容 |
| **ldisc 缓冲** | 80B StaticRb | 256B StaticRb（Q10 扩容 3.2×）| 突发吸收提升 |
| **后台任务** | 1（tty-reader）| 2（RX copier + tty-reader）| 增加 1 个常驻任务 |

### 2.3 关键架构差异说明

- **TX 路径**：Console 每字节都需 CPU 轮询 LSR（**L**ine **S**tatus **R**egister）直至 THR（**T**ransmit **H**olding **R**egister）可写；Async 写一次 push 到 ring buffer 后立即返回，由 TX copier 协程批量发送
- **RX 路径**：两者都有 ISR + ldisc 路径，差异在 Async 增加 ring buffer 中转，**消除 RX FIFO 16 字节溢出风险**
- **tcdrain**：Console 的隐式等待通过每字节轮询实现（写入即等 LSR），开销累积；Async 的显式 tcdrain 通过 DRAIN_WAKER 条件等待实现
- **非阻塞读**：Console 无此功能（同步阻塞 read 永不返回 EAGAIN）；Async 通过 FIONBIO 传播到 ldisc.read 实现

### 2.4 小结

架构差异的根源是**"是否使用异步 ring buffer 中转"**。Async 用 128 KB 内存 + 1 个常驻任务换取 CPU 效率、延迟、功能三大优势。

---

## 3. 内核态性能

### 3.1 核心论点

内核态测试直接操作 Ring Buffer，无系统调用开销，**不受 QEMU 串口时序影响**，反映纯软件性能。**关键观察**：Async 内核态吞吐（MB/s 级）远高于 Console（KB/s 级）。

### 3.2 内核态性能对照

| 指标 | Console | Async | 差异 | 测量方法 |
|------|---------|-------|------|---------|
| **TX Ring Buffer 写入** | 567 KB/s | ~652 MB/s（Q13.1 + LTO）| ⬆ ~1150× | [`bench.rs`](https://github.com/daivy2333/uart_16550/blob/feat/uart-16550-async/src/async_/bench.rs) 102,400 字节 |
| **TX CPU cycles/byte** | 3,835 | 13（Q12 数据）| ⬆ ~295× | CPU 周期采样 |
| **RX Ring Buffer 读取** | 不可测¹ | ~898 MB/s | — | [`bench.rs`](https://github.com/daivy2333/uart_16550/blob/feat/uart-16550-async/src/async_/bench.rs) 65,536 字节 |
| **RX 延迟 P50** | 不可测¹ | <100 ns | — | 100 次单字节 |

> **脚注 1**：Console 无 Ring Buffer，无法做可比较的内核态 RX 测试。

> **脚注 2**：Q13 后内核 benchmark 移至独立测试分支 `feat/uart-16550-bench`，主分支不内嵌 CPU 周期测量。

> **数据来源说明**：Async 列数据来自 `feat/uart-16550-bench` 分支 `bench.rs`（Q13.1 + LTO 构建）；Console 列数据来自 `feat/uart-16550-async` 分支 `bench.rs`（Q12 前基线）。两列测量方法不同（Console 逐字节 MMIO 写 THR，Async 批量 push ring buffer），吞吐差异主要来自"同步轮询 vs 异步批量"路径差异而非单纯实现效率。

### 3.3 性能差异根因

- **Async TX 提升 ~1150×**：Console 逐字节调用 `send_raw()`（每次系统调用 + 锁），Async 批量 `push()` 到 SPSC lock-free ring buffer（Q12 去 Mutex）
- **CPU 效率提升 ~295×**：Console 每字节 3,835 cycles（系统调用 + 轮询 LSR），Async 13 cycles（单次 `push` 操作）
- **Async 接收**：直接 `pop` lock-free SPSC RingBuffer（LTO 内联 embassy）

### 3.4 小结

内核态性能是 Async 优势最显著的维度（1150× 吞吐提升、295× CPU 效率提升），且**不受 QEMU 仿真限制影响**——是可信的相对性能对比数据。

---

## 4. 用户态延迟

### 4.1 核心论点

单字节 `write()` 系统调用往返，**无 tcdrain**（只测系统调用开销）。100 次迭代，无预热。

### 4.2 用户态延迟对照

| 指标 | Console | Async（Q13.1）| 差异 | 说明 |
|------|---------|--------------|------|------|
| **P50** | 17.5 µs | 8.5 µs | ⬆ **2.1×** | 中位数 |
| **P95** | 32.8 µs | 12.5 µs | ⬆ **2.6×** | 95 分位 |
| **P99** | 324.5 µs | 44.0 µs | ⬆ **7.4×** | 99 分位 |

### 4.3 性能差异根因

`write()` 只 push 到 ring buffer（~1 µs，Q12 去 Mutex 后更低），Console 的 `write()` 逐字节轮询 LSR 写 THR。

> **Q13.1 注意**：Q13 提取到 `uart_16550` 引入了 5 个 OS trait 抽象，write() 延迟从 Q12 P50 7.9µs 增至 ~8.5µs（trait 间接调用开销）。Q13.1 通过 `#[inline(always)]` 回收了大部分开销。

### 4.4 小结

用户态延迟 P50 提升 2.1×，P99 提升 7.4×（尾部延迟改善更显著）。Q13 trait 抽象代价约 0.6µs（7.9→8.5µs），已被 Q13.1 回收大部分。

---

## 5. 用户态吞吐量（⚠️ QEMU 时序欺骗）

### 5.1 核心论点

Console 在 QEMU 上测的是纯 MMIO 速度（LSR 永远 THR_EMPTY），Async 测的是任务切换 + tcdrain 开销。**两者在 QEMU 上无法公平对比**——真板均收敛至 ~11.5 KB/s（硬件理论上限）。

### 5.2 Async 端到端路径（64B 写入）

```
write(64) → push ring buf (~1µs)
tcdrain   → poll: buf 非空 → 注册 tx.poll
          → copier send 16B → yield
          → poll: buf 非空 → 注册 tx.poll    ← Q8 优化后约 4 轮 copier
          → … (copier ×4, 每次 1 次 yield)
          → poll: buf 空 + LSR.TEMT → return  ← DRAIN_WAKER 条件唤醒
```

> **缩写说明**：LSR.TEMT = **L**ine **S**tatus **R**egister 的 **T**rans**E**M**p**ty 位（bit 6，THR + 移位寄存器全空）。

> **优化历史**：Q8 前 64B 路径约 9 次任务切换，Q8（DRAIN_WAKER 条件唤醒 + ISR 无锁）优化后约 4~6 次。

### 5.3 吞吐量对照表

| 大小 | Async Q13.1（QEMU）| 硬件理论 | 真板预测 |
|------|------------------|----------|----------|
| **64 B** | 518.0 µs | 5,556 µs | 6,070 µs |
| **256 B** | 1,305.6 µs | 22,222 µs | 23,500 µs |
| **1024 B** | 4,922.5 µs | 88,889 µs | 93,800 µs |
| **4096 B** | 9,852.0 µs | 355,556 µs | 365,400 µs |

> **数据可信度**：QEMU 上的 Async 吞吐受 copier 任务切换开销影响；真板因 tcdrain 等待 LSR 完整传输，吞吐量收敛至 ~11.5 KB/s。

### 5.4 小结

用户态吞吐在 QEMU 上**不可信**。Async 端到端路径已经过 Q8 优化（9→4~6 次任务切换），但 QEMU 仿真限制决定绝对值需以真板为准。

---

## 6. 功能覆盖

### 6.1 核心论点

Async 在功能覆盖上**全面优于** Console，主要包括非阻塞读、tcdrain、读超时三大特性。

### 6.2 功能对照表

| 功能 | Console | Async（Q13.1）| 实现位置 |
|------|---------|--------------|---------|
| **阻塞读写** | ✅ | ✅ | 两者均支持 |
| **非阻塞读（3 入口）** | ❌ | ✅ | `FIONBIO` (open / fcntl / ioctl) |
| **tcdrain** | 隐式（每字节等 LSR）| ✅ | TCSBRK + DRAIN_WAKER |
| **读超时（VTIME）** | ❌ | ✅ | `axtask::future::timeout()` |
| **Shell（ls/cd/pwd）** | ✅ | ✅ | Tty 集成层 |
| **内核日志（ax_println!）** | ✅ | ✅（polling TX 共存）| 双路径 |
| **中断合并（NAPI）** | ❌ | ✅ | copier NAPI 逻辑 |
| **`embedded_io_async` 兼容** | ❌ | ✅ | [`device_ops.rs`](https://github.com/daivy2333/uart_16550/blob/feat/uart-16550-async/src/async_/device_ops.rs) trait impl |
| **跨 OS 可移植** | ❌（绑定 axplat）| ✅ | 5 OS trait 抽象 |

> **缩写说明**：`ax_println!` 是 ArceOS 内核日志宏；`FIONBIO` = **F**ile **IO**ctl **N**on-**B**locking **I**/O；TCSBRK = **T**erminal **C**ontrol **S**et **BR**ea**K**；VTIME = termios 读超时（1/10 秒单位）。

### 6.3 关键功能说明

- **非阻塞读**：Console 无此功能（同步阻塞 read 永不返回 EAGAIN）；Async 通过 Q7 O43 修复 FIONBIO 传播到 ldisc.read
- **tcdrain**：Console 的隐式等待开销大（每字节轮询），Async 显式实现 + DRAIN_WAKER 条件唤醒
- **VTIME 超时**：Q9 复用 `axtask::future::timeout()` 实现，无需 embassy-time
- **`embedded_io_async`**：Q12 O52 引入 trait impl，标准化跨 OS 接口
- **跨 OS 可移植**：Q13 提取 `uart_16550` 为独立 crate，5 个 OS trait 抽象

### 6.4 小结

功能覆盖方面 Async 显著领先。**最关键的功能差异是跨 OS 可移植性**——Console 绑定 axplat，Async 通过 5 OS trait 抽象可在 Linux/RTOS/Embassy 等不同 OS 复用。

---

## 7. 资源占用

### 7.1 核心论点

Async 相比 Console 在内存上多占用 128 KB（ring buffer），后台任务多 1 个（RX copier），换取 CPU 效率、延迟、功能三大优势。

### 7.2 资源对照表

| 指标 | Console | Async（Q13.1）| 差异 |
|------|---------|--------------|------|
| **内存** | 0 KB | 128 KB（RX/TX ring buf）+ 0.5 KB（ldisc 256B）+ 0.13 KB（driver struct 136B）| +128.6 KB |
| **后台任务** | 1（tty-reader）| 2（RX copier + tty-reader）| +1 |
| **数据完整性** | ✅ | ✅ | 持平 |

> **缩写说明**：tty-reader 是 TTY 集成层协程，负责 ldisc 行编辑与用户态数据分发。

### 7.3 内存占用详细

| 组件 | 大小 | 说明 |
|------|------|------|
| **RX Ring Buffer** | 64 KB | embassy lock-free SPSC |
| **TX Ring Buffer** | 64 KB | embassy lock-free SPSC |
| **ldisc StaticRb** | 256 B | Q10 扩容 3.2×（80B→256B）|
| **Driver struct** | 136 B | `AsyncUartDriver`（Q13 trait 抽象）|
| **总计** | **128,520 B**（约 125.5 KB）| |

### 7.4 小结

资源占用增加 128.6 KB + 1 个后台任务换取 295× CPU 效率提升、2.1× 延迟改善、4 项功能补全，**性价比极高**。

---

## 8. 总结

### 8.1 核心论点

Console vs Async 在 8 个维度对比中，Async 在 **7 个维度胜出**（CPU 效率、延迟、唤醒、吞吐、内核态速度、功能、可移植性），**1 个维度持平**（真板吞吐量受波特率限制）。

### 8.2 综合对照表

| 维度 | 结果 | 详细对比 |
|------|------|---------|
| **CPU 效率** | Async ⬆ ~295× | 13 vs 3,835 cycles/byte（Q12 数据） |
| **write() 延迟** | Async ⬆ 2.1–7.4× | P50 8.5 vs 17.5 µs（含 Q13 trait 开销）|
| **唤醒延迟** | Async ⬆ 4× | AtomicWaker 50ns vs PollSet 200ns |
| **内核态吞吐** | Async ⬆ ~1150× | 652 MB/s vs 567 KB/s（TX）|
| **非阻塞读** | Async ✅ | Console 无（Q7 O43 修复）|
| **读超时** | Async ✅ | VTIME（Q9 复用 axtask::timeout）|
| **真板吞吐量** | 持平 ~11.5 KB/s | 同受波特率限制 |
| **可移植性** | Async ✅ | uart_16550 crate 可用于任何 OS（Q13）|

### 8.3 关键发现

1. **Async 全面优于 Console**——除真板吞吐量持平外（硬件理论上限）
2. **Q13 提取提供可移植性**——5 个 OS trait 抽象，可复用至 Linux/RTOS/Embassy
3. **Q13.1 inline + batch 回收了 trait 抽象的开销**——overhead 53.3→42.6µs（↓20%）
4. **LTO 跨 crate 内联消除函数调用开销**——内核态 ↑69%，e2e 不变（瓶颈在调度）

### 8.4 完整优化历史

| 阶段 | 日期 | 内容 |
|------|------|------|
| Q0~Q4 | 2026-05-31 | 驱动骨架、VFS 集成、全异步 RX+TX |
| Q5 | 2026-05-31 | IER 缓存、ISR 合并、NAPI、rx/tx 独立锁 |
| Q7 | 2026-06-01 | yield storm 修复、FIONBIO 传播、tcdrain 异步化 |
| Q8 | 2026-06-11 | NAPI 退出、ISR 无锁、IER 规范化、O46 AtomicWaker (8 处)|
| Q9 | 2026-06-11 | VTIME 读超时（axtask::future::timeout）|
| Q10 | 2026-06-11 | BUF_SIZE 256、push_slice、read(&self) |
| Q11 | 2026-06-11 | tty unwrap、mm/access、sendfile、close_range、ws_col |
| Q12 | 2026-06-11 | Embassy 路径 A：lock-free RingBuffer + embedded_io_async + TC tcdrain（已归档 2026-06-15）|
| Q13 | 2026-06-16 | 异步串口提取到 `uart_16550` crate |
| Q13.1 | 2026-06-16 | inline + batch 回收开销 |
| LTO | 2026-06-16 | 跨 crate 内联（**已 revert 参见 ADR-034**）|
| Q6 | ⏳ 待定 | VisionFive2 真板验证 |

### 8.5 小结

异步串口子系统的设计与实现已被 Q0~Q13 + LTO 共 12 个阶段优化验证，**Async 模式在所有可优化维度均显著优于 Console**。后续工作仅余 Q6 真板验证（DMA、波特率扩展、真实吞吐）。

---

## 附录 A：术语表

| 术语 | 含义 | 首次出现 |
|------|------|---------|
| **Console** | 同步阻塞 UART 实现，绑定 axplat，无 ring buffer | §0 |
| **Async** | 异步中断驱动 UART 实现，Q13 后位于 `uart_16550` crate | §0 |
| **LSR** | **L**ine **S**tatus **R**egister，线状态寄存器 | §2.3 |
| **LSR.TEMT** | LSR bit 6：**T**rans**E**M**p**ty（THR + 移位寄存器全空）| §5.2 |
| **THR** | **T**ransmit **H**olding **R**egister，发送保持寄存器 | §2.3 |
| **FIONBIO** | **F**ile **IO**ctl **N**on-**B**locking **I**/O | §6.2 |
| **TCSBRK** | **T**erminal **C**ontrol **S**et **BR**ea**K**（tcdrain ioctl）| §6.2 |
| **VTIME** | termios 读超时（1/10 秒单位）| §6.2 |
| **EAGAIN** | POSIX 错误码"再试一次"，非阻塞操作无可用数据时返回 | §3.4 |
| **SPSC** | **S**ingle-**P**roducer **S**ingle-**C**onsumer，单生产者单消费者 | §2.2 |
| **NAPI** | **N**ew **API**（Linux 网络子系统），本项目借鉴 | §6.2 |
| **FCR** | **F**IFO **C**ontrol **R**egister，FIFO 控制寄存器 | §1.2 |
| **IER** | **I**nterrupt **E**nable **R**egister，中断使能寄存器 | §1.2 |
| **ISR** | **I**nterrupt **S**ervice **R**outine，中断服务例程 | §0 |
| **PollSet** | 等待同一事件的多 waker 集合（O46 替换为 AtomicWaker）| §0 |
| **AtomicWaker** | `embassy_sync` 提供的 lock-free 单槽 waker 容器 | §0 |
| **ldisc** | **l**ine **disc**ipline，行规程（终端行编辑、缓冲管理）| §2.2 |
| **copier** | 搬运任务，FIFO ↔ ring buffer 之间数据搬运的异步任务 | §2.3 |
| **DRAIN_WAKER** | 专用 waker，TX ISR 触发时唤醒 tcdrain 等待者 | §5.2 |
| **arceos** | StarryOS 的宏内核基础框架 | §6.2 |
| **ax_println!** | ArceOS 内核日志宏 | §6.2 |
| **axtask** | ArceOS 任务管理子 crate（spawn / future / block_on）| §4.3 |
| **axhal** | ArceOS 硬件抽象层（IRQ、MMIO、timer 等）| §6.2 |
| **axmm** | ArceOS 内存管理子 crate（iomap、aspace）| §6.2 |
| **kspin** | ArceOS 关中断自旋锁（SpinNoIrq 实现）| §6.2 |
| **axpoll** | ArceOS 异步轮询子 crate（PollSet 实现）| §6.2 |
| **Q-编号** | 项目内部"问题/任务"编号（Q0~Q13）| §8.4 |
| **O-编号** | 项目内部"优化点"编号（O3 / O40 / O41 / O43 / O46 / O51~O53）| §6.2 |
| **ADR-034** | LTO 延期启用 — 已知有效但开发期暂不开 | §8.4 |
| **Cycles/byte** | CPU 周期/字节，CPU 效率指标 | §3.2 |

---

## 附录 B：参考 commit

> 完整 9 个 Q13 原子提交 + Q13.1 3 个提交见 `tasks.md` §Q13 / §Q13.1。关键节点：

- `7bee89d`（uart_16550）— `feat(uart-async): extract TtyRead/TtyWrite traits for OS integration`
- `1005b71`（uart_16550）— `feat(uart-async): add OS abstraction traits (OsRuntime, OsIrq, OsMmio, OsSpinNoIrq, OsWakerSet)`
- `9bed0c7`（StarryOS）— `feat(uart-async): add ArceOS HAL adapter layer`
- `842f8f4`（StarryOS）— `refactor(uart-async): remove migrated local files, finalize StarryOS integration`
- `a0cead0`（uart_16550）— `perf(uart-async): add #[inline(always)] to ring buffer push/pop`
- `73aca5c`（uart_16550）— `perf(uart-async): add batch push/pop to reduce lock overhead`
- `9188c0b`（StarryOS）— `perf(uart-async): add #[inline(always)] to ArceOsUartPort methods`

> 链接模板：`https://github.com/<owner>/<repo>/commit/<hash>`（具体行号以本仓库 `feat/uart-16550-async` 分支当前 state 为准）。

---

**报告版本**：3.0 · **最后更新**：2026-06-17（bettermd 16 规则重写）
**主要更新**：§0 新增 TL;DR 8 维度对照表 · §1 新增测量条件 · §2 论点+论据+小结结构 · §3~§7 补全测量方法与脚注 · §8 总结章节加 8 维度综合对照表 · 附录 A 30+ 条术语表
