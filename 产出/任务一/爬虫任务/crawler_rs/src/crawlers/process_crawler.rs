use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;

use crate::common::{CrawlRecord, CrawlStats, compute_stats};
use crate::schools::SCHOOLS;

pub fn run(cache_dir: &Path, cpu_repeat: usize, io_repeat: usize) -> CrawlStats {
    let start_total = Instant::now();
    let mut records: Vec<CrawlRecord> = Vec::new();
    let mut children = Vec::new();
    let mut total_rss: usize = 0;
    let cache_str = cache_dir.to_string_lossy().to_string();

    for school in SCHOOLS {
        let exe = std::env::current_exe().expect("failed to get current exe path");
        let c = cache_str.clone();

        let child = Command::new(exe)
            .arg("worker")
            .arg("--name")
            .arg(school.name)
            .arg("--cache-dir")
            .arg(&c)
            .arg("--cpu-repeat")
            .arg(cpu_repeat.to_string())
            .arg("--io-repeat")
            .arg(io_repeat.to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to spawn worker process");

        children.push(child);
    }

    for mut child in children {
        let stdout = child.stdout.take();
        let _ = child.wait().ok();
        if let Some(stdout) = stdout {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                if let Ok(line) = line {
                    let parts: Vec<&str> = line.splitn(5, '|').collect();
                    if parts.len() == 5 {
                        records.push(CrawlRecord {
                            school_name: parts[0].to_string(),
                            latency_ms: parts[1].parse().unwrap_or(0.0),
                            content_len: parts[2].parse().unwrap_or(0),
                            success: parts[3] == "1",
                        });
                        total_rss += parts[4].parse::<usize>().unwrap_or(0);
                    } else if parts.len() == 4 {
                        records.push(CrawlRecord {
                            school_name: parts[0].to_string(),
                            latency_ms: parts[1].parse().unwrap_or(0.0),
                            content_len: parts[2].parse().unwrap_or(0),
                            success: parts[3] == "1",
                        });
                    }
                }
            }
        }
    }

    let total_time = start_total.elapsed();
    let mut stats = compute_stats(&records, total_time);
    stats.memory_kb = total_rss;
    stats
}
