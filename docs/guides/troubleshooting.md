# Troubleshooting

Start with the typed error or receipt decision, the exact transaction ID, stage timings
and resource counts. Keep program output and file contents out of general logs when
they can contain sensitive project data.

| Symptom | Likely explanation | Safe next step |
|---|---|---|
| `vsh_*` is undefined | Installed release predates the new integration | Build matching checkout artifacts, or use supported `pathlib` |
| Worker missing or handshake rejected | Missing, mismatched or untrusted worker path | Deploy the worker built with the extension/crate; inspect `VSH_MONTY_WORKER` |
| Import cannot load native module | Wrong interpreter/architecture or no compiled extension | Install a matching wheel or rebuild for the active interpreter |
| `pending_approval` | A risk flag or strict/paranoid mutation | Obtain independent trusted approval; do not downgrade policy to bypass it |
| `denied` after catching `PermissionError` | Protected access remains in the host ledger | Remove the prohibited operation; catching it cannot erase the event |
| `VshExecutionError` | Compilation, unsupported operation or hard execution budget | Inspect the cause and scope; do not remove all limits |
| `VshStateError` | Missing artifact, lifecycle/approval/replay/capacity failure | Check runtime lifetime, exact configuration, approval and whether already consumed |
| `VshStaleError` | Recorded host dependency changed | Preserve the external edit; create and review a fresh preview |
| Preview cannot see a prior preview's file | Each request starts a fresh snapshot | Put dependent stages in one program |
| Repeated previews eventually fail | Bounded cache or duplicate exact identity | Commit or discard auto-approved artifacts; include read-only analysis |
| Slow scan despite a narrow glob | Full workspace metadata snapshot remains | Narrow the trusted runtime root; inspect snapshot timing |
| No `changes` but nonzero `changed_paths` | Compact projection | Request full detail; diff computation still occurred |
| Output appears cut off in MCP | Adapter text projection was truncated | Check truncation flags and return a smaller result |
| Recovery conflict or orphan | Runtime cannot prove safe ownership | Stop automation and inspect with an operator; do not delete runtime state blindly |

## Diagnose the installed engine

```python
import vsh

print(vsh.__version__, vsh.engine_kind())
```

This reports the Python package version and native engine identity. A development build can
share the package version while containing unreleased code: record the Git revision,
lockfile and worker/extension hashes for reproducible investigations.

## A stale transaction is not a retry token

Commit reservations are single-use. Do not loop on the same transaction until it
works, or resubmit source automatically after drift. Re-snapshot, recompute and review
the changed proposal. See the [executable stale-input recipe](../python/examples.md#stale-input-rejection).

## Recovery requires care

`Runtime.open` performs startup recovery; SDK `recover()` returns counts and conflicts.
A `cleanup_pending` committed receipt means verified application succeeded but durable
cleanup remains. Keep its identity and inspect recovery, rather than rerunning the
transformation and applying it twice.

Do not treat all exceptions as proof that no host changes happened: a commit failure
after application started may require journal recovery. Preview isolation and commit
recovery are separate guarantees. See [transactions](transactions.md) and [security](../security.md).
