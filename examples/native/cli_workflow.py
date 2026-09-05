"""Exercise actual separate CLI processes against an owned temporary workspace."""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path
from tempfile import TemporaryDirectory


def run_workflow() -> dict[str, object]:
    with TemporaryDirectory(prefix="vsh-cookbook-cli-") as directory:
        workspace = Path(directory)
        command = [
            sys.executable,
            "-c",
            "from vsh.cli import main; main()",
            "run",
            "--workspace",
            str(workspace),
        ]
        code = "from pathlib import Path\nPath('/workspace/output.txt').write_text('ready')"
        preview = json.loads(
            subprocess.run(
                [*command, "--code", code, "--detail", "full"],
                check=True,
                capture_output=True,
                text=True,
            ).stdout
        )
        assert preview["decision"] == "auto_approved"
        assert not (workspace / "output.txt").exists()
        # An auto-approved artifact belonged to the now-exited first process.
        lost = subprocess.run(
            [*command, "--transaction", preview["transaction"], "--mode", "auto"],
            check=False,
            capture_output=True,
            text=True,
        )
        assert lost.returncode != 0
        assert not (workspace / "output.txt").exists()
        # Explicit one-shot automation is a new execution, not preview promotion.
        committed = json.loads(
            subprocess.run(
                [
                    *command,
                    "--code",
                    code,
                    "--mode",
                    "auto",
                    "--intent",
                    "Known fixture automation",
                ],
                check=True,
                capture_output=True,
                text=True,
            ).stdout
        )
        assert committed["commit"]["committed"] is True
        assert (workspace / "output.txt").read_text() == "ready"
        return {"lost_preview_rejected": True, "one_shot_state": committed["state"]}


if __name__ == "__main__":
    print(json.dumps(run_workflow(), indent=2))
