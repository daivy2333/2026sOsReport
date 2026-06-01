use std::path::PathBuf;
use std::process::Command;

// ── Helpers ──────────────────────────────────────────────────────────────

fn binary_path() -> PathBuf {
    let release = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/release/green-thread-crawler");
    if release.exists() {
        return release;
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/debug/green-thread-crawler")
}

fn run_experiment(mode: &str, cpu_repeat: usize, io_repeat: usize) -> std::process::Output {
    let cache_dir = std::env::temp_dir().join(format!("gt_test_cache_{}", std::process::id()));
    Command::new(binary_path())
        .arg("--mode")
        .arg(mode)
        .arg("--cpu-repeat")
        .arg(cpu_repeat.to_string())
        .arg("--io-repeat")
        .arg(io_repeat.to_string())
        .arg("--cache-dir")
        .arg(cache_dir.to_string_lossy().to_string())
        .output()
        .expect("failed to run experiment binary")
}

fn run_experiment_with_flags(mode: &str, cpu: usize, io: usize, flags: &[&str]) -> std::process::Output {
    let cache_dir = std::env::temp_dir().join(format!("gt_test_cache_{}", std::process::id()));
    let mut cmd = Command::new(binary_path());
    cmd.arg("--mode").arg(mode)
       .arg("--cpu-repeat").arg(cpu.to_string())
       .arg("--io-repeat").arg(io.to_string())
       .arg("--cache-dir").arg(cache_dir.to_string_lossy().to_string());
    for flag in flags {
        cmd.arg(flag);
    }
    cmd.output().expect("failed to run experiment binary")
}

fn parse_prio_lines(stdout: &str) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut high = Vec::new();
    let mut mid = Vec::new();
    let mut low = Vec::new();
    for line in stdout.lines() {
        if line.starts_with("[PRIO=2]") {
            high.push(line.to_string());
        } else if line.starts_with("[PRIO=1]") {
            mid.push(line.to_string());
        } else if line.starts_with("[PRIO=0]") {
            low.push(line.to_string());
        }
    }
    (high, mid, low)
}

fn parse_results(stdout: &str) -> Vec<(usize, String, f64, usize, bool)> {
    let mut results = Vec::new();
    let mut in_results = false;
    for line in stdout.lines() {
        if line == "=== RESULTS ===" {
            in_results = true;
            continue;
        }
        if line == "=== END RESULTS ===" {
            break;
        }
        if in_results {
            let parts: Vec<&str> = line.split('|').collect();
            if parts.len() == 5 {
                let idx: usize = parts[0].parse().unwrap_or(0);
                let name = parts[1].to_string();
                let latency: f64 = parts[2].parse().unwrap_or(0.0);
                let len: usize = parts[3].parse().unwrap_or(0);
                let success: bool = parts[4] == "1";
                results.push((idx, name, latency, len, success));
            }
        }
    }
    results
}

fn parse_latencies_by_batch(stdout: &str) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let mut high: Vec<f64> = Vec::new();
    let mut mid: Vec<f64> = Vec::new();
    let mut low: Vec<f64> = Vec::new();
    let results = parse_results(stdout);
    for (idx, _name, latency, _len, success) in &results {
        if !success {
            continue;
        }
        if *idx < 11 {
            high.push(*latency);
        } else if *idx < 22 {
            mid.push(*latency);
        } else {
            low.push(*latency);
        }
    }
    (high, mid, low)
}

fn timed_run(mode: &str, cpu: usize, io: usize) -> (String, std::time::Duration) {
    let start = std::time::Instant::now();
    let output = run_experiment(mode, cpu, io);
    let duration = start.elapsed();
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    (stdout, duration)
}

fn run_tokio(cpu: usize, io: usize, base_priority: u8) -> std::process::Output {
    let cache_dir = std::env::temp_dir().join(format!("gt_test_cache_{}", std::process::id()));
    Command::new(binary_path())
        .arg("--scheduler").arg("tokio")
        .arg("--mode").arg("priority")
        .arg("--cpu-repeat").arg(cpu.to_string())
        .arg("--io-repeat").arg(io.to_string())
        .arg("--base-priority").arg(base_priority.to_string())
        .arg("--cache-dir").arg(cache_dir.to_string_lossy().to_string())
        .output()
        .expect("failed to run tokio experiment")
}

// ═══════════════════════════════════════════════════════════════════════════
// GREEN-THREAD CORE TESTS (6)
// ═══════════════════════════════════════════════════════════════════════════

// ── GT: Heavy CPU Priority ──
#[test]
fn test_gt_priority_heavy_cpu() {
    let output = run_experiment("priority", 10000, 0);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success());
    let (high, _mid, low) = parse_prio_lines(&stdout);
    if !high.is_empty() && !low.is_empty() {
        let last_high = stdout.find(&high[high.len()-1]).unwrap_or(0);
        let first_low = stdout.find(&low[0]).unwrap_or(usize::MAX);
        assert!(last_high < first_low, "PRIO=0 must run after PRIO=2 in priority mode");
    }
}

