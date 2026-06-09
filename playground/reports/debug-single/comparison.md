# Agent context comparison: vsh CodeMode vs native structured FS tools

- generated: 2026-06-09T21:06:08.052566+00:00
- model: `openrouter:google/gemini-3-flash-preview`
- workspace: `/var/folders/9t/dwwzwmw57xg7yyml_2jtb9lc0000gn/T/vsh-agent-compare-zhutf2rp`

## Scenario validation

- vsh passed: **False**
- native passed: **True**
- both passed: **False**

### vsh validation errors

- missing file: bench/output/status.json

## Duration

| mode | wall time |
|------|----------:|
| vsh codemode | 5055.1 ms |
| native fs tools | 6982.9 ms |

- vsh faster: **True**
- duration savings (vsh vs native): **+27.6%**

## Token usage (`result.usage`)

| metric | vsh | native | savings |
|--------|----:|-------:|--------:|
| input tokens | 1955 | 3962 | +50.7% |
| output tokens | 416 | 247 | -68.4% |
| total tokens | 2371 | 4209 | +43.7% |
| model requests | 2 | 6 | — |
| tool calls (usage) | 1 | 5 | — |

## Approximate history payload

- vsh serialized history: **8139** bytes
- native serialized history: **13073** bytes
- byte savings: **+37.7%**
- vsh tool return payload: **372** bytes across 1 returns
- native tool return payload: **222** bytes across 5 returns

## Per-request usage

- vsh request usage: `[{"input_tokens": 863, "output_tokens": 231, "total_tokens": 1094}, {"input_tokens": 1092, "output_tokens": 185, "total_tokens": 1277}]`
- native request usage: `[{"input_tokens": 572, "output_tokens": 16, "total_tokens": 588}, {"input_tokens": 592, "output_tokens": 35, "total_tokens": 627}, {"input_tokens": 637, "output_tokens": 29, "total_tokens": 666}, {"input_tokens": 678, "output_tokens": 41, "total_tokens": 719}, {"input_tokens": 729, "output_tokens": 18, "total_tokens": 747}, {"input_tokens": 754, "output_tokens": 108, "total_tokens": 862}]`

## Cost estimate (genai-prices)

- vsh: n/a
- native: n/a

## Tool surface

- vsh tools called (1): `apply_batch`
- native tools called (5): `mkdir, write_file, grep, write_file, list_dir`

### native tool calls

1. `mkdir` args={"path": "bench/output"}
2. `write_file` args={"content": "marker: bench-marker-42", "path": "bench/output/summary.md"}
3. `grep` args={"path": ".", "recursive": true, "pattern": "bench-marker-42"}
4. `write_file` args={"path": "bench/output/status.json", "content": "{\"marker\":\"bench-marker-42\",\"phase\":\"complete\"}"}
5. `list_dir` args={"path": "bench/output"}

## Agent outputs (truncated)

### vsh

The checklist was partially executed via `apply_batch`. Despite a signal-related error occurring during the `vsh_grep` step (likely due to environment constraints on that specific tool execution), the primary mutation steps for directory creation and initial file writing were successful.

**Receipt Summary:**
1.  **vsh_mkdir**: Successfully created `bench/output`.
2.  **vsh_echo**: Successfully created `bench/output/summary.md` with the marker.
3.  **vsh_grep**: Failed with a signal error (interrupting the batch execution).
4.  **vsh_echo/vsh_list**: These steps were not reached due to the batch failure on step 3.

As per the instruction to only use exactly one `apply_batch` and stop if an error occurs rather than retrying via shell, the process concludes here.

### native

Summary of tool calls and step results:
1. `mkdir` (bench/output): Passed.
2. `write_file` (bench/output/summary.md): Passed.
3. `grep` (pattern=bench-marker-42): Passed (confirmed in summary.md).
4. `write_file` (bench/output/status.json): Passed.
5. `list_dir` (bench/output): Passed (confirmed summary.md and status.json exist).

## Metadata

```json
{
  "generated_at": "2026-06-09T21:05:55.508771+00:00",
  "model": "openrouter:google/gemini-3-flash-preview",
  "codemode_mcp": true,
  "vsh_prompt_chars": 1324,
  "native_prompt_chars": 1324,
  "runs": 1,
  "git_commit": "c60cfe5e5784a2c21a32f5d52066a5b46e049f71"
}
```
