# 底层探秘：Future 执行器与任务调度
https://beatai.org/rust-course/advance/async/future-excuting
## Future 特征

Future trait 是 Rust 异步编程的核心。异步函数的返回值即为实现了 Future 的类型，而 Future 的执行依赖于 poll 机制。

Future 特征的核心方法为 `poll`，执行器通过调用 `poll` 推进 Future 的执行。一次 poll 调用可能返回两种结果：

- `Poll::Ready(result)`：Future 已完成，产出结果。
- `Poll::Pending`：Future 尚未完成，此时需要注册一个 `wake` 函数，待 Future 可继续执行时由该函数通知执行器再次调用 `poll`。

这种"事件通知 -> 执行"的模型避免了执行器定期轮询所有 Future 的低效做法，实现了精确唤醒。

多个 Future 可以通过组合器（如 Join、AndThen）实现并发或顺序执行，整个过程无需内存分配，状态机由编译器自动生成。

## 真实的 Future 特征

标准库中的 Future trait 与简化版有两个关键区别：

1. **`self` 类型为 `Pin<&mut Self>` 而非 `&mut Self`**：`Pin` 保证 Future 在内存中不会被移动，从而允许自引用数据结构的存在。对于 `async`/`await` 生成的 Future 状态机而言，`Pin` 是不可或缺的。
2. **`wake: fn()` 替换为 `&mut Context<'_>`**：`Context` 中封装了 `Waker` 类型，可携带任务标识信息，使得执行器能够精确定位并唤醒特定任务。

## 使用 Waker 唤醒任务

当 Future 在首次 poll 时无法完成，它需要确保在准备好后能通知执行器再次对其 poll。这一通知机制通过 `Waker` 实现。`Waker` 提供 `wake()` 方法，调用后执行器将对相应的 Future 重新执行 poll。

## 执行器（Executor）

Rust 的 Future 是惰性的——仅在被 poll 时才会推进执行。执行器的职责是管理一批最外层 Future，通过持续的 poll 驱动它们直至完成。

执行器的基本工作流程如下：

1. 从任务通道中接收可执行的任务。
2. 对任务进行 poll。
3. 若任务返回 `Poll::Pending`，则等待该任务通过 `Waker` 重新将自己放入任务通道。
4. 重复上述过程，直至任务完成。

执行器通过 `ArcWake` 特征构建 `Waker`。当 `wake()` 被调用时，任务会复制自身（通过 `Arc` 克隆）并发送至任务通道，等待执行器再次 poll。

## 执行器与系统 IO

在实际场景中，Future 的 readiness 检测依赖操作系统提供的 IO 多路复用机制：

- Linux：epoll
- FreeBSD / macOS：kqueue
- Windows：IOCP
- Fuchsia：ports

Rust 可通过跨平台包 `mio` 统一使用这些机制。IO 多路复用允许单个线程同时阻塞等待多个异步 IO 事件，事件完成后立即返回并分发至对应的 `Waker`，进而触发执行器对相关 Future 的 poll。这一模型使得单线程执行器能够高效管理数千并发连接。

个人感觉，future有种激活的感觉，既然是未来要执行的东西那就静静的躺着等待信号激活，这样就变成一种静态的，高效激活指定块的设计。
然后就说waker，poll，这种有点半双工的感觉。