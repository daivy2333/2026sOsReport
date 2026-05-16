use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::schools::SCHOOLS;

#[allow(dead_code)]
pub struct CrawlRecord {
    pub school_name: String,
    pub latency_ms: f64,
    pub content_len: usize,
    pub success: bool,
}

pub struct CrawlStats {
    pub total_time_ms: f64,
    pub total_requests: usize,
    pub successful_requests: usize,
    pub latencies_ms: Vec<f64>,
    pub throughput_per_sec: f64,
    pub memory_kb: usize,
}

pub fn current_rss_kb() -> usize {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines().find_map(|line| {
                if line.starts_with("VmRSS:") {
                    line.split_whitespace()
                        .nth(1)
                        .and_then(|v| v.parse().ok())
                } else {
                    None
                }
            })
        })
        .unwrap_or(0)
}

pub fn workload_checksum(data: &[u8], repeat: usize) -> u64 {
    let mut sum: u64 = 0;
    for _ in 0..repeat {
        for &b in data {
            sum = sum.wrapping_add(b as u64);
            sum = sum.rotate_left(3);
        }
    }
    std::hint::black_box(sum)
}

pub fn process_task(
    school_name: &str,
    cache_dir: &Path,
    cpu_repeat: usize,
    io_repeat: usize,
) -> CrawlRecord {
    let start = Instant::now();
    let path = cache_dir.join(format!("{}.html", school_name));

    // First read (always happens)
    let bytes = match fs::read(&path) {
        Ok(b) => b,
        Err(_) => {
            return CrawlRecord {
                school_name: school_name.to_string(),
                latency_ms: start.elapsed().as_secs_f64() * 1000.0,
                content_len: 0,
                success: false,
            };
        }
    };

    // I/O repeats: re-read the file N times (each is a syscall)
    for _ in 0..io_repeat {
        let _ = fs::read(&path);
    }

    // CPU repeats: compute checksum N times on the initial data
    if cpu_repeat > 0 {
        workload_checksum(&bytes, cpu_repeat);
    }

    CrawlRecord {
        school_name: school_name.to_string(),
        latency_ms: start.elapsed().as_secs_f64() * 1000.0,
        content_len: bytes.len(),
        success: true,
    }
}

pub fn prefetch_all(cache_dir: &Path) -> usize {
    fs::create_dir_all(cache_dir).expect("failed to create cache dir");
    let mut success_count = 0;

    for school in SCHOOLS {
        let path = cache_dir.join(format!("{}.html", school.name));
        if path.exists() {
            success_count += 1;
            continue;
        }
        if let Ok(resp) = ureq::get(school.url)
            .timeout(Duration::from_secs(30))
            .call()
        {
            if let Ok(body) = resp.into_string() {
                fs::write(&path, &body).ok();
                success_count += 1;
            }
        }
    }

    success_count
}

pub fn compute_stats(records: &[CrawlRecord], total_time: Duration) -> CrawlStats {
    let successful: Vec<f64> = records
        .iter()
        .filter(|r| r.success)
        .map(|r| r.latency_ms)
        .collect();

    CrawlStats {
        total_time_ms: total_time.as_secs_f64() * 1000.0,
        total_requests: records.len(),
        successful_requests: successful.len(),
        latencies_ms: successful,
        throughput_per_sec: if total_time.as_secs_f64() > 0.0 {
            records.len() as f64 / total_time.as_secs_f64()
        } else {
            0.0
        },
        memory_kb: 0,
    }
}

pub fn print_stats(label: &str, repeat: usize, stats: &CrawlStats) {
    println!("\n======= {} (repeat={}) =======", label, repeat);
    println!("  总请求数:      {}", stats.total_requests);
    println!("  成功请求数:    {}", stats.successful_requests);
    println!("  总耗时:        {:.2} ms", stats.total_time_ms);
    println!("  吞吐率:        {:.2} 请求/秒", stats.throughput_per_sec);
    if stats.memory_kb > 0 {
        println!("  内存开销:      {} KB ({:.1} MB)", stats.memory_kb, stats.memory_kb as f64 / 1024.0);
    }

    if !stats.latencies_ms.is_empty() {
        let mut sorted = stats.latencies_ms.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let sum: f64 = sorted.iter().sum();
        let avg = sum / sorted.len() as f64;
        let min = sorted.first().unwrap();
        let max = sorted.last().unwrap();
        let med = sorted[sorted.len() / 2];
        let p95_idx = ((sorted.len() as f64) * 0.95).ceil() as usize - 1;
        let p95 = sorted[p95_idx.min(sorted.len() - 1)];

        println!("  延迟分布 (ms):");
        println!("    最小值:     {:.2}", min);
        println!("    平均值:     {:.2}", avg);
        println!("    中位数:     {:.2}", med);
        println!("    P95:        {:.2}", p95);
        println!("    最大值:     {:.2}", max);
    }
}

fn sorted_latencies(stats: &CrawlStats) -> Vec<f64> {
    let mut s = stats.latencies_ms.clone();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    s
}

fn compute_avg(stats: &CrawlStats) -> f64 {
    if stats.latencies_ms.is_empty() {
        return 0.0;
    }
    stats.latencies_ms.iter().sum::<f64>() / stats.latencies_ms.len() as f64
}

