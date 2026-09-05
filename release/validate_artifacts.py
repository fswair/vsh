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
    "vsh",
    "vbash",
    "vsh-monty-worker",
)
PRIMARY_PYTHON_NAME = "vsh-python"
PRIMARY_PYTHON_STEM = "vsh_python"
COMPAT_PYTHON_NAME = "vbash"
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


def validate_primary_wheel(path: Path, version: str) -> None:
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
        if (
            f"Name: {PRIMARY_PYTHON_NAME}\n" not in payload
            or f"Version: {version}\n" not in payload
        ):
            raise RuntimeError(
                f"{path.name} metadata does not identify {PRIMARY_PYTHON_NAME} {version}"
            )


def validate_primary_sdist(path: Path, version: str) -> None:
    prefix = f"{PRIMARY_PYTHON_STEM}-{version}/"
    required = {
        f"{prefix}Cargo.lock",
        f"{prefix}Cargo.toml",
        f"{prefix}LICENSE",
        f"{prefix}crates/vbash/Cargo.toml",
        f"{prefix}crates/vbash/src/lib.rs",
        f"{prefix}crates/vsh/Cargo.toml",
        f"{prefix}crates/vsh/src/lib.rs",
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
        if manifest is None:
            raise RuntimeError(f"{path.name} does not contain the workspace manifest")
        workspace_manifest = manifest.read()
        required_members = (b'"crates/vbash"', b'"crates/vsh"', b'"crates/vsh-worker"')
        if any(member not in workspace_manifest for member in required_members):
            raise RuntimeError(f"{path.name} does not retain the required workspace members")
        if b'get-size2 = { version = "=0.10.3"' not in workspace_manifest:
            raise RuntimeError(f"{path.name} is missing the downstream resolution guard")
    forbidden = (
        f"{prefix}src/vsh/agent/",
        f"{prefix}src/vsh/execute/",
        f"{prefix}src/vsh/simulate/",
        f"{prefix}src/vsh/snapshot/",
    )
    if any(name.startswith(root) for root in forbidden for name in names):
        raise RuntimeError(f"{path.name} contains a legacy Python engine path")


def validate_compat_metadata(payload: str, version: str, archive_name: str) -> None:
    expected_dependency = f"Requires-Dist: {PRIMARY_PYTHON_NAME}=={version}"
    expected_mcp_dependency = f"Requires-Dist: {PRIMARY_PYTHON_NAME}[mcp]=={version}"
    if (
        f"Name: {COMPAT_PYTHON_NAME}\n" not in payload
        or f"Version: {version}\n" not in payload
        or expected_dependency not in payload
        or expected_mcp_dependency not in payload
    ):
        raise RuntimeError(
            f"{archive_name} does not mirror exact {PRIMARY_PYTHON_NAME} {version} metadata"
        )


def validate_compat_wheel(path: Path, version: str) -> None:
    """Require the vbash wheel to contain metadata only, never an import package."""
    with zipfile.ZipFile(path) as archive:
        corrupt = archive.testzip()
        if corrupt is not None:
            raise RuntimeError(f"{path.name} contains corrupt member {corrupt}")
        names = archive.namelist()
        reject_generated_files(names, path.name)
        payload_files = [name for name in names if ".dist-info/" not in name]
        if payload_files:
            raise RuntimeError(f"{path.name} compatibility wheel contains code: {payload_files}")
        metadata = [name for name in names if name.endswith(".dist-info/METADATA")]
        if len(metadata) != 1:
            raise RuntimeError(f"{path.name} must contain one metadata file")
        validate_compat_metadata(archive.read(metadata[0]).decode("utf-8"), version, path.name)


def validate_compat_sdist(path: Path, version: str) -> None:
    """Require the vbash source distribution to contain packaging metadata only."""
    prefix = f"{COMPAT_PYTHON_NAME}-{version}/"
    with tarfile.open(path, "r:gz") as archive:
        names = archive.getnames()
        reject_generated_files(names, path.name)
        required = {f"{prefix}README.md", f"{prefix}pyproject.toml", f"{prefix}PKG-INFO"}
        missing = sorted(required - set(names))
        if missing:
            raise RuntimeError(f"{path.name} misses compatibility metadata: {missing}")
        source_files = [name for name in names if name.endswith((".py", ".pyi", ".so", ".pyd"))]
        if source_files:
            raise RuntimeError(f"{path.name} compatibility sdist contains code: {source_files}")
        metadata = archive.extractfile(f"{prefix}PKG-INFO")
        if metadata is None:
            raise RuntimeError(f"{path.name} does not contain PKG-INFO")
        validate_compat_metadata(metadata.read().decode("utf-8"), version, path.name)


def validate_crate(path: Path, package: str, version: str) -> None:
    with tarfile.open(path, "r:gz") as archive:
        names = archive.getnames()
        prefix = f"{package}-{version}/"
        manifest = archive.extractfile(f"{prefix}Cargo.toml")
        source = archive.extractfile(f"{prefix}src/lib.rs")
        manifest_payload = manifest.read().decode("utf-8") if manifest is not None else ""
        source_payload = source.read().decode("utf-8") if source is not None else ""
    reject_generated_files(names, path.name)
    if f"{prefix}Cargo.toml" not in names or not any(
        name.startswith(f"{prefix}src/") for name in names
    ):
        raise RuntimeError(f"{path.name} is missing Cargo metadata or Rust sources")
    if package in {"vsh", "vbash"}:
        expected_sources = {f"{prefix}src/lib.rs"}
        actual_sources = {name for name in names if name.startswith(f"{prefix}src/")}
        if actual_sources != expected_sources:
            raise RuntimeError(f"{path.name} compatibility facade contains extra sources")
        dependency = "vsh-runtime" if package == "vsh" else "vsh"
        reexport = "pub use vsh_runtime_core::*;" if package == "vsh" else "pub use vsh_primary::*;"
        if f'version = "={version}"' not in manifest_payload or dependency not in manifest_payload:
            raise RuntimeError(f"{path.name} does not exact-pin {dependency} {version}")
        if reexport not in source_payload:
            raise RuntimeError(f"{path.name} does not contain its compatibility re-export")
    if package == "vsh-monty" and (
        "get-size2" not in manifest_payload or 'version = "=0.10.3"' not in manifest_payload
    ):
        raise RuntimeError(f"{path.name} is missing the downstream get-size2 resolution guard")


def validate(
    directory: Path,
    version: str,
    wheel_count: int,
    crate_count: int,
    python_surface: str,
    require_full_matrix: bool,
) -> list[Path]:
    wheels = sorted(directory.glob(f"{PRIMARY_PYTHON_STEM}-*.whl"))
    sdists = sorted(directory.glob(f"{PRIMARY_PYTHON_STEM}-*.tar.gz"))
    compat_wheels = sorted(directory.glob(f"{COMPAT_PYTHON_NAME}-*.whl"))
    compat_sdists = sorted(directory.glob(f"{COMPAT_PYTHON_NAME}-*.tar.gz"))
    crates = sorted(directory.glob("*.crate"))
    expect_primary = python_surface in {"main", "both"}
    expect_compat = python_surface in {"compat", "both"}
    expected_sdists = 1 if expect_primary else 0
    expected_compat_wheels = 1 if expect_compat else 0
    expected_compat_sdists = 1 if expect_compat else 0
    if (
        len(wheels) != wheel_count
        or len(sdists) != expected_sdists
        or len(compat_wheels) != expected_compat_wheels
        or len(compat_sdists) != expected_compat_sdists
        or len(crates) != crate_count
    ):
        raise RuntimeError(
            f"artifact cardinality mismatch: native_wheels={len(wheels)}/{wheel_count}, "
            f"native_sdists={len(sdists)}/{expected_sdists}, "
            f"compat_wheels={len(compat_wheels)}/{expected_compat_wheels}, "
            f"compat_sdists={len(compat_sdists)}/{expected_compat_sdists}, "
            f"crates={len(crates)}/{crate_count}"
        )
    if expect_primary:
        expected_sdist = f"{PRIMARY_PYTHON_STEM}-{version}.tar.gz"
        if sdists[0].name != expected_sdist:
            raise RuntimeError(f"expected {expected_sdist}, found {sdists[0].name}")
        validate_primary_sdist(sdists[0], version)

    for wheel in wheels:
        validate_primary_wheel(wheel, version)
    if require_full_matrix:
        expected_full_count = len(PYTHON_TAGS) * (len(PLATFORM_TAGS) + 1)
        if wheel_count != expected_full_count:
            raise RuntimeError(
                f"full wheel matrix requires {expected_full_count} artifacts, got {wheel_count}"
            )
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
    if expect_compat:
        expected_compat_wheel = f"{COMPAT_PYTHON_NAME}-{version}-py3-none-any.whl"
        expected_compat_sdist = f"{COMPAT_PYTHON_NAME}-{version}.tar.gz"
        if compat_wheels[0].name != expected_compat_wheel:
            raise RuntimeError(f"expected {expected_compat_wheel}, found {compat_wheels[0].name}")
        if compat_sdists[0].name != expected_compat_sdist:
            raise RuntimeError(f"expected {expected_compat_sdist}, found {compat_sdists[0].name}")
        validate_compat_wheel(compat_wheels[0], version)
        validate_compat_sdist(compat_sdists[0], version)
    if crate_count:
        expected_crates = {f"{package}-{version}.crate": package for package in CRATES}
        if {path.name for path in crates} != set(expected_crates):
            raise RuntimeError("crate archive set does not match the release graph")
        for crate in crates:
            validate_crate(crate, expected_crates[crate.name], version)
    return sorted(
        [*wheels, *sdists, *compat_wheels, *compat_sdists, *crates],
        key=lambda path: path.name,
    )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("directory", type=Path)
    parser.add_argument("--version", required=True)
    parser.add_argument("--wheel-count", type=int, default=20)
    parser.add_argument("--crate-count", type=int, default=len(CRATES))
    parser.add_argument(
        "--python-surface",
        choices=("main", "compat", "both", "none"),
        default="main",
    )
    parser.add_argument("--require-full-matrix", action="store_true")
    arguments = parser.parse_args()
    artifacts = validate(
        arguments.directory,
        arguments.version,
        arguments.wheel_count,
        arguments.crate_count,
        arguments.python_surface,
        arguments.require_full_matrix,
    )
    sums = arguments.directory / "SHA256SUMS"
    sums.write_text(
        "".join(f"{digest(path)}  {path.name}\n" for path in artifacts),
        encoding="utf-8",
    )
    print(f"validated {len(artifacts)} release artifacts; hashes: {sums}")


if __name__ == "__main__":
    main()
