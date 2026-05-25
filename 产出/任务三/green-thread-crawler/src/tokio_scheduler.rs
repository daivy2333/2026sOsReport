use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::Semaphore;

fn process_task_sync(
    school_name: &str,
    cache_dir: &Path,
    cpu_repeat: usize,
    io_repeat: usize,
) -> (String, f64, usize, bool) {
    let start = Instant::now();
    let path = cache_dir.join(format!("{}.html", school_name));

    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(_) => {
            return (school_name.to_string(), start.elapsed().as_secs_f64() * 1000.0, 0, false);
        }
    };

    for _ in 0..io_repeat {
        let _ = std::fs::read(&path);
    }

    if cpu_repeat > 0 {
        let mut sum: u64 = 0;
        for _outer in 0..cpu_repeat {
            for &b in &bytes {
                sum = sum.wrapping_add(b as u64);
                sum = sum.rotate_left(3);
            }
        }
        std::hint::black_box(sum);
    }

    let elapsed = start.elapsed().as_secs_f64() * 1000.0;
    (school_name.to_string(), elapsed, bytes.len(), true)
}

pub fn run_tokio(
    cache_dir: &Path,
    cpu_repeat: usize,
    io_repeat: usize,
    concurrency: usize,
    num_batches: usize,
    base_priority: u8,
) -> Vec<(String, f64, usize, bool)> {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(run_tokio_async(
        cache_dir, cpu_repeat, io_repeat,
        concurrency, num_batches, base_priority,
    ))
}

async fn run_tokio_async(
    cache_dir: &Path,
    cpu_repeat: usize,
    io_repeat: usize,
    concurrency: usize,
    num_batches: usize,
    base_priority: u8,
) -> Vec<(String, f64, usize, bool)> {
    let batch_size = SCHOOLS.len() / num_batches;

    let mut task_queues: [Vec<String>; 3] = [const { Vec::new() }, const { Vec::new() }, const { Vec::new() }];

    for (i, school) in SCHOOLS.iter().enumerate() {
        let raw_prio = if i < batch_size {
            base_priority
        } else if i < 2 * batch_size {
            base_priority.saturating_sub(1)
        } else {
            base_priority.saturating_sub(2)
        };
        task_queues[raw_prio as usize].push(school.name.to_string());
    }

    let semaphore = Arc::new(Semaphore::new(concurrency));
    let results = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let mut handles = Vec::new();

    for prio_level in (0..=2).rev() {
        for school_name in task_queues[prio_level].drain(..) {
            let permit = semaphore.clone().acquire_owned().await.unwrap();
            let cache = cache_dir.to_path_buf();
            let res = results.clone();
            let cpu = cpu_repeat;
            let io = io_repeat;

            let handle = tokio::spawn(async move {
                let _permit = permit;

                let (name, latency, len, success) =
                    process_task_sync(&school_name, &cache, cpu, io);

                let prio_label = match prio_level {
                    2 => "2",
                    1 => "1",
                    _ => "0",
                };
                println!("[PRIO={}] {} done: {:.1}ms success={} len={}",
                    prio_label, name, latency, success, len);

                res.lock().await.push((name, latency, len, success));
            });
            handles.push(handle);
        }
    }

    for handle in handles {
        handle.await.unwrap();
    }

    Arc::try_unwrap(results).unwrap().into_inner()
}

use crate::schools::SCHOOLS;
