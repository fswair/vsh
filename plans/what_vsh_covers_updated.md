# vsh — What vsh Covers

Version: 2026-05-02  
Status: Updated planning document  
Audience: founder, implementer, agent, future collaborators

---

# 1. Executive summary

`vsh` is a **validation-first command engine** for agent-generated workspace operations.

It is not primarily a shell, not a bash clone, and not a virtual terminal runtime.  
Its main job is to take a command **as structured intent**, simulate that intent against a **snapshot-derived mock workspace graph**, decide whether the command should be allowed, and only then optionally execute it against the real filesystem.

In one line:

> `vsh` is the logic layer behind command execution for agent workspaces.

This document defines:

- what `vsh` is trying to become,
- what `vsh` explicitly covers,
- what `vsh` intentionally does not cover in the MVP,
- how the system is structured,
- what the first shell-like commands are,
- how those commands should be prioritized and implemented,
- and how the project should progress in 10 phases.

This version updates the older plan by preserving its core insight — `StructuredCommand`, simulation-first workflow, snapshot/manifest design, and approval before execution — while changing the implementation direction to **Python-first**, **FastMCP/MCP server exposure**, and a **CodeMode-style tool surface**. It also preserves the central claim that `vsh` should be positioned as a structured command engine rather than a bash runtime. fileciteturn1file0

---

# 2. What vsh is

## 2.1 Core identity

`vsh` is a **structured command planner, simulator, and decision engine**.

The main execution flow is:

```text
search("ls")
-> ["vsh_list"]

get_schema("vsh_list")
-> LsCommandSchema

snapshot_workspace(...)
-> snapshot_id

simulate("vsh_list", snapshot_id, params={...})
-> SimulationResult(plan_id, shell_preview, predicted_effects, decision)

approve(plan_id)
-> approval_token

execute_approved(approval_token)
-> ExecutionResult
```

The important thing here is that the system does **not** start from a shell string and then try to interpret it. The system starts from a **typed command schema**, and every later stage is derived from that canonical representation.

## 2.2 Mental model

The right mental model for `vsh` is:

- **StructuredCommand first**
- **workspace graph snapshot second**
- **simulation third**
- **policy/approval fourth**
- **real execution last**

Or more simply:

> Commands are not strings. Commands are effect plans.

## 2.3 Why this exists

Agents today often work like this:

- they inspect the workspace by repeatedly calling `pwd`, `ls`, `find`, `cat`,
- they emit shell-like text,
- they only discover mistakes late,
- they sometimes produce commands with unclear or overly broad side effects,
- and they do not reason in a typed way about what a command will do before it runs.

`vsh` exists to change that:

- command discovery should be structured,
- command schemas should be typed,
- workspace state should be represented in a machine-readable graph,
- command side effects should be predicted before the real FS is touched,
- and destructive or nonsensical commands should be rejected early.

---

# 3. What vsh is not

These boundaries are important because they protect the MVP from scope creep.

## 3.1 Not a full bash implementation

`vsh` is not trying to implement:

- full POSIX shell syntax,
- pipelines,
- subshells,
- shell arithmetic,
- shell functions,
- heredocs,
- redirections,
- variable expansion,
- command substitution,
- shell parsing parity.

A shell-like preview may exist, but shell syntax is not the canonical execution form.

## 3.2 Not a virtual bash runtime

This is not primarily a project whose value is "run bash scripts in memory".

That problem space already exists and can even be integrated later as a backend or oracle.  
`vsh` is a **control plane**, not a runtime-first interpreter.

## 3.3 Not a VCS

`vsh` is not trying to replace:

- git history,
- commits,
- staging,
- merge,
- diff review.

It cares about command intent and workspace state, not repository history.

## 3.4 Not an LSP platform in MVP

In later phases `vsh` may host analyzers such as:

- Python import resolution checks,
- Pyright/basedpyright validation,
- TypeScript module checks,
- syntax verification,
- incomplete-edit detection.

