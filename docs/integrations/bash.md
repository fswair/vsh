# Bounded Bash on the VSH filesystem

Bashkit parses and executes the shell; **VSH owns the filesystem authority and the
transaction**. This is an additional guest frontend, not a host `bash` subprocess,
a container, or a claim of complete POSIX compatibility. Monty remains the default.

Use Bash for familiar text pipelines, generated-file workflows, directory operations
and mode changes that need the same preview/review/stale/commit guarantees as Monty.
Use Monty for structured values and the native `vsh_*` Python tools. Neither guest
gets the host's filesystem, environment, executables or network.

## Enable it explicitly

!!! note "Development checkout additions"

    The nested Monty `vsh_bash` function described here is an
    unreleased addition after 0.6.0. Standalone `language=Language.BASH` is available
    in 0.6.0; the new guest function requires a build from this checkout.

VSH 0.6.0 Python wheels on Linux/macOS bundle two separate workers. In this development
checkout, the first execution on a Bash-enabled runtime initializes the Bash worker
pool, including a Monty execution that could call `vsh_bash`. `worker_path` still selects Monty; the Bash executable is
selected by `BashConfig.worker_path`, `VSH_BASH_WORKER`, or the wheel's script directory.
There is no implicit download, shell command lookup or fallback to real filesystem I/O.

In the development checkout, the same opt-in also enables `vsh_bash(code)` inside
Monty. It borrows the active VFS and shares I/O, evidence, output and outer deadline
budgets; parser/work-unit limits apply to each fresh shell. There is no nested snapshot,
approval or commit. Each shell starts at `/workspace` without prior shell variables.
Run `examples/native/monty_bash_workflow.py` for a complete disposable example that
stages a Monty write, copies it in Bash, searches the staged result and commits once.

```python
from vsh import BashConfig, BashResult, Language, ReceiptDetail, Runtime

runtime = Runtime.open("./workspace", bash=BashConfig())
preview = runtime.preview(
    "mkdir -p reports; printf 'verified\\n' > reports/status.txt; cat reports/status.txt",
    language=Language.BASH,
    intent="write a generated status report",
    detail=ReceiptDetail.FULL,
)
assert isinstance(preview.result, BashResult)
print(preview.state, preview.changes, preview.result.stdout)
# Inspect the exact proposal, then promote its existing identity:
committed = runtime.commit(preview.transaction, 0)
```

The final line requires a committable proposal. Strict policy returns pending approval
instead: use an independent approval or a configured [commit hook](../python/hooks.md).
Do not rerun the source to “commit the preview”; promote its exact transaction ID.

In a source checkout, build the independent executable first:

```bash
cargo build --release --locked -p vsh-bash --no-default-features --features worker --bin vsh-bash-worker
export VSH_BASH_WORKER="$PWD/target/release/vsh-bash-worker"
```

Rust enables only the lightweight host adapter in the application:

```toml
[dependencies]
vsh = { version = "=0.6.0", features = ["bash"] }
```

```rust
use vsh::{BashConfig, Language, RunRequest, Runtime, RuntimeConfig};

let runtime = Runtime::open(
    RuntimeConfig::new("./workspace")
        .with_bash(BashConfig::new("./vsh-bash-worker")),
)?;
let preview = runtime.preview(
    RunRequest::new("printf verified > report.txt")
        .with_language(Language::Bash)
        .with_intent("create a generated report"),
)?;
let committed = runtime.commit(preview.transaction, 0)?;
```

This surface is included in VSH 0.6.0. Rust hosts deploy a matching worker separately;
install it with `cargo install vsh-bash --version '=0.6.0' --locked --no-default-features
--features worker --bin vsh-bash-worker` or build it from the matching checkout.
The Bash profile is for Unix hosts (Linux/macOS). Windows Bash opt-in is
rejected; the existing Monty platform support is unchanged.

Relative Bash worker paths are resolved against the host's current directory when
the adapter opens, before workers switch to their isolated working directory. The
resolved executable is reused for pool replacements; it never becomes a guest command.

## Execution and evidence

Each call captures a fresh immutable workspace snapshot. The worker gets a fresh
Bash interpreter, with `/workspace` as its working directory and no persistent
variables, functions or shell state from earlier calls. Filesystem calls cross a
bounded typed protocol into the parent-owned gateway, which maps paths, authorizes
access, accounts budgets, records origins and changes the transaction's `VirtualFs`.