fn compute_med(stats: &CrawlStats) -> f64 {
    let s = sorted_latencies(stats);
    if s.is_empty() {
        return 0.0;
    }
    s[s.len() / 2]
}

fn compute_p95(stats: &CrawlStats) -> f64 {
    let s = sorted_latencies(stats);
    if s.is_empty() {
        return 0.0;
    }
    let idx = ((s.len() as f64) * 0.95).ceil() as usize - 1;
    s[idx.min(s.len() - 1)]
}

fn compute_min(stats: &CrawlStats) -> f64 {
    stats.latencies_ms.iter().cloned().fold(f64::MAX, f64::min)
}

fn compute_max(stats: &CrawlStats) -> f64 {
    stats.latencies_ms.iter().cloned().fold(f64::MIN, f64::max)
}

pub fn generate_comparison_report(
    label: &str,
    stats_list: &[(&str, &CrawlStats)],
    report_path: &Path,
) -> std::io::Result<()> {
    let mut output = String::new();
    output.push_str(&format!("# {} 性能对比\n\n", label));

    for (name, stats) in stats_list {
        let avg = compute_avg(stats);
        let med = compute_med(stats);
        let p95 = compute_p95(stats);
        let min = compute_min(stats);
        let max_val = compute_max(stats);
        let rate = if stats.total_requests > 0 {
            stats.successful_requests as f64 / stats.total_requests as f64 * 100.0
        } else {
            0.0
        };

        output.push_str(&format!("## {}\n\n", name));
        output.push_str("| 指标 | 值 |\n|------|-----|\n");
        output.push_str(&format!("| 总请求数 | {} |\n", stats.total_requests));
        output.push_str(&format!("| 成功请求数 | {} |\n", stats.successful_requests));
        output.push_str(&format!("| 总耗时 | {:.2} ms |\n", stats.total_time_ms));
        output.push_str(&format!("| 吞吐率 | {:.2} 请求/秒 |\n", stats.throughput_per_sec));
        output.push_str(&format!("| 成功率 | {:.1}% |\n", rate));
        output.push_str(&format!("| 延迟最小值 | {:.2} ms |\n", min));
        output.push_str(&format!("| 延迟平均值 | {:.2} ms |\n", avg));
        output.push_str(&format!("| 延迟中位数 | {:.2} ms |\n", med));
        output.push_str(&format!("| 延迟 P95 | {:.2} ms |\n", p95));
        output.push_str(&format!("| 延迟最大值 | {:.2} ms |\n", max_val));
        if stats.memory_kb > 0 {
            output.push_str(&format!(
                "| 内存开销 | {} KB ({:.1} MB) |\n",
                stats.memory_kb,
                stats.memory_kb as f64 / 1024.0
            ));
        }
        output.push_str("\n");
    }

    output.push_str("## 横向对比\n\n");
    output.push_str("| 方式 | 总耗时(ms) | 吞吐率(req/s) | 平均延迟(ms) | 中位数(ms) | P95(ms) | 成功率 | 内存(KB) |\n");
    output.push_str("|------|-----------|--------------|-------------|-----------|---------|-------|---------|\n");
    for (name, stats) in stats_list {
        let avg = compute_avg(stats);
        let med = compute_med(stats);
        let p95 = compute_p95(stats);
        let rate = if stats.total_requests > 0 {
            stats.successful_requests as f64 / stats.total_requests as f64 * 100.0
        } else {
            0.0
        };
        let mem = if stats.memory_kb > 0 {
            format!("{}", stats.memory_kb)
        } else {
            "-".to_string()
        };
        output.push_str(&format!(
            "| {} | {:.2} | {:.2} | {:.2} | {:.2} | {:.2} | {:.1}% | {} |\n",
            name, stats.total_time_ms, stats.throughput_per_sec, avg, med, p95, rate, mem
        ));
    }

    fs::write(report_path, output)
}

pub fn print_comparison_header() {
    println!();
    println!(
        "{:<6} | {:<8} | {:>10} | {:>12} | {:>10} | {:>10} | {:>10}",
        "repeat", "方式", "总耗时(ms)", "吞吐率(req/s)", "平均延迟(ms)", "中位数(ms)", "内存(KB)"
    );
    println!(
        "{:-<6}-+-{:-<8}-+-{:-<10}-+-{:-<12}-+-{:-<10}-+-{:-<10}-+-{:-<10}",
        "", "", "", "", "", "", ""
    );
}

pub fn print_comparison_row(label: &str, repeat: usize, stats: &CrawlStats) {
    let avg = if !stats.latencies_ms.is_empty() {
        stats.latencies_ms.iter().sum::<f64>() / stats.latencies_ms.len() as f64
    } else {
        0.0
    };
    let mut sorted = stats.latencies_ms.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let med = sorted.get(sorted.len() / 2).copied().unwrap_or(0.0);
    let mem = if stats.memory_kb > 0 {
        format!("{}", stats.memory_kb)
    } else {
        "-".to_string()
    };
    println!(
        "{:<6} | {:<8} | {:>10.2} | {:>12.2} | {:>10.3} | {:>10.3} | {:>10}",
        repeat, label, stats.total_time_ms, stats.throughput_per_sec, avg, med, mem,
    );
}
