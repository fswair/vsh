"""Validate the complete cross-platform release set and emit deterministic hashes."""

from __future__ import annotations

import argparse
import hashlib
import tarfile
import zipfile
from pathlib import Path

CRATES = (
    "vsh-types",
    "vsh-store",
    "vsh-vfs",
    "vsh-policy",
    "vsh-commit",
    "vsh-monty",
    "vsh-runtime",
    "vsh-monty-worker",
)
PYTHON_TAGS = ("cp311", "cp312", "cp313", "cp314")
PLATFORM_TAGS = (
    "manylinux_2_28_x86_64",
    "manylinux_2_28_aarch64",
    "macosx_",
    "win_amd64",
)
FORBIDDEN_WHEEL_PATHS = (
    "/vsh/agent/",
    "/vsh/execute/",
    "/vsh/persistence/",
    "/vsh/simulate/",
    "/vsh/snapshot/",
)
FORBIDDEN_GENERATED_SUFFIXES = (".profraw", ".pyc", ".pyo")


def reject_generated_files(names: list[str], archive_name: str) -> None:
    generated = [
        name
        for name in names
        if name.endswith(FORBIDDEN_GENERATED_SUFFIXES)
        or "/__pycache__/" in f"/{name}/"
        or "/target/" in f"/{name}/"
    ]
    if generated:
        raise RuntimeError(f"{archive_name} contains generated build/test files: {generated[:5]}")


def digest(path: Path) -> str:
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def validate_wheel(path: Path, version: str) -> None:
    with zipfile.ZipFile(path) as archive:
        corrupt = archive.testzip()
        if corrupt is not None:
            raise RuntimeError(f"{path.name} contains corrupt member {corrupt}")
        names = archive.namelist()
        reject_generated_files(names, path.name)
        lowered = [f"/{name.lower()}" for name in names]
        workers = [
            name for name in names if name.endswith(("/vsh-monty-worker", "/vsh-monty-worker.exe"))
        ]
        extensions = [
            name
            for name in names
            if (name.startswith("vsh/_native.") or "/vsh/_native." in name)
            and name.endswith((".so", ".pyd"))
        ]
        metadata = [name for name in names if name.endswith(".dist-info/METADATA")]
        notices = [name for name in names if name.endswith("THIRD_PARTY_NOTICES.md")]
        if len(workers) != 1 or len(extensions) != 1 or len(metadata) != 1 or len(notices) != 1:
            raise RuntimeError(
                f"{path.name} missing exact native payload: workers={workers}, "
                f"extensions={extensions}, notices={notices}"
            )
        if not workers[0].endswith(".exe"):
            mode = archive.getinfo(workers[0]).external_attr >> 16
            if mode & 0o111 == 0:
                raise RuntimeError(f"{path.name} worker is not executable")
        if any(forbidden in name for forbidden in FORBIDDEN_WHEEL_PATHS for name in lowered):
            raise RuntimeError(f"{path.name} contains a legacy Python engine path")
        payload = archive.read(metadata[0]).decode("utf-8")
        if "Name: vbash\n" not in payload or f"Version: {version}\n" not in payload:
            raise RuntimeError(f"{path.name} metadata does not identify vbash {version}")


def validate_sdist(path: Path, version: str) -> None:
    prefix = f"vbash-{version}/"
    required = {
        f"{prefix}Cargo.lock",
        f"{prefix}Cargo.toml",
        f"{prefix}crates/vsh-worker/Cargo.toml",
        f"{prefix}crates/vsh-worker/src/child.rs",
        f"{prefix}crates/vsh-worker/src/lib.rs",
        f"{prefix}crates/vsh-worker/src/main.rs",
        f"{prefix}rust-toolchain.toml",
        f"{prefix}scripts/vsh_build_backend.py",
    }
    with tarfile.open(path, "r:gz") as archive:
        names = set(archive.getnames())
        reject_generated_files(sorted(names), path.name)
        missing = sorted(required - names)
        if missing:
            raise RuntimeError(f"{path.name} misses build inputs: {missing}")
        manifest = archive.extractfile(f"{prefix}Cargo.toml")
        if manifest is None or b'"crates/vsh-worker"' not in manifest.read():
            raise RuntimeError(f"{path.name} does not retain the worker workspace member")
    forbidden = (
        f"{prefix}src/vsh/agent/",
        f"{prefix}src/vsh/execute/",
        f"{prefix}src/vsh/simulate/",
        f"{prefix}src/vsh/snapshot/",
    )
    if any(name.startswith(root) for root in forbidden for name in names):
        raise RuntimeError(f"{path.name} contains a legacy Python engine path")


