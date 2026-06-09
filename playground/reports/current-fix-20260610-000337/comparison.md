# Agent context comparison: vsh CodeMode vs native structured FS tools

- generated: 2026-06-09T21:04:17.751928+00:00
- model: `openrouter:google/gemini-3-flash-preview`
- workspace: `/var/folders/9t/dwwzwmw57xg7yyml_2jtb9lc0000gn/T/vsh-agent-compare-8z06ryj4`

## Scenario validation

- vsh passed: **True**
- native passed: **True**
- both passed: **True**

## Duration

| mode | wall time |
|------|----------:|
| vsh codemode | 4808.6 ms |
| native fs tools | 8280.6 ms |

- vsh faster: **True**
- duration savings (vsh vs native): **+41.9%**

## Token usage (`result.usage`)

| metric | vsh | native | savings |
|--------|----:|-------:|--------:|
| input tokens | 1950 | 4037 | +51.7% |
| output tokens | 369 | 278 | -32.7% |
| total tokens | 2319 | 4315 | +46.3% |
| model requests | 2 | 6 | — |
| tool calls (usage) | 1 | 5 | — |

## Approximate history payload

- vsh serialized history: **7874** bytes
- native serialized history: **13382** bytes
- byte savings: **+41.2%**
- vsh tool return payload: **372** bytes across 1 returns
- native tool return payload: **338** bytes across 5 returns

## Per-request usage

- vsh request usage: `[{"input_tokens": 863, "output_tokens": 226, "total_tokens": 1089}, {"input_tokens": 1087, "output_tokens": 143, "total_tokens": 1230}]`
- native request usage: `[{"input_tokens": 572, "output_tokens": 16, "total_tokens": 588}, {"input_tokens": 592, "output_tokens": 35, "total_tokens": 627}, {"input_tokens": 637, "output_tokens": 29, "total_tokens": 666}, {"input_tokens": 703, "output_tokens": 41, "total_tokens": 744}, {"input_tokens": 754, "output_tokens": 18, "total_tokens": 772}, {"input_tokens": 779, "output_tokens": 139, "total_tokens": 918}]`

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
4. `write_file` args={"content": "{\"marker\":\"bench-marker-42\",\"phase\":\"complete\"}", "path": "bench/output/status.json"}
5. `list_dir` args={"path": "bench/output"}

## Agent outputs (truncated)

### vsh

The benchmark checklist was partially completed in a single batch, but the `vsh_grep` step failed due to a system signal error.

**Receipt Summary:**
1.  **vsh_mkdir**: Successfully created `bench/output`.
2.  **vsh_echo**: Successfully wrote `marker: bench-marker-42` to `bench/output/summary.md`.
3.  **vsh_grep**: **Failed** (`signal only works in main thread`). Subsequent steps in the batch (status.json and vsh_list) were not executed.

To comply with the "Exactly ONE apply_batch" rule, no further attempts will be made.

### native

The checklist is complete. Here is the summary of the steps performed:

1.  **mkdir bench/output**: Passed. Directory created successfully.
2.  **write_file summary.md**: Passed. Content `marker: bench-marker-42` written.
3.  **grep bench-marker-42**: Passed. Confirmed the pattern exists in `bench/output/summary.md`.
4.  **write_file status.json**: Passed. JSON content written successfully.
5.  **list_dir bench/output**: Passed. Confirmed both `summary.md` and `status.json` exist in the directory.

## Metadata

```json
{
  "generated_at": "2026-06-09T21:03:39.209300+00:00",
  "model": "openrouter:google/gemini-3-flash-preview",
  "codemode_mcp": true,
  "vsh_prompt_chars": 1324,
  "native_prompt_chars": 1324,
  "runs": 3,
  "git_commit": "c60cfe5e5784a2c21a32f5d52066a5b46e049f71"
}
```
