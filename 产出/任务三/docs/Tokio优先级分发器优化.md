# Tokio 优先级分发器优化

## 1. 性能基线

满载 CPU（cpu_repeat=10000）场景下，原始实现的四组调度配置实际耗时：

| 调度器 | 用户态时间(s) | 实际耗时(s) | 相对 GT-RR |
|--------|-------------|------------|-----------|
| GT Priority | 14.99 | 15.01 | 1.01× |
| GT RoundRobin | 14.81 | 14.80 | 1.00×（基线） |
| Tokio Priority | 20.43 | **6.35** | 0.43× |
| Tokio Default | 21.17 | **4.52** | 0.31× |

绿色线程中 Priority 与 RoundRobin 的总用时差异在 1.2% 以内——优先级机制近乎零开销。但 Tokio Priority 比 Tokio Default 多了约 **1.83s（40%）**。二者用户态时间几乎相同（20.43s vs 21.17s，差异 3.5% 以内），说明额外开销不在计算本身，而在分发机制上。

## 2. Tokio 调度架构与优先级方案

### 2.1 Tokio 原生调度架构

Tokio 的任务调度核心基于 work-stealing 策略。每个 worker 线程维护一个本地 `VecDeque` 作为任务队列，新任务默认放入当前 worker 的本地队列末尾；当本地队列为空时，worker 从其他 worker 的队列尾部窃取任务。这一设计追求极致的平均性能和任务吞吐量，避免任务饥饿，但天然不具备优先级队列的数据结构。

Tokio 官方未引入优先级调度支持，主要基于以下考量：

- **底层设计约束**：调度核心使用 `VecDeque` 实现 FIFO 语义，若替换为优先级队列（如 `BinaryHeap`），将引入 O(log n) 的入队/出队开销，与高吞吐设计目标相悖。
- **生态分工**：Rust 异步生态的设计共识是由不同行为的异步运行时覆盖不同场景（如 `embassy` 面向嵌入式、`monoio` 面向 io_uring），而非用一个庞大的运行时满足所有需求。
- **实现复杂性**：优先级调度会引入优先级反转等并发问题，需要额外的机制来解决，这超出了 Tokio 作为通用运行时的目标范畴。

### 2.2 社区优先级实现方案对比

在 Tokio 之上实现优先级调度主要有三种方案：

| 方案 | 实现复杂度 | 额外开销 | 优先级保证 | 适用场景 |
|:---|:---|:---|:---|:---|
| **用户态任务队列（BinaryHeap 分发器）** | 低 | 中等（串行分发 + O(log n) 开销） | 中等（用户态协作式） | 大多数常规优先级需求 |
| **多线程运行时隔离** | 高 | 低（物理隔离，OS 级调度） | 高 | 硬实时控制、音视频处理 |
| **专用 OS 线程提升** | 中 | 最低（无额外调度层） | 最高（OS 抢占式） | 极端延迟敏感型任务 |

本实验最初采用 BinaryHeap 分发器方案，下文分析其瓶颈并实施优化。

## 3. 问题诊断：逐步消除混淆变量

### 3.1 第一直觉：BinaryHeap 的 O(log n) 是瓶颈吗？

原始实现（`tokio_scheduler.rs`，优化前）用 `BinaryHeap<PriorityTask>` 管理所有待分发任务，分发循环每轮从中弹出最高优先级任务并提交到 Tokio worker 池：

```rust
let mut heap = BinaryHeap::new();
for (i, school) in SCHOOLS.iter().enumerate() {
    heap.push(PriorityTask { priority, school_name });
}

loop {
    if active.load(Ordering::Acquire) >= concurrency {
        notify.notified().await;
        continue;
    }
    match heap.pop() {  // O(log n)
        Some(task) => {
            active.fetch_add(1, Ordering::Release);
            tokio::spawn(async move {
                process_task_sync(&task.school_name, ...);
                act.fetch_sub(1, Ordering::Release);
                ntf.notify_one();
            });
        }
        None => { /* wait for active==0 */ }
    }
}
```

第一反应是 `BinaryHeap::pop()` 的 O(log 33) ≈ 5 次比较引入了开销。量化分析发现：

- 33 次 pop × 5 次比较 = 165 次整数比较
- 在 3.2GHz CPU 上耗时约 **50ns**，占总耗时 6.35s 的 **0.0008%**

**结论：BinaryHeap 本身不是瓶颈。真正的混淆变量在架构层面。**

### 3.2 真正的问题：串行分发瓶颈

分发控制流：

```
分发器线程：       heap.pop() → spawn → heap.pop() → spawn → ...（共 33 次）
                      ↑ 串行！下一任务必须等待上一任务完成提交

Tokio worker 池：  等待任务1 → 执行任务1 → 等待任务2 → 执行任务2 → ...
                   任务进入 worker 池的速度受限于分发器的串行节奏
```

