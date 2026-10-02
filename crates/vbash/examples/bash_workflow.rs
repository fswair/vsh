//! Run with `cargo run -p vsh-runtime --features bash --example bash_workflow -- WORKSPACE WORKER`.
use std::error::Error;
use std::io;
use vsh::{
    BashConfig, ExecutionOutput, Language, ReceiptDetail, RunRequest, Runtime, RuntimeConfig,
};

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let workspace = arguments
        .next()
        .ok_or_else(|| io::Error::other("workspace required"))?;
    let worker = arguments
        .next()
        .ok_or_else(|| io::Error::other("Bash worker path required"))?;
    let runtime = Runtime::open(RuntimeConfig::new(workspace).with_bash(BashConfig::new(worker)))?;
    let preview = runtime.preview(
        RunRequest::new("printf 'verified\\n' > report.txt; cat report.txt")
            .with_language(Language::Bash)
            .with_intent("create the verified fixture report")
            .with_detail(ReceiptDetail::Full),
    )?;
    if let ExecutionOutput::Bash(result) = &preview.output {
        println!("exit={}, stdout={:?}", result.exit_code, result.stdout);
    }
    println!("state={:?}, changes={:?}", preview.state, preview.changes);
    // Preview never applies user-file changes. The caller decides when to promote.
    let committed = runtime.commit(preview.transaction, 0)?;
    println!("committed={:?}", committed.state);
    Ok(())
}
