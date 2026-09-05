# Where VSH fits

Use VSH when you need to compute a filesystem change, understand its consequences,
and apply exactly that reviewed result despite possible input drift.

## Multi-service configuration migrations

Discover a bounded set of service manifests, verify each expected old value, patch
them, and return a small before/after bundle. Commit only after checking both scope
and content. If a developer edits one of the observed inputs before application, the
old preview must not silently overwrite it.

The [Python migration recipe](../python/examples.md#bulk-configuration-migration)
demonstrates cap+1 discovery, occurrence checks, exact path assertions and promotion.
VSH does not parse arbitrary TOML schemas for you; choose transformations whose
semantics your program explicitly verifies.

## Staged generation and release manifests

Copy templates, patch generated content, rename staged files and write an index in one
program. Every step can read the previous step's virtual output. The host sees none
of those user-file changes until commit.

The same [staged release recipe](../python/examples.md#staged-release-generation)
runs through [Rust](../rust/examples.md). It also shows why a rename may need approval
even when final changes are only creations. Running compilers, package managers or
network deployment commands remains outside the guest.

## Coding agents and editor assistants

An agent can express a complete bounded edit as one Monty program. This reduces
tool-schema breadth and tool round trips; loops, search and validation stay within
one transaction. It does not guarantee a particular token-price reduction or make
agent output trustworthy.

Use [MCP](../integrations/mcp.md) for protocol integration or a
[trusted SDK wrapper](../integrations/agents.md) for fixed roots, enforced budgets,
preview cleanup and independent approval. Review returned content as untrusted data,
not as instructions to the reviewer.

## Reviewed automation that survives restart

Strict policy persists mutation previews as pending artifacts. An authenticated
review service can bind a short approval window, reopen the same runtime configuration,
and commit the exact transaction. The runtime provides binding and revalidation; your
service supplies authentication and reviewer authorization.

See [approval and lifecycle](../guides/transactions.md). Auto-approved previews are
process-local and are not the right durable review queue.

## Bounded workspace analysis

Read files and return a typed summary without proposing changes: inventory, literal
search, config consistency or migration readiness. Keep result objects small and
discard auto-approved analysis previews. For large codebases, narrow the configured
workspace; a narrow guest glob alone does not avoid full snapshot metadata traversal.

## Choose something else for

- A general POSIX shell, subprocess execution, network calls or unrestricted CPython packages.
- Long-running scientific computation or unbounded data processing.
- A container/VM security boundary, tenant isolation or arbitrary host code execution.
- Transactions spanning unrelated roots, databases and external services.
- Transparent live views of concurrent host edits; each request uses a captured base.

VSH does not infer human intent. Deterministic policy can enforce scope and risk
thresholds, but only a trusted reviewer or application can decide whether an otherwise
valid change is the right one.
