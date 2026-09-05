# VSH functions inside Monty

!!! note "Available in VSH 0.4.0"

    These functions use the Monty 0.0.22 integration. Python wheels bundle the matching
    worker; Rust deployments must provide it as described in the
    [installation instructions](../start/index.md).

Every program submitted through the Rust SDK, Python SDK, or `vsh_run` receives ten
high-level filesystem functions. They are ordinary callable names inside the Monty
sandbox—not additional MCP tools.

```python
vsh_mkdir('/workspace/generated')
vsh_write('/workspace/generated/status.txt', 'ready\n')

matches = vsh_search('TODO', path='/workspace/src', max_results=20)
files = vsh_glob('**/*.toml', path='/workspace')
{'matches': len(matches), 'manifests': len(files)}
```

## One active virtual filesystem

The functions operate directly on the caller-owned active Rust `VirtualFs`. `pathlib`
and VSH functions in the same program see each other's writes immediately:

```python
from pathlib import Path

vsh_write('/workspace/value.txt', 'one')
assert Path('/workspace/value.txt').read_text() == 'one'

Path('/workspace/value.txt').write_text('two')
assert vsh_read('/workspace/value.txt') == 'two'
```

There is no nested runtime or second snapshot. A call does not create an inner
transaction, call back through MCP, serialize through JSON, or gain a host filesystem
handle. The outer VSH request still owns preview/commit behavior, the canonical diff,
policy, budgets, revalidation, and the final receipt.

They are injected guest callables: do not write `from vsh import vsh_read` in the
program or call them as Python SDK host methods. A later outer `preview()` gets a
fresh snapshot; it cannot read this program's uncommitted files.

In subprocess mode the function objects are sent with the program's initial typed
Monty feed. A function invocation uses Monty's existing function-call suspension and is
handled by the parent that already owns `VirtualFs`. This keeps one worker round trip per
high-level call and avoids an extra name-lookup round trip.

## Function reference

### `vsh_read`

```text
vsh_read(path, binary=False) -> str | bytes
```

Read one regular file. Text mode decodes strict UTF-8 and returns `str`; binary mode
returns `bytes`. `path` accepts a string or `pathlib.Path`.

### `vsh_write`

```text
vsh_write(path, data, append=False) -> int
```

Write or append `str`/`bytes`. The return value is the number of Unicode characters for
text or bytes for binary data. The parent directory must already exist. Append reads the
existing immutable content into the copy-on-write overlay, so both the read and write
byte ceilings apply.

### `vsh_list`

```text
vsh_list(path='/workspace') -> list[Path]
```

Return the immediate children of one directory as canonical absolute virtual paths.
Results are deterministic and protected children are hidden before metadata reaches the
sandbox.

### `vsh_mkdir`

```text
vsh_mkdir(path, parents=True, exist_ok=True) -> None
```

Create a directory. The ergonomic defaults create missing parents and accept an
existing directory. Set either flag to `False` for strict `mkdir` behavior. Every
created parent is policy-authorized before the overlay is mutated.

### `vsh_remove`

```text
vsh_remove(path, recursive=False, missing_ok=False) -> None
```

Remove a file, symlink, or empty directory. Set `recursive=True` for a directory tree.
Recursive removal walks and authorizes the complete tree before deleting anything, so a
protected descendant or budget failure leaves the tree unchanged. `missing_ok=True`
makes an absent target a successful no-op.

### `vsh_move`

```text
vsh_move(source, destination) -> Path
```

Rename a file or subtree inside the virtual workspace and return the destination as an
absolute virtual `Path`. The destination parent must exist. Source and destination
capabilities are checked independently.

### `vsh_copy`

```text
vsh_copy(source, destination, recursive=False, overwrite=False) -> Path
```

Copy one file or, with `recursive=True`, a directory tree. A file destination may be
replaced only with `overwrite=True`. Directory-tree merge/overwrite and symlink copies
are deliberately rejected. Recursive copy preflights source reads, destination writes,
directory-entry counts, and byte budgets before creating the destination tree. A
destination may not equal or sit inside its source.

### `vsh_glob`

```text
vsh_glob(pattern, path='/workspace', max_results=1000) -> list[Path]
```

Recursively discover paths below `path` using a relative pattern:

- `*` matches zero or more characters inside one path component;
- `?` matches one character inside one component;
- `**` matches zero or more complete components.

Character-class syntax is unsupported: brackets match literally. Absolute and
parent-traversing patterns are rejected. Use `*.rs` for
direct children and `**/*.rs` for every depth. Results are canonical and deterministic.
Traversal stops as soon as `max_results` matches have been collected; `0` validates the
root without walking it.

There is no truncation flag. To prove a migration has at most N matches, request
N+1 and assert the returned length is at most N. A narrow `path` reduces guest
traversal, not the initial whole-workspace snapshot scan.

### `vsh_search`

```text
vsh_search(
    query,
    path='/workspace',
    case_sensitive=True,
    max_results=100,
) -> list[dict]
```

Search UTF-8 regular files recursively, or search one file when `path` names a file.
The search is literal and reports the first match on each matching line. Each item has
`path`, one-based `line`, one-based Unicode `column`, and the complete line in `text`.
Binary/non-UTF-8 files and policy-hidden files are skipped. Case-insensitive mode uses
Unicode lowercase matching while preserving columns from the original source text.
Traversal and file reads stop when `max_results` is reached.

### `vsh_patch`

```text
vsh_patch(path, old, new, count=1) -> int
```

Replace exact UTF-8 text and return the replacement count. `old` must be non-empty,
`count` must be positive, and a missing match is an error rather than a silent no-op.
The operation uses the same bounded read/write path as `vsh_read` and `vsh_write`.

`count` is a maximum, not an expected occurrence count. For an exact migration,
check `vsh_read(path).count(old)` before patching and assert the returned count.

## Paths, policy, and limits

Use `/workspace/...` or a relative virtual path. Other absolute namespaces, traversal,
NULs, drive/UNC forms, and overlong paths never become host paths.

Each high-level invocation consumes one `max_os_calls` suspension slot. Parent-side
recursive work is bounded by read/write, per-call I/O, path and directory-entry limits,
as well as the captured snapshot's limits. Guest bytecode duration and worker heap
limits are **not** a total wall-time or parent-memory bound for this work.
Function effects are recorded as
`MontyToolCall`; `pathlib` effects are recorded as `MontyOsCall`. Both contribute to the
same canonical diff and receipt counters.

Expected argument and virtual-filesystem failures are normal Python exceptions and may
be caught inside the program. A protected capability attempt raises `PermissionError`
and is also retained outside the sandbox; catching it cannot erase the denial from the
final transaction policy. Hard host limits, protocol failures, corruption, and worker
failures terminate the request and cannot be caught to continue past the boundary.

## Choosing VSH functions or `pathlib`

Use `pathlib` for familiar single-file Python logic. Prefer VSH functions when one
bounded call expresses the work more directly—recursive copy/remove, bounded discovery,
literal search, or exact patching. Mixing them is safe because they share the same
overlay.

For agent workloads, keep the complete related operation in one `vsh_run` program. The
agent discovers one MCP schema, crosses MCP/PyO3 once, and performs compact high-level
operations inside the supervised worker. Always inspect the returned diff and policy
decision before promoting a preview.
