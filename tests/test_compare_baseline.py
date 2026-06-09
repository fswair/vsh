from __future__ import annotations as _annotations

import json
import sys
from pathlib import Path

PLAYGROUND = Path(__file__).resolve().parent.parent / "playground"
if str(PLAYGROUND) not in sys.path:
    sys.path.insert(0, str(PLAYGROUND))

from compare_baseline import _compare_agent, _compare_playground  # noqa: E402


def test_compare_playground_reports_regression(tmp_path: Path) -> None:
    baseline = tmp_path / "baseline"
    current = tmp_path / "current"
    for name in ("baseline", "current"):
        root = tmp_path / name
        playground = root / "playground"
        playground.mkdir(parents=True)
        payload = {
            "results": [
                {"name": "grep", "mode": "native", "median_ms": 10.0},
                {"name": "grep", "mode": "vsh_full", "median_ms": 2.0},
            ]
        }
        (playground / "results.json").write_text(json.dumps(payload), encoding="utf-8")
    report, ok = _compare_playground(baseline, current)
    assert "grep" in report
    assert ok is True


def test_compare_agent_missing_current_is_not_ok(tmp_path: Path) -> None:
    baseline = tmp_path / "baseline"
    agent = baseline / "agent-context"
    agent.mkdir(parents=True)
    (agent / "comparison.json").write_text(
        json.dumps(
            {
                "comparison": {
                    "vsh": {
                        "duration_ms": 1,
                        "usage": {"input_tokens": 1, "tool_calls": 1},
                        "validation_passed": True,
                    },
                    "native": {
                        "duration_ms": 2,
                        "usage": {"input_tokens": 2, "tool_calls": 1},
                        "validation_passed": True,
                    },
                }
            }
        ),
        encoding="utf-8",
    )
    report, ok = _compare_agent(baseline, tmp_path / "current")
    assert "missing" in report
    assert ok is False