But the MVP should not wait for any of that.

## 3.5 Not a content-heavy shadow of the whole workspace

The snapshot model is deliberately **schema-first**, not content-first.

The system should not begin by reading all file contents into memory.  
It should begin by snapshotting the **workspace graph**, node metadata, and opaque content references.

---

# 4. Product vision

## 4.1 Near-term vision

Build a working MVP where an agent can:

1. discover available commands,
2. ask for a schema,
3. receive a current workspace snapshot graph,
4. simulate a command on that graph,
5. see predicted effects and touched nodes,
6. approve or reject that command,
7. and then optionally execute it.

## 4.2 Medium-term vision

`vsh` becomes the command decision layer for agent workspaces.

That means:

- fewer blind shell calls,
- fewer accidental destructive operations,
- better visibility into command effects,
- deterministic planning before mutation,
- reusable tool surface over MCP,
- and eventually semantic analyzers on top.

## 4.3 Long-term vision

In its strongest form, `vsh` could evolve into:

- a command registry for agent workspace operations,
- a simulation engine for effect prediction,
- a control plane for approval and policy,
- a backend-agnostic executor,
- and a plugin host for language-specific post-simulation analysis.

But none of those long-term ideas are allowed to block the MVP.

---

# 5. What vsh covers

This section is the core scope definition.

## 5.1 vsh covers these layers

### Layer A — Command discovery

The agent should not memorize the entire tool surface.

`vsh` covers:

- command search,
- command schema retrieval,
- command examples,
- command metadata,
- command categories,
- command capability lookup.

This is why the initial external surface should look like:

- `search(query)`
- `get_schema(name)`

### Layer B — Structured command modeling

`vsh` covers the typed representation of commands.

Examples:

- `PwdCommand`
- `CdCommand`
- `LsCommand`
- `MkdirCommand`
- `TouchCommand`
- `MoveCommand`
- `CopyCommand`
- `RemoveCommand`

Every supported command must have:

- a schema,
- a typed model,
- a simulation implementation,
- a shell-like renderer,
- and optionally an execution implementation.

### Layer C — Workspace snapshot graph

`vsh` covers the schema snapshot of a workspace.

That includes:

- file/directory/symlink nodes,
- path relationships,
- cwd state,
- parent-child relationships,
- node metadata,
- opaque content references,
- node revision/state,
- and agent-facing projections of current cwd.

### Layer D — Simulation

`vsh` covers command simulation against the snapshot graph.

That includes:

- path resolution,
- state transitions on an overlay,
- tracking which nodes were visited,
- tracking which nodes were mutated,
- predicting side effects,
- and generating a simulation plan.

### Layer E — Decision and policy

`vsh` covers whether a command should be allowed.

It includes:

- rejecting invalid paths,
- rejecting out-of-bounds workspace escape,
- rejecting destructive operations that violate local policy,
- marking approval requirements,
- and returning clear reject reasons.

### Layer F — Optional real execution

`vsh` covers applying approved plans to the real filesystem.

However:

- real execution is downstream of simulation,
- not the source of truth,
- and must use the approved plan rather than fresh params.

### Layer G — MCP/CodeMode-facing tool surface

`vsh` covers being exposed as a small agent-facing toolset over MCP.

That includes:

- tools,
- resources,
- command discovery,
- simulation results,
- and structured machine-readable workspace views.

---

# 6. What vsh does not cover in the MVP

To keep the project real, the MVP explicitly excludes the following:

- shell parsing,
- command chaining,
- pipes,
- redirections,
- subshells,
- environment variable expansion,
- heredocs,
- shell functions,
- language-aware semantic analyzers,
- LSP integration,
- full-content workspace mirroring,
- arbitrary code execution sandboxing,
- full POSIX/GNU compatibility,
- and exhaustive command parity.

These may appear later, but they are not part of MVP acceptance.

---

# 7. System design

## 7.1 Top-level architecture

