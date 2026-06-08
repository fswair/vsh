# vsh — Validation-First Workspace Command Engine

## Final System Design and 10-Phase Python MVP Plan

---

## 0. Document purpose

This document defines the **product vision, system design, architecture, data model, tool surface, and implementation roadmap** for `vsh`.

It is intentionally written as a **build document**, not as a brainstorm note.

The goal is to make the project executable by an engineering agent or developer without ambiguity.

This document answers all of the following:

- What `vsh` is
- Why it exists
- What problem it solves
- What it is explicitly **not** trying to be
- Why `StructuredCommand` is the center of the system
- How workspace snapshotting works
- How mock simulation works without mutating the real filesystem
- How the MCP/CodeMode-like tool surface should look
- What the Python MVP should contain
- What the first 10 implementation phases are
- What each phase unlocks technically
- What classes, modules, APIs, and acceptance criteria each phase should contain
- Which advanced ideas are intentionally deferred to later phases

This document is the canonical design reference for the MVP.

---

# 1. Product vision

## 1.1 What `vsh` is

`vsh` is a **validation-first command engine for AI agents operating on workspaces**.

At a high level, `vsh` takes commands that would normally be expressed as shell strings and instead represents them as **typed structured commands**, simulates their effects over a **workspace snapshot graph**, and decides whether the command should be allowed before touching the real filesystem.

The shortest correct description is:

> `vsh` is not the layer that blindly executes commands. It is the logic layer that decides whether a command should run.

More specifically, `vsh` is trying to become:

- a **StructuredCommand runtime**
- a **workspace graph simulator**
- a **side-effect analysis engine**
- an **approval / rejection control plane**
- an **agent-facing MCP toolset**

---

## 1.2 What problem `vsh` solves

Current agent workflows that operate over codebases and working directories have several recurring problems:

### Problem A — command strings are too lossy
Agents typically produce shell-like strings such as:

```bash
mv src/agent.py src/runtime_agent.py
```

That string hides important semantics:

- Which paths are actually read
- Whether the target exists
- Whether the operation escapes the allowed workspace
- Whether the command is destructive
- Whether the command is incomplete
- Whether the command is equivalent to its apparent intent

The shell string is readable, but it is a poor canonical representation of intent.

### Problem B — agents spam discovery commands
To understand the workspace, agents repeatedly do things like:

- `pwd`
- `ls`
- `find`
- `cat`
- `tree`

This creates noise, wasted turns, and fragile context reconstruction.

### Problem C — the agent learns too late that the command was wrong
An agent may:

- move the wrong file
- create a directory at the wrong level
- try to remove too much
- typo an import path
- rename a module without updating its dependents

Often the system only learns this **after** the real filesystem has already been changed.

### Problem D — approval comes too late or at the wrong abstraction level
Today, approval is often given to:

- a raw shell string
- a tool call
- a process execution request

But the meaningful thing to approve is actually:

- the structured intent
- the predicted side effects
- the exact set of nodes that will be touched

### Problem E — simulation and execution are usually disconnected
Many systems can either:

- execute commands
- or fake commands in a sandbox

But they do not keep one unified semantic model that allows:

- structured planning
- mock simulation
- predicted effect generation
- approval
- eventual real execution

`vsh` exists to solve these problems.

---

## 1.3 What `vsh` is not

It is critical to keep the project identity clean.

`vsh` is **not**:

### Not a full shell
It does not aim to become a full Bash/POSIX-compatible shell.

### Not a shell interpreter clone
It is not trying to faithfully reimplement shell parsing, quoting, pipes, subshells, heredocs, and all external shell semantics.

### Not a virtual bash runtime first
It may eventually support a shell-like frontend, but the product is not centered around interpreting shell syntax.

### Not a VCS
It is not Git, not JJ, not a history engine, not a commit graph.

### Not an LSP platform in the MVP
Language-aware semantic checking is important, but it is explicitly **deferred**.

### Not a sandbox container platform
A sandbox may later become an execution backend, but sandboxing is not the MVP.

### Not a general arbitrary-code execution service
The MVP is command-oriented and workspace-oriented, not arbitrary Python execution.

---

## 1.4 What `vsh` is trying to become long term

Long term, `vsh` could become a command intelligence layer for agents that can:

- expose a structured view of the workspace
- let agents plan with `StructuredCommand`
- simulate command effects without touching the real filesystem
- reject nonsensical or dangerous edits early
- later attach semantic analyzers such as Python import checks or TypeScript module checks
- eventually support multiple execution backends:
  - real filesystem
  - shadow workspace
  - sandbox runtime
  - virtual execution runtime

But the MVP is deliberately smaller.

---

