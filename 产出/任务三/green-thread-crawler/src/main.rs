use std::arch::naked_asm;
use std::time::Instant;

mod schools;
mod tokio_scheduler;

#[cfg_attr(target_os = "windows", path = "win64.rs")]
#[cfg_attr(all(target_os = "linux", not(target_arch = "riscv64")), path = "linux64.rs")]
#[cfg_attr(any(target_arch = "riscv64"), path = "rv64.rs")]
mod os;
use os::ThreadContext;

#[derive(Debug, Clone)]
struct CrawlRecord {
    school_name: String,
    latency_ms: f64,
    content_len: usize,
    success: bool,
}

const DEFAULT_STACK_SIZE: usize = 1024 * 1024 * 2;
const MAX_THREADS: usize = 10;
static mut RUNTIME: usize = 0;
static mut G_SCHOOLS_NAMES: [&str; 33] = [""; 33];
static mut G_CACHE_DIR: Option<String> = None;
static mut G_CPU_REPEAT: usize = 0;
static mut G_IO_REPEAT: usize = 0;
static mut G_BATCH_RESULTS: Vec<CrawlRecord> = Vec::new();
static mut T0: Option<Instant> = None;
static mut G_NO_YIELD: bool = false;
static mut G_NUM_BATCHES: usize = 3;
static mut G_MODE_NAME: Option<String> = None;
static mut G_REPORT_PATH: Option<String> = None;

fn trace(msg: &str) {
    let t = unsafe { T0.unwrap() };
    let elapsed = t.elapsed();
    println!("[{:>6.3?}] {}", elapsed.as_secs_f64() * 1000.0, msg);
}

fn state_name(s: &State) -> &'static str {
    match s {
        State::Available => "Available",
        State::Ready => "Ready",
        State::Running => "Running",
    }
}

#[derive(PartialEq, Eq, Debug)]
enum State {
    Available,
    Running,
    Ready,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SchedulerMode {
    RoundRobin,
    HighestPriorityFirst,
}

#[allow(dead_code)]
struct Thread {
    id: usize,
    stack: Vec<u8>,
    ctx: ThreadContext,
    state: State,
    priority: u8,
    task: Option<Box<dyn Fn()>>,
}

impl Thread {
    fn new(id: usize) -> Self {
        Thread {
            id,
            stack: vec![0_u8; DEFAULT_STACK_SIZE],
            ctx: ThreadContext::default(),
            state: State::Available,
            priority: 0,
            task: None,
        }
    }
}

pub struct Runtime {
    threads: Vec<Thread>,
    current: usize,
    mode: SchedulerMode,
}

impl Runtime {
    pub fn new() -> Self {
        let base_thread = Thread {
            id: 0,
            stack: vec![0_u8; DEFAULT_STACK_SIZE],
            ctx: ThreadContext::default(),
            state: State::Running,
            priority: 0,
            task: None,
        };

        let mut threads = vec![base_thread];
        let mut available_threads: Vec<Thread> = (1..MAX_THREADS).map(Thread::new).collect();
        threads.append(&mut available_threads);

        Runtime {
            threads,
            current: 0,
            mode: SchedulerMode::HighestPriorityFirst,
        }
    }

    pub fn set_scheduler_mode(&mut self, mode: SchedulerMode) {
        self.mode = mode;
    }

    pub fn init(&self) {
        unsafe {
            let r_ptr: *const Runtime = self;
            RUNTIME = r_ptr as usize;
            T0 = Some(Instant::now());
        }
    }

    pub fn run(&mut self) -> ! {
        trace("Runtime started");
        while self.t_yield() {}
        trace("All threads completed");

        // Print experiment results
        unsafe {
            if !G_BATCH_RESULTS.is_empty() {
                println!("\n=== RESULTS ===");
                for (i, r) in G_BATCH_RESULTS.iter().enumerate() {
                    println!("{}|{}|{:.1}|{}|{}",
                        i, r.school_name, r.latency_ms, r.content_len, r.success as u8);
                }
                println!("=== END RESULTS ===");
            }
        }

        generate_report();

        std::process::exit(0);
    }

    fn t_return(&mut self) {
        let id = self.current;
        trace(&format!(
            "Thread {} state: Running → Available (task completed)",
            id
        ));
        if self.current != 0 {
            self.threads[self.current].state = State::Available;
            self.t_yield();
        }
    }

