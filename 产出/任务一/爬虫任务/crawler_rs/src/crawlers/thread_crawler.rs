use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::Instant;

use crate::common::{compute_stats, process_task, CrawlStats};
use crate::schools::SCHOOLS;

pub fn run(cache_dir: &Path, cpu_repeat: usize, io_repeat: usize) -> CrawlStats {
    let start_total = Instant::now();
    let (tx, rx) = mpsc::channel();
    let mut handles = Vec::new();

    for school in SCHOOLS {
        let tx = tx.clone();
        let cache = cache_dir.to_path_buf();
        handles.push(thread::spawn(move || {
            let record = process_task(school.name, &cache, cpu_repeat, io_repeat);
            tx.send(record).ok();
        }));
    }

    drop(tx);

    for handle in handles {
        handle.join().ok();
    }

    let mut records = Vec::new();
    while let Ok(record) = rx.try_recv() {
        records.push(record);
    }

    let total_time = start_total.elapsed();
    compute_stats(&records, total_time)
}