# 2. Core insight

## 2.1 The key idea

The core insight behind `vsh` is:

> Shell commands should not be treated as strings first.
> They should be treated as **structured intent plus effect prediction**.

A command like:

```bash
cp -r src runtime/src
```

is not really “text”.

It is an operation with meaning:

- resolve `src`
- resolve `runtime`
- read subtree metadata
- create destination nodes
- maybe overwrite targets
- maybe recurse
- maybe preserve metadata

So the actual object of the system should not be shell text.

It should be:

- a typed command schema
- a predicted effect graph
- an access journal
- a policy decision

That is the heart of `vsh`.

---

## 2.2 Why `StructuredCommand` is the center

If `StructuredCommand` is the canonical object, several things become cleaner immediately:

### The same command can be rendered as shell
This is useful for:

- human readability
- approval UI
- logging
- debugging

### The same command can be simulated
No shell parsing round-trip is required.

### The same command can later be executed
The execution backend can consume the same structured intent.

### The same command can be analyzed
Policy rules do not have to reason over strings.

### The same command can be versioned
Schemas and semantics become explicit.

This means:

> Shell is a projection. `StructuredCommand` is truth.

---

## 2.3 Why snapshot simulation matters

The second core insight is:

> Agents do not need the full real filesystem in memory to reason about command effects.

They need a **workspace graph snapshot** that captures:

- the file tree
- node types
- parent-child structure
- enough metadata to reason about path and mutation behavior

From there, many commands can be simulated **without real content hydration**.

That allows `vsh` to answer:

- Which nodes does the command touch?
- Is the command read-only?
- Is it destructive?
- Does it escape the workspace?
- Does it create or remove nodes?
- Does it change the logical cwd?

All without mutating the real workspace.

---

# 3. MVP goals and boundaries

## 3.1 MVP goal

The MVP must prove the following thesis:

> An agent can discover commands, read schemas, snapshot the current workspace tree, simulate structured commands over that snapshot graph, observe predicted side effects, and obtain an approval decision — all before mutating the real filesystem.

This is the actual MVP.

---

## 3.2 MVP success criteria

The MVP is successful if it can do all of the following:

1. Expose a CodeMode-like discovery surface over MCP
2. Return JSON schema for commands
3. Snapshot a workspace into a graph model
4. Expose a cwd-centric projection for discovery
5. Simulate at least these commands:
   - `pwd`
   - `cd`
   - `ls`
6. Produce a journal of reads/writes/touches
7. Produce a predicted effect model
8. Decide `approve` or `reject`
9. Persist a simulation as a plan
10. Later execute approved plans for a small command subset

---

## 3.3 Explicit non-goals for MVP

The following are intentionally **not** in MVP scope:

- shell parser
- full Bash compatibility
- pipelines and redirections
- heredocs and subshells
- arbitrary process execution
- language-aware semantic analyzers
- shadow workspace semantic verification
- full file content ingestion into memory
- LSP / Pyright / TypeScript checker integration
- full symlink parity with Unix tools
- exact GNU/POSIX parity for all flags

These may come later, but must not pollute the MVP.

---

# 4. High-level architecture

## 4.1 System layers

`vsh` should be implemented as the following layered system:

```text
LLM / client
  -> MCP tool surface
    -> command registry
    -> schema provider
    -> workspace snapshot service
    -> simulation engine
    -> approval / plan store
    -> real executor
```

More concretely:

### Layer 1 — Discovery layer
For command discovery and schema inspection.

### Layer 2 — Command modeling layer
Defines structured commands as Pydantic models.

### Layer 3 — Workspace graph layer
Defines the snapshot representation of the workspace.

### Layer 4 — Simulation layer
Runs commands over a mock graph view.

### Layer 5 — Policy layer
Interprets journal + effects and decides approve/reject.

### Layer 6 — Execution layer
Executes only already-approved plans.

### Layer 7 — MCP surface
Exposes all of this through a very small tool API.

---

## 4.2 Fundamental lifecycle

The canonical lifecycle must be:

```text
search
-> get_schema
-> snapshot_workspace
-> simulate
-> approve / reject
-> execute_approved
```

Not this:

```text
search
-> get_schema
-> execute
```

The second lifecycle loses the entire value proposition of `vsh`.

---

# 5. External API model

## 5.1 Tool surface

The MCP tool surface should remain intentionally small.

### Tools

- `search(query)`
- `get_schema(name)`
- `snapshot_workspace(workspace_root, cwd)`
- `simulate(name, snapshot_id, params)`
- `approve(plan_id)`
- `execute_approved(approval_token)`

This is enough for the entire MVP.

---

## 5.2 Resource surface

