from __future__ import annotations as _annotations

from pathlib import Path

from vsh.policy import load_policy_profile


def test_load_policy_profile_defaults_to_balanced(tmp_path: Path) -> None:
    profile = load_policy_profile(tmp_path)
    assert profile.name == "balanced"
    assert profile.require_execution_reason is True


def test_load_policy_profile_from_vsh_toml(tmp_path: Path) -> None:
    (tmp_path / "vsh.toml").write_text('preset = "yolo"\n', encoding="utf-8")
    profile = load_policy_profile(tmp_path)
    assert profile.name == "yolo"
    assert profile.require_execution_reason is False
