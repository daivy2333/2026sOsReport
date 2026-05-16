mod common;
mod crawlers;
mod schools;

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};

use crate::common::{generate_comparison_report, prefetch_all, print_comparison_header,
                    print_comparison_row, print_stats};
use crate::schools::SCHOOLS;

#[derive(Parser)]
#[command(name = "crawler", version, about = "三种爬虫实现对比: 进程/线程/协程")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    Process {
        #[arg(long, default_value_t = 0)]
        cpu_repeat: usize,
        #[arg(long, default_value_t = 0)]
        io_repeat: usize,
    },
    Thread {
        #[arg(long, default_value_t = 0)]
        cpu_repeat: usize,
        #[arg(long, default_value_t = 0)]
        io_repeat: usize,
    },
    Async {
        #[arg(long, default_value_t = 0)]
        cpu_repeat: usize,
        #[arg(long, default_value_t = 0)]
        io_repeat: usize,
        #[arg(long, default_value_t = 20)]
        concurrency: usize,
    },
    All {
        #[arg(long, default_value = "性能对比报告.md")]
        report: String,
    },
    Worker {
        #[arg(long)]
        name: String,
        #[arg(long)]
        cache_dir: String,
        #[arg(long, default_value_t = 0)]
        cpu_repeat: usize,
        #[arg(long, default_value_t = 0)]
        io_repeat: usize,
    },
}

fn worker_read(name: &str, cache_dir: &str, cpu_repeat: usize, io_repeat: usize) {
    use std::time::Instant;
    let start = Instant::now();
    let cache_path = PathBuf::from(cache_dir);

    let rss = common::current_rss_kb();
    let record = common::process_task(name, &cache_path, cpu_repeat, io_repeat);
    let elapsed = start.elapsed().as_secs_f64() * 1000.0;
    println!(
        "{}|{:.2}|{}|{}|{}",
        name, elapsed, record.content_len, record.success as u8, rss
    );
}

fn make_temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("crawler_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).ok();
    dir
}

fn cleanup(dir: &PathBuf) {
    let _ = std::fs::remove_dir_all(dir);
}

fn run_one_comparison(
    cache_dir: &Path,
    label_prefix: &str,
    cpu_repeat: usize,
    io_repeat: usize,
    header: &str,
) {
    println!("\n>>> {} CPU={} I/O={}", header, cpu_repeat, io_repeat);
    if cpu_repeat > 0 || io_repeat > 0 {
        println!("预计耗时请耐心等待...");
    }

    let stats_p = crawlers::process_crawler::run(cache_dir, cpu_repeat, io_repeat);
    print_comparison_row("进程", io_repeat.max(cpu_repeat), &stats_p);

    let before_t = common::current_rss_kb();
    let mut stats_t = crawlers::thread_crawler::run(cache_dir, cpu_repeat, io_repeat);
    stats_t.memory_kb = common::current_rss_kb().saturating_sub(before_t);
    print_comparison_row("线程", io_repeat.max(cpu_repeat), &stats_t);

    let before_a = common::current_rss_kb();
    let mut stats_a = crawlers::async_crawler::run(cache_dir, cpu_repeat, io_repeat, 20);
    stats_a.memory_kb = common::current_rss_kb().saturating_sub(before_a);
    print_comparison_row("协程", io_repeat.max(cpu_repeat), &stats_a);

    let fname = format!("性能对比_{}.md", label_prefix);
    let _ = generate_comparison_report(
        &format!("{}", label_prefix),
        &[("进程", &stats_p), ("线程", &stats_t), ("协程", &stats_a)],
        &PathBuf::from(&fname),
    );
}

fn run_all(_report_path: &str) {
    let cache_dir = make_temp("cache");
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(0);
    println!("CPU逻辑核心数: {}", cores);

    println!("\n第0步: 预取HTML缓存 (仅一次网络开销)...");
    let ok = prefetch_all(&cache_dir);
    println!("  预取完成: {}/{} 成功", ok, SCHOOLS.len());

    println!("\n========== CPU密集型负载对比 ==========");
    print_comparison_header();
    run_one_comparison(&cache_dir, "cpu_0",   0,     0, "CPU基线 (纯I/O)");
    run_one_comparison(&cache_dir, "cpu_1k",  1_000, 0, "CPU轻量 (checksum×1k)");
    run_one_comparison(&cache_dir, "cpu_10k", 10_000, 0, "CPU中量 (checksum×10k)");

    println!("\n========== I/O密集型负载对比 ==========");
    print_comparison_header();
    run_one_comparison(&cache_dir, "io_100",  0, 100,   "I/O轻量 (重复读×100)");
    run_one_comparison(&cache_dir, "io_1k",   0, 1_000, "I/O中量 (重复读×1k)");

    println!("\n综合报告已保存");

    let _ = cleanup(&cache_dir);
}

fn run_single(cmd: &Commands) {
    let cache_dir = make_temp("cache");

    println!("\n预取HTML缓存...");
    let ok = prefetch_all(&cache_dir);
    println!("  预取完成: {}/{} 成功", ok, SCHOOLS.len());

    let (stats, label, cpu_repeat, io_repeat) = match cmd {
        Commands::Process { cpu_repeat, io_repeat } => {
            println!("\n运行进程爬虫... (CPU={}, I/O={})", cpu_repeat, io_repeat);
            (
                crawlers::process_crawler::run(&cache_dir, *cpu_repeat, *io_repeat),
                "进程爬虫 (Process)",
                *cpu_repeat,
                *io_repeat,
            )
        }
        Commands::Thread { cpu_repeat, io_repeat } => {
            println!("\n运行线程爬虫... (CPU={}, I/O={})", cpu_repeat, io_repeat);
            (
                crawlers::thread_crawler::run(&cache_dir, *cpu_repeat, *io_repeat),
                "线程爬虫 (Thread)",
                *cpu_repeat,
                *io_repeat,
            )
        }
        Commands::Async { cpu_repeat, io_repeat, concurrency } => {
            println!("\n运行协程爬虫... (CPU={}, I/O={}, 并发={})", cpu_repeat, io_repeat, concurrency);
            (
                crawlers::async_crawler::run(&cache_dir, *cpu_repeat, *io_repeat, *concurrency),
                "协程爬虫 (Async)",
                *cpu_repeat,
                *io_repeat,
            )
        }
        _ => unreachable!(),
    };

    print_stats(label, io_repeat.max(cpu_repeat), &stats);
    cleanup(&cache_dir);
}

fn main() {
    let cli = Cli::parse();
    let command = cli.command.unwrap_or(Commands::All {
        report: "性能对比报告.md".to_string(),
    });

    match &command {
        Commands::Process { .. }
        | Commands::Thread { .. }
        | Commands::Async { .. } => {
            run_single(&command);
        }
        Commands::All { .. } => {
            run_all("性能对比报告.md");
        }
        Commands::Worker { name, cache_dir, cpu_repeat, io_repeat } => {
            worker_read(name, cache_dir, *cpu_repeat, *io_repeat);
        }
    }
}
