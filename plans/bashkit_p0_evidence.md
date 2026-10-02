# Bashkit feasibility evidence

Measured 2026-09-30 on macOS 26.1 / arm64, 8 logical CPUs, Python 3.14.6.
VSH baseline: `8550facbe94485cb12bfe007fbb3575a168b6afc`.
Bashkit: exact `0.18.2`, default features off; tag commit `567511386572d4c4b9f649ab1da97aad53b9f341`. No fork, host filesystem or host process resolver.

## Reproduction

The isolated [contract probe](../scripts/bashkit_contract_probe/Cargo.toml) contains assertions against the actual upstream filesystem, dispatch and output contracts. It is not a production VFS or another shell implementation.

```sh
cargo test --manifest-path scripts/bashkit_contract_probe/Cargo.toml --locked -- --nocapture
cargo build --manifest-path scripts/bashkit_contract_probe/Cargo.toml --locked
scripts/bashkit_contract_probe/target/debug/vsh-bashkit-feasibility within
scripts/bashkit_contract_probe/target/debug/vsh-bashkit-feasibility exceed
scripts/bashkit_contract_probe/target/debug/vsh-bashkit-feasibility thread-exceed
scripts/bashkit_contract_probe/target/debug/vsh-bashkit-feasibility reset
scripts/bashkit_contract_probe/target/debug/vsh-bashkit-feasibility bash-loop
scripts/bashkit_contract_probe/target/debug/vsh-bashkit-feasibility bash-sleep
cargo audit --file scripts/bashkit_contract_probe/Cargo.lock
uv run --frozen --no-sync python benchmarks/native_pyo3.py --iterations 200 --cold-iterations 30 --output target/bashkit-evidence/p0/monty-baseline.json
```

`exceed` and `thread-exceed` must terminate with exit code 65. They intentionally allocate beyond the cap in a separate process; never install this allocator into the Python extension or parent runtime. Generated evidence stays under ignored `target/`; the probe lock/source is durable.

## Results and contract consequences

Fourteen upstream contract tests passed, including the three additional namespace/nested-dispatch assertions added during extraction. The pre-extraction workspace baseline passed 166 tests. Shared-gateway verification is recorded separately; it must not be mistaken for production Bash enablement.

| Observation | Consequence |
| --- | --- |
| Pipelines, sequential background append, `xargs -P`, and `sed -i` use the injected FS | Bashkit owns shell semantics; VSH supplies the authoritative FS |
| Atomic `sed -i` calls chmod on a sibling temp before rename; 0600 is preserved | VSH needs a metadata-only mode primitive, not another chmod builtin |
| A write before final `false` remains in virtual state | Final nonzero is noncommittable; no implicit partial-success approval |
| Protected read followed by `|| true` can end with exit 0 | Parent policy denials are sticky and must reach review/decision evidence |
| Before-dispatch guard sees `parallel` through `command`, functions, `eval` and nested `sh`; an unresolved alias is seen by the deny-only resolver | Keep both observers; neither executes host commands |
| Builder `max_file_size` did not constrain the custom filesystem | Parent gateway must enforce real quotas; upstream default-FS limits are not inherited |
| An identity `after_tool` hook changed bytes `ff 00 fe` into UTF-8 replacement characters | No output-rewriting hooks; byte-preserving streams are authoritative |
| Nested `sh` pipeline/capture with an 8-byte upstream output cap wrote 8 of 16 bytes, exit 0, final truncation flags false | Final flags are insufficient; disable these truncating knobs |
| With truncating knobs disabled and a bounded intermediate cap, the same nested pipeline preserved all 16 bytes; an oversized intermediate produced an error even with `|| true` | Use fail-on-limit work/allocation/output bounds, never silently truncated program input |
| One MiB allocator session cap rejected an 8 MiB allocation both on the main thread and a new thread with exit 65; reset/re-enable succeeded | Existing exact-pinned worker allocator can enforce a process-wide session ceiling |
| A 20 ms loop timeout returned at about 120 ms; `sleep 10` stopped around 26 ms | Cooperative timing is insufficient; enforce parent wall deadline and kill/reap |
| `/dev/null` input/output redirects succeeded without calling the injected FS | They are interpreter operations: bound guest work/allocation/time, do not claim FS accounting or fabricate workspace effects |
| Executable workspace scripts, `source` and `.` retained the `parallel` tool guard; handled `./missing` did not invoke the command resolver | Guard actual dispatch; missing slash-path scripts are ordinary filesystem failures, not resolver-observed external commands |
| `before_exec` ran once for a program containing both `eval` and nested `sh` | Outer-source validation does not cover every nested parse; disabling AST-only constructs remains an open production-profile gate |

Scratch allocator process startup was approximately 4–7 ms after warm loading. This is not a production worker/startup benchmark or an RSS cap. The production memory contract still requires baseline + request budget + bounded headroom, handshake and reset tests.

## Monty baseline

The existing native PyO3 benchmark ran 200 warm / 30 cold iterations with its isolated workspace fixtures. No live LLM calls. Values below are milliseconds, not optimization claims.

| Case | p50 | p95 |
| --- | ---: | ---: |
| No-op | 0.144 | 0.159 |
| Read 10 files | 0.777 | 1.013 |
| Edit 20 files | 1.604 | 2.690 |
| Cold runtime open | 37.250 | 48.373 |
| First call | 7.584 | 8.970 |

Independent-runtime concurrency speedup in that run: 4.25×. Preserve the same fixtures/build/machine for paired comparison; no performance improvement has been established yet.

## Dependency / remaining gates

The isolated lock resolved 133 packages, including the probe; `cargo audit --no-fetch --no-yanked` reported no advisory findings against RustSec database commit `9b3a3b73a7f42606494c943e95f8196e9994df46` (1,277 entries). That invocation did not check yanked versions. The latest release API was rechecked at implementation time and returned 0.18.2.

Still required: fresh combined-workspace audit/deny/feature inspection when the worker dependencies are added; Linux/macOS commit-mode conformance; complete source/recursive/path/device/profile matrix against the production gateway; nested work-budget enforcement; real protocol malformed-input/cancellation/reset tests; paired Monty performance evidence. Windows Bash remains disabled. These findings authorize a staged extraction, not public Bash enablement or a release.
