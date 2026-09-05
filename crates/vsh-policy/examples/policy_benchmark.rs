//! Dependency-free hot-path benchmark; no timing assertion in correctness tests.

use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::hint::black_box;
use std::time::Instant;

use vsh_policy::{AccessKind, CallPolicy};
use vsh_types::VPath;

fn main() -> Result<(), Box<dyn Error>> {
    let output = std::env::args_os()
        .nth(1)
        .ok_or("provide an output JSON path")?;
    let policy = CallPolicy::secure_default();
    let paths = (0..10_000)
        .map(|index| {
            let name = if index % 10 == 0 {
                "private.key"
            } else {
                "file.txt"
            };
            VPath::parse(&format!("src/dir-{index:05}/{name}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut authorize = Vec::new();
    let mut parse = Vec::new();
    for _ in 0..11 {
        let started = Instant::now();
        let denied = paths
            .iter()
            .filter(|path| {
                black_box(policy.authorize(black_box(path), AccessKind::MetadataRead)).is_err()
            })
            .count();
        authorize.push(started.elapsed().as_nanos());
        assert_eq!(denied, 1_000);
        let started = Instant::now();
        for path in &paths {
            black_box(VPath::parse(black_box(path.as_str()))?);
        }
        parse.push(started.elapsed().as_nanos());
    }
    // One warmup, ten retained samples. Preserve every raw duration.
    authorize.remove(0);
    parse.remove(0);
    let mut json = String::new();
    writeln!(json, "{{")?;
    writeln!(json, "  \"schema\": \"vsh-policy-path-benchmark-v1\",")?;
    writeln!(json, "  \"paths_per_sample\": {},", paths.len())?;
    writeln!(json, "  \"denied_paths_per_sample\": 1000,")?;
    writeln!(json, "  \"authorize_ns\": {authorize:?},")?;
    writeln!(json, "  \"parse_ns\": {parse:?}")?;
    writeln!(json, "}}")?;
    fs::write(output, json)?;
    Ok(())
}