```text
LLM / Client
  -> MCP tools/resources
  -> vsh registry
  -> structured command schema validation
  -> workspace snapshot graph
  -> simulation engine
  -> access journal + predicted effects
  -> policy decision
  -> plan store
  -> optional real executor
```

This is deliberately layered so that:

- discovery is independent from execution,
- simulation is independent from real filesystem mutation,
- policy is independent from command schema,
- and future analyzers can sit on top rather than distort the core.

## 7.2 Canonical object flow

```text
search(query)
-> CommandSpec
-> get_schema(name)
-> StructuredCommand(params)
-> simulate(snapshot)
-> SimulationResult(plan_id, preview, journal, effects, decision)
-> approve(plan_id)
-> execute_approved(token)
```

The critical design rule is:

> `execute(name, params)` is not the main path.

The correct path is:

> `simulate -> approve -> execute_approved`

---

# 8. Data model and class definitions

This section is intentionally concrete.

## 8.1 Base command schema types

```python
from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Literal, Optional
from pydantic import BaseModel, Field

CommandName = Literal[
    "vsh_pwd",
    "vsh_cd",
    "vsh_list",
    "vsh_mkdir",
    "vsh_touch",
    "vsh_move",
    "vsh_copy",
    "vsh_remove",
]
```

## 8.2 Structured command

```python
class StructuredCommand(BaseModel):
    tool_name: CommandName
    version: int = 1
    params: dict[str, Any]
```

Purpose:

- canonical command payload,
- immutable input to simulation,
- source for shell preview,
- source for execution plan.

## 8.3 Concrete command schemas

```python
class PwdCommandSchema(BaseModel):
    physical: bool = False


class CdCommandSchema(BaseModel):
    path: str
    physical: bool = False


class LsCommandSchema(BaseModel):
    path: str = "."
    all: bool = False
    long: bool = False
    one: bool = False
    recursive: bool = False


class MkdirCommandSchema(BaseModel):
    path: str
    parents: bool = False
    mode: Optional[str] = None


class TouchCommandSchema(BaseModel):
    path: str
    no_create: bool = False


class MoveCommandSchema(BaseModel):
    sources: list[str]
    dest: str
    overwrite: Literal["always", "never"] = "always"


class CopyCommandSchema(BaseModel):
    sources: list[str]
    dest: str
    recursive: bool = False
    overwrite: Literal["always", "never"] = "always"


class RemoveCommandSchema(BaseModel):
    paths: list[str]
    recursive: bool = False
    force: bool = False
```

These classes exist so that:

- schemas are concrete,
- JSON Schema can be exported over MCP,
- and each command gains a typed identity before simulation.

## 8.4 Command registry

```python
class CommandSpec(BaseModel):
    name: CommandName
    summary: str
    tags: list[str] = Field(default_factory=list)
    mutates_fs: bool = False
    requires_approval: bool = False
    supports_execute: bool = False
    schema_json: dict[str, Any]
    examples: list[dict[str, Any]] = Field(default_factory=list)
```

Purpose:

- searchable catalog entry,
- source for `search`,
- source for `get_schema`,
- source for MCP resources like `commands://spec/<name>`.

## 8.5 Snapshot node

```python
NodeKind = Literal["file", "dir", "symlink"]


class SnapshotNode(BaseModel):
    path: str
    parent: str | None = None
    kind: NodeKind
    children: list[str] = Field(default_factory=list)

    size: int | None = None
    mode: int | None = None
    mtime_ns: int | None = None

    content_ref: str | None = None
    revision: int = 0
```

Purpose:

- represent a workspace node,
- without requiring real file contents,
- while still supporting graph reasoning.

## 8.6 Workspace snapshot

```python
class WorkspaceSnapshot(BaseModel):
    snapshot_id: str
    workspace_root: str
    cwd_logical: str
    cwd_physical: str
    generated_at_ns: int
    nodes: dict[str, SnapshotNode]
```

Purpose:

- immutable base view of the workspace,
- source for projections,
- source for simulation,
- source for later revalidation.

