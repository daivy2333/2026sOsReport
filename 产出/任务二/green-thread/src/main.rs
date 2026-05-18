use std::arch::naked_asm;
use std::time::Instant;

#[cfg_attr(target_os = "windows", path = "win64.rs")]
#[cfg_attr(all(target_os = "linux", not(target_arch = "riscv64")), path = "linux64.rs")]
#[cfg_attr(any(target_arch = "riscv64"), path = "rv64.rs")]
mod os;
use os::ThreadContext;

const DEFAULT_STACK_SIZE: usize = 1024 * 1024 * 2;
const MAX_THREADS: usize = 8;
static mut RUNTIME: usize = 0;
static mut T0: Option<Instant> = None;

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
        }
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

    fn t_yield(&mut self) -> bool {
        if let Some(next) = self.find_highest_priority_ready() {
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

pub fn main() {
    let mut runtime = Runtime::new();
    runtime.init();

    runtime.spawn_with_priority(
        || {
            let id = 1;
            for i in 0..3 {
                println!("[task] thread {}: step {} (priority=0)", id, i);
                yield_thread();
            }
        },
        0,
    );

    runtime.spawn_with_priority(
        || {
            let id = 2;
            for i in 0..3 {
                println!("[task] thread {}: step {} (priority=1)", id, i);
                yield_thread();
            }
        },
        1,
    );

    runtime.spawn_with_priority(
        || {
            let id = 3;
            for i in 0..3 {
                println!("[task] thread {}: step {} (priority=2)", id, i);
                yield_thread();
            }
        },
        2,
    );

    runtime.run();
}
