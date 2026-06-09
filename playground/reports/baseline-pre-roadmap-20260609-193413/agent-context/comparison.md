# Agent context comparison: vsh CodeMode vs native structured FS tools

- generated: 2026-06-09T19:46:56.937393+00:00
- model: `openrouter:google/gemini-3-flash-preview`
- workspace: `/var/folders/9t/dwwzwmw57xg7yyml_2jtb9lc0000gn/T/vsh-agent-compare-wwbox3xx`

## Scenario validation

- vsh passed: **True**
- native passed: **True**
- both passed: **True**

## Duration

| mode | wall time |
|------|----------:|
| vsh codemode | 12253.9 ms |
| native fs tools | 9254.2 ms |

- vsh faster: **False**
- duration savings (vsh vs native): **-32.4%**

## Token usage (`result.usage`)

| metric | vsh | native | savings |
|--------|----:|-------:|--------:|
| input tokens | 7359 | 4135 | -78.0% |
| output tokens | 1031 | 313 | -229.4% |
| total tokens | 8390 | 4448 | -88.6% |
| model requests | 6 | 7 | — |
| tool calls (usage) | 5 | 5 | — |

## Approximate history payload

- vsh serialized history: **19141** bytes
- native serialized history: **14968** bytes
- byte savings: **-27.9%**
- vsh tool return payload: **2458** bytes across 5 returns
- native tool return payload: **338** bytes across 5 returns

## Per-request usage

- vsh request usage: `[{"input_tokens": 521, "output_tokens": 232, "total_tokens": 753}, {"input_tokens": 860, "output_tokens": 232, "total_tokens": 1092}, {"input_tokens": 1150, "output_tokens": 205, "total_tokens": 1355}, {"input_tokens": 1421, "output_tokens": 149, "total_tokens": 1570}, {"input_tokens": 1665, "output_tokens": 50, "total_tokens": 1715}, {"input_tokens": 1742, "output_tokens": 163, "total_tokens": 1905}]`
- native request usage: `[{"input_tokens": 475, "output_tokens": 16, "total_tokens": 491}, {"input_tokens": 495, "output_tokens": 35, "total_tokens": 530}, {"input_tokens": 540, "output_tokens": 29, "total_tokens": 569}, {"input_tokens": 575, "output_tokens": 29, "total_tokens": 604}, {"input_tokens": 641, "output_tokens": 41, "total_tokens": 682}, {"input_tokens": 692, "output_tokens": 18, "total_tokens": 710}, {"input_tokens": 717, "output_tokens": 145, "total_tokens": 862}]`

## Cost estimate (genai-prices)

- vsh: n/a
- native: n/a

## Tool surface

- vsh tools called (5): `apply_batch, apply_batch, apply_batch, apply_batch, apply`
- native tools called (6): `mkdir, write_file, grep, grep, write_file, list_dir`

### native tool calls

1. `mkdir` args={"path": "bench/output"}
2. `write_file` args={"path": "bench/output/summary.md", "content": "marker: bench-marker-42"}
3. `grep` args={"recursive": true, "parent": ".", "pattern": "bench-marker-42"}
4. `grep` args={"path": ".", "recursive": true, "pattern": "bench-marker-42"}
5. `write_file` args={"content": "{\"marker\":\"bench-marker-42\",\"phase\":\"complete\"}", "path": "bench/output/status.json"}
6. `list_dir` args={"path": "bench/output"}

## Agent outputs (truncated)

### vsh

The checklist is complete.

### Checklist Receipts:
1.  **Create `bench/output/`**: Passed. Directory created recursively using `vsh_mkdir`.
2.  **Write `summary.md`**: Passed. Wrote `marker: bench-marker-42` using `vsh_echo`.
3.  **Recursive grep**: Passed. `vsh_grep` confirmed the marker exists in `bench/output/summary.md`.
4.  **Write `status.json`**: Passed. Wrote JSON content using `vsh_echo`.
5.  **List `bench/output/`**: Passed. `vsh_list` confirmed both `summary.md` and `status.json` are present in the directory.

### native

The checklist has been completed successfully:

1.  **mkdir**: Path `bench/output` was created. (Pass)
2.  **write_file**: `bench/output/summary.md` was created with the required marker. (Pass)
3.  **grep**: Found `bench-marker-42` in `bench/output/summary.md`. (Pass)
4.  **write_file**: `bench/output/status.json` was created with the required JSON content. (Pass)
5.  **list_dir**: Confirmed both `summary.md` and `status.json` exist in `bench/output`. (Pass)

## Metadata

```json
{
  "generated_at": "2026-06-09T19:46:35.078980+00:00",
  "model": "openrouter:google/gemini-3-flash-preview",
  "codemode_mcp": true,
  "vsh_prompt_chars": 932,
  "native_prompt_chars": 932
}
```