def validate_crate(path: Path, package: str, version: str) -> None:
    with tarfile.open(path, "r:gz") as archive:
        names = archive.getnames()
    reject_generated_files(names, path.name)
    prefix = f"{package}-{version}/"
    if f"{prefix}Cargo.toml" not in names or not any(
        name.startswith(f"{prefix}src/") for name in names
    ):
        raise RuntimeError(f"{path.name} is missing Cargo metadata or Rust sources")


def validate(directory: Path, version: str, wheel_count: int, crate_count: int) -> list[Path]:
    wheels = sorted(directory.glob("*.whl"))
    sdists = sorted(directory.glob("*.tar.gz"))
    crates = sorted(directory.glob("*.crate"))
    if len(wheels) != wheel_count or len(sdists) != 1 or len(crates) != crate_count:
        raise RuntimeError(
            f"artifact cardinality mismatch: wheels={len(wheels)}/{wheel_count}, "
            f"sdists={len(sdists)}/1, crates={len(crates)}/{crate_count}"
        )
    expected_sdist = f"vbash-{version}.tar.gz"
    if sdists[0].name != expected_sdist:
        raise RuntimeError(f"expected {expected_sdist}, found {sdists[0].name}")
    validate_sdist(sdists[0], version)

    for wheel in wheels:
        validate_wheel(wheel, version)
    if wheel_count:
        for python_tag in PYTHON_TAGS:
            matches = [wheel for wheel in wheels if python_tag in wheel.name]
            if len(matches) != wheel_count // len(PYTHON_TAGS):
                raise RuntimeError(f"incomplete {python_tag} wheel set: {len(matches)}")
        for python_tag in PYTHON_TAGS:
            tagged = [wheel for wheel in wheels if python_tag in wheel.name]
            for platform_tag in PLATFORM_TAGS:
                if not any(platform_tag in wheel.name for wheel in tagged):
                    raise RuntimeError(f"{python_tag} wheel set lacks platform tag {platform_tag}")
            tagged_macos = [wheel for wheel in tagged if "macosx_" in wheel.name]
            if not any("_arm64.whl" in wheel.name for wheel in tagged_macos):
                raise RuntimeError(f"{python_tag} wheel set lacks macOS arm64")
            if not any("_x86_64.whl" in wheel.name for wheel in tagged_macos):
                raise RuntimeError(f"{python_tag} wheel set lacks macOS x86_64")
    if crate_count:
        expected_crates = {f"{package}-{version}.crate": package for package in CRATES}
        if {path.name for path in crates} != set(expected_crates):
            raise RuntimeError("crate archive set does not match the eight-package release graph")
        for crate in crates:
            validate_crate(crate, expected_crates[crate.name], version)
    return sorted([*wheels, *sdists, *crates], key=lambda path: path.name)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("directory", type=Path)
    parser.add_argument("--version", required=True)
    parser.add_argument("--wheel-count", type=int, default=20)
    parser.add_argument("--crate-count", type=int, default=8)
    arguments = parser.parse_args()
    artifacts = validate(
        arguments.directory,
        arguments.version,
        arguments.wheel_count,
        arguments.crate_count,
    )
    sums = arguments.directory / "SHA256SUMS"
    sums.write_text(
        "".join(f"{digest(path)}  {path.name}\n" for path in artifacts),
        encoding="utf-8",
    )
    print(f"validated {len(artifacts)} release artifacts; hashes: {sums}")


if __name__ == "__main__":
    main()
