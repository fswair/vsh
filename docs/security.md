# Security model

VSH protects a host workspace from unreviewed filesystem effects by removing ambient
authority, simulating through typed capabilities, binding approval to exact evidence,
and revalidating before one trusted writer commits.

## Trust boundaries

| Component | Trust level | Authority |
|---|---|---|
| Monty source | Untrusted | Bounded computation and typed virtual calls only |
| Supervised worker | Contained | No host mount, subprocess, network, or ambient environment |
| Rust runtime | Trusted | Snapshot, policy, state, artifact, and orchestration |
| Committer | Most privileged | Capability-rooted mutation and recovery |
| Python/MCP adapter | Boundary code | Shape conversion and error mapping; no semantics |

## Main controls

- Workspace and protected runtime/data roots are opened as capabilities and their
  identities are rechecked across observation and mutation boundaries.
- Internal symlinks, redirected storage, workspace/data overlap, and root replacement
  fail closed.
- Guest-visible paths are normalized under a synthetic `/workspace` root.
- Protected paths are denied before their names or bytes reach the worker.
- Program, intent, snapshot, dependencies, diff, policy, and configuration bind the
  transaction ID.
- Commit consumes a single-use reservation and verifies read/write preconditions.
- State logs and commit metadata are checksummed and bounded; complete corrupt frames
  fail rather than being treated as torn appends.
- Journals, plans, markers, and recovery reads use no-follow identity checks and hard
  size limits.
- Rust panics cannot unwind through PyO3.

## What approval means

Approval means a principal accepts one exact pending artifact for a bounded time. It
does not grant general workspace access, change policy, authorize reruns, or waive
revalidation.

Auto-approval is a deterministic policy result, not an AI judgement. Strict and
paranoid profiles force mutations to an independent approval boundary.

### Policy changes invalidate pending authority

Approval, commit preparation, hook resolution and direct commit require the pending
artifact's policy digest to equal the runtime's active policy digest. A profile,
protected-rule or policy-semantics change requires a fresh preview, even if the old
transaction was already approved. Rust returns `VshError::PolicyChanged`; Python
raises `VshStateError` with a fresh-preview instruction. Old lifecycle records remain
inspectable. Do not reuse the old approval for the new transaction ID.

The wildcard correction uses the `vsh-policy-v3` digest domain. `*` in a protected
pattern always means a wildcard, even when the actual filename contains literal
stars: `*.key` protects `*.key.key`, and `.env.*` protects `.env.**`. Pending artifacts
created under v2 require a new preview. This check does not interrupt recovery of an
already-entered durable commit: recovery must complete or roll back according to its
journal and ownership evidence, including after a policy change.

## Explicit non-goals

VSH is not:

- a kernel, VM, container, seccomp, or multi-tenant isolation boundary;
- a POSIX shell or subprocess sandbox;
- a network sandbox;
- an unrestricted CPython environment;
- protection against a malicious trusted host or replaced VSH binary;
- a substitute for filesystem permissions, backups, deployment rollback, or secrets
  management.

## Deployment rules

1. Pin the runtime, worker, Python wheel, and Cargo/Python locks together.
2. Keep the durable data directory trusted and separate from agent-controlled content.
3. Treat `worker_path`, `workspace_root`, policy, and maximum budgets as host config.
4. Run one of the shipped supervised worker modes for untrusted code.
5. Stop on recovery conflicts; do not automate deletion of ambiguous artifacts.
6. Retain receipts and reviewer identity according to your audit policy.
7. Run RustSec, `cargo-deny`, strict hash-locked Python audit, and artifact validation
   before release.

For attack assumptions, controls, security test families, and residual risk, read the
full [threat model](threat-model.md). The exact supported/non-supported promises are in
[Guarantees](guarantees.md).
