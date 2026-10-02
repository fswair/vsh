# Command-line workflows

The `vsh` executable ships with `vsh-python`. `vsh run` needs no MCP dependency;
`vsh serve` requires the `mcp` extra. The CLI delegates to the same native engine as
the SDK and writes a JSON receipt to stdout.

## Preview one program

Monty is the default. To use the optional Unix Bash frontend, select both the host
opt-in and guest language: `vsh run --enable-bash --language bash --workspace ./demo-workspace
--code 'printf verified > report.txt'`. `--bash-worker` selects an explicit trusted
executable; it does not enable host shell commands. See [bounded Bash](../integrations/bash.md).

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

### Bash failure diagnostics

A Bash execution failure writes one JSON object to **stderr**, with no successful
receipt on stdout and no committable transaction handle. `state` is `"failed"` and
`committable` is `false`. `error.kind` distinguishes failure categories;
`exit_code` preserves the guest status when available. The CLI exits with that status
when it is between 1 and 255, otherwise with 1.

The diagnostic `stdout` and `stderr` fields are base64 strings, each limited to
65,536 raw bytes. Check `encoding` and `output_truncated` before decoding. `changes`
describes virtual changes only; `changes_complete: false` means a complete diagnostic
diff was unavailable, not that nothing changed in the simulation. No partial guest
changes are committed when execution fails.

```bash
vsh run --enable-bash --language bash --workspace ./demo-workspace \
  --mode auto --code 'printf diagnostic >&2; printf draft > report.txt; exit 7'
```

This exits with 7, reports the virtual `report.txt` change and leaves the host file
unchanged. Ordinary receipt decisions such as pending approval still require inspecting
the receipt; they are not Bash execution failures.

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