    /// Find the highest-priority Ready thread.
    fn find_highest_priority_ready(&self) -> Option<usize> {
        let mut best_idx = None;
        let mut best_prio: u8 = 0;
        for (i, t) in self.threads.iter().enumerate() {
            if t.state == State::Ready && (best_idx.is_none() || t.priority > best_prio) {
                best_idx = Some(i);
                best_prio = t.priority;
            }
        }
        best_idx
    }

    fn find_next_round_robin(&self) -> Option<usize> {
        let n = self.threads.len();
        let start = (self.current + 1) % n;
        for offset in 0..n {
            let idx = (start + offset) % n;
            if self.threads[idx].state == State::Ready {
                return Some(idx);
            }
        }
        None
    }

    fn t_yield(&mut self) -> bool {
        let next = match self.mode {
            SchedulerMode::HighestPriorityFirst => self.find_highest_priority_ready(),
            SchedulerMode::RoundRobin => self.find_next_round_robin(),
        };
        if let Some(next) = next {
            let old_state = &self.threads[self.current].state;
            if *old_state != State::Available {
                let old_name = state_name(old_state);
                self.threads[self.current].state = State::Ready;
                trace(&format!(
                    "Thread {} state: {} → Ready (yield), selecting Thread {} (Ready → Running, prio={})",
                    self.current, old_name, next, self.threads[next].priority
                ));
            }
            self.threads[next].state = State::Running;
            let old_pos = self.current;
            self.current = next;
            unsafe {
                let old: *mut ThreadContext = &mut self.threads[old_pos].ctx;
                let new: *const ThreadContext = &self.threads[next].ctx;
                os::switch(old, new);
            }
            true
        } else {
            let states: Vec<String> = self
                .threads
                .iter()
                .map(|t| format!("{}:{}", t.id, state_name(&t.state)))
                .collect();
            trace(&format!("No ready threads left {:?}", states));
            false
        }
    }

    fn spawn_inner(&mut self, f: fn(), priority: u8) {
        let available = self
            .threads
            .iter_mut()
            .find(|t| t.state == State::Available)
            .expect("no available thread.");

        let id = available.id;
        available.task = Some(Box::new(f));
        available.priority = priority;
        trace(&format!(
            "Spawning task on Thread {} with priority {} (state: Available → Ready)",
            id, priority
        ));
        unsafe {
            os::init_stack(
                &mut available.stack,
                &mut available.ctx,
                f as usize,
                guard as usize,
                skip as usize,
            );
        }
        available.state = State::Ready;
    }

    pub fn spawn(&mut self, f: fn()) {
        self.spawn_inner(f, 0);
    }

