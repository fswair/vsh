from __future__ import annotations as _annotations

import pytest

import vsh
from vsh import _native


def test_native_binding_owns_version_and_engine_identity() -> None:
    assert vsh.__version__ == "0.3.0"
    assert _native.__version__ == vsh.__version__
    assert _native.version() == vsh.__version__
    assert vsh.engine_kind() == "rust"


def test_native_budget_exposes_the_worker_memory_limit() -> None:
    budget = vsh.ExecutionBudget(
        max_memory_bytes=8 * 1024 * 1024,
        max_io_call_bytes=2 * 1024 * 1024,
        max_path_bytes=4096,
    )

    assert budget.max_memory_bytes == 8 * 1024 * 1024
    assert budget.max_io_call_bytes == 2 * 1024 * 1024
    assert budget.max_path_bytes == 4096


def test_native_exception_hierarchy_has_one_catchable_base() -> None:
    assert issubclass(vsh.VshRuntimeError, RuntimeError)
    assert issubclass(vsh.VshExecutionError, vsh.VshRuntimeError)
    assert issubclass(vsh.VshStateError, vsh.VshRuntimeError)
    assert issubclass(vsh.VshStaleError, vsh.VshRuntimeError)
    assert issubclass(vsh.VshRecoveryError, vsh.VshRuntimeError)
    assert issubclass(vsh.VshInternalError, vsh.VshRuntimeError)


@pytest.mark.parametrize(
    ("raw", "normalized"),
    [
        (".", "."),
        ("src\\vsh/./core/../lib.rs", "src/vsh/lib.rs"),
        ("a/../b", "b"),
    ],
)
def test_native_path_normalization_matches_rust_contract(raw: str, normalized: str) -> None:
    assert vsh.normalize_path(raw) == normalized


@pytest.mark.parametrize("raw", ["", "/etc/passwd", "C:\\Windows", "../secret"])
def test_native_path_normalization_maps_rust_errors(raw: str) -> None:
    with pytest.raises(ValueError):
        vsh.normalize_path(raw)