## 8.7 Overlay mutations

```python
class Overlay(BaseModel):
    created: dict[str, SnapshotNode] = Field(default_factory=dict)
    updated: dict[str, SnapshotNode] = Field(default_factory=dict)
    deleted: set[str] = Field(default_factory=set)
    renames: list[tuple[str, str]] = Field(default_factory=list)
    cwd_override: str | None = None
```

Purpose:

- represent hypothetical changes during simulation,
- allow rollback for free,
- support deterministic effect prediction,
- avoid mutating the base snapshot.

## 8.8 Access journal

```python
class AccessJournal(BaseModel):
    metadata_reads: set[str] = Field(default_factory=set)
    content_reads: set[str] = Field(default_factory=set)
    creates: set[str] = Field(default_factory=set)
    deletes: set[str] = Field(default_factory=set)
    metadata_writes: set[str] = Field(default_factory=set)
    content_writes: set[str] = Field(default_factory=set)
    renames: list[tuple[str, str]] = Field(default_factory=list)
    cwd_changes: list[str] = Field(default_factory=list)
```

Purpose:

- capture what the command touched,
- support policy and approval,
- support future explainability,
- support future actual-vs-predicted comparisons.

## 8.9 Predicted effects

```python
class PredictedEffects(BaseModel):
    reads: list[str] = Field(default_factory=list)
    creates: list[str] = Field(default_factory=list)
    updates: list[str] = Field(default_factory=list)
    deletes: list[str] = Field(default_factory=list)
    renames: list[tuple[str, str]] = Field(default_factory=list)
    cwd_change: str | None = None
```

Purpose:

- user-facing and agent-facing effect summary,
- policy input,
- approval payload,
- later execution comparison target.

## 8.10 Decision model

```python
DecisionStatus = Literal["approve", "reject", "approve_with_warning"]


class PolicyDecision(BaseModel):
    status: DecisionStatus
    reason: str | None = None
    warnings: list[str] = Field(default_factory=list)
```

Purpose:

- encode final simulation decision,
- make reject reasons explicit,
- support soft approval in future.

## 8.11 Simulation result

```python
class SimulationResult(BaseModel):
    plan_id: str
    command: StructuredCommand
    shell_preview: str
    decision: PolicyDecision
    predicted_effects: PredictedEffects
    journal: AccessJournal
```

Purpose:

- immutable output of the simulator,
- approval input,
- execution plan basis,
- audit/debug record.

## 8.12 Approval token

```python
class ApprovalToken(BaseModel):
    token: str
    plan_id: str
    approved_at_ns: int
```

Purpose:

- bind execution to an already-reviewed plan,
- prevent param drift,
- preserve lifecycle integrity.

## 8.13 Execution result

```python
class ExecutionResult(BaseModel):
    status: Literal["ok", "failed"]
    plan_id: str
    actual_effects: PredictedEffects | None = None
    error: str | None = None
```

Purpose:

- capture execution status,
- report actual effects,
- later compare to predicted effects.

---

# 9. Snapshot strategy

## 9.1 Snapshot philosophy

A snapshot is **not** a full content mirror.

A snapshot is a **schema/graph model** of the workspace:

- paths,
- parent/child structure,
- node kinds,
- metadata,
- opaque content identity,
- cwd state.

## 9.2 Why not store all file contents?

Because the MVP needs:

- fast workspace discovery,
- fast command simulation,
- cheap reject paths,
- low memory overhead,
- and deterministic graph operations.

Reading megabytes of content into memory is not needed for:

- `pwd`
- `cd`
- `ls`
- `mkdir`
- `touch`
- `mv`
- `rm`
- many `cp` cases

## 9.3 Opaque content references

Instead of content hashes, the MVP should store opaque content refs such as:

```text
opaque:/repo/src/main.py:4123:1714760000000000000
```

These exist only to say:

- the file exists,
- we know its identity at snapshot time,
- but we do not claim to know its content yet.

