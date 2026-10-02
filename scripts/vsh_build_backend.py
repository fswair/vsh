"""PEP 517 wrapper bundling separately built Monty and bounded Bash workers."""

from __future__ import annotations

import os
import shutil
import stat
import subprocess
from collections.abc import Iterator, Mapping
from contextlib import contextmanager
from pathlib import Path
from typing import Any

from maturin import build_editable as _maturin_build_editable
from maturin import build_sdist as build_sdist
from maturin import build_wheel as _maturin_build_wheel
from maturin import get_requires_for_build_editable as get_requires_for_build_editable
from maturin import get_requires_for_build_sdist as get_requires_for_build_sdist
from maturin import get_requires_for_build_wheel as get_requires_for_build_wheel
from maturin import (
    prepare_metadata_for_build_editable as prepare_metadata_for_build_editable,
)
from maturin import prepare_metadata_for_build_wheel as prepare_metadata_for_build_wheel

_ROOT = Path(__file__).resolve().parents[1]
_WORKER_NAME = "vsh-monty-worker"
_BASH_WORKER_NAME = "vsh-bash-worker"
_MONTY_VERSION = "0.0.22"


def _target_root() -> Path:
    configured = os.environ.get("CARGO_TARGET_DIR")
    if configured is None:
        return _ROOT / "target"
    path = Path(configured)
    return path if path.is_absolute() else _ROOT / path


def _worker_filename(name: str = _WORKER_NAME) -> str:
    target = os.environ.get("CARGO_BUILD_TARGET", "")
    windows = "windows" in target if target else os.name == "nt"
    return f"{name}.exe" if windows else name


def _built_worker(name: str = _WORKER_NAME) -> Path:
    target = os.environ.get("CARGO_BUILD_TARGET")
    directory = _target_root()
    if target:
        directory /= target
    return directory / "release" / _worker_filename(name)


def _build_worker(name: str = _WORKER_NAME) -> Path:
    package = "vsh-bash" if name == _BASH_WORKER_NAME else "vsh-monty-worker"
    features = (
        ["--no-default-features", "--features", "worker", "--bin", name]
        if name == _BASH_WORKER_NAME
        else []
    )
    subprocess.run(
        [
            "cargo",
            "build",
            "--package",
            package,
            "--release",
            "--locked",
            *features,
        ],
        cwd=_ROOT,
        check=True,
    )
    worker = _built_worker(name)
    if not worker.is_file():
        raise RuntimeError(f"Cargo did not produce {name} at {worker}")
    if os.environ.get("CARGO_BUILD_TARGET") is None:
        completed = subprocess.run(
            [worker, "--version"],
            check=True,
            capture_output=True,
            text=True,
            timeout=10,
        )
        valid = (
            completed.stdout.split()[-1:] == [_MONTY_VERSION]
            if name == _WORKER_NAME
            else "vsh-bash-worker/4 bashkit/0.18.2 vsh/" in completed.stdout
        )
        if not valid:
            raise RuntimeError(
                f"worker must report Monty {_MONTY_VERSION}, got {completed.stdout.strip()!r}"
            )
    return worker


@contextmanager
def _stage_worker(name: str = _WORKER_NAME) -> Iterator[None]:
    worker = _build_worker(name)
    scripts = _ROOT / "python-data" / "scripts"
    scripts.mkdir(parents=True, exist_ok=True)
    staged = scripts / _worker_filename(name)
    backup = scripts / f".{staged.name}.previous-{os.getpid()}"
    had_previous = staged.exists()
    if had_previous:
        os.replace(staged, backup)
    try:
        shutil.copy2(worker, staged)
        staged.chmod(staged.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
        yield
    finally:
        staged.unlink(missing_ok=True)
        if had_previous:
            os.replace(backup, staged)


def build_wheel(
    wheel_directory: str,
    config_settings: Mapping[str, Any] | None = None,
    metadata_directory: str | None = None,
) -> str:
    """Build a PyO3 wheel containing its matching supervised worker executable."""
    with _stage_worker(), _stage_worker(_BASH_WORKER_NAME):
        return _maturin_build_wheel(wheel_directory, config_settings, metadata_directory)


def build_editable(
    wheel_directory: str,
    config_settings: Mapping[str, Any] | None = None,
    metadata_directory: str | None = None,
) -> str:
    """Build an editable wheel containing its matching supervised worker executable."""
    with _stage_worker(), _stage_worker(_BASH_WORKER_NAME):
        return _maturin_build_editable(wheel_directory, config_settings, metadata_directory)
