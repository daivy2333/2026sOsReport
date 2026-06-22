# 异步操作系统训练营学习记录

基于 Rust 协程异步机制，对设备驱动、调度器、系统调用、IPC 等内核模块做异步改造的练习记录。

## 仓库结构

```
.
├── reports/                    # 周报、规划、阶段总结
│   ├── 第一次交流纪要.md
│   ├── 第一周总结和下周计划.md
│   ├── 六月第一周.md
│   ├── 个人规划.md
│   ├── weekly-2026-W20.md
│   ├── weekly-2026-W21.md
│   ├── weekly-2026-W22.md
│   ├── weekly-2026-W23.md
│   ├── 2026s训练营阶段总结.md          # 串口驱动主线完整总结
│   ├── StarryOS异步串口驱动：从阻塞到唤醒.pdf
│   └── StarryOS异步串口驱动：从阻塞到唤醒.pptx
├── 产出/
│   ├── 任务一/                  # 进程、线程、协程
│   │   ├── docs/
│   │   │   ├── async底层-future.md
│   │   │   ├── 异步特性变化.md
│   │   │   ├── 进程线程协程.md
│   │   │   └── 爬虫任务.md
│   │   └── 爬虫任务/
│   │       └── crawler_rs/              # Rust 进程/线程/协程爬虫
│   │           └── 性能对比_*.md         # 5 份负载场景实测
│   ├── 任务二/                  # 用户态线程与协程
│   │   ├── docs/
│   │   │   ├── 异步：从上到下.md
│   │   │   ├── 无栈协程原理.md
│   │   │   ├── 绿色线程实现分析.md
│   │   │   └── 执行流状态变迁分析.md
│   │   └── green-thread/                 # 绿色线程实现 + 优先级扩展
│   │       ├── src/                      # x86-64 Linux/Win64 + RISC-V 64
│   │       ├── tests/priority_scheduling.rs
│   │       ├── priority-extension.md
│   │       └── README.md
│   ├── 任务三/                  # 调度器对比与优化
│   │   ├── docs/
│   │   │   ├── Tokio优先级分发器优化.md
│   │   │   └── 优先级调度在满负载下的效果分析.md
│   │   └── green-thread-crawler/         # 4 调度器 × 多负载
│   │       ├── src/                      # main.rs / tokio_scheduler.rs 等
│   │       ├── tests/integration.rs
│   │       ├── memory_*.md               # 4 份内存实测
│   │       └── *.md                      # 5 份延迟实测
│   ├── 任务四/                  # 异步串口驱动
│   │   ├── 不在这里.md                  # 路标：实现在外部仓库
│   │   └── serial-optimization-preview.md
│   └── 一些别的/                # 训练营外但相关的产出
│       ├── 内核态用户态.md
│       ├── 异步数据库练习.md
│       └── 新学习范式的讨论.md
└── README.md
```

## 训练营目标

基于 Rust `Future` 机制，对设备驱动、调度器、系统调用、IPC 等内核模块做异步改造。

**基础**：完成前两阶段练习，撰写清晰的分析文档。
**进阶**：在理解代码基础上做力所能及的改进。
**优秀**：做出独到见解的重大改进或创新功能。

## 第一阶段：进程、线程、协程

学习三种并发模型并做性能对比。

**产出**：
- Rust 进程/线程/协程三版爬虫（`crawler_rs/`，5 份负载实测）
- `爬虫任务.md`、`进程线程协程.md`、`async底层-future.md`、`异步特性变化.md`

## 第二阶段：用户态线程与协程

跟踪执行流状态变迁，扩展绿色线程调度器支持优先级。

**产出**：
- `green-thread/` 调度器（x86-64 Linux/Win64、RISC-V 64，priority-extension）
- `priority_scheduling.rs` 集成测试
- `无栈协程原理.md`、`绿色线程实现分析.md`、`执行流状态变迁分析.md`、`异步：从上到下.md`

## 第三阶段：调度器对比与优化

在优先级调度器上接入爬虫负载，对比 GT Priority / GT RoundRobin / Tokio Priority / Tokio Default，并优化 Tokio 分发器。

**产出**：
- `green-thread-crawler/`（4 调度器 × 多负载，9 份实测报告）
- `Tokio优先级分发器优化.md`：BinaryHeap → 三队列批量分发，Priority vs Default 差距从 1.83s（40%）缩至 0.11s（2.4%）
- `优先级调度在满负载下的效果分析.md`：含协程内存对比，priority 字段 1 字节/协程，占比 0.000048%

## 第四阶段：异步串口驱动

选择 UART 作为目标内核组件，做性能与通用性优化，在 StarryOS 上集成并经 QEMU 验证。

**产出**：
- `serial-optimization-preview.md`：从同步阻塞到异步高性能的方案预览
- `不在这里.md`：路标。完整实现在外部仓库。
- 阶段总结 `reports/2026s训练营阶段总结.md`

## 一些别的

训练营外但与异步主题相关的产出。

- `内核态用户态.md`：传统隔离 vs Unikernel/Library OS 的设计取舍
- `异步数据库练习.md`：[RTsql](https://github.com/daivy2333/RTsql) 异步嵌入式数据库的设计与性能
- `新学习范式的讨论.md`：AI 时代经验形成机制与刻意重建 PEL 循环

## 周报与阶段总结

- `weekly-2026-W20.md` ~ `weekly-2026-W23.md`：四周周报
- `第一周总结和下周计划.md`、`六月第一周.md`：阶段小结
- `2026s训练营阶段总结.md`：第四阶段主线总结（8 处 bug 修复 + 5 项性能优化）
- `StarryOS异步串口驱动：从阻塞到唤醒.{pdf,pptx}`：答辩材料

## 外部仓库

- StarryOS（`asyncuart-dev` 分支）：<https://github.com/daivy2333/StarryOS/tree/asyncuart-dev>
- `uart_16550` 驱动库 fork：<https://github.com/daivy2333/uart_16550>
- `embassy` fork（仅做验证用）：<https://github.com/daivy2333/embassy>
- RTsql 异步数据库：<https://github.com/daivy2333/RTsql>

## 作者

daivy2333
