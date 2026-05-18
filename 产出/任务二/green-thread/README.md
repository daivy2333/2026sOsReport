# Green threads in Rust

## 背景

[futures-archive]: https://web.archive.org/web/20230203001355/https://cfsamson.github.io/books-futures-explained/introduction.html
[green-thread-archive]: https://web.archive.org/web/20220527113808/https://cfsamson.gitbook.io/green-threads-explained-in-200-lines-of-rust/supporting-windows

## 本项目改动

### 兼容性修复

- `#[naked]` → `#[unsafe(naked)]`（Rust 1.88+）
- 移除 `#![feature(naked_functions)]`（Rust 1.88 已稳定）
- 修复 raw pointer 隐式引用转换（Rust 1.90 安全检查）
- 修复函数指针到整数的类型转换

### 优先级扩展

在 round-robin 调度的基础上，新增**最高优先级优先**调度策略：

```rust
pub fn spawn_with_priority(&mut self, f: fn(), priority: u8);
```

- `Thread` 结构体新增 `priority: u8` 和 `task: Option<Box<dyn Fn()>>` 字段
- 调度器从所有 Ready 线程中选择优先级最高的一个
- 详细文档见 `priority-extension.md`

### 动态跟踪

在调度器关键点插入带微秒级时间戳的 trace 日志，可完整观察每个绿色线程的 `Available → Ready → Running → Available` 状态变迁过程。实测 trace 分析见 [执行流状态变迁分析.md](../docs/执行流状态变迁分析.md)。

## 架构支持

| 架构 | 测试方式 | 工具链要求 |
|------|---------|-----------|
| x86-64 Linux | 直接运行 | stable Rust（1.88+） |
| x86-64 Windows | 直接运行 | stable Rust（1.88+） |
| RISC-V 64 Linux | QEMU 用户态模拟 | QEMU + riscv64 toolchain |

各架构的栈布局和寄存器上下文切换实现在对应平台文件中：
- `src/linux64.rs` — x86-64 Linux（System V AMD64 ABI）
- `src/win64.rs` — x86-64 Windows（Microsoft x64 ABI）
- `src/rv64.rs` — RISC-V 64（LP64 ABI）

`src/main.rs` 的 `os::init_stack` 调用由条件编译自动选择正确的平台实现，栈初始化、寄存器保存/恢复、任务入口和退出处理均架构自适应。

## 使用方式

该代码已在 stable Rust（1.90+）上测试通过，**不需要 nightly**。

### x86-64 Linux

```bash
cargo run
```

输出带时间戳的调度 trace：

```
[ 0.002] Spawning task on Thread 1 with priority 0
[ 0.030] Runtime started
[ 0.035] Thread 0 → Ready, selecting Thread 3 (prio=2)
[task] thread 3: step 0 (priority=2)
...
[ 0.075] All threads completed
```

总耗时约 50-80μs，验证优先级调度正确性。

### RISC-V 64（QEMU 模拟）

首先安装依赖：

```bash
# RISC-V 交叉编译工具链
sudo apt install gcc-riscv64-linux-gnu g++-riscv64-linux-gnu libc6-dev-riscv64-cross
# QEMU 用户态模拟
sudo apt install qemu-user-static
# Rust target
rustup target add riscv64gc-unknown-linux-gnu
```

运行：

```bash
cargo run --target riscv64gc-unknown-linux-gnu
```

输出与 x86-64 行为一致，但受 QEMU 模拟影响，总耗时约 4-5ms（比物理机慢 ~60 倍）。**QEMU 仅用于验证功能正确性，性能测试需在真实 RISC-V 硬件上进行。**

### Windows

```bash
cargo run
```

在 Windows 上编译时自动使用 `win64.rs` 中的线程上下文实现（包含 TIB 保存/恢复）。

### 遗留示例

`examples/` 目录下的两个文件是代码演进过程中的历史版本，**仅作参考，不保证与当前版本功能一致**：

- **`examples/updated.rs`** — 原作者 cfsamson 于 2022 年发布的更新版。修复了部分兼容性问题，但仍需 nightly 工具链（`#![feature(naked_functions)]`），使用旧的 `asm!` 宏语法。支持 Linux 和 Windows。
- **`examples/linux-only.rs`** — 训练营参与者基于更早期的代码修改的版本。将 `#[naked]` 替换为纯汇编写法，可在 stable Rust 上运行，但仅支持 Linux。栈大小改为 4KB 并增加了栈使用量统计。

当前 `src/main.rs` 是上述版本的集大成者：吸收了兼容性修复、统一了跨平台实现、新增了优先级调度和动态跟踪功能。