## 9.4 Agent-facing projection

The snapshot should also be projected into a smaller, easier-to-read current-view resource.

Example:

```json
{
  "workspace_root": "/repo",
  "cwd": "/repo/src",
  "entries": [
    {"path": ".", "kind": "dir"},
    {"path": "./main.py", "kind": "file", "size": 1200},
    {"path": "../README.md", "kind": "file", "size": 4000}
  ]
}
```

This should be the discovery surface the agent uses instead of repeated `ls` spam.

---

# 10. Shell compatibility philosophy

This section matters because the older plan leaned heavily toward shell implementation priority.

The updated rule is:

> vsh should implement **command semantics first**, shell compatibility second.

## 10.1 What this means

For every command, the canonical implementation order is:

1. define the command schema,
2. define the structured semantics,
3. define path/precondition behavior,
4. define predicted effects,
5. define simulation behavior,
6. define optional execution behavior,
7. only then define shell-like rendering.

## 10.2 Shell rendering is still useful

Even though shell strings are not canonical, they remain useful for:

- debug,
- user trust,
- approval screens,
- logs,
- and reasoning about familiar command intent.

Example:

```json
{
  "tool_name": "vsh_move",
  "params": {
    "sources": ["src/a.py"],
    "dest": "src/b.py",
    "overwrite": "always"
  }
}
```

can render to:

```bash
mv src/a.py src/b.py
```

But the renderer is only a projection.

## 10.3 Compatibility target

The compatibility goal for early commands is:

- POSIX-inspired semantics,
- selected GNU-compatible behavior where useful,
- deterministic internal behavior,
- explicit unsupported cases,
- and no silent compatibility lies.

If a behavior is not supported, the command should reject clearly rather than pretend.

---

# 11. Initial commands and implementation priority

These commands are the first important surface.

## 11.1 Priority group A — navigation/discovery

### `vsh_pwd`
Why first:

- validates session cwd model,
- simplest read-only command,
- foundational for all others.

### `vsh_cd`
Why second:

- makes session state real,
- validates path resolution,
- establishes logical cwd semantics.

### `vsh_list`
Why third:

- replaces a large fraction of early agent discovery behavior,
- reveals whether the snapshot graph is useful,
- and becomes the first truly visible value of the system.

## 11.2 Priority group B — first mutations

### `vsh_mkdir`
Why:

- simplest structured create operation,
- low ambiguity,
- good first mutation test for overlay.

### `vsh_touch`
Why:

- validates create/update split,
- introduces metadata mutation without complex content handling.

## 11.3 Priority group C — mutation core

### `vsh_move`
Why:

- critical real-world operation,
- validates rename semantics,
- useful for refactor workflows.

### `vsh_copy`
Why:

- validates duplication semantics,
- useful for backup and patch workflows,
- required before many agent edits become safe.

### `vsh_remove`
Why:

- validates destructive operation policy,
- forces clear approval semantics,
- essential for trust in the engine.

## 11.4 Group D — second wave reads

These come after the core navigation/mutation engine works:

- `cat`
- `grep`
- `find`
- `head`
- `tail`
- `wc`
- `sort`

They matter, but they depend on the workspace graph model and may later interact with optional lazy content hydration.

---

# 12. MCP and CodeMode-style tool surface

## 12.1 Why expose over MCP?

Because `vsh` should be consumed as a tool surface by LLM clients rather than as a giant prompt contract.

MCP is a natural fit because it lets the system expose:

- tools,
- resources,
- and machine-readable schemas.

## 12.2 Why CodeMode-style discovery?

Because the model should not be forced to ingest every possible command schema up front.

Instead:

- search for the command,
- ask for the schema,
- then simulate.

That gives a compact discovery-first interaction model.

## 12.3 Tool set

### `search`
Search available commands.

### `get_schema`
Return the JSON schema, metadata, and examples for a command.

### `snapshot_workspace`
Create a workspace snapshot graph and return a `snapshot_id`.

