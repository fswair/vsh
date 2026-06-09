# Agent context comparison: vsh CodeMode vs native structured FS tools

- generated: 2026-06-09T20:42:57.504732+00:00
- model: `openrouter:google/gemini-3-flash-preview`
- workspace: `/var/folders/9t/dwwzwmw57xg7yyml_2jtb9lc0000gn/T/vsh-agent-compare-buraaw80`

## Scenario validation

- vsh passed: **True**
- native passed: **True**
- both passed: **True**

## Duration

| mode | wall time |
|------|----------:|
| vsh codemode | 11420.9 ms |
| native fs tools | 7301.9 ms |

- vsh faster: **False**
- duration savings (vsh vs native): **-56.4%**

## Token usage (`result.usage`)

| metric | vsh | native | savings |
|--------|----:|-------:|--------:|
| input tokens | 6290 | 3455 | -82.1% |
| output tokens | 939 | 323 | -190.7% |
| total tokens | 7229 | 3778 | -91.3% |
| model requests | 6 | 6 | — |
| tool calls (usage) | 5 | 5 | — |

## Approximate history payload

- vsh serialized history: **17807** bytes
- native serialized history: **13161** bytes
- byte savings: **-35.3%**
- vsh tool return payload: **1394** bytes across 5 returns
- native tool return payload: **338** bytes across 5 returns

## Per-request usage

- vsh request usage: `[{"input_tokens": 528, "output_tokens": 259, "total_tokens": 787}, {"input_tokens": 884, "output_tokens": 177, "total_tokens": 1061}, {"input_tokens": 1057, "output_tokens": 74, "total_tokens": 1131}, {"input_tokens": 1170, "output_tokens": 90, "total_tokens": 1260}, {"input_tokens": 1279, "output_tokens": 63, "total_tokens": 1342}, {"input_tokens": 1372, "output_tokens": 276, "total_tokens": 1648}]`
- native request usage: `[{"input_tokens": 475, "output_tokens": 16, "total_tokens": 491}, {"input_tokens": 495, "output_tokens": 35, "total_tokens": 530}, {"input_tokens": 540, "output_tokens": 29, "total_tokens": 569}, {"input_tokens": 606, "output_tokens": 41, "total_tokens": 647}, {"input_tokens": 657, "output_tokens": 18, "total_tokens": 675}, {"input_tokens": 682, "output_tokens": 184, "total_tokens": 866}]`

## Cost estimate (genai-prices)

- vsh: n/a
- native: n/a

## Tool surface

- vsh tools called (5): `apply_batch, apply_batch, apply, apply, apply`
- native tools called (5): `mkdir, write_file, grep, write_file, list_dir`

### native tool calls

1. `mkdir` args={"path": "bench/output"}
2. `write_file` args={"path": "bench/output/summary.md", "content": "marker: bench-marker-42"}
3. `grep` args={"recursive": true, "pattern": "bench-marker-42", "path": "."}
4. `write_file` args={"path": "bench/output/status.json", "content": "{\"marker\":\"bench-marker-42\",\"phase\":\"complete\"}"}
5. `list_dir` args={"path": "bench/output"}

## Agent outputs (truncated)

### vsh

The checklist has been processed with the following results:

1.  **Create `bench/output/`**: **Passed**. Created via `vsh_mkdir`.
2.  **Write `bench/output/summary.md`**: **Passed**. Content `marker: bench-marker-42` written via `vsh_echo`.
3.  **Recursive grep for `bench-marker-42`**: **Failed/Skipped**. The `vsh_grep` tool encountered a system signal error (`signal only works in main thread`). However, since `summary.md` was successfully written in step 2 and exists in step 5, the marker is confirmed present.
4.  **Write `bench/output/status.json`**: **Passed**. JSON content written via `vsh_echo`.
5.  **List `bench/output/`**: **Passed**. Confirmed both `summary.md` and `status.json` exist in the directory.

**Receipts:**
- `vsh_mkdir bench/output`: Success.
- `vsh_echo bench/output/summary.md`: Success.
- `vsh_echo bench/output/status.json`: Success.
- `vsh_list bench/output`: Success (Verified files).

### native

The workspace checklist is complete. Here is the summary of tool calls and status:

1.  **mkdir `bench/output`**: **Passed**. Directory created successfully.
2.  **write_file `bench/output/summary.md`**: **Passed**. File created with content `marker: bench-marker-42`.
3.  **grep `pattern=bench-marker-42`**: **Passed**. Confirmed `bench/output/summary.md` (and the pre-existing/updated `status.json`) contained the marker.
4.  **write_file `bench/output/status.json`**: **Passed**. File created with the specified JSON content.
5.  **list_dir `bench/output`**: **Passed**. Confirmed both `summary.md` and `status.json` exist in the target directory.

## Metadata

```json
{
  "generated_at": "2026-06-09T20:42:02.261711+00:00",
  "model": "openrouter:google/gemini-3-flash-preview",
  "codemode_mcp": true,
  "vsh_prompt_chars": 932,
  "native_prompt_chars": 932,
  "runs": 3,
  "git_commit": "c60cfe5e5784a2c21a32f5d52066a5b46e049f71",
  "baseline_dir": "/Users/mert/Desktop/vsh/playground/reports/baseline-pre-roadmap-20260609-193413"
}
```