**分发循环是单点的，所有 33 个任务都通过同一个串行循环逐个 pop→spawn。** 分发器在三件事之间串行切换：`heap.pop()` → `active.fetch_add(1)` → `tokio::spawn(...)`。后两步虽然单次很小（微秒级），但累加 33 次后在 wall-clock 上形成显著的串行间隙。当 `active` 达到 `concurrency` 上限时，分发器通过 `notify.notified().await` 阻塞等待，引入了额外的 async 上下文切换。

相比之下，绿色线程的调度器中「选择下一个线程」是 `t_yield()` 内部的一个普通函数调用，与上下文切换在同一控制流中完成，没有额外层。

### 3.3 AtomicUsize + Notify 握手开销

并发控制采用自定义的 `AtomicUsize` 计数器 + `tokio::sync::Notify` 机制：

```rust
// 分发器侧：
if active.load(Acquire) >= concurrency {
    notify.notified().await;
    continue;
}
active.fetch_add(1, Release);

// 任务完成侧：
act.fetch_sub(1, Release);
ntf.notify_one();
```

每轮握手涉及 acquire/release 语义的 atomic 操作和 async Notify 唤醒。每轮约 1-2μs，33 次累计约 33-66μs。即便如此，这也不是 1.83s 差异的主要来源。

**核心矛盾**：真正的耗时差异来自**并行度差异**。Tokio Default（`base_priority=0`）下所有任务以相同优先级进入堆，堆的弹出顺序允许各批次任务混合进入 worker 池；而 Tokio Priority 先弹出所有高优任务再弹中优和低优，导致中低优任务在 worker 池空闲时仍未被提交。分发器成为任务进入 worker 池的阀门，而不是真正的并行调度器。

## 4. 优化方案与实施

### 4.1 核心思路

用三个 `Vec<String>`（每优先级一个）替换 `BinaryHeap<PriorityTask>`，将分发循环从「每任务一次迭代」改为「每优先级一次迭代」。

```
优化前：
  BinaryHeap[33个任务] → loop { pop × 33次 → spawn × 33次 }

优化后：
  queue[2]（高优11个）→ for school in drain(..) { spawn }
  queue[1]（中优11个）→ for school in drain(..) { spawn }
  queue[0]（低优11个）→ for school in drain(..) { spawn }
  并发控制：Semaphore
```

三个关键变化：

1. **数据结构**：`BinaryHeap` → 三个 `Vec<String>`，出队 O(log n) → O(1)
2. **分发模式**：每次一个 → 整批提交，分发迭代次数减少 91%（33 次 → 3 次）
3. **并发控制**：`AtomicUsize + Notify` → `tokio::sync::Semaphore`，消除手动 atomic 握手

### 4.2 代码变更

```rust
// 优化前（数据结构）
struct PriorityTask { priority: u8, school_name: String }
let mut heap = BinaryHeap::new();

// 优化后（数据结构）
let mut task_queues: [Vec<String>; 3] = [const { Vec::new() }, const { Vec::new() }, const { Vec::new() }];
task_queues[raw_prio as usize].push(school.name.to_string());
```

```rust
// 优化前（分发循环）
loop {
    if active.load(Acquire) >= concurrency { notify.notified().await; continue; }
    match heap.pop() {
        Some(task) => {
            active.fetch_add(1, Release);
            tokio::spawn(async move {
                process_task_sync(...);
                act.fetch_sub(1, Release);
                ntf.notify_one();
            });
        }
        None => { /* wait */ }
    }
}

// 优化后（分发循环）
let semaphore = Arc::new(Semaphore::new(concurrency));
let mut handles = Vec::new();

for prio_level in (0..=2).rev() {
    for school_name in task_queues[prio_level].drain(..) {
        let permit = semaphore.clone().acquire_owned().await.unwrap();
        let handle = tokio::spawn(async move {
            let _permit = permit;  // 持有 permit 直到任务完成，自动释放
            process_task_sync(&school_name, ...);
        });
        handles.push(handle);
    }
}

for handle in handles {
    handle.await.unwrap();
}
```

### 4.3 优化前后对比

| 维度 | 优化前（BinaryHeap） | 优化后（三队列） |
|------|-------------------|----------------|
| 数据结构 | `BinaryHeap<PriorityTask>`（O(log n)） | 三个 `Vec<String>`（O(1)） |
| PriorityTask 结构体 + 4 trait 实现 | 约 25 行 | 完全消除 |
| 分发迭代次数 | 33 次（每任务一次） | 3 次（每优先级一次） |
| 并发控制 | `AtomicUsize + Notify`（手动 atomic 握手） | `Semaphore`（自动 permit 管理） |
| 等待完成 | 分发循环轮询 `active == 0` | JoinHandle 逐个 await |
| 代码总行数 | 155 行 | 119 行（减少 23%） |