### `simulate`
Validate params, simulate the command, and return a `SimulationResult`.

### `approve`
Approve a previously simulated plan.

### `execute_approved`
Execute an already-approved plan against the real filesystem.

## 12.4 Resources

### `workspace://snapshot/current`
The full snapshot graph.

### `workspace://projection/current`
The current cwd-oriented lightweight projection.

### `commands://spec/<name>`
The metadata/schema/spec card for a command.

### `simulations://<plan_id>`
The last simulation details for a specific plan.

---

# 13. Repository shape

Recommended Python package layout:

```text
vsh/
  README.md
  pyproject.toml
  src/vsh/
    __init__.py
    app.py
    registry/
      __init__.py
      specs.py
      search.py
    schemas/
      __init__.py
      pwd.py
      cd.py
      ls.py
      mkdir.py
      touch.py
      move.py
      copy.py
      remove.py
    commands/
      __init__.py
      pwd.py
      cd.py
      ls.py
      mkdir.py
      touch.py
      move.py
      copy.py
      remove.py
    snapshot/
      __init__.py
      models.py
      builder.py
      projection.py
    session/
      __init__.py
      state.py
      resolver.py
    simulate/
      __init__.py
      overlay.py
      journal.py
      effects.py
      engine.py
      policy.py
      renderer.py
    execute/
      __init__.py
      realfs.py
      revalidate.py
      plans.py
    mcp/
      __init__.py
      server.py
      tools.py
      resources.py
  tests/
```

---

# 14. Detailed 10-phase implementation plan

This is the main implementation roadmap.

## Phase 1 — Define the external contract and registry

### Goal
Create the stable outer shape of `vsh` before any serious execution logic exists.

### Why this phase exists
Without a stable contract, the project will drift:

- command names will be unstable,
- schemas will change constantly,
- MCP exposure will become inconsistent,
- and the system will stop feeling like a command engine.

### Deliverables
- Python project scaffold
- `CommandSpec`
- `StructuredCommand`
- registry implementation
- `search(query)`
- `get_schema(name)`
- schema classes for:
  - `vsh_pwd`
  - `vsh_cd`
  - `vsh_list`

### Internal design decisions frozen in this phase
- command discovery is registry-first,
- schemas are Pydantic-based,
- every command has a canonical name,
- schemas are retrievable independently of execution.

### What this unlocks
The model can now discover commands and reason about them without raw prompt stuffing.

### Acceptance criteria
- registry contains at least 3 commands,
- `search("ls")` works,
- `get_schema("vsh_list")` returns JSON schema,
- examples exist for each command.

---

## Phase 2 — Build the workspace snapshot graph

### Goal
Create a filesystem snapshot model that captures structure, not content.

### Why this phase exists
This is the first thing that makes `vsh` different from a direct shell wrapper.  
Without a graph snapshot, there is nothing to simulate against.

### Deliverables
- `SnapshotNode`
- `WorkspaceSnapshot`
- recursive snapshot builder
- ignore configuration
- opaque `content_ref`
- `workspace://snapshot/current`
- `workspace://projection/current`

### Internal design decisions frozen in this phase
- snapshots are schema-first,
- full content is not loaded,
- cwd state is part of snapshot context,
- projections are separate from the full graph.

### What this unlocks
The agent can inspect a workspace as data rather than by issuing repeated shell commands.

### Acceptance criteria
- snapshot JSON can be generated,
- cwd projection is correct,
- ignored directories are excluded,
- graph relationships are consistent.

---

## Phase 3 — Implement path resolution and read-only simulation

### Goal
Produce the first real `SimulationResult` objects.

### Why this phase exists
This is where `vsh` stops being a catalog and starts becoming a simulator.

### Deliverables
- logical path resolver
- absolute/relative path normalization
- workspace escape detection
- `Overlay`
- `AccessJournal`
- `PredictedEffects`
- `simulate()` for:
  - `vsh_pwd`
  - `vsh_cd`
  - `vsh_list`
