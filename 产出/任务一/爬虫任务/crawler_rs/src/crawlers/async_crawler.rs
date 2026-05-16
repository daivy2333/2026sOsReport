use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::Semaphore;

use crate::common::{compute_stats, process_task, CrawlRecord, CrawlStats};
use crate::schools::{SchoolInfo, SCHOOLS};

async fn crawl_one(
    school: &SchoolInfo,
    cache_dir: &Path,
    cpu_repeat: usize,
    io_repeat: usize,
    semaphore: Arc<Semaphore>,
) -> CrawlRecord {
    let _permit = semaphore.acquire().await.unwrap();
    process_task(school.name, cache_dir, cpu_repeat, io_repeat)
}

pub async fn run_async(
    cache_dir: &Path,
    cpu_repeat: usize,
    io_repeat: usize,
    concurrency: usize,
) -> CrawlStats {
    let start_total = Instant::now();
    let semaphore = Arc::new(Semaphore::new(concurrency));
    let mut handles = Vec::new();

    for school in SCHOOLS {
        let cache = cache_dir.to_path_buf();
        let sem = semaphore.clone();
        handles.push(tokio::spawn(async move {
            crawl_one(school, &cache, cpu_repeat, io_repeat, sem).await
        }));
    }

    let mut records = Vec::new();
    for handle in handles {
        if let Ok(record) = handle.await {
            records.push(record);
        }
    }

    let total_time = start_total.elapsed();
    compute_stats(&records, total_time)
}

pub fn run(cache_dir: &Path, cpu_repeat: usize, io_repeat: usize, concurrency: usize) -> CrawlStats {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(run_async(cache_dir, cpu_repeat, io_repeat, concurrency))
}

#[allow(dead_code)]
pub fn concurrency_sweep(cache_dir: &Path, cpu_repeat: usize, io_repeat: usize) -> Vec<(usize, CrawlStats)> {
    let levels = [1, 2, 4, 8, 16, 32, 64, 128];
    let mut results: Vec<(usize, CrawlStats)> = Vec::new();

    for &c in &levels {
        print!("  并发={}: ", c);
        let stats = run(cache_dir, cpu_repeat, io_repeat, c);
        println!(
            "{:.2} ms, {:.2} req/s",
            stats.total_time_ms, stats.throughput_per_sec
        );
        results.push((c, stats));
    }

    results.sort_by(|a, b| {
        b.1.throughput_per_sec
            .partial_cmp(&a.1.throughput_per_sec)
            .unwrap()
    });

    results
}
