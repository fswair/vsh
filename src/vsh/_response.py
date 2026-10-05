"""Agent presentation bounds, independent of native evidence and commit budgets."""

from __future__ import annotations

import json

DEFAULT_RESPONSE_BYTES = 16 * 1024
MIN_RESPONSE_BYTES = 4096


def response_size(value: object) -> int:
    """Measure compact ASCII JSON bytes, including escaping and field names."""
    return len(json.dumps(value, ensure_ascii=True, allow_nan=False, separators=(",", ":")))


def bound_mcp_response(payload: dict[str, object], maximum: int) -> dict[str, object]:
    """Omit display fields when necessary without changing the transaction outcome."""
    result = {**payload, "response_truncated": False}
    for field in ("result_repr", "stdout", "stderr", "bash", "changes", "deny_reason"):
        if response_size(result) <= maximum:
            break
        result["response_truncated"] = True
        result[field] = (
            [] if field == "changes" else None if field in {"bash", "deny_reason"} else ""
        )
        if field in {"result_repr", "stdout", "stderr"}:
            result["result_truncated" if field == "result_repr" else f"{field}_truncated"] = True
    # The remaining native scalar fields have fixed bounds; never replace a
    # committed receipt with an error merely because presentation was too large.
    return result