The final canonical diff compares base and overlay state. It is not a list of command
names: writes later restored to their original content need not remain in the diff.
The ordered effect ledger still records what happened, including `bash_call` origins;
read/write dependencies still participate in stale checking. Snapshot, diff, policy,
output evidence, source, intent, language and profile are bound into the transaction.

A successful shell exit is necessary but not sufficient. Policy can still deny or
require review. Nonzero exit, timeout, cancellation, protocol/resource failure or an
unsupported operation produces **no approvable transaction**. A diagnostic diff
after failure is not permission to commit partial work. Unsupported-profile and
resource failures remain terminal even if shell code catches a failing command.

## The reviewed profile: `vsh-bash-bounded-v4`

| Area | Contract |
| --- | --- |
| Paths | `/workspace` maps to the snapshot root; traversal/other namespaces fail closed |
| Files | `cat`, writes/redirection, append, lists, plain `cp`/`mv`, bounded `rm` and directory operations use VSH's gateway |
| Pipelines | Virtual pipelines, expansions, loops, functions and supported Bashkit builtins; no host process spawn |
| Permissions | Ordinary permission-bit `chmod` is virtual and participates in policy, durable diff and verified commit; special bits/unsupported flags are rejected |
| Time metadata | Deterministic synthetic stat times; timestamp mutation/preservation is not promised; `touch` is unsupported |
| Links | Existing snapshot links stay opaque; guest link creation/following is unsupported in this profile |
| Scheduling | Background/parallel syntax is run sequentially in the virtual interpreter, not as concurrent filesystem mutations; no host job-control claim |
| Coprocess/FDs | Bounded synchronous virtual `coproc`; process-substitution `/dev/fd` access is unsupported |
| Extras | RealFs, HTTP/network, SSH, Python/TS bridges, SQLite, arbitrary host binaries, `env`, `tar` and other excluded builtins are unavailable |
| Streams | Raw bytes are authoritative for filesystem/`cat` streams; text builtins use Bashkit's UTF-8 string model |

This is not every upstream builtin or flag. In particular `cp`/`mv` accept plain
operands, and `rm` has the narrow supported recursive/force contract. There is no
source-text security scanner: guards inspect actual resolved command arguments and
all filesystem access still crosses the policy gateway.

Replacing an existing opaque symlink quarantines the old entry even when its target
text is unchanged. Before any workspace mutation, commit checks that the staged link
can reproduce the requested metadata. A host-specific symlink mode that cannot be
reproduced fails early with the original workspace intact; VSH does not silently
normalize link permissions or follow the link to change its target's mode.

`printf` accepts literal text, `%s` and `%%` only. Variable assignment (`-v`),
precision, width, `%c`, `%b` and
other conversions are rejected rather than silently changing binary output.
Numeric byte escapes in `printf`/`echo` (`\\x..`, octal forms) are rejected because
upstream's text formatter can lose or replace bytes. Use `cat`/`cp` for binary data.
Do not rely on Bash variables/arguments for arbitrary binary strings or on full Bash
ANSI-C quoting compatibility. This restriction is explicit, not silent data repair.

## Outputs, errors and limits

`BashResult` contains `profile`, `exit_code`, `stdout: bytes`, `stderr: bytes`.
`receipt.stdout_bytes` and `stderr_bytes` expose those same authoritative bytes.
The convenience text properties decode with replacement; never use them to hash,
approve, round-trip or compare binary evidence. Rust uses `ExecutionOutput::Bash`;
the Monty branch retains its typed value and UTF-8 stdout.

```python
from vsh import VshBashError

try:
    runtime.preview("printf partial > failed.txt; exit 7", language=Language.BASH)
except VshBashError as error:
    diagnostics = error.diagnostics
    print(diagnostics.kind, diagnostics.exit_code, diagnostics.changes)
    # failed.txt was not applied; diagnostics have no commit handle.
```

