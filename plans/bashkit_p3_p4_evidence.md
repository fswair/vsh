# Bashkit integration — mode and private worker checkpoint

Recorded: 2026-10-01. Workspace baseline: `8550facbe94485cb12bfe007fbb3575a168b6afc`.
Authority: [system design](bashkit_system_design.md) and [implementation plan](bashkit_implementation.md).

This records implemented, locally verified preparation. It does not close the complete P3/P4 acceptance gates, enable public Bash, or authorize publication. The test counts below are historical checkpoint counts, not the latest full-workspace results. See the later [evidence/artifact checkpoint](bashkit_p6_evidence.md) for current checks and the [P2 record](bashkit_p2_evidence.md) for the original gateway measurements.

## Implemented ownership and worker boundary

- `vsh-execution` owns the borrowed, policy-aware `FsGateway`; Monty and the private Bash transport use the same filesystem authority, budgets and path mapping.
- The sole `VirtualFs`, overlay, read/write dependencies and ordered effect ledger stay in the parent. The child is a filesystem RPC proxy, not a copied transaction or a host-filesystem mount.
- `vsh-bash` has separate host and worker features. The production worker build is `--no-default-features --features worker`; its dependency graph excludes the VSH store, commit, VFS and runtime crates.
- Bashkit is exactly `0.18.2`, with default features disabled; Tokio is exactly `1.53.1`, with only runtime/time support. The default `vsh` production graph includes neither Bashkit nor Tokio.
- The parent checks the worker version, frame bounds, direction, session, sequence, terminal result and readiness. Frame/direction, per-I/O, path and combined-output bounds are checked before allocating nested payloads.
- A request gets fresh interpreter state: variables, cwd, functions, history and environment do not leak across reuse. `HOME` is `/workspace`; synthetic metadata timestamps are the Unix epoch.
- A bounded worker pool supports independent transactions. IPC writes have an acknowledgement/deadline, oversized stderr is terminal, and failed or malformed workers are killed and reaped.
- Only a complete zero-exit run with a verified ready message yields a private successful outcome. Guest-caught policy denials remain in evidence; guest-caught limit/profile failures cannot become successful proposals.

The child process boundary is not an OS sandbox for a compromised worker binary. No arbitrary host executables, network or RealFs features are enabled by this integration.

## Mode changes and durable commit

- `VirtualFs::set_mode` preserves content identity and lazy backing while recording a typed metadata effect. Root changes, symlinks and bits outside `0777` are rejected; an unchanged mode is an observed no-op.
- Existing-node permission changes are a policy risk and require review under the balanced policy. Metadata-only changes charge zero changed content bytes. Execute-bit risk compares the exact execute mask.
- File metadata commits pin a descriptor, reject multiply linked targets, preserve the file inode/data/mtime, and record the post-operation stamp. Blob-backed evidence is streamed into a hash rather than allocated as a complete file.
- `VSHCMT02`/`VSHLOG02` are used only for the new file-mode operation and its full post-stamp witness. Ordinary plans still use the old format; old readers and recovery fixtures are tested.
- Recovery verifies inode ownership and expected content before undoing a mode. It does not rewrite content and does not mutate an unrelated replacement file. Already undone file modes are treated as undone after restart.
- Existing-directory owner-write grants are lowered before content operations, parent first; restrictive mode changes are lowered afterward, deepest directory first. Rollback restores traversable parent permissions before undoing child operations. Descriptor/completion identities and unexpected third modes are checked without overwriting an unrelated host change.

### Deliberately unsupported mode states

The current portable commit protocol must be able to reopen and verify committed targets. The plan is rejected before host mutation when these preconditions cannot be met:

- A file must retain owner read or owner write access (`0600` mask nonzero). A blob-backed file must additionally retain owner read access for verification.
- A directory must retain owner read and search (`0500`). Newly created/replaced directories must retain owner write as well (`0700`) because installation uses an ownership marker.
- Existing directory transitions also require a reopenable original mode.

