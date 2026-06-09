# Agent context comparison: vsh CodeMode vs native structured FS tools

- generated: 2026-06-09T21:00:28.913668+00:00
- model: `openrouter:google/gemini-3-flash-preview`
- workspace: `/var/folders/9t/dwwzwmw57xg7yyml_2jtb9lc0000gn/T/vsh-agent-compare-87n6m_dm`

## Scenario validation

- vsh passed: **True**
- native passed: **True**
- both passed: **True**

## Duration

| mode | wall time |
|------|----------:|
| vsh codemode | 10032.2 ms |
| native fs tools | 9201.3 ms |

- vsh faster: **False**
- duration savings (vsh vs native): **-9.0%**

## Token usage (`result.usage`)

| metric | vsh | native | savings |
|--------|----:|-------:|--------:|
| input tokens | 4606 | 4160 | -10.7% |
| output tokens | 819 | 321 | -155.1% |
| total tokens | 5425 | 4481 | -21.1% |
| model requests | 5 | 7 | — |
| tool calls (usage) | 4 | 5 | — |

## Approximate history payload

- vsh serialized history: **15233** bytes
- native serialized history: **14986** bytes
- byte savings: **-1.6%**
- vsh tool return payload: **1204** bytes across 4 returns
- native tool return payload: **343** bytes across 5 returns

## Per-request usage

- vsh request usage: `[{"input_tokens": 528, "output_tokens": 233, "total_tokens": 761}, {"input_tokens": 741, "output_tokens": 148, "total_tokens": 889}, {"input_tokens": 935, "output_tokens": 150, "total_tokens": 1085}, {"input_tokens": 1137, "output_tokens": 115, "total_tokens": 1252}, {"input_tokens": 1265, "output_tokens": 173, "total_tokens": 1438}]`
- native request usage: `[{"input_tokens": 475, "output_tokens": 16, "total_tokens": 491}, {"input_tokens": 495, "output_tokens": 35, "total_tokens": 530}, {"input_tokens": 540, "output_tokens": 35, "total_tokens": 575}, {"input_tokens": 581, "output_tokens": 29, "total_tokens": 610}, {"input_tokens": 647, "output_tokens": 41, "total_tokens": 688}, {"input_tokens": 698, "output_tokens": 18, "total_tokens": 716}, {"input_tokens": 724, "output_tokens": 147, "total_tokens": 871}]`

## Cost estimate (genai-prices)

- vsh: n/a
- native: n/a

## Tool surface

- vsh tools called (4): `apply_batch, apply_batch, apply_batch, apply_batch`
- native tools called (6): `mkdir, write_file, grep, grep, write_file, list_dir`

### native tool calls

1. `mkdir` args={"path": "bench/output"}
2. `write_file` args={"path": "bench/output/summary.md", "content": "marker: bench-marker-42"}
3. `grep` args={"parent_only": true, "recursive": true, "path": ".", "pattern": "bench-marker-42"}
4. `grep` args={"recursive": true, "pattern": "bench-marker-42", "path": "."}
5. `write_file` args={"path": "bench/output/status.json", "content": "{\"marker\":\"bench-marker-42\",\"phase\":\"complete\"}"}
6. `list_dir` args={"path": "bench/output"}

## Agent outputs (truncated)

### vsh

The workspace checklist is complete.

### Summary of Receipts
1.  **Create Directory**: `bench/output/` created successfully using `vsh_mkdir`.
2.  **Write summary.md**: `bench/output/summary.md` written with `marker: bench-marker-42`.
3.  **Recursive Grep**: Attempted with `vsh_grep`, but encountered a system limitation. Verification was indirectly confirmed by the existence and content of the file in step 2.
4.  **Write status.json**: `bench/output/status.json` written with the specified JSON payload.
5.  **List Files**: `vsh_list` confirmed both `summary.md` and `status.json` exist in `bench/output/`.

All mutation steps passed.

### native

The checklist is complete. Here is the summary of the steps:

1.  **mkdir**: Created `bench/output`. (Passed)
2.  **write_file**: Created `bench/output/summary.md` with marker content. (Passed)
3.  **grep**: Confirmed `bench-marker-42` exists in `bench/output/summary.md`. (Passed)
4.  **write_file**: Created `bench/output/status.json` with the specified JSON content. (Passed)
5.  **list_dir**: Confirmed both `summary.md` and `status.json` exist in `bench/output`. (Passed)

## Metadata

```json
{
  "generated_at": "2026-06-09T20:59:36.912593+00:00",
  "model": "openrouter:google/gemini-3-flash-preview",
  "codemode_mcp": true,
  "vsh_prompt_chars": 932,
  "native_prompt_chars": 932,
  "runs": 3,
  "git_commit": "c60cfe5e5784a2c21a32f5d52066a5b46e049f71",
  "baseline_dir": "/Users/mert/Desktop/vsh/playground/reports/baseline-pre-roadmap-20260609-193413"
}
```
