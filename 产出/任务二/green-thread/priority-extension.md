# Green Thread 优先级扩展

## 改动概述

在原有的 round-robin 调度器基础上，为每个 green thread 增加 `priority` 字段，并将调度策略改为**最高优先级优先**。

## 具体修改

### 1. Thread 结构体增加优先级字段

```rust
struct Thread {
    id: usize,
    stack: Vec<u8>,
    ctx: ThreadContext,
    state: State,
    priority: u8,          // 新增：0 为最低优先级，数字越大优先级越高
    task: Option<Box<dyn Fn()>>,
}
```

### 2. 调度算法：从 round-robin 改为最高优先级优先

原调度算法（`t_yield`）采用 round-robin：从当前线程的下一个位置开始，顺序查找下一个 Ready 状态的线程。

改为最高优先级优先：在所有 Ready 状态的线程中选择 `priority` 值最大的一个:

```rust
fn find_highest_priority_ready(&self) -> Option<usize> {
    let mut best_idx = None;
    let mut best_prio: u8 = 0;
    for (i, t) in self.threads.iter().enumerate() {
        if t.state == State::Ready && (best_idx.is_none() || t.priority > best_prio) {
            best_idx = Some(i);
            best_prio = t.priority;
        }
    }
    best_idx
}
```

### 3. 扩展 spawn 接口

保留原有的 `spawn(f)`（优先级默认为 0），新增 `spawn_with_priority(f, priority)`:

```rust
pub fn spawn(&mut self, f: fn()) {
    self.spawn_inner(f, 0);
}

pub fn spawn_with_priority(&mut self, f: fn(), priority: u8) {
    self.spawn_inner(f, priority);
}
```

## 测例设计

`main()` 函数中创建三个不同优先级的 green thread：

| 线程 | 优先级 | 预期行为 |
|------|--------|---------|
| Thread 1 | 0 (低) | 最后执行完 |
| Thread 2 | 1 (中) | 在高优先级线程让出时执行 |
| Thread 3 | 2 (高) | 优先获得 CPU 时间 |

每个线程在循环中调用 `yield_thread()` 主动让出 CPU，让调度器有机会重新选择最高优先级的就绪线程。

### 运行结果

```
thread: 3 counter: 0 (priority=2)
thread: 2 counter: 0 (priority=1)
thread: 3 counter: 1 (priority=2)
thread: 2 counter: 1 (priority=1)
thread: 3 counter: 2 (priority=2)
thread: 2 counter: 2 (priority=1)
thread: 3 counter: 3 (priority=2)
thread: 2 counter: 3 (priority=1)
thread: 3 counter: 4 (priority=2)
thread: 2 counter: 4 (priority=1)
THREAD 3 FINISHED    // 高优先级线程完成
THREAD 2 FINISHED    // 中优先级线程完成
thread: 1 counter: 0 (priority=0)
...
THREAD 1 FINISHED    // 低优先级线程最后完成
```

结果验证了优先级机制的正确性：
- 高优先级（2）和中优先级（1）线程交替执行——高优先级线程每次 yield 后，调度器选中下一个最高优先级的就绪线程。
- 低优先级（0）线程在所有更高优先级线程完成后才得到执行。
- 优先级越高，获得 CPU 时间的机会越多。

## 局限性

1. 当前实现是**协作式**优先级，而非抢占式——线程必须主动调用 `yield_thread()` 才能让出 CPU。一个不 yield 的高优先级线程会无限占用 CPU。
2. 优先级是静态的，在 `spawn` 时确定后不可变更。
3. 未处理优先级反转问题（低优先级线程持有高优先级线程需要的资源）。