Expose structured state as resources.

### Resources

- `workspace://snapshot/current`
- `workspace://projection/current`
- `commands://spec/<name>`
- `simulations://<plan_id>`

The distinction is important:

- tools trigger actions
- resources expose state / context

---

## 5.3 Why this resembles CodeMode

The reason for this design is that agents should not need every command schema in prompt context at once.

They should:

1. search for the command family they need
2. request the schema for the chosen command
3. build a valid structured command
4. simulate it
5. only then decide whether to execute

This keeps the interaction compact and agent-friendly.

---

# 6. Core data model

Below is the canonical class model for the MVP.

---

## 6.1 Command registry classes

```python
from __future__ import annotations

from typing import Any, Callable, Literal
from pydantic import BaseModel, Field


class CommandExample(BaseModel):
    title: str
    params: dict[str, Any]


class CommandSpec(BaseModel):
    name: str
    summary: str
    description: str
    tags: list[str] = Field(default_factory=list)
    mutates_fs: bool = False
    supports_execute: bool = False
    schema_model_name: str
    examples: list[CommandExample] = Field(default_factory=list)
```

### Purpose
This is the discovery-facing card for a command family.

### Why it exists
It lets the system answer:

- what the command is called
- what it does
- whether it mutates the filesystem
- whether it supports real execution yet
- which schema model it maps to
- how the model should think about using it

---

## 6.2 Structured command model

```python
class StructuredCommand(BaseModel):
    tool_name: str
    version: int = 1
    params: dict[str, Any]
```

### Purpose
This is the canonical command instance.

### Why it exists
This is the object that is:

- schema validated
- simulated
- rendered for approval
- eventually executed

In the MVP we keep it generic. Later it can become a tagged union of command-specific models.

---

## 6.3 Session state

```python
class SessionState(BaseModel):
    workspace_root: str
    cwd_logical: str
    cwd_physical: str
    oldpwd: str | None = None
```

### Purpose
Represents the command session state, independent from the workspace tree.

### Why it exists
`cd` and `pwd` are not just filesystem operations — they are session operations.

This separation avoids polluting the tree model with process/session semantics.

---

## 6.4 Snapshot graph classes

```python
class SnapshotNode(BaseModel):
    path: str
    parent: str | None
    kind: Literal["file", "dir", "symlink"]
    children: list[str] = Field(default_factory=list)
    size: int | None = None
    mode: int | None = None
    mtime_ns: int | None = None
    content_ref: str | None = None
    revision: int = 0
```

### Purpose
Represents a single node in the workspace graph.

### Key design choice
`content_ref` is not necessarily a real content hash. In the MVP it is an **opaque identity token**, not the actual file contents.

Examples:

- `opaque:/repo/src/app.py:1483:1712345678901`
- `opaque:/repo/README.md:8192:1712345679901`

This allows the system to model the tree and reason about mutations without loading file contents.

---

## 6.5 Snapshot container

```python
class WorkspaceSnapshot(BaseModel):
    snapshot_id: str
    session: SessionState
    generated_at_ns: int
    nodes: dict[str, SnapshotNode]
```

### Purpose
Represents the full graph snapshot used by the simulation engine.

### Why it exists
It is the immutable base state for all command simulation.

---

## 6.6 Overlay model

```python
class Overlay(BaseModel):
    created: dict[str, SnapshotNode] = Field(default_factory=dict)
    updated: dict[str, SnapshotNode] = Field(default_factory=dict)
    deleted: set[str] = Field(default_factory=set)
    renames: list[tuple[str, str]] = Field(default_factory=list)
    cwd_override: str | None = None
```

### Purpose
Represents hypothetical mutations made by a command during simulation.

### Why it exists
It allows simulation to be:

- deterministic
- rollbackable
- cheap
- independent from the real filesystem

### Important invariant
The base snapshot is immutable. All simulation effects live in the overlay.

---

## 6.7 Access journal

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

### Purpose
Captures exactly what the command touched during simulation.

### Why it exists
The journal is the basis for:

- approval UI
- policy checks
- debug output
- later comparison against actual execution

This is one of the most important objects in the whole system.

---

## 6.8 Predicted effects

```python
class PredictedEffects(BaseModel):
    reads: list[str] = Field(default_factory=list)
    creates: list[str] = Field(default_factory=list)
    deletes: list[str] = Field(default_factory=list)
    updates: list[str] = Field(default_factory=list)
    renames: list[tuple[str, str]] = Field(default_factory=list)
    cwd_after: str | None = None
```

### Purpose
Represents the compact, approval-facing description of what the command is expected to do.

### Why it exists
The journal is low-level. Predicted effects are the human-facing summary.

---