`ExecutionBudget` retains shared program, wall-time, filesystem-call, I/O, path,
directory, active-evidence and output bounds. `BashLimits` adds interpreter-work
ceilings for work units, aggregate intermediate input, live intermediate bytes,
commands, single/total loop iterations and nested parser operations.
Shared execution limits additionally bound output, recursion and memory.
`BashConfig` also bounds active workers (default
4), idle workers (default 4), and an optional host watchdog. Zero idle disables reuse;
active must be positive and idle cannot exceed active. Queue waiting consumes the
watchdog. An idle process is retained only after a clean reset; its next call still
constructs a fresh interpreter.

Async capability/hook execution runs native work off the event loop. Cancellation
joins that work and retires its worker. If cancellation wins before commit entry,
user files remain unchanged; an unseen ephemeral receipt is dropped and durable
automatic approval is revoked into pending review. If commit already entered, VSH
finishes its recovery protocol and returns its actual outcome, not a false rollback.
Synchronous host loaders are cooperative: cancellation is checked when they return.

## Capability, judge, MCP and CLI

```python
from vsh import BashConfig, HookDecision, HookScope
from vsh.pydantic_ai import VshCapability

capability = VshCapability(
    "./workspace",
    bash=BashConfig(),
    hook_handler=lambda event: HookDecision.review("confirm the actual changed paths"),
    hook_scope=HookScope.ALL_REQUESTS,
)
result = await capability.vsh_run("printf draft > report.txt", "draft a report", language="bash")
assert result.requires_review
```

The filesystem tools remain Monty-powered; the existing `vsh_run` tool gains a
host-enabled language selector. A judge is wired as `judge.hook_handler`, not as the
handler itself. `review_instructions` extend its evidence-first instructions. The
judge sees the bound execution context, canonical before/after diff, effects,
dependencies, policy and explicitly permitted content. It cannot override native
denials or stale/commit checks. Review/reject feedback withholds guest results and
both streams from the main agent. See the [guided judge tutorial](../tutorials/pydantic-ai-judge.md).

For MCP/CodeMode, set `VSH_ENABLE_BASH=1` in the **server host environment** before
starting the server. `VSH_BASH_WORKER` is optional with a wheel. The tool schema
advertises only host-enabled languages; model arguments cannot enable a backend.
MCP's Bash output includes bounded base64 stdout/stderr and an explicit truncation
flag. Display truncation does not change the sealed native output or approval identity.

```bash
vsh run --workspace ./workspace --enable-bash --language bash --code 'printf verified > report.txt' --mode preview
```

For one-shot auto mode, replace `preview` with `auto`. An auto-approved CLI preview
is process-local and cannot be promoted by a later CLI process; use a live SDK or
MCP runtime. Run `examples/native/bash_workflow.py` for a disposable, binary-safe
preview/commit plus deterministic capability review. Rust's compiled counterpart is
`crates/vbash/examples/bash_workflow.rs`.

## Guided Bash review workflow

1. Install the wheel, or follow the source-checkout worker setup above. Then run
   `python examples/native/bash_workflow.py`. It creates a disposable workspace,
   previews a binary-safe `cat` redirection, checks that the host file is still absent,
   and commits that exact receipt. It then enables strict policy and a deterministic
   handler on a capability. This requires no model credentials or paid calls.
2. Start with a narrow host-owned decision rule. Inspect `event.canonical_diff`,
   `event.effects`, `event.evidence_complete` and `event.execution_context`; do not approve
   solely because the caller says “cleanup.” A read-only request can also reach a
   strict review handler. Returning `HookDecision.review(...)` preserves the pending
   transaction and gives the main agent concrete feedback without releasing streams.
3. For an independent model reviewer, run `examples/native/bash_judge.py` only after
   configuring authenticated `VSH_MAIN_MODEL` and `VSH_JUDGE_MODEL` IDs. This optional
   live example makes model requests against a disposable `service.toml` fixture.
   Its agent uses Bash `sed -i`; the judge must verify that the canonical final config
   retains authentication. The example authorizes content forwarding for that one
   path, uses strict policy, and supplies the adapter as `judge.hook_handler`.

The live example is not a deployment security policy. Set your own path/content
permissions, model usage limits and trusted review rules before adapting it to a real
workspace. Tests exercise an equivalent `FunctionModel` reviewer without network or
LLM cost. Bash never receives a special approval path: all of these examples finish
through the native policy, binding, stale-dependency and single-use commit checks.
