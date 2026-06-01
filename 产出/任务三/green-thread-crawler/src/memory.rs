// Memory consumption measurement module.
// Reads RSS from /proc/self/statm (Linux only) and reports
// static struct sizes + dynamic memory usage during green-thread / tokio benchmarks.

#[cfg(target_os = "linux")]
use std::fs;

use crate::{yield_thread, Runtime, SchedulerMode, DEFAULT_STACK_SIZE};
use std::mem::size_of;

/// Read current RSS in KB from /proc/self/statm.
/// Returns pages × 4 (4KB per page), or None on error.
#[cfg(target_os = "linux")]
pub fn read_rss_kb() -> Option<u64> {
    let statm = fs::read_to_string("/proc/self/statm").ok()?;
    let fields: Vec<&str> = statm.split_whitespace().collect();
    if fields.len() < 2 {
        return None;
    }
    let pages: u64 = fields[1].parse().ok()?;
    Some(pages * 4)
}

#[cfg(not(target_os = "linux"))]
pub fn read_rss_kb() -> Option<u64> {
    None
}

/// Return a single-line parseable string with compile-time sizes.
pub fn static_sizes_report() -> String {
    let thread_size = size_of::<crate::Thread>();
    let ctx_size = size_of::<crate::os::ThreadContext>();
    let state_size = size_of::<crate::State>();
    let prio_u8 = size_of::<u8>();
    let stack_alloc = DEFAULT_STACK_SIZE as u64;
    let per_coroutine = thread_size as u64 + stack_alloc;

    format!(
        "STATIC_SIZES: thread={} ctx={} state={} priority_u8={} stack_alloc={} per_coroutine_total={}\n",
        thread_size, ctx_size, state_size, prio_u8, stack_alloc, per_coroutine
    )
}

/// Format an RSS reading with a label.
fn rss_line(label: &str) -> String {
    let kb = read_rss_kb().unwrap_or(0);
    format!("RSS: {}={}\n", label, kb)
}

// Minimal workload functions for green-thread memory benchmark.
// Each does arithmetic + cooperative yields so threads interleave.

fn mem_batch_high() {
    let mut s: u64 = 0;
    for i in 0..1000 {
        s = s.wrapping_add(i);
        if i % 200 == 0 {
            yield_thread();
        }
    }
    std::hint::black_box(s);
}

fn mem_batch_mid() {
    let mut s: u64 = 0;
    for i in 0..1000 {
        s = s.wrapping_add(i);
        if i % 200 == 0 {
            yield_thread();
        }
    }
    std::hint::black_box(s);
}

fn mem_batch_low() {
    let mut s: u64 = 0;
    for i in 0..1000 {
        s = s.wrapping_add(i);
        if i % 200 == 0 {
            yield_thread();
        }
    }
    std::hint::black_box(s);
}

fn mode_str(mode: &SchedulerMode) -> &'static str {
    match mode {
        SchedulerMode::HighestPriorityFirst => "priority",
        SchedulerMode::RoundRobin => "roundrobin",
    }
}

