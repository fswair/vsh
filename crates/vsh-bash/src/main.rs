//! The guest worker never receives a host workspace path or a commit capability.

#[global_allocator]
static ALLOCATOR: monty_alloc::LimitedAllocator = monty_alloc::LimitedAllocator;

fn main() {
    let mut arguments = std::env::args_os().skip(1);
    if let Some(argument) = arguments.next() {
        if arguments.next().is_some() {
            eprintln!("vsh-bash-worker: unexpected extra arguments");
            std::process::exit(64);
        }
        if argument == "--version" {
            println!("{}", vsh_bash::WORKER_VERSION);
            return;
        }
        if argument != "--worker" {
            eprintln!("vsh-bash-worker: expected --version or --worker");
            std::process::exit(64);
        }
    }
    if let Err(error) = vsh_bash::worker_main() {
        eprintln!("vsh-bash-worker: {error}");
        std::process::exit(64);
    }
}