## 6.9 Simulation result and plan

```python
class SimulationResult(BaseModel):
    plan_id: str
    command: StructuredCommand
    shell_preview: str
    decision: Literal["approve", "reject", "approve_with_warning"]
    reason: str | None = None
    predicted_effects: PredictedEffects
    journal: AccessJournal
```

### Purpose
Represents the result of running a command over the snapshot graph.

### Why it exists
It is the object that gets:

- stored
- approved
- later executed

### Critical design rule
The plan must be immutable once created.

Approval attaches to the **plan**, not to a later ad-hoc execution call.

---

## 6.10 Approval token

```python
class ApprovalToken(BaseModel):
    token: str
    plan_id: str
    approved_at_ns: int
```

### Purpose
Represents an approved simulation artifact that may be executed.

### Why it exists
It prevents accidental drift between simulation and execution.

---

# 7. Snapshotting model

## 7.1 What “snapshot” means in `vsh`

In `vsh`, snapshotting **does not** mean loading the entire real filesystem and all file contents into memory.

It means creating a **workspace graph snapshot** that contains:

- directory topology
- node identity
- node metadata
- opaque content references
- session cwd state

This is a schema snapshot, not a content mirror.

---

## 7.2 Why full content snapshotting is rejected in MVP

Full content snapshotting is the wrong default for this system because it would:

- waste memory
- slow down snapshot creation
- slow down agent iterations
- make simulation heavy for no reason

Many commands do not need content at all.

Examples:

- `pwd`
- `cd`
- `ls`
- `mkdir`
- `touch`
- `mv`
- `rm`

For those commands, content is irrelevant.

---

## 7.3 What the snapshot must contain

Every snapshot must contain at minimum:

- `workspace_root`
- `cwd_logical`
- `cwd_physical`
- a node map
- parent-child structure
- type info
- node metadata
- content identity placeholders

---

## 7.4 Snapshot JSON shape

A concrete JSON representation should look like this:

```json
{
  "snapshot_id": "snap_123",
  "generated_at_ns": 1760000000000000000,
  "session": {
    "workspace_root": "/repo",
    "cwd_logical": "/repo/src",
    "cwd_physical": "/repo/src",
    "oldpwd": null
  },
  "nodes": {
    "/repo": {
      "path": "/repo",
      "parent": null,
      "kind": "dir",
      "children": ["/repo/src", "/repo/README.md"],
      "mode": 16877,
      "mtime_ns": 1760000000000000000,
      "content_ref": null,
      "revision": 0
    },
    "/repo/src": {
      "path": "/repo/src",
      "parent": "/repo",
      "kind": "dir",
      "children": ["/repo/src/main.py"],
      "mode": 16877,
      "mtime_ns": 1760000000000000000,
      "content_ref": null,
      "revision": 0
    },
    "/repo/src/main.py": {
      "path": "/repo/src/main.py",
      "parent": "/repo/src",
      "kind": "file",
      "children": [],
      "size": 4123,
      "mode": 33188,
      "mtime_ns": 1760000000000000000,
      "content_ref": "opaque:/repo/src/main.py:4123:1760000000000000000",
      "revision": 0
    }
  }
}
```

---

## 7.5 Agent-facing projection

The full snapshot is not necessarily what the agent should read directly.

The agent should instead get a projection optimized for discovery.

Example:

```json
{
  "workspace_root": "/repo",
  "cwd": "/repo/src",
  "entries": [
    {"path": ".", "kind": "dir"},
    {"path": "./main.py", "kind": "file", "size": 4123},
    {"path": "../README.md", "kind": "file", "size": 2200}
  ]
}
```

### Why this matters
The projection allows the agent to understand the workspace without doing repeated shell discovery spam.

---

# 8. Simulation model

## 8.1 Core idea

Simulation means taking a `StructuredCommand` and applying it over:

- the immutable snapshot graph
- plus a temporary overlay

without touching the real filesystem.

---

## 8.2 Read path

For a read-only command:

1. resolve paths against session cwd
2. look up nodes in the graph
3. record metadata reads in the journal
4. compute a result
5. produce predicted effects

Example for `ls`:

- resolve `.`
- read the target directory node
- read its children
- record metadata reads for the visited nodes
- no overlay mutations
- decision is usually `approve`

---

## 8.3 Mutation path

For a mutating command:

1. resolve paths
2. validate preconditions
3. create/update/delete/rename nodes in overlay
4. record journal events
5. derive predicted effects
6. run policy decision

Example for `mkdir tmp`:

- resolve `tmp` relative to cwd
- check parent exists
- check target does not conflict illegally
- create a new node in overlay
- record `creates`
- approve if policy allows