/// Run a complete memory benchmark. Returns STATIC_SIZES and RSS lines in output.
/// If report_path is non-empty, writes a .md report with data tables to that path.
pub fn run_memory_benchmark(scheduler: &str, mode: SchedulerMode, base_priority: u8, report_path: &str) -> String {
    let mut output = String::new();
    struct RssSample { label: String, kb: u64 }
    let mut rss_samples: Vec<RssSample> = Vec::new();

    fn sample_rss(label: &str, output: &mut String, samples: &mut Vec<RssSample>) {
        let kb = read_rss_kb().unwrap_or(0);
        samples.push(RssSample { label: label.to_string(), kb });
        output.push_str(&format!("RSS: {}={}\n", label, kb));
    }

    output.push_str(&static_sizes_report());

    if scheduler == "tokio" {
        // Tokio path — spawn 3 async tasks, measure peak RSS
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            sample_rss("before_spawn", &mut output, &mut rss_samples);

            let h1 = tokio::spawn(async {
                let mut s: u64 = 0;
                for i in 0..1000 {
                    s = s.wrapping_add(i);
                    if i % 200 == 0 {
                        tokio::task::yield_now().await;
                    }
                }
                std::hint::black_box(s);
            });
            let h2 = tokio::spawn(async {
                let mut s: u64 = 0;
                for i in 0..1000 {
                    s = s.wrapping_add(i);
                    if i % 200 == 0 {
                        tokio::task::yield_now().await;
                    }
                }
                std::hint::black_box(s);
            });
            let h3 = tokio::spawn(async {
                let mut s: u64 = 0;
                for i in 0..1000 {
                    s = s.wrapping_add(i);
                    if i % 200 == 0 {
                        tokio::task::yield_now().await;
                    }
                }
                std::hint::black_box(s);
            });

            sample_rss("after_spawn", &mut output, &mut rss_samples);

            let (r1, r2, r3) = tokio::join!(h1, h2, h3);
            r1.unwrap();
            r2.unwrap();
            r3.unwrap();

            sample_rss("after_exit", &mut output, &mut rss_samples);
        });
    } else {
        // Green-thread path
        let mut runtime = Runtime::new();
        runtime.init();
        runtime.set_scheduler_mode(mode);

        sample_rss("before_runtime", &mut output, &mut rss_samples);

        runtime.spawn_with_priority(mem_batch_high, base_priority);
        runtime.spawn_with_priority(mem_batch_mid, base_priority.saturating_sub(1));
        runtime.spawn_with_priority(mem_batch_low, base_priority.saturating_sub(2));

        sample_rss("after_spawn", &mut output, &mut rss_samples);

        // Yield a few times, sampling RSS each cycle. Take the maximum.
        let mut max_during: u64 = 0;
        for _ in 0..5 {
            yield_thread();
            if let Some(kb) = read_rss_kb() {
                max_during = max_during.max(kb);
            }
        }
        rss_samples.push(RssSample { label: "during_yield".into(), kb: max_during });
        output.push_str(&format!("RSS: during_yield={}\n", max_during));

        // Continue yielding to let remaining threads complete
        for _ in 0..30 {
            yield_thread();
        }

        sample_rss("after_exit", &mut output, &mut rss_samples);
    }

    // Write report file
    if !report_path.is_empty() {
        let mstr = mode_str(&mode);
        let mut report = String::new();
        report.push_str("# 协程内存消耗测量报告\n\n");
        report.push_str(&format!("**调度器**: {} | **模式**: {} | **基准优先级**: {}\n\n",
            scheduler, mstr, base_priority));

        // Static sizes table — parse from static_sizes_report()
        report.push_str("## 静态结构体大小\n\n");
        report.push_str("| 项 | 字节 |\n|----|------|\n");
        for line in output.lines() {
            if let Some(rest) = line.strip_prefix("STATIC_SIZES:") {
                for kv in rest.split_whitespace() {
                    if let Some((k, v)) = kv.split_once('=') {
                        if let Ok(n) = v.parse::<u64>() {
                            report.push_str(&format!("| {} | {} |\n", k, n));
                        }
                    }
                }
            }
        }

        // RSS checkpoints table
        report.push_str("\n## RSS 检查点\n\n");
        report.push_str("| 检查点 | RSS (KB) |\n|--------|----------|\n");
        let peak = rss_samples.iter().map(|s| s.kb).max().unwrap_or(0);
        for s in &rss_samples {
            report.push_str(&format!("| {} | {} |\n", s.label, s.kb));
        }
        report.push_str(&format!("\n**峰值 RSS**: {} KB\n", peak));

        // Conclusion
        let thread_size: u64 = output.lines()
            .find(|l| l.starts_with("STATIC_SIZES:"))
            .and_then(|l| l.split_whitespace().find(|kv| kv.starts_with("thread=")))
            .and_then(|kv| kv.split_once('='))
            .and_then(|(_, v)| v.parse().ok())
            .unwrap_or(0);
        report.push_str(&format!("\n## 结论\n\n"));
        report.push_str(&format!("Thread 结构体 = {} 字节（含 priority_u8 = 1 字节），栈 = 2MB\n", thread_size));
        report.push_str(&format!("峰值 RSS = {} KB\n", peak));

        let _ = std::fs::write(report_path, &report);
        output.push_str(&format!("报告已保存: {}\n", report_path));
    }

    output
}
