//! Process allocator probe; each invocation is an isolated worker experiment.

#[global_allocator]
static ALLOCATOR: monty_alloc::LimitedAllocator = monty_alloc::LimitedAllocator;

fn main() {
    let mode = std::env::args().nth(1).expect("probe mode required");
    monty_alloc::set_limit(Some(1024 * 1024), false).expect("allocator installed");
    match mode.as_str() {
        "within" => {
            let bytes = vec![0x41_u8; 512 * 1024];
            std::hint::black_box(bytes);
            println!("within-limit");
        }
        "exceed" => {
            let bytes = vec![0x41_u8; 8 * 1024 * 1024];
            std::hint::black_box(bytes);
            panic!("hard ceiling was not enforced");
        }
        "thread-exceed" => {
            std::thread::spawn(|| {
                let bytes = vec![0x41_u8; 8 * 1024 * 1024];
                std::hint::black_box(bytes);
            })
            .join()
            .expect("thread completed");
            panic!("thread allocation escaped the ceiling");
        }
        "reset" => {
            monty_alloc::set_limit(None, false).unwrap();
            let bytes = vec![0x41_u8; 8 * 1024 * 1024];
            std::hint::black_box(bytes);
            monty_alloc::set_limit(Some(1024 * 1024), false).unwrap();
            let bytes = vec![0x41_u8; 512 * 1024];
            std::hint::black_box(bytes);
            println!("reset-limit");
        }
        "bash-sleep" | "bash-loop" => {
            use std::time::{Duration, Instant};
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let mut limits = bashkit::ExecutionLimits::default();
            limits.timeout = Duration::from_millis(20);
            limits.max_loop_iterations = usize::MAX;
            limits.max_total_loop_iterations = usize::MAX;
            limits.max_commands = usize::MAX;
            let mut bash = bashkit::Bash::builder().limits(limits).build();
            let started = Instant::now();
            let script = if mode == "bash-sleep" {
                "sleep 10"
            } else {
                "while true; do :; done"
            };
            let result = runtime.block_on(bash.exec(script));
            println!(
                "elapsed_ms={} result={result:?}",
                started.elapsed().as_millis()
            );
        }
        _ => panic!("unknown probe mode"),
    }
}