---

## 8.4 Why overlay matters

The overlay is what makes the simulation engine cheap and safe.

Without overlay, the engine would need to:

- clone the graph
- rollback real mutations
- or mutate the base snapshot directly

All of those are worse.

The correct invariant is:

> Snapshot immutable, overlay mutable.

---

## 8.5 What simulation must produce

Every simulation must produce all of the following:

- a stable `plan_id`
- a `shell_preview`
- an `AccessJournal`
- a `PredictedEffects`
- a `decision`
- an optional `reason`

---

# 9. Policy and decision model

## 9.1 Why policy exists

Policy is not a later nice-to-have.

Policy is part of the core value of `vsh`.

Without policy, the system would merely be a graph evaluator.

Policy is what turns simulation into **approval intelligence**.

---

## 9.2 MVP decision states

The MVP should support exactly these decision states:

- `approve`
- `reject`
- `approve_with_warning`

No more complexity is needed yet.

---

## 9.3 MVP reject conditions

Initial reject conditions should include:

- path escapes workspace root
- target parent does not exist
- target node kind mismatch
- invalid cwd transition
- deleting `.` or `..`
- removing protected root path
- overwriting when overwrite policy disallows it

---

## 9.4 Warning conditions for later expansion

Warning decisions can later be used for:

- large subtree mutation
- suspicious wide read range
- possibly stale snapshot
- mutation touching too many nodes

But MVP can initially use warnings sparingly.

---

# 10. Real execution model

## 10.1 Why real execution is not first

The MVP’s deepest risk is not execution.

The deepest risk is whether the **simulation and approval model** is correct.

That is why simulation is developed before real execution.

---

## 10.2 Rule for execution

Real execution must never accept raw `name + params` directly.

It may only execute a previously approved plan.

This is non-negotiable.

---

## 10.3 Execution API

The execution API is:

```text
execute_approved(approval_token)
```

not:

```text
execute(name, params)
```

---

## 10.4 MVP executor subset

The real executor should initially support only:

- `pwd`
- `cd`
- `ls`
- `mkdir`
- `touch`

Later phases add:

- `mv`
- `cp`
- `rm`

---

# 11. MCP and FastMCP design

## 11.1 Why MCP is the right exposure layer

The system is meant to be consumed by agents.

MCP is the natural surface because it already models:

- tools
- resources
- local server execution
- stdio transport

This aligns well with `vsh`.

---

## 11.2 Why FastMCP is good for the Python MVP

FastMCP gives us:

- quick MCP server creation
- natural Python function-to-tool mapping
- easy use of Pydantic models
- clean local stdio server iteration

This is ideal for a Python prototype.

---

## 11.3 CodeMode-like pattern

The intended use pattern is:

```text
search
-> get_schema
-> simulate
-> approve
-> execute_approved
```

This resembles a CodeMode-style tool discovery flow but inserts `simulate` and `approve` as first-class lifecycle steps.

That is the core `vsh` difference.

---

# 12. Implementation plan — 10 phases

The roadmap below is intentionally detailed. Each phase contains:

- purpose
- what gets built
- what problem it solves
- what is unlocked after it
- concrete implementation tasks
- deliverables
- acceptance criteria

---

## Phase 1 — Foundational product contracts and command registry

### Purpose
Define the external shape of the system before implementing the deeper engine.

### Why this phase exists
Without clear command contracts, the rest of the system will drift.

This phase creates the discovery layer and freezes the first version of the command surface.

### What this phase must build
- Python package layout
- command registry
- `CommandSpec`
- `StructuredCommand`
- first command schemas
- `search(query)`
- `get_schema(name)`

### Commands included in Phase 1
- `vsh_pwd`
- `vsh_cd`
- `vsh_list`

### Detailed implementation tasks
1. Create Python project and package structure
2. Add Pydantic models for registry objects
3. Add `schemas/pwd.py`, `schemas/cd.py`, `schemas/ls.py`
4. Define a `registry/specs.py` file with static command registrations
5. Implement `search(query)`
6. Implement `get_schema(name)` using `model_json_schema()`
7. Add examples to each command spec
8. Add tags such as `read`, `filesystem`, `discovery`, `mutate`

### Deliverables
- `CommandSpec` registry
- JSON schema responses
- basic MCP-compatible function surface

### Unlocks after this phase
- agents can discover commands
- agents can ask for schemas
- the system becomes command-model-first

### Acceptance criteria
- `search("ls")` returns `vsh_list`
- `get_schema("vsh_list")` returns valid JSON schema
- registry contains at least 3 commands

---

## Phase 2 — Workspace snapshot graph and session state

### Purpose
Build the immutable graph view of the workspace that simulation will operate on.

