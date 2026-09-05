# Policies and budgets

Policy controls whether a valid virtual result may progress. Budgets control how much
work can be performed to produce that result. They are independent, fail-closed layers.

## Built-in policy profiles

| Profile | Mutation posture | Escalation thresholds | Typical use |
|---|---|---|---|
| `balanced` | Small non-destructive changes may auto-approve | 500 paths or 64 MiB changed | Interactive local agents and routine automation |
| `strict` | Every mutation escalates | 100 paths or 8 MiB changed | Reviewed CI and organizational workflows |
| `paranoid` | Every mutation escalates; lower denial ceilings | 25 paths or 1 MiB changed | High-sensitivity workspaces |

All profiles deny protected access attempts and protected mutations. Deletes, renames,
executable changes, symlink changes, large touched sets, and large byte changes produce
risk flags. Catastrophic path, byte, delete-count, and delete-ratio ceilings are hard
denials, not approval prompts.

!!! warning "Profiles are not permission grants"

    Selecting `balanced` does not grant Monty ambient filesystem access. Call policy,
    capability roots, transaction identity, revalidation, and commit recovery remain in
    force for every profile.

## Default execution budget

| Limit | Default | Protects |
|---|---:|---|
| Program source | 1 MiB | parser/compiler allocation |
| Duration | 1 second | runaway bytecode |
| Recursion | 512 frames | stack growth |
| Worker heap | 256 MiB | guest allocation |
| Typed OS / high-level VSH calls | 10,000 | call amplification |
| Read bytes | 64 MiB | cumulative materialization |
| Write bytes | 64 MiB | cumulative virtual output |
| One I/O call | 4 MiB | single-frame allocation |
| One path | 16 KiB | path/protocol abuse |
| Directory entries | 100,000 | traversal fan-out |
| Captured stdout | 1 MiB | output flooding |
| Returned value | 1 MiB | host conversion |
| Exception payload | 256 KiB | traceback/error flooding |

The supervised worker additionally enforces protocol frame and process boundaries.
Snapshot, artifact, state-log, journal, plan, and commit paths have their own trusted
host limits.

Duration is cumulative Monty **bytecode** time, not total request wall time. Worker
heap is not parent-process or process-tree RSS. Parent-side high-level function work,
snapshot traversal, storage and commit have different bounds. A worker event deadline
does not interrupt synchronous parent dispatch. Add trusted service-level admission
and resource controls for a complete deployment budget.

Snapshot defaults are 250,000 nodes, depth 128 and 16 GiB metadata-represented file/link
bytes. Metadata traverses the whole configured workspace; file contents remain lazy.
Rust exposes `SnapshotLimits`; the Python convenience constructor does not expose
these knobs. The default auto-preview cache is separately capped at 64 entries and
128 MiB encoded artifact bytes.

## Python configuration

Unspecified values keep native defaults:

```python
from vsh import ExecutionBudget, RunRequest

budget = ExecutionBudget(
    max_duration_ms=250,
    max_memory_bytes=64 * 1024 * 1024,
    max_os_calls=2_000,
    max_read_bytes=8 * 1024 * 1024,
    max_write_bytes=4 * 1024 * 1024,
    max_output_bytes=64 * 1024,
    max_result_bytes=128 * 1024,
)
request = RunRequest(code, budget=budget)
```

## Rust configuration

`ExecutionBudget` is an alias of `ExecutionLimits`. Start from `default()` and replace
only the fields the workload owns:

```rust
use std::time::Duration;
use vsh::{ExecutionBudget, RunRequest};

let budget = ExecutionBudget {
    max_duration: Duration::from_millis(250),
    max_memory_bytes: 64 * 1024 * 1024,
    max_os_calls: 2_000,
    max_read_bytes: 8 * 1024 * 1024,
    max_write_bytes: 4 * 1024 * 1024,
    ..ExecutionBudget::default()
};
let request = RunRequest::new(code).with_budget(budget);
```

## Budget design guidance

`max_os_calls` counts both typed operations produced by `pathlib` and high-level VSH
function invocations. Recursive high-level work intentionally uses one suspension slot;
its parent-side scope is independently bounded by cumulative/per-call I/O,
directory-entry, path, result and snapshot ceilings. Guest bytecode/heap limits have
the narrower scope described above.

1. Measure a representative successful workload.
2. Set each independent limit above the measured p99 plus operational headroom.
3. Keep result and stdout ceilings much smaller than filesystem read ceilings.
4. Treat a budget failure as a rejected transaction, never as permission to rerun with
   unlimited values.
5. Separate interactive, CI, and bulk-migration profiles in the trusted host.

Execution budgets contribute to the recorded execution configuration identity. Later
promotion consumes that stored artifact; passing a different budget does not rewrite
what was executed. Keep deployment roots, worker and policy configuration consistent
when reopening. Do not infer that commit compares every property of a newly opened
`RuntimeConfig` with the original execution configuration.