Consequently `chmod 000`, unreadable blob-backed final files, and newly created read-only directories are not yet supported. Stamp-backed write-only file mode transitions are supported and covered by recovery tests. These are explicit temporary containment restrictions, not a claim of complete POSIX chmod support. Safe platform-specific pinning and staged directory installation remain future work within the planned gate.

## Tests and quality checks

Historical complete local checks at the initial mode/worker checkpoint:

| Check | Result |
| --- | --- |
| Rust workspace, all features and targets, locked/offline | 227 tests passed; none ignored |
| Rust Clippy, workspace/all features/all targets, `-D warnings` | Passed |
| Rust formatting | Passed |
| Rebuilt release PyO3 extension, CPython 3.14.6 | Passed |
| Six current Python API/runtime/hook/capability suites | 153 tests passed |
| Python line and branch coverage for `src/vsh` in those suites | 100% (564 statements, 152 branches) |
| Cargo audit with refreshed RustSec data | No vulnerability advisory; existing maintenance warning below |
| Cargo deny, locked/all features, refreshed data | Passed |

Focused Rust coverage includes 33 commit tests, 29 gateway tests, four budget tests, 22 VFS tests and 15 Bash protocol/subprocess tests. The standalone upstream contract probe has 16 passing tests from this integration work. Adversarial cases include hard links before and during commit, same-content inode replacement, write-only rollback, interrupted rollback followed by restart, read-only final parent plus child creation, malformed workers, wrong identity/session/direction, output/heap/work limits, sticky denials and fresh pooled state.

This is a local macOS/arm64 checkpoint. Linux/Windows hosted results are not available for these changes. Current Rust coverage is recorded in the later [evidence/artifact checkpoint](bashkit_p6_evidence.md); the historical P2 percentage must not be presented as current coverage.

## Dependency policy

The all-feature combined graph needs narrow, version-specific duplicate exceptions for Monty's versus Bashkit's `getrandom` and `fancy-regex` lines. The default Monty-only consumer does not acquire the Bash dependencies. `libbz2-rs-sys@0.2.5` has a package-specific `bzip2-1.0.6` license exception; the license and Bashkit's MIT notice are retained in `THIRD_PARTY_NOTICES.md`. There is no global license allowance or vulnerability ignore.

Audit still reports the existing `atomic-polyfill@1.0.3` maintenance warning, `RUSTSEC-2023-0089`, in the all-target dependency graph. Passing these checks is not a guarantee that a dependency can never have a CVE.

## Open gates and next work

1. **Complete Bash profile:** upstream `coproc` executes virtually and sequentially, but no supported hook rejects it inside all nested scripts. A profile decision has been requested. Strict `coproc`-off is not currently proven; public Bash stays disabled.
2. **Resource semantics:** filesystem capacity-dependent operations cannot claim real quota metadata. `df` and its command wrappers fail closed; RPC usage queries are sticky unsupported. The inherited upstream `limits()` is not a truthful complete quota report; ordinary glob prefilters require it, so making it universally fatal is not valid.
3. **Parent work deadlines/cancellation:** channel waits and writes are bounded, but synchronous gateway traversal/lazy loaders are not preempted by the worker watchdog. Request-scoped cancellation and cooperative parent checks remain unfinished.
4. **P2 performance:** small no-op and large-delete p95 measurements have not met the stated regression gate. Previous timings were noisy; quiet interleaved control measurements and measured remediation are still needed.
5. **Mode platform/coverage:** unsupported portable permission transitions and non-local platform tests remain open. No new coverage exclusion or weakened floor was added.
6. **Public result/evidence integration:** Monty execution-evidence sealing and V3 artifacts are now implemented, with active parent-evidence bounds. The runtime has not yet dispatched Bash or persisted binary Bash output; complete P5/P6 integration is still followed by PyO3/capability/MCP work and adversarial final validation.

Independent read-only reviews have informed the descriptor, hard-link, rollback, protocol allocation, environment and quota fixes. Review is not a substitute for platform tests, and no phase is marked complete solely on a source inspection.
