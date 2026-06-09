# Agent context comparison: vsh CodeMode vs native structured FS tools

- generated: 2026-06-09T10:16:14.645999+00:00
- model: `openrouter:google/gemini-3-flash-preview`
- workspace: `/var/folders/9t/dwwzwmw57xg7yyml_2jtb9lc0000gn/T/vsh-agent-compare-5v6u4xny`

## Scenario validation

- vsh passed: **True**
- native passed: **True**
- both passed: **True**

## Duration

| mode | wall time |
|------|----------:|
| vsh codemode | 5555.3 ms |
| native fs tools | 7857.3 ms |

- vsh faster: **True**
- duration savings (vsh vs native): **+29.3%**

## Token usage (`result.usage`)

| metric | vsh | native | savings |
|--------|----:|-------:|--------:|
| input tokens | 1298 | 3455 | +62.4% |
| output tokens | 425 | 284 | -49.6% |
| total tokens | 1723 | 3739 | +53.9% |
| model requests | 2 | 6 | — |
| tool calls (usage) | 1 | 5 | — |

## Approximate history payload

- vsh serialized history: **6769** bytes
- native serialized history: **12941** bytes
- byte savings: **+47.7%**
- vsh tool return payload: **333** bytes across 1 returns
- native tool return payload: **338** bytes across 5 returns

## Per-request usage

- vsh request usage: `[{"input_tokens": 521, "output_tokens": 272, "total_tokens": 793}, {"input_tokens": 777, "output_tokens": 153, "total_tokens": 930}]`
- native request usage: `[{"input_tokens": 475, "output_tokens": 16, "total_tokens": 491}, {"input_tokens": 495, "output_tokens": 35, "total_tokens": 530}, {"input_tokens": 540, "output_tokens": 29, "total_tokens": 569}, {"input_tokens": 606, "output_tokens": 41, "total_tokens": 647}, {"input_tokens": 657, "output_tokens": 18, "total_tokens": 675}, {"input_tokens": 682, "output_tokens": 145, "total_tokens": 827}]`

## Cost estimate (genai-prices)

- vsh: n/a
- native: n/a

## Tool surface

- vsh tools called (1): `apply_batch`
- native tools called (5): `mkdir, write_file, grep, write_file, list_dir`

### native tool calls

1. `mkdir` args={"path": "bench/output"}
2. `write_file` args={"path": "bench/output/summary.md", "content": "marker: bench-marker-42"}
3. `grep` args={"recursive": true, "path": ".", "pattern": "bench-marker-42"}
4. `write_file` args={"content": "{\"marker\":\"bench-marker-42\",\"phase\":\"complete\"}", "path": "bench/output/status.json"}
5. `list_dir` args={"path": "bench/output"}

## Agent outputs (truncated)

### vsh

The checklist has been successfully completed using a single `apply_batch` call.

### Receipt Summary:
1. **vsh_mkdir**: Passed. Created `bench/output/`.
2. **vsh_echo**: Passed. Created `bench/output/summary.md` with the marker.
3. **vsh_grep**: Passed. Confirmed "marker: bench-marker-42" exists in the file.
4. **vsh_echo**: Passed. Created `bench/output/status.json` with the required JSON payload.
5. **vsh_list**: Passed. Confirmed both `summary.md` and `status.json` are present in `bench/output/`.

### native

The workspace checklist has been completed successfully:

1.  **mkdir**: `bench/output` created. (Pass)
2.  **write_file**: `bench/output/summary.md` created with content `marker: bench-marker-42`. (Pass)
3.  **grep**: Found `bench-marker-42` in `bench/output/summary.md`. (Pass)
4.  **write_file**: `bench/output/status.json` created with JSON content. (Pass)
5.  **list_dir**: Verified `summary.md` and `status.json` exist in `bench/output`. (Pass)

## Metadata

```json
{
  "generated_at": "2026-06-09T10:16:00.585830+00:00",
  "model": "openrouter:google/gemini-3-flash-preview",
  "codemode_mcp": true,
  "vsh_prompt_chars": 932,
  "native_prompt_chars": 932
}
```