### Why this phase exists
A decision engine cannot exist without a stable model of the workspace.

This phase creates that model.

### What this phase must build
- `SessionState`
- `SnapshotNode`
- `WorkspaceSnapshot`
- `snapshot_workspace()`
- snapshot JSON persistence
- workspace projection resource

### Detailed implementation tasks
1. Implement recursive tree scanning with `os.scandir()`
2. Normalize paths into a single canonical representation
3. Build node objects for files, dirs, symlinks
4. Populate `children` for directories
5. Add metadata fields:
   - `size`
   - `mode`
   - `mtime_ns`
6. Create `content_ref` using opaque identity tokens
7. Add ignore patterns:
   - `.git`
   - `.venv`
   - `node_modules`
   - `dist`
   - `build`
   - `target`
8. Build cwd-centric projection output
9. Persist snapshot to JSON
10. Expose snapshot and projection as resources

### Deliverables
- snapshot builder
- snapshot JSON format
- current projection JSON

### Unlocks after this phase
- the system can reason over a workspace tree without reading file contents
- the agent can inspect a stable representation of the current workspace

### Acceptance criteria
- `snapshot_workspace()` returns a stable `snapshot_id`
- snapshot JSON contains a valid node graph
- projection correctly reflects cwd and nearby nodes

---

## Phase 3 — Path resolution and read-only simulation engine

### Purpose
Turn structured commands into simulated behavior over the snapshot graph.

### Why this phase exists
This is the first phase where `vsh` stops being only a modeling exercise and becomes a command engine.

### What this phase must build
- path resolver
- `Overlay`
- `AccessJournal`
- `simulate()`
- shell preview renderer
- simulation for read-only commands

### Commands implemented in simulation
- `vsh_pwd`
- `vsh_cd`
- `vsh_list`

### Detailed implementation tasks
1. Build logical path resolution relative to `cwd_logical`
2. Support:
   - `.`
   - `..`
   - relative paths
   - absolute paths
3. Reject workspace escape attempts
4. Implement graph lookup helpers
5. Implement `Overlay` structure
6. Implement `AccessJournal`
7. Simulate `pwd`
   - read session state
   - no graph mutation
8. Simulate `cd`
   - resolve path
   - validate target is a directory
   - set `cwd_override`
   - record `cwd_changes`
9. Simulate `ls`
   - resolve target
   - if file: list file object
   - if dir: list child nodes
   - record metadata reads
10. Implement `shell_preview` rendering for all three commands
11. Derive `PredictedEffects`
12. Return `SimulationResult`

### Deliverables
- read-only simulator
- journal generation
- effect prediction
- decision generation

### Unlocks after this phase
This is the first milestone where the system can do all of the following:

- accept structured commands
- use a current cwd workspace tree snapshot
- simulate commands over a mock graph
- produce predicted effects
- return approve/reject

This phase is one of the main MVP proof points.

### Acceptance criteria
- `simulate("vsh_pwd")` returns current cwd
- `simulate("vsh_cd", {"path": "src"})` updates simulated cwd
- `simulate("vsh_cd", {"path": "../../.."})` rejects when escaping root
- `simulate("vsh_list", {"path": "."})` reads correct nodes
- journal contains expected metadata read set

---

## Phase 4 — Plan store and approval lifecycle

### Purpose
Separate simulation from execution using immutable plans.

### Why this phase exists
Without this phase, the system collapses back into direct execution of commands.

The approval model is a central product feature.

### What this phase must build
- plan storage
- stable `plan_id`
- `approve(plan_id)`
- `ApprovalToken`
- immutable simulation artifact lifecycle

### Detailed implementation tasks
1. Create `PlanStore` abstraction
2. Persist `SimulationResult` by `plan_id`
3. Hash/fingerprint the command + snapshot basis if desired
4. Implement `approve(plan_id)`
5. Emit `ApprovalToken`
6. Ensure approved plans are immutable
7. Make resources available for stored simulations

### Deliverables
- approval store
- immutable plan artifacts

### Unlocks after this phase
- real execution can now be attached cleanly
- the system gains a real approval concept

### Acceptance criteria
- a simulation can be stored and later reloaded
- approval returns a token bound to a single plan
- plans cannot be mutated after approval

---

## Phase 5 — First mutating simulation commands: `mkdir` and `touch`

### Purpose
Extend the simulation engine from read-only discovery into controlled mutation planning.

### Why this phase exists
A workspace engine must prove that it can model writes, not just reads.

These are the safest first mutations.

### What this phase must build
- `vsh_mkdir`
- `vsh_touch`
- overlay create/update logic
- mutation journal updates
- policy checks for mutating commands

