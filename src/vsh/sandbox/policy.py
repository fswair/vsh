from __future__ import annotations as _annotations

from typing import Literal

SandboxPolicy = Literal[
    "read_only",
    "read_write",
    "write_only",
    "delete_only",
    "read_delete_write",
    "no_delete",
    "no_write",
    "no_read",
    "yolo",
]

EffectKind = Literal["read", "write", "delete", "rename"]

__all__ = (
    "EffectKind",
    "SandboxPolicy",
    "allowed_effect_kinds",
    "classify_simulation_effects",
    "effect_kinds_allowed_by_policy",
    "mount_mode_for_policy",
    "policy_allows_simulation",
)

_POLICY_EFFECTS: dict[SandboxPolicy, frozenset[EffectKind] | None] = {
    "yolo": None,
    "read_only": frozenset({"read"}),
    "read_write": frozenset({"read", "write"}),
    "write_only": frozenset({"write"}),
    "delete_only": frozenset({"delete"}),
    "read_delete_write": frozenset({"read", "write", "delete", "rename"}),
    "no_delete": frozenset({"read", "write", "rename"}),
    "no_write": frozenset({"read", "delete", "rename"}),
    "no_read": frozenset({"write", "delete", "rename"}),
}


def effect_kinds_allowed_by_policy(policy: SandboxPolicy) -> frozenset[EffectKind] | None:
    """Return allowed effect kinds, or None when all effects are allowed."""
    return _POLICY_EFFECTS[policy]


def allowed_effect_kinds(policy: SandboxPolicy) -> frozenset[EffectKind] | None:
    """Alias for effect_kinds_allowed_by_policy."""
    return effect_kinds_allowed_by_policy(policy)


def classify_simulation_effects(
    *,
    reads: list[str],
    creates: list[str],
    updates: list[str],
    deletes: list[str],
    renames: list[tuple[str, str]],
    content_reads: set[str] | None = None,
) -> set[EffectKind]:
    """Classify a simulation into coarse effect kinds for policy checks."""
    kinds: set[EffectKind] = set()
    if reads or content_reads:
        kinds.add("read")
    if creates or updates:
        kinds.add("write")
    if deletes:
        kinds.add("delete")
    if renames:
        kinds.add("rename")
    return kinds


def policy_allows_simulation(policy: SandboxPolicy, effect_kinds: set[EffectKind]) -> bool:
    """Return whether the policy permits the given effect kinds."""
    allowed = effect_kinds_allowed_by_policy(policy)
    if allowed is None:
        return True
    return effect_kinds.issubset(allowed)


def mount_mode_for_policy(policy: SandboxPolicy) -> Literal["read-only", "read-write", "overlay"]:
    """Map sandbox policy to a Monty MountDir mode for workspace introspection."""
    if policy in {"read_only", "no_write", "no_read"}:
        return "read-only"
    if policy == "delete_only":
        return "read-only"
    return "overlay"