- shell preview rendering

### Internal design decisions frozen in this phase
- commands simulate against immutable snapshot + overlay,
- journals are first-class output,
- shell preview is derived from command state,
- reject reasons are explicit.

### What this unlocks
This is the first moment where the following is true:

- the agent can discover a command,
- request its schema,
- take a snapshot,
- simulate the command,
- and receive an approval/reject decision.

### Acceptance criteria
- `pwd` simulates cleanly,
- `cd` updates simulated cwd,
- invalid `cd` rejects,
- `ls` reads from projection/graph,
- journal is populated.

---

## Phase 4 — Add immutable plans and approval lifecycle

### Goal
Make simulation results durable and approval-driven.

### Why this phase exists
Without this phase, it is too easy to drift back into direct execution.

### Deliverables
- `PlanStore`
- `SimulationResult` persistence
- `approve(plan_id)`
- `ApprovalToken`
- shell preview standardization
- predicted-effect summary standardization

### Internal design decisions frozen in this phase
- execution can only happen from an approved plan,
- approval refers to a plan, not fresh params,
- simulation results must be reproducible.

### What this unlocks
You now have an actual control plane rather than a raw simulator.

### Acceptance criteria
- plans are stored,
- approval token is issued,
- the token links to a specific simulation result,
- params cannot be mutated at execution time.

---

## Phase 5 — Add first mutation simulation (`mkdir`, `touch`)

### Goal
Prove that the engine can simulate safe writes.

### Why this phase exists
Until this phase, the system has only demonstrated read and navigation behavior.
Mutation is what turns it into a workspace command engine.

### Deliverables
- `vsh_mkdir`
- `vsh_touch`
- create/update behavior on overlay
- metadata revision updates
- create/write journal population
- decision rules for invalid paths and parents

### Internal design decisions frozen in this phase
- create/update semantics live in overlay,
- no real FS mutation occurs during simulation,
- command effects are explicit and inspectable.

### What this unlocks
The system now models hypothetical writes and can be used to gate them.

### Acceptance criteria
- `mkdir` simulation creates overlay nodes,
- `touch` simulation updates or creates correctly,
- invalid parent path rejects,
- effects summary is meaningful.

---

## Phase 6 — Add real executor v1

### Goal
Apply approved plans to the real filesystem for the first safe commands.

### Why this phase exists
A command engine that can never execute approved plans is incomplete as a workflow tool.

### Deliverables
- `RealFsExecutor`
- `execute_approved()`
- execution support for:
  - `pwd`
  - `cd`
  - `ls`
  - `mkdir`
  - `touch`
- pre-execution revalidation
- `ExecutionResult`

### Internal design decisions frozen in this phase
- no direct `execute(name, params)` path,
- execution always follows simulation,
- revalidation happens before mutation,
- actual effects are reported separately.

### What this unlocks
End-to-end lifecycle exists for safe commands.

### Acceptance criteria
- approved `mkdir` executes on the real FS,
- approved `touch` executes on the real FS,
- stale conditions fail clearly,
- execution result is structured.

---

## Phase 7 — Add mutation core (`move`, `copy`, `remove`)

### Goal
Support the first serious workspace-transforming commands.

### Why this phase exists
This is the point where the tool becomes useful for real agent work rather than only setup and discovery.

### Deliverables
- `vsh_move`
- `vsh_copy`
- `vsh_remove`
- overwrite policy
- delete safety rules
- subtree handling for limited recursive operations
- rename tracking in journal and effects

### Internal design decisions frozen in this phase
- destructive commands require explicit policy treatment,
- recursive behavior remains intentionally narrow,
- unsupported cases reject instead of approximating silently.

### What this unlocks
The command engine can now express and gate the majority of basic workspace mutations.

### Acceptance criteria
- move/copy/remove simulate correctly,
- dangerous deletes reject,
- overwrite behavior is explicit,
- journals show correct node coverage.

---

## Phase 8 — Add drift detection and targeted refresh