### Detailed implementation tasks
1. Add `schemas/mkdir.py`
2. Add `schemas/touch.py`
3. Implement path validation for parent existence
4. Implement create-node overlay path
5. Implement metadata update path for `touch`
6. Record `creates`, `metadata_writes`
7. Add shell previews:
   - `mkdir tmp`
   - `touch file.txt`
8. Add reject cases:
   - parent missing
   - illegal overwrite
   - type mismatch

### Deliverables
- mutating simulation for safe commands
- first overlay writes
- write-oriented effect journal

### Unlocks after this phase
- `vsh` becomes more than a read planner
- approval now matters for mutation commands

### Acceptance criteria
- simulated `mkdir` creates a new overlay node
- simulated `touch` updates metadata or creates a file node
- mutation journal records correct touches

---

## Phase 6 — Real executor v1 for approved plans

### Purpose
Allow a small, safe subset of approved plans to affect the real filesystem.

### Why this phase exists
An MVP needs to prove the full lifecycle from planning to real application.

### What this phase must build
- `RealFsExecutor`
- `execute_approved()`
- pre-execution checks
- minimal actual effect collection

### Commands executable in v1
- `vsh_pwd`
- `vsh_cd`
- `vsh_list`
- `vsh_mkdir`
- `vsh_touch`

### Detailed implementation tasks
1. Create executor abstraction
2. Implement command dispatch for the initial subset
3. Revalidate key preconditions before execution
4. Execute using Python stdlib (`os`, `pathlib`)
5. Collect actual basic results
6. Add basic compare between predicted and actual effects

### Deliverables
- first real executor
- approved execution lifecycle

### Unlocks after this phase
- full end-to-end demo becomes possible
- `vsh` is no longer only a simulator

### Acceptance criteria
- approved `mkdir` creates a real directory
- approved `touch` creates or updates a real file
- execution refuses unapproved plans

---

## Phase 7 — Workspace mutation core: `move`, `copy`, `remove`

### Purpose
Support the most important file-manipulation commands used by coding agents.

### Why this phase exists
Many real agent tasks revolve around:

- moving files
- copying files
- removing files/directories

Without these, the engine is limited.

### What this phase must build
- `vsh_move`
- `vsh_copy`
- `vsh_remove`
- richer overlay behavior
- stronger destructive policy rules

### Detailed implementation tasks
1. Add schema models for `move`, `copy`, `remove`
2. Implement rename semantics in overlay
3. Implement subtree copy semantics in overlay
4. Implement delete semantics in overlay
5. Add guards:
   - protect workspace root
   - reject `.` and `..`
   - require recursive flag where needed
6. Implement shell previews
7. Add predicted effect mapping for destructive commands

### Deliverables
- simulation support for core mutating commands

### Unlocks after this phase
- `vsh` becomes a credible workspace mutation planner

### Acceptance criteria
- move/copy/remove simulate correctly
- dangerous remove attempts reject correctly
- journal accurately shows subtree impact

---

## Phase 8 — Revalidation, drift detection, and targeted refresh

### Purpose
Prevent stale snapshot assumptions from causing unsafe execution.

### Why this phase exists
A command can be valid at simulation time and invalid at execution time if the workspace changed.

### What this phase must build
- path fingerprinting
- stale detection
- targeted refresh
- pre-execution revalidation

### Detailed implementation tasks
1. Define lightweight node fingerprint rules
2. Compare relevant touched paths before execution
3. Reject stale plans when necessary
4. Refresh only touched paths if possible
5. Update snapshot/resource state after execution

### Deliverables
- stale plan detection
- safer approval-to-execution transition

### Unlocks after this phase
- significantly better reliability
- makes the engine more robust under concurrent edits

### Acceptance criteria
- stale external changes are detected
- invalid plans refuse to execute
- targeted refresh works for touched nodes

---

## Phase 9 — MCP server hardening and CodeMode-style UX

### Purpose
Turn the system into a real agent-facing local server.

### Why this phase exists
The engine should be usable from agent clients, not only as a local library.

### What this phase must build
- FastMCP server
- stdio entrypoint
- tool registration
- resource registration
- end-to-end local invocation tests

### Detailed implementation tasks
1. Build `mcp/server.py`
2. Register all tools
3. Register all resources
4. Add startup script / entrypoint
5. Ensure JSON serialization is stable
6. Test from a local MCP-compatible client

### Deliverables
- working local MCP server
- external discoverability of `vsh`

### Unlocks after this phase
- Codex-like clients can use `vsh`
- IDE/CLI integration becomes realistic

