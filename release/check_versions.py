"""Fail a release before building when Rust, Python, and tag versions diverge."""

from __future__ import annotations

import argparse
import json
import subprocess
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
RUST_PACKAGES = {
    "vsh-commit",
    "vsh-monty",
    "vsh-monty-worker",
    "vsh-policy",
    "vsh-python",
    "vsh-runtime",
    "vsh-store",
    "vsh-types",
    "vsh-vfs",
}


def project_version() -> str:
    with (ROOT / "pyproject.toml").open("rb") as stream:
        document = tomllib.load(stream)
    value = document["project"]["version"]
    if not isinstance(value, str):
        raise TypeError("project.version must be a string")
    return value


def cargo_versions() -> dict[str, str]:
    completed = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--no-deps", "--locked"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    metadata = json.loads(completed.stdout)
    packages = metadata["packages"]
    return {
        package["name"]: package["version"]
        for package in packages
        if package["name"] in RUST_PACKAGES
    }


def check(tag: str | None) -> str:
    version = project_version()
    observed = cargo_versions()
    if set(observed) != RUST_PACKAGES:
        missing = sorted(RUST_PACKAGES - set(observed))
        unexpected = sorted(set(observed) - RUST_PACKAGES)
        raise RuntimeError(
            f"workspace package mismatch: missing={missing}, unexpected={unexpected}"
        )
    mismatched = {name: value for name, value in observed.items() if value != version}
    if mismatched:
        raise RuntimeError(f"workspace versions do not match Python {version}: {mismatched}")
    source = (ROOT / "src/vsh/_version.py").read_text(encoding="utf-8")
    if "__version__ = _native_version()" not in source:
        raise RuntimeError("src/vsh/_version.py does not delegate to the native crate version")
    if tag is not None and tag != f"v{version}":
        raise RuntimeError(f"release tag {tag!r} does not match v{version}")
    return version


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--tag")
    parser.add_argument("--github-output", type=Path)
    arguments = parser.parse_args()
    version = check(arguments.tag)
    if arguments.github_output is not None:
        with arguments.github_output.open("a", encoding="utf-8") as stream:
            stream.write(f"version={version}\n")
    print(version)


if __name__ == "__main__":
    main()
