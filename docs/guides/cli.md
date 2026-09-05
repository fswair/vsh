# Command-line workflows

The `vsh` executable ships with `vsh-python`. `vsh run` needs no MCP dependency;
`vsh serve` requires the `mcp` extra. The CLI delegates to the same native engine as
the SDK and writes a JSON receipt to stdout.

## Preview one program

Choose an existing workspace and save supported Monty source in `transform.py`:

```python
from pathlib import Path
source = Path('/workspace/input.txt').read_text()
Path('/workspace/output.txt').write_text(source.upper())
{'before': source, 'after': source.upper()}
```

```bash
vsh run --workspace ./demo-workspace --file transform.py --mode preview --detail full
```

`--file` is a host-side UTF-8 source file. Paths *inside* that source are virtual.
You can instead use `--code` with a source string. These options are mutually exclusive
with `--transaction`. Other options are `--intent`, `--policy` and `--detail`.

Inspect `decision`, `changes`, `result_repr` and `commit.committed`, not only the
process exit code. Policy denial or pending approval can be returned as a normal
receipt. A runtime/compilation failure exits unsuccessfully instead of supplying a
successful transaction receipt.

## Important cross-process limit

An auto-approved preview is retained only by the live runtime that created it. A
normal CLI command then exits. **A second CLI process cannot promote that balanced
preview with `--transaction`.** The flag does not make process-local artifacts durable.

For a review-then-apply workflow, use one live Python/Rust runtime or an appropriate
long-lived MCP server. Strict pending artifacts are durable, but need a trusted SDK
approval before promotion; the CLI does not provide an approval command.

## Explicit one-shot automation

For a known transformation already approved by your application, run:

```bash
vsh run --workspace ./demo-workspace --file transform.py --mode auto --detail full
```

This is a **new execution**, not promotion of a previous CLI preview. It commits only
when native policy auto-approves. Pending or denied work remains unapplied.

## Run the disposable acceptance example

From the source checkout:

```bash
uv run --no-sync python examples/native/cli_workflow.py
```

The driver launches real separate CLI processes, proves preview isolation and rejection
of the lost auto-approved handle, then verifies explicit one-shot fixture automation.
It does not use or modify your repository as its workspace.

For CodeMode, transport configuration and receipt fields, continue to [MCP](../integrations/mcp.md).
