# Agent context comparison: vsh CodeMode vs native structured FS tools

- generated: 2026-06-09T21:24:33.227975+00:00
- model: `openrouter:google/gemini-3-flash-preview`
- workspace: `/var/folders/9t/dwwzwmw57xg7yyml_2jtb9lc0000gn/T/vsh-agent-compare-b48_opdc`

## Scenario validation

- vsh passed: **True**
- native passed: **True**
- both passed: **True**

## Duration

| mode | wall time |
|------|----------:|
| vsh codemode | 3674.1 ms |
| native fs tools | 7497.0 ms |

- vsh faster: **True**
- duration savings (vsh vs native): **+51.0%**

## Token usage (`result.usage`)

| metric | vsh | native | savings |
|--------|----:|-------:|--------:|
| input tokens | 2093 | 4037 | +48.2% |
| output tokens | 360 | 302 | -19.2% |
| total tokens | 2453 | 4339 | +43.5% |
| model requests | 2 | 6 | — |
| tool calls (usage) | 1 | 5 | — |

## Approximate history payload

- vsh serialized history: **8171** bytes
- native serialized history: **13446** bytes
- byte savings: **+39.2%**
- vsh tool return payload: **746** bytes across 1 returns
- native tool return payload: **338** bytes across 5 returns

## Per-request usage

- vsh request usage: `[{"input_tokens": 863, "output_tokens": 223, "total_tokens": 1086}, {"input_tokens": 1230, "output_tokens": 137, "total_tokens": 1367}]`
- native request usage: `[{"input_tokens": 572, "output_tokens": 16, "total_tokens": 588}, {"input_tokens": 592, "output_tokens": 35, "total_tokens": 627}, {"input_tokens": 637, "output_tokens": 29, "total_tokens": 666}, {"input_tokens": 703, "output_tokens": 41, "total_tokens": 744}, {"input_tokens": 754, "output_tokens": 18, "total_tokens": 772}, {"input_tokens": 779, "output_tokens": 163, "total_tokens": 942}]`

## Cost estimate (genai-prices)

- vsh: n/a
- native: n/a

## Tool surface

- vsh tools called (1): `apply_batch`
- native tools called (5): `mkdir, write_file, grep, write_file, list_dir`

### native tool calls

1. `mkdir` args={"path": "bench/output"}
2. `write_file` args={"path": "bench/output/summary.md", "content": "marker: bench-marker-42"}
3. `grep` args={"pattern": "bench-marker-42", "path": ".", "recursive": true}
4. `write_file` args={"path": "bench/output/status.json", "content": "{\"marker\":\"bench-marker-42\",\"phase\":\"complete\"}"}
5. `list_dir` args={"path": "bench/output"}

## Agent outputs (truncated)

### vsh

I have completed the workspace checklist in a single batch.

**Receipt Summary:**
1.  **vsh_mkdir**: Created directory `bench/output`.
2.  **vsh_echo**: Created `bench/output/summary.md` with marker `bench-marker-42`.
3.  **vsh_grep**: Successfully found `bench-marker-42` in `summary.md`.
4.  **vsh_echo**: Created `bench/output/status.json` with completion status.
5.  **vsh_list**: Confirmed both files exist in the `bench/output` directory.

### native

The checklist steps have been completed successfully:

1.  **mkdir**: Path `bench/output` was created. (Pass)
2.  **write_file**: `bench/output/summary.md` was created with the content `marker: bench-marker-42`. (Pass)
3.  **grep**: A search for `bench-marker-42` confirmed the presence of the marker in `bench/output/summary.md`. (Pass)
4.  **write_file**: `bench/output/status.json` was created with the required JSON content. (Pass)
5.  **list_dir**: Confirmed that both `summary.md` and `status.json` exist in `bench/output`. (Pass)

## Metadata

```json
{
  "generated_at": "2026-06-09T21:23:55.138381+00:00",
  "model": "openrouter:google/gemini-3-flash-preview",
  "codemode_mcp": true,
  "vsh_prompt_chars": 1324,
  "native_prompt_chars": 1324,
  "runs": 3,
  "git_commit": "c60cfe5e5784a2c21a32f5d52066a5b46e049f71"
}
```
