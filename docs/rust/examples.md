# Rust examples

The examples below use the `vsh` library target from the `vsh-runtime` package.

## Preview and inspect

```rust
use vsh::{MontyObject, ReceiptDetail, RunRequest, Runtime, RuntimeConfig};

fn main() -> Result<(), vsh::VshError> {
    let runtime = Runtime::open(RuntimeConfig::new("project"))?;
    let receipt = runtime.preview(
        RunRequest::new(
            "from pathlib import Path\n\
             value = Path('/workspace/input.txt').read_text()\n\
             Path('/workspace/output.txt').write_text(value.upper())\n\
             len(value)",
        )
        .with_intent("Generate uppercase output")
        .with_detail(ReceiptDetail::Full),
    )?;

    if let MontyObject::Int(length) = receipt.value {
        println!("input length: {length}");
    }
    for change in &receipt.changes {
        println!("{:?} {}", change.kind, change.path);
    }
    Ok(())
}
```

## Promote only auto-approved work

```rust
use vsh::RuntimeDecision;

match receipt.decision {
    RuntimeDecision::AutoApproved => {
        let committed = runtime.commit(receipt.transaction, now_unix_ms())?;
        assert!(committed.commit.is_some());
    }
    RuntimeDecision::PendingApproval(manifest) => {
        eprintln!("review required: {:?}", manifest.flags);
    }
    RuntimeDecision::Denied(manifest) => {
        eprintln!("denied: {:?}", manifest.reason);
    }
}
```

`now_unix_ms()` is a trusted-host clock helper. A preview transaction is single-use;
do not rerun source as a substitute for promotion.

## Strict approval

```rust
use vsh::{PolicyProfile, PrincipalId, RunRequest, Runtime, RuntimeConfig};

fn reviewed_change(now_ms: u64) -> Result<(), vsh::VshError> {
    let runtime = Runtime::open(
        RuntimeConfig::new("project").with_policy_profile(PolicyProfile::Strict),
    )?;
    let preview = runtime.preview(RunRequest::new(
        "from pathlib import Path\n\
         Path('/workspace/reviewed.txt').write_text('yes')",
    ))?;

    let principal = PrincipalId::digest_label("reviewer:alice/change:1842");
    runtime.approve(preview.transaction, principal, now_ms, now_ms + 30_000)?;
    runtime.commit(preview.transaction, now_ms + 1)?;
    Ok(())
}
```

## Custom limits

```rust
use std::time::Duration;
use vsh::{ExecutionBudget, RunRequest};

let budget = ExecutionBudget {
    max_program_bytes: 64 * 1024,
    max_duration: Duration::from_millis(300),
    max_memory_bytes: 64 * 1024 * 1024,
    max_os_calls: 1_500,
    max_read_bytes: 8 * 1024 * 1024,
    max_write_bytes: 2 * 1024 * 1024,
    max_output_bytes: 32 * 1024,
    max_result_bytes: 64 * 1024,
    ..ExecutionBudget::default()
};

let request = RunRequest::new(source).with_budget(budget);
let preview = runtime.preview(request)?;
```

## External data directory and worker

```rust
use vsh::{Runtime, RuntimeConfig};

let config = RuntimeConfig::new("/srv/workspaces/job-17")
    .with_data_directory("/srv/vsh-state/job-17")
    .with_worker_path("/opt/vsh/0.3.0/vsh-monty-worker")
    .with_max_idle_workers(8);
let runtime = Runtime::open(config)?;
```

The data directory must remain separate from the workspace. Treat the worker path and
state root as trusted deployment configuration, not agent input.

## Recovery reporting

```rust
let startup = runtime.startup_recovery();
if !startup.conflicts.is_empty() || startup.orphaned > 0 {
    return Err("operator review required".into());
}

let report = runtime.recover()?;
for conflict in report.conflicts {
    eprintln!("recovery conflict: {conflict:?}");
}
```

Recovery conflicts are an operational signal. VSH leaves ambiguous ownership untouched
instead of turning recovery into an unsafe cleanup routine.
