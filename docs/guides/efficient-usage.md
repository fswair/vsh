# Use VSH efficiently

Optimize the work you ask the engine to do before increasing resource ceilings.
The [measured performance report](../performance.md) separates warm execution, cold
startup, native stages and memory; use the same distinctions in your application.

## 1. Choose a small workspace boundary

Every run traverses the configured workspace's metadata. Content capture is lazy,
but metadata traversal is not. A runtime rooted at one service can be substantially
cheaper than one rooted at a monorepo containing build and dependency trees.

The root must still contain all required inputs and outputs. Do not move it merely
to evade policy. There are no implicit `.gitignore` exclusions. Narrowing
`vsh_search(path=...)` reduces execution work, not snapshot capture.

## 2. Reuse the runtime; batch dependent operations

Open a runtime once per stable trusted workspace/configuration and reuse it. Put a
coherent read → transform → validate → write workflow in one request. This pays for
one snapshot, one Python/native crossing and one policy decision, and lets later
steps read earlier virtual writes.

Do not concatenate unrelated jobs into an enormous transaction. Larger diffs retain
more evidence, require more review and revalidation, and can exceed hard limits.
A batch boundary should be a meaningful unit that can be reviewed independently.

## 3. Use high-level calls where they eliminate round trips

In the current checkout, `vsh_glob`, `vsh_search`, `vsh_copy` and `vsh_remove` perform
bounded traversal in the Rust parent against the active overlay. One high-level call
uses one worker suspension; a guest `pathlib` loop can require many suspensions.

High-level calls are not free: reads, writes, directory entries and policy checks still
count. Use `pathlib` when it expresses an operation naturally; use the high-level
functions for bounded compound work. Both have the same security boundary.

## 4. Bound discovery without silently missing work

Glob/search stop at `max_results` and do not return a truncation flag. A result count
equal to the cap is ambiguous. For a complete batch of at most 20 paths:

```python
files = vsh_glob('**/*.toml', path='/workspace/services', max_results=21)
assert len(files) <= 20, 'Too many files; choose a smaller reviewed scope'
```

Search is literal, not regex, and returns the first occurrence on each matching line.
`vsh_patch(count=n)` replaces **up to** n occurrences; assert the original occurrence
count when exact cardinality matters. See the [function reference](../integrations/monty-tools.md).

## 5. Return evidence, not a whole repository

Prefer counts, selected paths and bounded before/after snippets to full recursive
listings or file contents. Compact receipts reduce projection and transport size;
they do not skip canonical diff computation, policy or dependency binding.

Use full detail for review and verify intended content separately. Python's
`result_repr` constructs a full representation on access; use `result` when you need
structured data. MCP/CLI truncate representations after conversion, so their text cap
is not a parent-process allocation budget.

## 6. Finish the preview lifecycle

Commit or discard an auto-approved preview using the same live runtime. This includes
read-only analysis. The default cache is fail-closed at 64 artifacts or 128 MiB encoded
artifact bytes; those bytes are not a whole-process RSS limit. Duplicate exact pending
identities are rejected. Discard before repeating an identical analysis.

Raw MCP does not expose a discard tool. Its 16-entry runtime LRU can also evict a
runtime and lose auto-approved handles. Use an SDK-owned wrapper for high-rate analysis
or durable review services, and deliberately manage lifecycle and concurrency.

## 7. Measure and tune independent limits

Record `timings_ns()` in Python or `StageTimings` in Rust together with path/byte/call
counts. Inspect p50 and tail latency on representative data. Distinguish warm worker
reuse from first-call startup, and measure process-tree memory separately from latency.

Guest heap and bytecode duration limits do not bound the complete parent process or
end-to-end wall time. Apply service-level deadlines, admission control and process
resource isolation in the trusted host as needed. Do not disable revalidation,
durability or protected-path checks to meet a latency target.

## Practical cost model

| Cost | Reduce it by | Do not assume |
|---|---|---|
| Snapshot metadata | Smaller trusted workspace | Glob scope excludes snapshot nodes |
| Worker round trips | One program; compound VSH functions | One tool call means constant work |
| CPU and allocations | Bounded traversal and small results | Rust removes I/O or serialization costs |
| Retained memory | Commit/discard; limited concurrency | Guest heap equals total RSS |
| Storage | Bounded artifacts and operational retention planning | Discard is a general blob-store garbage collector |
| Agent context | Small schemas and concise review evidence | Local benchmarks establish token-price savings |
