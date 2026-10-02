"""Verify Linux CI retains Rust coverage after forced worker termination."""

from __future__ import annotations

import json
import os
import selectors
import shlex
import signal
import subprocess
import sys
import tempfile
from pathlib import Path

SOURCE = r"""
use std::io::{self, Write};

#[inline(never)]
fn reached_before_kill() -> u32 {
    std::hint::black_box(42)
}

#[inline(never)]
fn never_reached() -> u32 {
    std::hint::black_box(99)
}

fn main() {
    assert_eq!(reached_before_kill(), 42);
    if std::hint::black_box(false) {
        println!("{}", never_reached());
    }
    println!("ready");
    io::stdout().flush().unwrap();
    loop {
        std::thread::park();
    }
}
"""

if __name__ == "__main__":
    if sys.platform != "linux":
        raise SystemExit("This check targets the Linux coverage job, not Darwin's profiling ABI")
    sysroot = Path(
        subprocess.check_output(["rustc", "--print", "sysroot"], text=True, timeout=30).strip()
    )
    version = subprocess.check_output(["rustc", "-vV"], text=True, timeout=30)
    host = next(
        line.removeprefix("host: ") for line in version.splitlines() if line.startswith("host: ")
    )
    llvm_bin = sysroot / "lib" / "rustlib" / host / "bin"
    flags = shlex.split(os.environ["RUSTFLAGS"])
    profile_name = os.environ["LLVM_PROFILE_FILE_NAME"]
    with tempfile.TemporaryDirectory(prefix="vsh-coverage-") as temporary:
        directory = Path(temporary)
        source = directory / "probe.rs"
        executable = directory / "probe"
        source.write_text(SOURCE)
        subprocess.run(
            ["rustc", "-C", "instrument-coverage", *flags, str(source), "-o", str(executable)],
            check=True,
            timeout=60,
        )
        environment = {**os.environ, "LLVM_PROFILE_FILE": str(directory / profile_name)}
        with subprocess.Popen(
            [str(executable)], stdout=subprocess.PIPE, env=environment
        ) as process:
            try:
                assert process.stdout is not None
                with selectors.DefaultSelector() as selector:
                    selector.register(process.stdout, selectors.EVENT_READ)
                    if not selector.select(timeout=10):
                        raise RuntimeError("coverage probe did not become ready")
                    if process.stdout.readline() != b"ready\n":
                        raise RuntimeError("coverage probe exited before its execution marker")
                process.kill()
                if process.wait(timeout=10) != -signal.SIGKILL:
                    raise RuntimeError("coverage probe was not forcibly terminated")
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait(timeout=10)

        profiles = sorted(directory.glob("*.profraw"))
        if len(profiles) != 1:
            raise RuntimeError(f"expected one killed-process profile, got {len(profiles)}")
        merged = directory / "probe.profdata"
        subprocess.run(
            [
                str(llvm_bin / "llvm-profdata"),
                "merge",
                "-sparse",
                str(profiles[0]),
                "-o",
                str(merged),
            ],
            check=True,
            timeout=30,
        )
        report = json.loads(
            subprocess.check_output(
                [str(llvm_bin / "llvm-cov"), "export", str(executable), f"-instr-profile={merged}"],
                text=True,
                timeout=30,
            )
        )
        functions = report["data"][0]["functions"]
        for name, expected in (("reached_before_kill", 1), ("never_reached", 0)):
            counts = [function["count"] for function in functions if name in function["name"]]
            if counts != [expected]:
                raise RuntimeError(f"{name}: expected [{expected}], got {counts}")
        print("Forced-termination coverage verified: reached=1, unreached=0")