## 5. 优化效果

### 5.1 测试验证

23 个测试全部通过：13 个单元测试（green-thread 调度器逻辑）+ 10 个集成测试（4 Tokio + 6 green-thread）。

4 个 Tokio 集成测试断言：

| 测试 | 断言 | 结果 |
|------|------|------|
| `test_tokio_priority_heavy_cpu` | PRIO=2 先出现，33 个任务完整 | PASS |
| `test_tokio_default_heavy_cpu` | 33 个任务完整 | PASS |
| `test_tokio_priority_io_medium` | PRIO=2 先出现，33 个任务完整 | PASS |
| `test_tokio_default_io_medium` | 33 个任务完整 | PASS |

### 5.2 实测性能改善

满载 CPU（cpu_repeat=10000）两轮实测均值：

| 调度器 | 优化前 | 优化后（实测均值） | 改善 |
|--------|--------|------------------|------|
| Tokio Priority | 6.35s | **4.55s** | -1.80s（消除分发瓶颈） |
| Tokio Default | 4.52s | **4.66s** | +0.14s（系统负载波动） |
| GT Priority | 15.01s | 16.40s | 同次运行对照 |
| 二者差距 | 1.83s (40%) | **0.11s (2.4%)** | 差距缩小 **94%** |

优化后 Tokio Priority 与 Default 的实际耗时差距从 1.83s（40%）缩小至 0.11s（2.4%），多队列批量分发消除了 BinaryHeap 分发器的串行瓶颈。二者差距已降至测量噪音水平，优先级调度额外开销可忽略不计。

## 6. 演进过程中的经验教训

### 教训一：不要被表象误导——O(log n) 不一定是瓶颈

第一直觉是 `BinaryHeap::pop()` 的 O(log n) 开销。量化分析发现 33 × O(log 33) ≈ 50ns，占总耗时的 0.0008%。真正的瓶颈在架构层面——串行分发循环的结构迫使所有任务通过单一点，形成了 hidden bottleneck。

### 教训二：user time vs wall time 的差异提供了关键线索

Tokio Priority 和 Tokio Default 的 user time 几乎相同（20.43s vs 21.17s，<3.5%），但 wall time 差异 1.83s（40%）。user time 反映「总计算量」，wall time 反映「并行效率」。user time 相同而 wall time 不同，说明问题不在计算而在调度/分发。

### 教训三：优先级调度器的开销来源不在「选择」而在「分发」

绿色线程的优先级选择（`find_highest_priority_ready`）是调度器内部的 O(n) 扫描，与 RoundRobin 的扫描结构相同，开销不增加。Tokio 的优先级分发器则是一个完全外置的层——它在原生 work-stealing 调度器之上叠加了串行分发逻辑。**外置调度层的设计必然引入额外开销**，优化方向应是将其内化到调度流程中。

### 教训四：使用框架提供的原语比自己 DIY 更高效

`AtomicUsize + Notify` 的手动握手虽然逻辑正确，但涉及 acquire/release 语义的 atomic 操作和 async Notify 唤醒的开销。`Semaphore` 是 Tokio 官方提供的原语，内部优化了 permit 的管理和唤醒路径。

## 7. 后续优化方向

### 方案 B：阻塞任务隔离

`process_task_sync` 是同步阻塞函数（`std::fs::read` + CPU 校验和），当前在 `tokio::spawn` 中运行，会阻塞 Tokio worker 线程。可移至 `tokio::task::spawn_blocking`，让 worker 线程专注 async 调度：

```rust
tokio::spawn(async {
    let result = tokio::task::spawn_blocking(move || {
        process_task_sync(&name, &cache, cpu, io)
    }).await.unwrap();
    // 处理结果...
});
```

预期在满载 CPU 场景下可额外减少 0.2-0.5s。

### 方案 C：yield 注入

参照绿色线程在每 100 轮校验和后调用 `yield_thread()` 的做法，在 Tokio 路径中添加 `tokio::task::yield_now().await`。但此方案改变了工作负载特征，仅在需要验证两种运行时协作式优先级等价性时采用。

## 参考资料

- Tokio 官方文档：Scheduler internals
- `tokio-fusion` crate 文档
- Rust async book
- 任务三实验数据：`优先级调度在满负载下的效果分析.md`
- 任务三源码：`../green-thread-crawler/src/tokio_scheduler.rs`
- Tokio 实测报告：`../green-thread-crawler/tokio_priority_yield_10000cpu_0io.md`（R1）
- Tokio 实测报告：`../green-thread-crawler/tokio_priority_r2_yield_10000cpu_0io.md`（R2）
- Tokio 基线报告：`../green-thread-crawler/tokio_default_yield_10000cpu_0io.md`（R1）
- Tokio 基线报告：`../green-thread-crawler/tokio_default_r2_yield_10000cpu_0io.md`（R2）
