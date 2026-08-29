# Rust rewrite Phase 2 — immutable snapshot and VirtualFs

Implemented: 2026-08-28

## Delivered core

- `vsh-store::BlobStore`
  - BLAKE3 content addressing,
  - sharded immutable layout,
  - same-directory temporary write, file sync, atomic rename, and read-time hash
    verification,
  - deduplication and corruption fail-closed behavior.
- `vsh-vfs::SnapshotBuilder` / `BaseSnapshot`
  - canonical path-ordered manifest identity,
  - validated directory parents,
  - eager metadata and eager or lazy content,
  - first-read capture that accepts bytes only when both surrounding `FileStamp`
    observations equal the snapshot stamp and byte length still matches,
  - one shared capture across concurrent snapshot readers.
- `vsh-vfs::VirtualFs`
  - `exists`, `metadata`, `read`, `read_link`, `read_dir`, `write`, `append`,
    `mkdir`, `unlink`, `rmdir`, `remove_tree`, and `rename`,
  - copy-on-write overlay with component-aware subtree whiteouts,
  - opaque symlinks that are never followed implicitly,
  - operation-owned effect ledger,
  - base `ReadObservation` and `WritePrecondition` sets,
  - canonical path-ordered diff and domain-separated digest,
  - full descendant expansion for subtree deletes,
  - transaction-local size/work metrics.

The crate has no host commit API. Apart from immutable blob installation, all writes
remain inside the transaction overlay.

## Correctness invariants covered

1. Base snapshot nodes never change due to virtual mutations.
2. A lazy blob becomes immutable after exactly one stable capture.
3. Stamp or byte-length drift fails the read; stale bytes are never returned.
4. Reads of file content and directory listings retain commit-time dependencies.
5. Every written host path retains its original base precondition.
6. Create-then-delete and same-content writes disappear from canonical diff while
   remaining visible in the observed effect ledger.
7. Recursive delete expands every base descendant in the canonical diff.
8. Deleting or renaming a subtree hides stale base and overlay descendants even if its
   root directory is recreated later.
9. Subtree membership uses normalized path components, not string prefixes or fragile
   lexicographic range assumptions.
10. Repeated canonical-diff generation is stable, including lazy renamed content.

## Property/model gate

The std-only deterministic generator executes 48 mixed operations for each of 128
seeds. For every sequence it:

1. applies operations to `VirtualFs`,
2. derives canonical diff,
3. applies that diff to an independent flat base-state model,
4. compares the model with materialized final virtual state,
5. derives the diff again and verifies exact equality.

No property-testing crate was added; this avoids another runtime/dev dependency while
retaining deterministic reproduction by seed. More adversarial generators will be
added with Monty call integration.

## Touched-state performance evidence

A 10,002-node snapshot test modifies one file. The diff reports:

```text
candidate_paths: 1
expanded_delete_paths: 0
changed_paths: 1
materialized_after_bytes: 7
```

Normal file updates therefore do not scan the base manifest. Recursive deletion and
rename deliberately scale with their affected subtree because policy, revalidation,
and commit must account for every changed node; hiding that work would recreate the
old massive-delete bug.

## Dependency delta

The only new direct crate is the exact pin `blake3 =1.8.7` with default features
disabled and only `std` enabled. The resolved lockfile has no duplicate versions.
`cargo audit` and the strengthened all-graph `cargo deny` policy passed after the
addition with no advisory waiver.

## Deferred to owning phases

- The capability-rooted host `ContentLoader` adapter belongs to Monty/host integration.
  Phase 2 defines and verifies its before/read/after contract but does not grant host
  authority itself.
- Host revalidation and commit remain forbidden until the trusted committer phase.
- Rename is correctness-equivalent delete/create in canonical diff. A compact rename
  annotation is performance/UX work only and may not become a correctness dependency.
- Persistent snapshot indexes, mmap, compression, Merkle DAGs, and parallel hashing
  remain benchmark-gated; none is pre-authorized as a dependency.
