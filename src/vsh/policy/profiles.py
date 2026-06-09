from __future__ import annotations as _annotations

import tomllib
from dataclasses import dataclass
from pathlib import Path
from typing import Literal, cast

PolicyPreset = Literal["strict", "balanced", "yolo"]

__all__ = ("PolicyProfile", "load_policy_profile")


@dataclass(frozen=True, slots=True)
class PolicyProfile:
    name: PolicyPreset
    protected_patterns: tuple[str, ...]
    max_touched_paths: int
    require_execution_reason: bool


_PRESETS: dict[PolicyPreset, PolicyProfile] = {
    "strict": PolicyProfile(
        name="strict",
        protected_patterns=(".env", ".env.*", "*.pem", "*.key", ".git/**"),
        max_touched_paths=100,
        require_execution_reason=True,
    ),
    "balanced": PolicyProfile(
        name="balanced",
        protected_patterns=(".env", ".env.*", "*.pem", "*.key"),
        max_touched_paths=500,
        require_execution_reason=True,
    ),
    "yolo": PolicyProfile(
        name="yolo",
        protected_patterns=(".env",),
        max_touched_paths=2000,
        require_execution_reason=False,
    ),
}


def load_policy_profile(workspace_root: str | Path) -> PolicyProfile:
    root = Path(workspace_root)
    for candidate in (root / "vsh.toml", root / ".vsh" / "policy.toml"):
        if not candidate.is_file():
            continue
        payload = tomllib.loads(candidate.read_text(encoding="utf-8"))
        preset = str(payload.get("preset", "balanced"))
        if preset not in _PRESETS:
            msg = f"unknown policy preset: {preset!r}"
            raise ValueError(msg)
        base = _PRESETS[cast(PolicyPreset, preset)]
        extra = payload.get("protected_patterns", [])
        patterns = tuple({*base.protected_patterns, *(str(item) for item in extra)})
        max_touched = int(payload.get("max_touched_paths", base.max_touched_paths))
        require_reason = bool(
            payload.get("require_execution_reason", base.require_execution_reason)
        )
        return PolicyProfile(
            name=base.name,
            protected_patterns=patterns,
            max_touched_paths=max_touched,
            require_execution_reason=require_reason,
        )
    return _PRESETS["balanced"]
