from __future__ import annotations as _annotations

import hashlib
import json

from vsh.schemas import StructuredCommand

__all__ = ("compute_plan_fingerprint",)


def compute_plan_fingerprint(
    *,
    snapshot_id: str,
    command: StructuredCommand,
    shell_preview: str,
    path_fingerprints: dict[str, str],
) -> str:
    payload = {
        "snapshot_id": snapshot_id,
        "command": command.model_dump(mode="json"),
        "shell_preview": shell_preview,
        "path_fingerprints": dict(sorted(path_fingerprints.items())),
    }
    encoded = json.dumps(payload, sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(encoded.encode()).hexdigest()[:16]