// ── GT: Heavy CPU RoundRobin ──
#[test]
fn test_gt_roundrobin_heavy_cpu() {
    let output = run_experiment("roundrobin", 10000, 0);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success());
    let (high, _mid, low) = parse_prio_lines(&stdout);
    if !high.is_empty() && !low.is_empty() {
        let last_high_pos = stdout.find(&high[high.len()-1]).unwrap_or(0);
        let has_interleaving = low.iter().any(|l| stdout.find(l).unwrap_or(usize::MAX) < last_high_pos);
        assert!(has_interleaving, "RR mode must show PRIO=0 interleaving with PRIO=2");
    }
}

// ── GT: No-yield starvation ──
#[test]
fn test_gt_no_yield_starvation() {
    let output = run_experiment_with_flags("priority", 10000, 0, &["--no-yield"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success());
    let (high, _mid, low) = parse_prio_lines(&stdout);
    if !high.is_empty() && !low.is_empty() {
        let last_high = stdout.find(&high[high.len()-1]).unwrap_or(0);
        let first_low = stdout.find(&low[0]).unwrap_or(usize::MAX);
        assert!(last_high < first_low, "Without yield, PRIO=0 must run after PRIO=2");
    }
}

// ── GT: Same priority baseline ──
#[test]
fn test_gt_same_priority() {
    let output = run_experiment_with_flags("priority", 10000, 0, &["--base-priority", "0"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success());
    // With all-same priority and priority-first scheduler, execution is sequential.
    let (high, _mid, low) = parse_prio_lines(&stdout);
    if !high.is_empty() && !low.is_empty() {
        let last_high = stdout.find(&high[high.len()-1]).unwrap_or(0);
        let first_low = stdout.find(&low[0]).unwrap_or(usize::MAX);
        assert!(last_high < first_low, "Same priority: lowest-index thread wins tiebreaker");
    }
}

// ── GT: I/O medium Priority ──
#[test]
fn test_gt_io_medium_priority() {
    let output = run_experiment("priority", 0, 1000);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success());
    let (high, _mid, low) = parse_prio_lines(&stdout);
    if !high.is_empty() && !low.is_empty() {
        let last_high = stdout.find(&high[high.len()-1]).unwrap_or(0);
        let first_low = stdout.find(&low[0]).unwrap_or(usize::MAX);
        assert!(last_high < first_low, "I/O mode: PRIO=0 runs last");
    }
}

// ── GT: I/O medium RoundRobin ──
#[test]
fn test_gt_io_medium_roundrobin() {
    let output = run_experiment("roundrobin", 0, 1000);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success());
    assert!(stdout.contains("=== RESULTS ==="));
}

// ═══════════════════════════════════════════════════════════════════════════
// TOKIO COMPARISON TESTS (4)
// ═══════════════════════════════════════════════════════════════════════════

// ── Tokio: Priority CPU ──
#[test]
fn test_tokio_priority_heavy_cpu() {
    let output = run_tokio(10000, 0, 2);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success());
    let (high, mid, low) = parse_prio_lines(&stdout);
    // Concurrent execution interleaves completion order, but priority dispatch
    // guarantees the FIRST task popped is high-priority (PRIO=2).
    assert!(!high.is_empty(), "PRIO=2 tasks must exist");
    assert!(!low.is_empty(), "PRIO=0 tasks must exist");
    let first_line = stdout.lines().find(|l| l.starts_with("[PRIO=")).unwrap_or("");
    assert!(first_line.starts_with("[PRIO=2]"),
        "First dispatched task must be PRIO=2, got: {}", first_line);
    let total = high.len() + mid.len() + low.len();
    assert_eq!(total, 33, "Expected 33 tasks, got {}", total);
}

// ── Tokio: Default (same priority) CPU ──
#[test]
fn test_tokio_default_heavy_cpu() {
    let output = run_tokio(10000, 0, 0);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success());
    let (high, mid, low) = parse_prio_lines(&stdout);
    // base_priority=0: all tasks get priority=0, label maps to PRIO=2
    // (task.priority == base_priority → label "2"). No actual differentiation.
    let total = high.len() + mid.len() + low.len();
    assert_eq!(total, 33, "Expected 33 tasks total, got {}", total);
}

// ── Tokio: Priority I/O ──
#[test]
fn test_tokio_priority_io_medium() {
    let output = run_tokio(0, 1000, 2);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success());
    let (high, mid, low) = parse_prio_lines(&stdout);
    assert!(!high.is_empty(), "PRIO=2 tasks must exist");
    assert!(!low.is_empty(), "PRIO=0 tasks must exist");
    let first_line = stdout.lines().find(|l| l.starts_with("[PRIO=")).unwrap_or("");
    assert!(first_line.starts_with("[PRIO=2]"),
        "First dispatched I/O task must be PRIO=2, got: {}", first_line);
    let total = high.len() + mid.len() + low.len();
    assert_eq!(total, 33, "Expected 33 tasks, got {}", total);
}

