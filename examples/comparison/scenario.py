from __future__ import annotations as _annotations

from dataclasses import dataclass
from pathlib import Path

MARKER = "bench-marker-42"
OUTPUT_DIR = Path("bench/output")
SUMMARY_FILE = OUTPUT_DIR / "summary.md"
STATUS_FILE = OUTPUT_DIR / "status.json"
SUMMARY_LINE = f"marker: {MARKER}"
STATUS_JSON = f'{{"marker":"{MARKER}","phase":"complete"}}'


@dataclass(frozen=True, slots=True)
class ScenarioPrompts:
    vsh_user_prompt: str
    native_user_prompt: str
    vsh_system_instructions: str
    native_system_instructions: str


def prepare_workspace(root: Path) -> None:
    """Seed a minimal workspace; agents must create bench/output themselves."""
    (root / "README.md").write_text(
        "# Context comparison workspace\n\nStarter tree for vsh vs native benchmark.\n",
        encoding="utf-8",
    )
    notes = root / "notes"
    notes.mkdir(exist_ok=True)
    (notes / "context.txt").write_text(
        "Unrelated filler file to make grep/noise realistic.\n",
        encoding="utf-8",
    )


def _pad_to_length(text: str, target_len: int, *, pad_label: str) -> str:
    if len(text) >= target_len:
        return text
    pad_line = f"# {pad_label}: keep checklist order; no extra files outside bench/output/.\n"
    padded = text
    index = 0
    while len(padded) < target_len:
        padded += pad_line.replace(pad_label, f"{pad_label}-{index}")
        index += 1
    return padded[:target_len] if len(padded) > target_len else padded


def build_scenario_prompts() -> ScenarioPrompts:
    vsh_body = """\
Complete this workspace checklist using vsh CodeMode MCP tools only.
Prefer one apply_batch call for the full workflow. Do not use raw shell commands.

Checklist:
1) Create bench/output/ if missing.
2) Write bench/output/summary.md containing exactly one line: marker: bench-marker-42
3) Recursive grep for bench-marker-42; confirm summary.md is in the hits.
4) Write bench/output/status.json with JSON:
   {"marker":"bench-marker-42","phase":"complete"}
5) List bench/output/ and confirm summary.md and status.json exist.

Rules:
- Use apply_batch steps with tool_name/params. Use vsh_mkdir, vsh_echo, vsh_grep, vsh_echo, vsh_list.
- Every mutation step MUST include execution_reason.
- Do not create files outside bench/output/ except directories needed to reach it.
- Reuse the returned snapshot_id; do not call snapshot_workspace unless apply_batch fails.
- Finish with a short summary of receipts and whether each step passed.
"""

    native_body = """\
Complete this workspace checklist using only the structured filesystem tools:
mkdir, write_file, read_file, grep, list_dir. No shell, no bash, no raw commands.

Checklist:
1) mkdir path=bench/output
2) write_file path=bench/output/summary.md content with exactly: marker: bench-marker-42
3) grep pattern=bench-marker-42 path=. recursive=true; confirm summary.md in hits
4) write_file path=bench/output/status.json content JSON:
   {"marker":"bench-marker-42","phase":"complete"}
5) list_dir path=bench/output and confirm summary.md and status.json exist

Rules:
- write_file only accepts bench/output/summary.md or bench/output/status.json.
- mkdir only accepts bench/output. grep/list_dir scopes are limited to . or bench/output.
- Do not create files outside bench/output/ except directories needed to reach it.
- Finish with a short summary of tool calls and whether each step passed.
"""

    target_len = max(len(vsh_body), len(native_body))
    vsh_user = _pad_to_length(vsh_body, target_len, pad_label="vsh-checklist")
    native_user = _pad_to_length(native_body, target_len, pad_label="native-checklist")

    vsh_system = """\
You are a vsh CodeMode agent. Follow simulate-before-execute for every filesystem change.
For known benchmark steps, use apply_batch directly and keep verbosity compact.
"""
    native_system = """\
You are a workspace agent with structured filesystem tools (mkdir, write_file, read_file,
grep, list_dir). Paths are workspace-relative; mutations are restricted to bench/output.
Pick the smallest tool per step — never invent shell commands.
"""
    sys_target = max(len(vsh_system), len(native_system))
    vsh_system = _pad_to_length(vsh_system, sys_target, pad_label="vsh-system")
    native_system = _pad_to_length(native_system, sys_target, pad_label="native-system")

    return ScenarioPrompts(
        vsh_user_prompt=vsh_user,
        native_user_prompt=native_user,
        vsh_system_instructions=vsh_system,
        native_system_instructions=native_system,
    )
