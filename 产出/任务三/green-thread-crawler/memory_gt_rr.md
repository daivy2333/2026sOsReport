# 协程内存消耗测量报告

**调度器**: green-thread | **模式**: roundrobin | **基准优先级**: 2

## 静态结构体大小

| 项 | 字节 |
|----|------|
| thread | 112 |
| ctx | 56 |
| state | 1 |
| priority_u8 | 1 |
| stack_alloc | 2097152 |
| per_coroutine_total | 2097264 |

## RSS 检查点

| 检查点 | RSS (KB) |
|--------|----------|
| before_runtime | 4916 |
| after_spawn | 5452 |
| during_yield | 5456 |
| after_exit | 5712 |

**峰值 RSS**: 5712 KB

## 结论

Thread 结构体 = 112 字节（含 priority_u8 = 1 字节），栈 = 2MB
峰值 RSS = 5712 KB