// ── Tokio: Default (same priority) I/O ──
#[test]
fn test_tokio_default_io_medium() {
    let output = run_tokio(0, 1000, 0);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success());
    let (high, mid, low) = parse_prio_lines(&stdout);
    let total = high.len() + mid.len() + low.len();
    assert_eq!(total, 33, "Expected 33 tasks total, got {}", total);
}

// ═══════════════════════════════════════════════════════════════════════════
// MEMORY CONSUMPTION TESTS (3)
// ═══════════════════════════════════════════════════════════════════════════

// ── Memory helpers ──

fn run_measure_memory(scheduler: &str, mode: &str, base_priority: u8) -> std::process::Output {
    let cache_dir = std::env::temp_dir().join(format!("gt_memtest_{}", std::process::id()));
    Command::new(binary_path())
        .arg("--scheduler").arg(scheduler)
        .arg("--mode").arg(mode)
        .arg("--base-priority").arg(base_priority.to_string())
        .arg("--measure-memory")
        .arg("--cache-dir").arg(cache_dir.to_string_lossy().to_string())
        .output()
        .expect("failed to run memory benchmark")
}

fn parse_static_sizes(stdout: &str) -> std::collections::HashMap<String, u64> {
    let mut map = std::collections::HashMap::new();
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("STATIC_SIZES:") {
            for kv in rest.split_whitespace() {
                if let Some((k, v)) = kv.split_once('=') {
                    if let Ok(n) = v.parse::<u64>() {
                        map.insert(k.to_string(), n);
                    }
                }
            }
        }
    }
    map
}

fn parse_rss_labels(stdout: &str) -> std::collections::HashMap<String, u64> {
    let mut map = std::collections::HashMap::new();
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("RSS:") {
            for kv in rest.split_whitespace() {
                if let Some((k, v)) = kv.split_once('=') {
                    if let Ok(n) = v.parse::<u64>() {
                        map.insert(k.to_string(), n);
                    }
                }
            }
        }
    }
    map
}

// ── Memory test: static sizes ──

#[test]
fn test_memory_static_sizes() {
    let output = run_measure_memory("green-thread", "priority", 2);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "binary failed: {}", stdout);
    let sizes = parse_static_sizes(&stdout);
    assert!(sizes.contains_key("thread"), "missing 'thread' key, got: {}", stdout);
    assert!(sizes.contains_key("ctx"), "missing 'ctx' key, got: {}", stdout);
    assert!(sizes.contains_key("priority_u8"), "missing 'priority_u8' key, got: {}", stdout);
    let thread = sizes["thread"];
    let ctx = sizes["ctx"];
    let prio = sizes["priority_u8"];
    assert!(thread > ctx, "Thread ({}) should be larger than ThreadContext ({})", thread, ctx);
    assert_eq!(prio, 1, "size_of::<u8>() should be 1, got {}", prio);
    // The 2MB stack dominates per-coroutine total
    let per_coroutine = sizes["per_coroutine_total"];
    assert!(per_coroutine > 2_000_000, "per-coroutine total should include 2MB stack, got {}", per_coroutine);
}

// ── Memory test: Priority vs RR RSS ──

#[test]
fn test_memory_priority_vs_rr_rss() {
    // Run both modes; peak RSS should be similar (within 2MB) since priority
    // is just 1 byte per thread — negligible vs 2MB stack allocation
    let out_prio = run_measure_memory("green-thread", "priority", 2);
    let out_rr = run_measure_memory("green-thread", "roundrobin", 2);
    let s_prio = String::from_utf8_lossy(&out_prio.stdout).to_string();
    let s_rr = String::from_utf8_lossy(&out_rr.stdout).to_string();
    assert!(out_prio.status.success(), "priority run failed: {}", s_prio);
    assert!(out_rr.status.success(), "rr run failed: {}", s_rr);

    let rss_prio = parse_rss_labels(&s_prio);
    let rss_rr = parse_rss_labels(&s_rr);

    // Compare peak RSS (largest sampled value)
    let peak_prio = rss_prio.values().max().copied().unwrap_or(0);
    let peak_rr = rss_rr.values().max().copied().unwrap_or(0);
    assert!(peak_prio > 0, "priority run reported no RSS samples: {}", s_prio);
    assert!(peak_rr > 0, "rr run reported no RSS samples: {}", s_rr);

    let diff = (peak_prio as i64 - peak_rr as i64).abs();
    assert!(diff < 2_000, "Priority vs RR peak RSS should differ by < 2MB (got {} KB vs {} KB, diff {} KB)",
        peak_prio, peak_rr, diff);
}

// ── Memory test: Tokio path ──

#[test]
fn test_memory_tokio_path() {
    let output = run_measure_memory("tokio", "priority", 2);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "tokio path failed: {}", stdout);
    let sizes = parse_static_sizes(&stdout);
    assert!(sizes.contains_key("thread"), "tokio path should still report static sizes: {}", stdout);
}