### Goal
Prevent execution from relying on stale assumptions.

### Why this phase exists
Between simulation and execution, the real filesystem may change.

### Deliverables
- touched-path fingerprint model
- targeted revalidation
- stale snapshot detection
- targeted snapshot refresh
- mismatch diagnostics

### Internal design decisions frozen in this phase
- full resnapshot is not required for every execution,
- only touched paths need strict revalidation,
- stale plans are invalid plans.

### What this unlocks
More trustworthy execution and better future concurrency behavior.

### Acceptance criteria
- stale touched path is detected,
- stale plan cannot execute silently,
- refresh path works for changed nodes.

---

## Phase 9 — Expose the full MCP/CodeMode-style server

### Goal
Make `vsh` usable by external LLM clients over MCP.

### Why this phase exists
The command engine only becomes broadly useful when it is exposed as a proper tool surface.

### Deliverables
- FastMCP server
- stdio serving
- MCP tools:
  - `search`
  - `get_schema`
  - `snapshot_workspace`
  - `simulate`
  - `approve`
  - `execute_approved`
- MCP resources:
  - `workspace://snapshot/current`
  - `workspace://projection/current`
  - `commands://spec/<name>`
  - `simulations://<plan_id>`

### Internal design decisions frozen in this phase
- MCP is discovery-first,
- the tool surface remains small,
- resources are used for stateful context rather than bloating tool responses.

### What this unlocks
Codex-like and IDE-like clients can consume `vsh` directly.

### Acceptance criteria
- MCP server can be started locally,
- a client can call `search`, `get_schema`, and `simulate`,
- resources can be read.

---

## Phase 10 — Harden, document, and prepare future analyzer hooks

### Goal
Turn the MVP into a stable foundation rather than a loose prototype.

### Why this phase exists
Without this phase, the project remains a fragile demo.

### Deliverables
- test fixtures for snapshots and simulations
- golden tests for core commands
- docs:
  - vision
  - architecture
  - command model
  - snapshot model
  - MCP contract
- future interfaces for:
  - lazy content hydration
  - Python semantic analyzer
  - TypeScript semantic analyzer
  - shadow workspace

### Internal design decisions frozen in this phase
- the MVP boundary is explicit,
- future advanced capabilities live behind extension points,
- analyzers are optional layers on top of the core.

### What this unlocks
A stable launch point for future versions.

### Acceptance criteria
- core flows are documented,
- tests cover the main lifecycle,
- future hooks exist without polluting the MVP.

---

# 15. First commands to fully finish before anything advanced

Before adding analyzers, LSP, or content-heavy logic, the following commands should be complete end-to-end:

- `vsh_pwd`
- `vsh_cd`
- `vsh_list`
- `vsh_mkdir`
- `vsh_touch`

Then:

- `vsh_move`
- `vsh_copy`
- `vsh_remove`

Only after that should second-wave reads be considered.

---

# 16. Strategic conclusion

The old plan correctly established that the project should be treated as a **structured command engine** rather than a bash runtime, and that `StructuredCommand`, snapshot simulation, approval before execution, registry-driven command discovery, and command spec cards are the right backbone. It also correctly emphasized that shell repr is only a projection and that simulation and execution should share the same semantic contract. fileciteturn1file1

What changes in this updated plan is the implementation direction:

- **Python-first**, not Rust-first,
- **FastMCP/CodeMode-style tool exposure** in the MVP,
- **small MCP surface** with `search`, `get_schema`, `snapshot_workspace`, `simulate`, `approve`, and `execute_approved`,
- **graph snapshot mock FS**, not full in-memory content mirroring,
- and **read-only/navigation plus first mutations** as the first meaningful milestone.

If the first 3 phases are completed successfully, `vsh` will already prove its core insight:

- structured command generation,
- current cwd/workspace snapshot tree,
- mock simulation on a snapshot graph,
- access journaling,
- and approval/reject behavior.

That is enough for a real MVP.
