"""Reject interpreter/authority dependencies crossing execution feature boundaries."""

from __future__ import annotations

import subprocess

checks = (
    ("default Rust SDK", "vsh", (), {"bashkit", "tokio"}),
    ("Python parent", "vsh-python", (), {"bashkit", "tokio"}),
    (
        "Bash worker",
        "vsh-bash",
        ("--no-default-features", "--features", "worker"),
        {"vsh-runtime", "vsh-store", "vsh-commit", "vsh-monty", "vsh-execution"},
    ),
)

for label, package, features, forbidden in checks:
    tree = subprocess.check_output(
        ["cargo", "tree", "--locked", "-p", package, "-e", "normal", "--prefix", "none", *features],
        text=True,
    )
    packages = {line.split()[0] for line in tree.splitlines() if line.strip()}
    unexpected = packages & forbidden
    if unexpected:
        raise RuntimeError(f"{label} includes forbidden dependencies: {sorted(unexpected)}")
    print(f"{label}: execution dependency boundary verified")
