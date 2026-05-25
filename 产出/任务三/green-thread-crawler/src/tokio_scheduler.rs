use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::time::Instant;

use tokio::sync::Notify;

struct PriorityTask {
    priority: u8,
    school_name: String,
}

impl Eq for PriorityTask {}
impl PartialEq for PriorityTask {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority
    }
}
impl PartialOrd for PriorityTask {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for PriorityTask {
    fn cmp(&self, other: &Self) -> Ordering {
        self.priority.cmp(&other.priority)
    }
}

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
    let mut heap = BinaryHeap::new();
    for (i, school) in SCHOOLS.iter().enumerate() {
        let priority = if i < batch_size {
            base_priority
        } else if i < 2 * batch_size {
            base_priority.saturating_sub(1)
        } else {
            base_priority.saturating_sub(2)
        };
        heap.push(PriorityTask {
            priority,
            school_name: school.name.to_string(),
        });
    }

    let notify = Arc::new(Notify::new());
    let results = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let active = Arc::new(AtomicUsize::new(0));

    loop {
        if active.load(AtomicOrdering::Acquire) >= concurrency {
            notify.notified().await;
            continue;
        }

        match heap.pop() {
            Some(task) => {
                active.fetch_add(1, AtomicOrdering::Release);
                let cache = cache_dir.to_path_buf();
                let res = results.clone();
                let ntf = notify.clone();
                let act = active.clone();
                let cpu = cpu_repeat;
                let io = io_repeat;

                tokio::spawn(async move {
                    let (name, latency, len, success) =
                        process_task_sync(&task.school_name, &cache, cpu, io);
                    let prio_label = if task.priority == base_priority {
                        "2"
                    } else if task.priority == base_priority.saturating_sub(1) {
                        "1"
                    } else {
                        "0"
                    };
                    println!("[PRIO={}] {} done: {:.1}ms success={} len={}",
                        prio_label, name, latency, success, len);
                    res.lock().await.push((name, latency, len, success));
                    act.fetch_sub(1, AtomicOrdering::Release);
                    ntf.notify_one();
                });
            }
            None => {
                if active.load(AtomicOrdering::Acquire) == 0 {
                    break;
                }
                notify.notified().await;
            }
        }
    }

    Arc::try_unwrap(results).unwrap().into_inner()
}

use crate::schools::SCHOOLS;