    pub fn spawn_with_priority(&mut self, f: fn(), priority: u8) {
        self.spawn_inner(f, priority);
    }
}

#[unsafe(naked)]
unsafe extern "C" fn skip() {
    naked_asm!("ret")
}

fn guard() {
    unsafe {
        let rt_ptr = RUNTIME as *mut Runtime;
        let id = (&(*rt_ptr).threads)[(*rt_ptr).current].id;
        trace(&format!("Guard called for Thread {} (entering t_return)", id));
        (*rt_ptr).t_return();
    };
}

pub fn yield_thread() {
    unsafe {
        let rt_ptr = RUNTIME as *mut Runtime;
        (*rt_ptr).t_yield();
    };
}

fn workload_checksum_yielding(data: &[u8], repeat: usize) -> u64 {
    let mut sum: u64 = 0;
    for outer in 0..repeat {
        if outer > 0 && outer % 100 == 0 {
            if unsafe { !G_NO_YIELD } {
                yield_thread();
            }
        }
        for &b in data {
            sum = sum.wrapping_add(b as u64);
            sum = sum.rotate_left(3);
        }
    }
    std::hint::black_box(sum)
}

fn process_task_yielding(school_name: &str, cache_dir: &std::path::Path, cpu_repeat: usize, io_repeat: usize) -> CrawlRecord {
    let start = std::time::Instant::now();
    let path = cache_dir.join(format!("{}.html", school_name));

    let bytes = match std::fs::read(&path) {
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

    for _ in 0..io_repeat {
        let _ = std::fs::read(&path);
    }

    if cpu_repeat > 0 {
        workload_checksum_yielding(&bytes, cpu_repeat);
    }

    CrawlRecord {
        school_name: school_name.to_string(),
        latency_ms: start.elapsed().as_secs_f64() * 1000.0,
        content_len: bytes.len(),
        success: true,
    }
}

fn sorted_latencies(records: &[CrawlRecord]) -> Vec<f64> {
    let mut s: Vec<f64> = records.iter().filter(|r| r.success).map(|r| r.latency_ms).collect();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    s
}

fn compute_stats(records: &[CrawlRecord]) -> (f64, f64, f64, f64, f64) {
    let s = sorted_latencies(records);
    if s.is_empty() {
        return (0.0, 0.0, 0.0, 0.0, 0.0);
    }
    let sum: f64 = s.iter().sum();
    let avg = sum / s.len() as f64;
    let min = s[0];
    let max = s[s.len() - 1];
    let med = s[s.len() / 2];
    let p95_idx = ((s.len() as f64) * 0.95).ceil() as usize - 1;
    let p95 = s[p95_idx.min(s.len() - 1)];
    (avg, med, p95, min, max)
}

fn prefetch_all(cache_dir: &std::path::Path) -> usize {
    std::fs::create_dir_all(cache_dir).expect("failed to create cache dir");
    let mut success_count = 0;

    for school in crate::schools::SCHOOLS {
        let path = cache_dir.join(format!("{}.html", school.name));
        if path.exists() {
            success_count += 1;
            continue;
        }
        match ureq::get(school.url)
            .timeout(std::time::Duration::from_secs(30))
            .call()
        {
            Ok(resp) => {
                if let Ok(body) = resp.into_string() {
                    let _ = std::fs::write(&path, &body);
                    success_count += 1;
                }
            }
            Err(_) => {
                eprintln!("  WARN: failed to fetch {}", school.name);
            }
        }
    }
    success_count
}

fn crawl_batch_high() {
    unsafe {
        let cache_dir = std::path::Path::new(G_CACHE_DIR.as_ref().unwrap());
        let batch_size = 33 / G_NUM_BATCHES;
        for i in 0..batch_size {
            let name = G_SCHOOLS_NAMES[i];
            if name.is_empty() { continue; }
            let record = process_task_yielding(name, cache_dir, G_CPU_REPEAT, G_IO_REPEAT);
            println!("[PRIO=2] {} done: {:.1}ms success={} len={}",
                name, record.latency_ms, record.success, record.content_len);
            G_BATCH_RESULTS.push(record);
        }
    }
}

fn crawl_batch_mid() {
    unsafe {
        let cache_dir = std::path::Path::new(G_CACHE_DIR.as_ref().unwrap());
        let batch_size = 33 / G_NUM_BATCHES;
        for i in batch_size..(2*batch_size) {
            let name = G_SCHOOLS_NAMES[i];
            if name.is_empty() { continue; }
            let record = process_task_yielding(name, cache_dir, G_CPU_REPEAT, G_IO_REPEAT);
            println!("[PRIO=1] {} done: {:.1}ms success={} len={}",
                name, record.latency_ms, record.success, record.content_len);
            G_BATCH_RESULTS.push(record);
        }
    }
}

fn crawl_batch_low() {
    unsafe {
        let cache_dir = std::path::Path::new(G_CACHE_DIR.as_ref().unwrap());
        let batch_size = 33 / G_NUM_BATCHES;
        for i in (2*batch_size)..33 {
            let name = G_SCHOOLS_NAMES[i];
            if name.is_empty() { continue; }
            let record = process_task_yielding(name, cache_dir, G_CPU_REPEAT, G_IO_REPEAT);
            println!("[PRIO=0] {} done: {:.1}ms success={} len={}",
                name, record.latency_ms, record.success, record.content_len);
            G_BATCH_RESULTS.push(record);
        }
    }
}

fn generate_report() {
    unsafe {
        let path_str = match &G_REPORT_PATH {
            Some(p) => p.clone(),
            None => return,
        };

        let records: Vec<&CrawlRecord> = G_BATCH_RESULTS.iter().collect();
        let num_batches = G_NUM_BATCHES;
        let batch_size = 33 / num_batches;

        let mut high: Vec<&CrawlRecord> = Vec::new();
        let mut mid: Vec<&CrawlRecord> = Vec::new();
        let mut low: Vec<&CrawlRecord> = Vec::new();

        for r in &records {
            let idx = G_SCHOOLS_NAMES.iter().position(|&n| n == r.school_name.as_str());
            if let Some(idx) = idx {
                if idx < batch_size {
                    high.push(r);
                } else if idx < 2 * batch_size {
                    mid.push(r);
                } else {
                    low.push(r);
                }
            }
        }

        let (h_avg, h_med, h_p95, h_min, h_max) = compute_stats(&high.iter().map(|r| (*r).clone()).collect::<Vec<_>>());
        let (m_avg, m_med, m_p95, m_min, m_max) = compute_stats(&mid.iter().map(|r| (*r).clone()).collect::<Vec<_>>());
        let (l_avg, l_med, l_p95, l_min, l_max) = compute_stats(&low.iter().map(|r| (*r).clone()).collect::<Vec<_>>());

        let all_success: Vec<f64> = records.iter().filter(|r| r.success).map(|r| r.latency_ms).collect();
        let total_success = all_success.len();
        let total_requests = records.len();

        let mode_name = G_MODE_NAME.as_deref().unwrap_or("unknown");
        let cpu_repeat = G_CPU_REPEAT;
        let io_repeat = G_IO_REPEAT;
        let no_yield = G_NO_YIELD;

        let mut output = String::new();
        output.push_str(&format!("# 优先级调度对比报告\n\n"));
        output.push_str(&format!(
            "**调度模式**: {} | **CPU重复**: {} | **I/O重复**: {} | **Yield关闭**: {}\n\n",
            mode_name, cpu_repeat, io_repeat, no_yield
        ));

        output.push_str("## 按优先级批次统计\n\n");
        output.push_str("| 批次 | 优先级 | 请求数 | 成功率 | 平均(ms) | 中位数(ms) | P95(ms) | 最小(ms) | 最大(ms) |\n");
        output.push_str("|------|--------|--------|--------|----------|-----------|---------|----------|----------|\n");

        let (h_succ, m_succ, l_succ) = (
            high.iter().filter(|r| r.success).count(),
            mid.iter().filter(|r| r.success).count(),
            low.iter().filter(|r| r.success).count(),
        );

        output.push_str(&format!(
            "| 高优批 | 2 | {} | {} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} |\n",
            high.len(), h_succ, h_avg, h_med, h_p95, h_min, h_max
        ));
        if num_batches >= 2 {
            output.push_str(&format!(
                "| 中优批 | 1 | {} | {} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} |\n",
                mid.len(), m_succ, m_avg, m_med, m_p95, m_min, m_max
            ));
        }
        if num_batches >= 3 {
            output.push_str(&format!(
                "| 低优批 | 0 | {} | {} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} |\n",
                low.len(), l_succ, l_avg, l_med, l_p95, l_min, l_max
            ));
        }

        output.push_str("\n## 汇总\n\n");
        output.push_str(&format!("- 总请求: {}\n", total_requests));
        output.push_str(&format!("- 总成功: {}\n", total_success));
        output.push_str(&format!(
            "- 成功率: {:.1}%\n",
            if total_requests > 0 {
                total_success as f64 / total_requests as f64 * 100.0
            } else {
                0.0
            }
        ));

        if !all_success.is_empty() {
            let sum: f64 = all_success.iter().sum();
            output.push_str(&format!("- 总平均延迟: {:.1}ms\n", sum / all_success.len() as f64));
        }

        let _ = std::fs::write(&path_str, &output);
        println!("\n报告已保存: {}", path_str);
    }
}

pub fn main() {
    // Parse simple CLI args
    let args: Vec<String> = std::env::args().collect();
    let mut mode = SchedulerMode::HighestPriorityFirst;
    let mut cpu_repeat: usize = 0;
    let mut io_repeat: usize = 0;
    let mut cache_dir_path = String::new();
    let mut no_yield: bool = false;
    let mut num_batches: usize = 3;
    let mut base_priority: u8 = 2;
    let mut report_path = String::new();
    let mut scheduler = String::from("green-thread");
    let mut concurrency: usize = 10;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--mode" => {
                i += 1;
                mode = match args.get(i).map(|s| s.as_str()) {
                    Some("roundrobin") | Some("rr") => SchedulerMode::RoundRobin,
                    _ => SchedulerMode::HighestPriorityFirst,
                };
            }
            "--cpu-repeat" => {
                i += 1;
                cpu_repeat = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(0);
            }
            "--io-repeat" => {
                i += 1;
                io_repeat = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(0);
            }
            "--cache-dir" => {
                i += 1;
                cache_dir_path = args.get(i).cloned().unwrap_or_default();
            }
            "--no-yield" => {
                no_yield = true;
            }
            "--batches" => {
                i += 1;
                num_batches = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(3).max(1).min(3);
            }
            "--base-priority" => {
                i += 1;
                base_priority = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(2);
            }
            "--report" => {
                i += 1;
                report_path = args.get(i).cloned().unwrap_or_default();
            }
            "--scheduler" => {
                i += 1;
                scheduler = args.get(i).cloned().unwrap_or_else(|| String::from("green-thread"));
            }
            "--concurrency" => {
                i += 1;
                concurrency = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(10).max(1);
            }
            _ => {}
        }
        i += 1;
    }

    let mode_name = match mode {
        SchedulerMode::HighestPriorityFirst => "priority",
        SchedulerMode::RoundRobin => "roundrobin",
    };
    println!("=== EXPERIMENT: scheduler={} mode={} cpu={} io={} no_yield={} batches={} base_prio={} ===",
        scheduler, mode_name, cpu_repeat, io_repeat, no_yield, num_batches, base_priority);

    // Setup cache directory
    let cache_dir = if cache_dir_path.is_empty() {
        let dir = std::env::temp_dir().join(format!("gt_crawler_cache_{}", std::process::id()));
        dir
    } else {
        std::path::PathBuf::from(&cache_dir_path)
    };

    // Prefetch HTML files
    println!("\n--- Prefetch (total 33 schools) ---");
    let ok = prefetch_all(&cache_dir);
    println!("Prefetch done: {}/{} success", ok, 33);

    // Setup global state
    let names: Vec<&str> = crate::schools::SCHOOLS.iter().map(|s| s.name).collect();
    unsafe {
        for (i, &name) in names.iter().enumerate() {
            G_SCHOOLS_NAMES[i] = name;
        }
        G_CACHE_DIR = Some(cache_dir.to_string_lossy().to_string());
        G_CPU_REPEAT = cpu_repeat;
        G_IO_REPEAT = io_repeat;
        G_NO_YIELD = no_yield;
        G_NUM_BATCHES = num_batches;
        G_BATCH_RESULTS = Vec::new();
        G_MODE_NAME = Some(mode_name.to_string());
        let default_report = format!(
            "{}_{}_{}cpu_{}io.md",
            mode_name,
            if no_yield { "noyield" } else { "yield" },
            cpu_repeat,
            io_repeat
        );
        G_REPORT_PATH = Some(if report_path.is_empty() { default_report } else { report_path });
    }

    // Tokio scheduler path
    if scheduler == "tokio" {
        println!("\n--- Tokio mode: concurrency={} ---", concurrency);
        let records = tokio_scheduler::run_tokio(
            &cache_dir, cpu_repeat, io_repeat,
            concurrency, num_batches, base_priority,
        );
        unsafe {
            G_BATCH_RESULTS = records.into_iter().map(|(name, latency, len, success)| {
                CrawlRecord {
                    school_name: name,
                    latency_ms: latency,
                    content_len: len,
                    success,
                }
            }).collect();
        }
        generate_report();
        std::process::exit(0);
    }

    // Green-thread scheduler path
    let mut runtime = Runtime::new();
    runtime.init();
    runtime.set_scheduler_mode(mode);

    println!("\n--- Spawning {} batch threads (base_priority={}) ---", num_batches, base_priority);
    runtime.spawn_with_priority(crawl_batch_high, base_priority);
    if num_batches >= 2 {
        runtime.spawn_with_priority(crawl_batch_mid, base_priority.saturating_sub(1));
    }
    if num_batches >= 3 {
        runtime.spawn_with_priority(crawl_batch_low, base_priority.saturating_sub(2));
    }

    // Run (never returns - exits after printing results)
    runtime.run();
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── find_highest_priority_ready ────────────────────────────────────────

    #[test]
    fn test_pick_highest_priority_among_ready() {
        let mut rt = Runtime::new();
        rt.threads[1].state = State::Ready;
        rt.threads[1].priority = 0;
        rt.threads[2].state = State::Ready;
        rt.threads[2].priority = 2;
        rt.threads[3].state = State::Ready;
        rt.threads[3].priority = 1;

        assert_eq!(rt.find_highest_priority_ready(), Some(2));
    }

    #[test]
    fn test_pick_highest_priority_all_equal() {
        let mut rt = Runtime::new();
        for i in 1..4 {
            rt.threads[i].state = State::Ready;
            rt.threads[i].priority = 0;
        }
        let result = rt.find_highest_priority_ready();
        assert!(result.is_some_and(|id| (1..=3).contains(&id)));
    }

    #[test]
    fn test_no_ready_threads_returns_none() {
        let rt = Runtime::new();
        assert_eq!(rt.find_highest_priority_ready(), None);
    }

    #[test]
    fn test_running_thread_not_selected() {
        let mut rt = Runtime::new();
        rt.threads[0].priority = 255;
        rt.threads[1].state = State::Ready;
        rt.threads[1].priority = 1;

        assert_eq!(rt.find_highest_priority_ready(), Some(1));
    }

    // ── spawn / spawn_with_priority ────────────────────────────────────────

    #[test]
    fn test_spawn_sets_priority_and_ready_state() {
        let mut rt = Runtime::new();
        rt.init();
        rt.spawn_with_priority(|| {}, 7);

        assert_eq!(rt.threads[1].priority, 7);
        assert_eq!(rt.threads[1].state, State::Ready);
    }

    #[test]
    fn test_spawn_defaults_to_priority_zero() {
        let mut rt = Runtime::new();
        rt.init();
        rt.spawn(|| {});

        assert_eq!(rt.threads[1].priority, 0);
        assert_eq!(rt.threads[1].state, State::Ready);
    }

    #[test]
    #[should_panic(expected = "no available thread")]
    fn test_spawn_exhausted_threads_panics() {
        let mut rt = Runtime::new();
        rt.init();
        for _ in 1..MAX_THREADS {
            rt.spawn(|| {});
        }
        rt.spawn(|| {});
    }

    // ── initial state ──────────────────────────────────────────────────────

    #[test]
    fn test_runtime_initial_state() {
        let rt = Runtime::new();
        assert_eq!(rt.threads[0].state, State::Running);
        assert_eq!(rt.threads[0].priority, 0);
        assert_eq!(rt.current, 0);
        assert_eq!(rt.mode, SchedulerMode::HighestPriorityFirst);
        for i in 1..MAX_THREADS {
            assert_eq!(rt.threads[i].state, State::Available, "thread {i}");
            assert_eq!(rt.threads[i].priority, 0);
        }
    }

    // ── runtime construction ───────────────────────────────────────────────

    #[test]
    fn test_runtime_has_correct_number_of_threads() {
        let rt = Runtime::new();
        assert_eq!(rt.threads.len(), MAX_THREADS);
    }

    #[test]
    fn test_new_thread_builder_sets_available() {
        let t = Thread::new(5);
        assert_eq!(t.id, 5);
        assert_eq!(t.state, State::Available);
        assert_eq!(t.priority, 0);
        assert_eq!(t.stack.len(), DEFAULT_STACK_SIZE);
    }

    // ── round-robin scheduler mode ─────────────────────────────────────────

    #[test]
    fn test_round_robin_selects_next_ready_in_order() {
        let mut rt = Runtime::new();
        rt.mode = SchedulerMode::RoundRobin;
        // Make threads 1,2,3 Ready, thread 0 is Running
        rt.threads[1].state = State::Ready;
        rt.threads[2].state = State::Ready;
        rt.threads[3].state = State::Ready;
        // Current is 0, so next round-robin should select 1
        assert_eq!(rt.find_next_round_robin(), Some(1));
    }

    #[test]
    fn test_round_robin_wraps_around() {
        let mut rt = Runtime::new();
        rt.mode = SchedulerMode::RoundRobin;
        rt.current = 8; // near end
        rt.threads[1].state = State::Ready; // only thread 1 is Ready
        assert_eq!(rt.find_next_round_robin(), Some(1)); // wraps: 9,0,1
    }

    #[test]
    fn test_round_robin_no_ready_returns_none() {
        let mut rt = Runtime::new();
        rt.mode = SchedulerMode::RoundRobin;
        assert_eq!(rt.find_next_round_robin(), None);
    }
}
