"""Exercise an installed wheel without importing the repository source tree."""

from __future__ import annotations

import os
import tempfile
from pathlib import Path

from vsh import RunRequest, Runtime, __version__, engine_kind


def exercise_runtime(workspace: Path) -> None:
    """Keep native handles inside a scope that ends before workspace cleanup."""
    runtime = Runtime.open(workspace)
    preview = runtime.preview(
        RunRequest(
            "from pathlib import Path\n"
            "Path('/workspace/release-smoke.txt').write_text('native-wheel')\n"
            "'ok'\n",
            intent="release wheel smoke",
        )
    )
    if preview.state != "auto_approved" or preview.changed_paths != 1:
        raise RuntimeError(
            f"unexpected preview state={preview.state!r} changed_paths={preview.changed_paths}"
        )
    committed = runtime.commit(preview.transaction, 0)
    if committed.state != "committed":
        raise RuntimeError(f"unexpected commit state {committed.state!r}")
    if (workspace / "release-smoke.txt").read_text(encoding="utf-8") != "native-wheel":
        raise RuntimeError("native commit did not produce the expected host file")


def main() -> None:
    expected = os.environ.get("VSH_EXPECT_VERSION")
    if expected is not None and __version__ != expected:
        raise RuntimeError(f"installed version {__version__!r} does not match {expected!r}")
    if engine_kind() != "rust":
        raise RuntimeError(f"wheel loaded unexpected engine {engine_kind()!r}")

    with tempfile.TemporaryDirectory(prefix="vsh-release-smoke-") as raw_workspace:
        workspace = Path(raw_workspace)
        exercise_runtime(workspace)

    print(f"vbash {__version__}: rust engine, bundled worker, preview, and commit verified")


if __name__ == "__main__":
    main()
