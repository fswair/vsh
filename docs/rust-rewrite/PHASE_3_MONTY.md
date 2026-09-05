# Phase 3 Monty typed-call integration

Checked: 2026-09-05

## Delivered production path

`vsh-monty` consumes Monty 0.0.22's public `OsFunctionCall` and suspension APIs directly. Production
execution runs in the supervised `vsh-monty-worker` child; the parent alone owns the
`VirtualFs` transaction and answers typed calls. The interpreter receives no host
mount, environment inheritance, process API, socket, or commit authority.

The implemented filesystem surface includes:

- existence and kind predicates,
- text/byte reads, writes, and appends,
- metadata and deterministic directory listing,
- lexical `absolute`/`resolve` inside the synthetic namespace,
- `mkdir`, including `parents`/`exist_ok`,
- `unlink`, `rmdir`, and subtree-aware rename,
- open-time read checks, truncation, and append-create semantics,
- synthetic `HOME`/`PWD` plus explicit `getenv`/`environ`,
- denied clock calls until a transaction-frozen clock is supplied.

Every typed OS-call effect is attributed as `EffectOrigin::MontyOsCall`. Store
corruption, stale lazy content, and internal path invariants stop execution outside
Python exception handling; expected virtual filesystem errors retain Python exception
semantics.

Monty programs also receive ten high-level functions in their initial typed input feed:
`vsh_read`, `vsh_write`, `vsh_list`, `vsh_mkdir`, `vsh_remove`, `vsh_move`,
`vsh_copy`, `vsh_glob`, `vsh_search`, and `vsh_patch`. The parent services each
function-call suspension directly against the caller-owned active `VirtualFs` and marks
its effects as `EffectOrigin::MontyToolCall`. These functions and `pathlib` therefore
observe one copy-on-write overlay, one read/write/effect ledger, one policy, and one
budget. There is no nested runtime, snapshot, transaction, MCP callback, JSON layer, or
host filesystem fallback.

## Namespace and authority

Monty sees `/workspace`; core VSH retains only relative `VPath` values. Relative paths
and `/workspace/...` map to that single namespace. Other absolute paths, drive/UNC
forms, NULs, traversal, and overlong paths cannot become host paths. Boolean predicates
return false for out-of-namespace inputs; read/write operations fail with a virtual
`PermissionError`.

Directory results and file handles carry canonical synthetic absolute paths. No adapter
method can commit, resolve a host root, read host environment variables, spawn a
process, or open a socket.

## Isolation and supervision

`vsh-monty-worker` uses Monty's public `MontyRun`/`RunProgress` seam and official
protobuf messages without enabling `monty-proto/worker`. That official feature and the
published `monty-runtime` binary were rejected because their 0.0.22 dependency graph
selects yanked `chacha20 0.10.0`. VSH neither forks Monty nor admits the yanked graph.

The parent verifies the exact worker version, clears its environment, uses piped typed
messages, applies a wall-clock watchdog, bounds diagnostics, kills/reaps failed workers,
and only returns a worker to the short-lock idle pool after a successful reset. Limit,
protocol, crash, timeout, and reset failures discard the process. `InProcessMonty`
remains available only as an explicitly selected trusted/test harness.

## Independent limits

The worker's limited allocator enforces a process-wide memory ceiling. The parent
independently enforces:

- source bytes, bytecode duration, recursion depth, and wall-clock duration,
- typed OS-call plus high-level VSH-function count and total worker-event count,
- per-call and cumulative read/write bytes,
- path bytes and returned directory entries,
- output, result, exception, frame, and diagnostic bytes.

Frame kind and declared length are classified before nested protobuf decoding. An
oversized result is therefore rejected as `ResultBytes` without allocating or decoding
the contained Monty object. A hard host limit stops without resuming the suspended
program, so sandbox code cannot catch and continue past it.

The current append implementation materializes existing immutable content before
writing the next blob. Its bytes count against the read budget. Chunked/rope-style
append remains measurement-gated because it must preserve canonical diff identity.

## Verification

The in-process adapter suite covers the complete typed mapping, high-level VSH function
composition, shared-overlay visibility, path/per-call/cumulative I/O limits, protected
reads and writes, recursive-delete preflight, namespace denial, and exact diff
semantics. Ten
end-to-end subprocess tests additionally cover:

- exact typed VFS execution and clean warm reuse,
- native `Runtime` auto-commit through the worker,
- catchable protected-read denial without secret materialization,
- output overflow followed by process discard,
- oversized result rejection before object decode,
- bounded floods of tiny print events,
- wall-timeout kill/reap followed by a fresh successful worker,
- rejection of an executable reporting the wrong Monty version,
- high-level VSH functions and `pathlib` observing the same active overlay through a
  cleanly reused worker, and
- a large valid VSH function payload using the typed-call frame budget instead of the
  smaller control-message limit.

The resolved graph has no known applicable RustSec/CVE advisory and no yanked package;
see `DEPENDENCY_POLICY.md` for the dated evidence and the transparent lock-only embedded
target caveat.

## Remaining measurement work

The production isolation boundary and warm-worker reuse are implemented. Release-mode
cold-start, warm-checkout, reset, discard, steady-state throughput, and RSS distributions
still need recorded benchmark baselines on each supported platform. Those measurements
may tune pool and budget defaults, but may not weaken typed authority or dependency
policy.
