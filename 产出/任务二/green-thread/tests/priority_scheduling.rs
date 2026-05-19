use std::process::Command;

#[test]
fn test_priority_completion_order() {
    let output = Command::new(env!("CARGO_BIN_EXE_green-thread"))
        .output()
        .expect("failed to run green-thread binary");
    assert!(
        output.status.success(),
        "green-thread exited with status: {}",
        output.status
    );

    let stdout = String::from_utf8(output.stdout).expect("non-UTF-8 output");
    eprintln!("=== green-thread output ===\n{stdout}");

    let t3_finish = stdout
        .find("Thread 3 state: Running → Available (task completed)")
        .expect("Thread 3 completion trace not found");
    let t2_finish = stdout
        .find("Thread 2 state: Running → Available (task completed)")
        .expect("Thread 2 completion trace not found");
    let t1_finish = stdout
        .find("Thread 1 state: Running → Available (task completed)")
        .expect("Thread 1 completion trace not found");

    assert!(
        t3_finish < t2_finish,
        "Thread 3 (priority=2) should finish BEFORE Thread 2 (priority=1)"
    );
    assert!(
        t2_finish < t1_finish,
        "Thread 2 (priority=1) should finish BEFORE Thread 1 (priority=0)"
    );
}

#[test]
fn test_priority_scheduling_trace_in_output() {
    let output = Command::new(env!("CARGO_BIN_EXE_green-thread"))
        .output()
        .expect("failed to run green-thread binary");
    let stdout = String::from_utf8(output.stdout).expect("non-UTF-8 output");

    assert!(
        stdout.contains("selecting Thread 3 (Ready → Running, prio=2)"),
        "Should select highest priority thread first"
    );
    assert!(
        stdout.contains("All threads completed"),
        "Should print completion message"
    );
}

#[test]
fn test_spawn_trace_shows_priority() {
    let output = Command::new(env!("CARGO_BIN_EXE_green-thread"))
        .output()
        .expect("failed to run green-thread binary");
    let stdout = String::from_utf8(output.stdout).expect("non-UTF-8 output");

    assert!(
        stdout.contains("priority 0"),
        "Should trace spawn with priority 0"
    );
    assert!(
        stdout.contains("priority 1"),
        "Should trace spawn with priority 1"
    );
    assert!(
        stdout.contains("priority 2"),
        "Should trace spawn with priority 2"
    );
}