### Acceptance criteria
- MCP client can connect over stdio
- `search`, `get_schema`, `snapshot_workspace`, `simulate`, `approve`, `execute_approved` all work via MCP

---

## Phase 10 — Hardening, documentation, tests, and extension hooks

### Purpose
Turn the MVP into a stable foundation for future work.

### Why this phase exists
An MVP without tests and extension seams becomes difficult to evolve.

### What this phase must build
- golden tests
- fixtures
- documentation
- future analyzer hooks
- future hydration hooks

### Detailed implementation tasks
1. Create snapshot fixtures
2. Create golden command simulation tests
3. Create execution tests for supported commands
4. Write architecture docs
5. Write extension interfaces for:
   - content hydration
   - semantic analyzers
   - shadow workspaces
6. Create a demo walkthrough

### Deliverables
- stable MVP
- extension-friendly codebase
- usable docs

### Unlocks after this phase
- semantic analyzers can later be attached
- TypeScript/Python-specific checks can later be layered in
- future product work no longer requires redesigning the core

### Acceptance criteria
- golden tests pass
- docs explain system clearly
- project is demoable end-to-end

---

# 13. Recommended package structure

```text
vsh/
  README.md
  pyproject.toml
  src/vsh/
    __init__.py
    registry/
      __init__.py
      specs.py
      search.py
    schemas/
      __init__.py
      common.py
      pwd.py
      cd.py
      ls.py
      mkdir.py
      touch.py
      move.py
      copy.py
      remove.py
    session/
      __init__.py
      state.py
      resolver.py
    snapshot/
      __init__.py
      models.py
      builder.py
      projection.py
    simulate/
      __init__.py
      overlay.py
      journal.py
      engine.py
      policy.py
      renderer.py
    plans/
      __init__.py
      models.py
      store.py
      approval.py
    execute/
      __init__.py
      realfs.py
      revalidate.py
    mcp/
      __init__.py
      server.py
      tools.py
      resources.py
  tests/
    fixtures/
    test_registry.py
    test_snapshot.py
    test_simulate_pwd.py
    test_simulate_cd.py
    test_simulate_ls.py
    test_simulate_mkdir.py
    test_simulate_touch.py
```

---

# 14. Suggested initial command schemas

## `vsh_pwd`

```python
class PwdCommand(BaseModel):
    physical: bool = False
```

## `vsh_cd`

```python
class CdCommand(BaseModel):
    path: str
    physical: bool = False
```

## `vsh_list`

```python
class LsCommand(BaseModel):
    path: str = "."
    all: bool = False
    long: bool = False
    one: bool = False
    recursive: bool = False
```

## `vsh_mkdir`

```python
class MkdirCommand(BaseModel):
    path: str
    parents: bool = False
```

## `vsh_touch`

```python
class TouchCommand(BaseModel):
    path: str
    no_create: bool = False
```

## `vsh_move`

```python
class MoveCommand(BaseModel):
    src: str
    dst: str
    overwrite: bool = False
```

## `vsh_copy`

```python
class CopyCommand(BaseModel):
    src: str
    dst: str
    recursive: bool = False
    overwrite: bool = False
```

## `vsh_remove`

```python
class RemoveCommand(BaseModel):
    path: str
    recursive: bool = False
    force: bool = False
```

---

# 15. What comes after MVP

These are important, but explicitly deferred:

## 15.1 Lazy content hydration
Later, when commands need actual file contents, hydrate only the files that matter.

## 15.2 Python semantic analyzer
Later, add import/syntax/incomplete-edit checks over changed nodes.

## 15.3 TypeScript semantic analyzer
Later, add import/export/path alias checks.

## 15.4 Shadow workspace verification
Later, run `py_compile`, type checkers, or other analyzers over a shadow workspace.

## 15.5 Derived commands
Later, support deriving higher-level commands from primitive effect plans.

But none of these belong in the MVP core.

---

# 16. Final summary

`vsh` exists to turn agent shell-like actions into a safer, more explicit, more analyzable lifecycle.

Its core idea is simple:

1. commands are **structured**, not string-first
2. workspace is represented as a **snapshot graph**, not fully loaded content
3. commands are **simulated before execution**
4. the system records **exact node-level side effects**
5. approval is given to a **plan**, not a raw invocation
6. only approved plans may reach real execution

The MVP should prove exactly that.

The first major milestone is reached as soon as these are all true together:

- command discovery works
- command schemas are inspectable
- workspace snapshot graph exists
- current cwd projection exists
- `pwd`, `cd`, and `ls` can be simulated on the mock graph
- journal and predicted effects are produced
- a plan can be approved or rejected

Once that exists, the rest of `vsh` becomes an expansion problem rather than a definition problem.

That is the right MVP.
