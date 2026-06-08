from __future__ import annotations as _annotations

import runpy
import sys

import pytest


def test_main_module_executes_cli(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(sys, "argv", ["vsh", "--help"])
    with pytest.raises(SystemExit) as exc_info:
        runpy.run_module("vsh.__main__", run_name="__main__")
    assert exc_info.value.code == 0


def test_main_module_import_does_not_invoke_cli() -> None:
    import importlib

    module = importlib.import_module("vsh.__main__")
    assert callable(module.main)
