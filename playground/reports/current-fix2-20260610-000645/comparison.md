# Agent context comparison: vsh CodeMode vs native structured FS tools

- generated: 2026-06-09T21:07:24.652559+00:00
- model: `openrouter:google/gemini-3-flash-preview`
- workspace: `/var/folders/9t/dwwzwmw57xg7yyml_2jtb9lc0000gn/T/vsh-agent-compare-5e__ibqi`

## Scenario validation

- vsh passed: **True**
- native passed: **True**
- both passed: **True**

## Duration

| mode | wall time |
|------|----------:|
| vsh codemode | 4330.6 ms |
| native fs tools | 7718.0 ms |

- vsh faster: **True**
- duration savings (vsh vs native): **+43.9%**

## Token usage (`result.usage`)

| metric | vsh | native | savings |
|--------|----:|-------:|--------:|
| input tokens | 2098 | 4037 | +48.0% |
| output tokens | 392 | 278 | -41.0% |
| total tokens | 2490 | 4315 | +42.3% |
| model requests | 2 | 6 | — |
| tool calls (usage) | 1 | 5 | — |

## Approximate history payload

- vsh serialized history: **8244** bytes
- native serialized history: **13318** bytes
- byte savings: **+38.1%**
- vsh tool return payload: **746** bytes across 1 returns
- native tool return payload: **338** bytes across 5 returns

## Per-request usage

- vsh request usage: `[{"input_tokens": 863, "output_tokens": 230, "total_tokens": 1093}, {"input_tokens": 1235, "output_tokens": 162, "total_tokens": 1397}]`
- native request usage: `[{"input_tokens": 572, "output_tokens": 16, "total_tokens": 588}, {"input_tokens": 592, "output_tokens": 35, "total_tokens": 627}, {"input_tokens": 637, "output_tokens": 29, "total_tokens": 666}, {"input_tokens": 703, "output_tokens": 41, "total_tokens": 744}, {"input_tokens": 754, "output_tokens": 18, "total_tokens": 772}, {"input_tokens": 779, "output_tokens": 139, "total_tokens": 918}]`

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
4. `write_file` args={"content": "{\"marker\":\"bench-marker-42\",\"phase\":\"complete\"}", "path": "bench/output/status.json"}
5. `list_dir` args={"path": "bench/output"}

## Agent outputs (truncated)

### vsh

I have completed the workspace checklist in a single batch. Here is the summary:

1.  **vsh_mkdir**: Created `bench/output` (status: ok).
2.  **vsh_echo**: Wrote `marker: bench-marker-42` to `bench/output/summary.md`.
3.  **vsh_grep**: Verified the marker `bench-marker-42` exists; results confirmed in `summary.md` and `status.json`.
4.  **vsh_echo**: Wrote the completion status to `bench/output/status.json`.
5.  **vsh_list**: Confirmed both `status.json` and `summary.md` exist in `bench/output`.

### native

All steps of the checklist have been completed successfully:

1.  **mkdir**: `bench/output` created. (Pass)
2.  **write_file**: `bench/output/summary.md` created with content. (Pass)
3.  **grep**: Found `bench-marker-42` in `bench/output/summary.md`. (Pass)
4.  **write_file**: `bench/output/status.json` created with JSON content. (Pass)
5.  **list_dir**: Confirmed `summary.md` and `status.json` exist in `bench/output`. (Pass)

## Metadata

```json
{
  "generated_at": "2026-06-09T21:06:46.523973+00:00",
  "model": "openrouter:google/gemini-3-flash-preview",
  "codemode_mcp": true,
  "vsh_prompt_chars": 1324,
  "native_prompt_chars": 1324,
  "runs": 3,
  "git_commit": "c60cfe5e5784a2c21a32f5d52066a5b46e049f71"
}
```
