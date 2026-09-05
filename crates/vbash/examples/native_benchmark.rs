//! Dependency-free release benchmark for the public native Rust API.

use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use vsh::{RunRequest, Runtime, RuntimeConfig, StageTimings, TransactionState, VERSION};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);
const PARALLEL_TRANSACTIONS_PER_RUNTIME: usize = 20;

#[derive(Clone, Copy, Debug)]
struct Sample {
    wall_ns: u64,
    internal_ns: u64,
    stages: StageSample,
    state: TransactionState,
    changed_paths: u64,
}

impl Sample {
    const fn api_envelope_ns(self) -> u64 {
        self.wall_ns.saturating_sub(self.internal_ns)
    }
}

#[derive(Clone, Copy, Debug)]
struct StageSample {
    snapshot: u64,
    execute: u64,
    diff: u64,
    policy: u64,
    bind_and_store: u64,
    commit: u64,
}

impl From<StageTimings> for StageSample {
    fn from(value: StageTimings) -> Self {
        Self {
            snapshot: value.snapshot_ns,
            execute: value.execute_ns,
            diff: value.diff_ns,
            policy: value.policy_ns,
            bind_and_store: value.bind_and_store_ns,
            commit: value.commit_ns,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Distribution {
    min: u64,
    p50: u64,
    p95: u64,
    p99: u64,
    max: u64,
}

#[derive(Debug)]
struct CaseSummary {
    samples: Vec<Sample>,
    wall: Distribution,
    internal: Distribution,
    api_envelope: Distribution,
    changed_paths: Distribution,
    stages: StageDistributions,
}

#[derive(Clone, Copy, Debug)]
struct StageDistributions {
    snapshot: Distribution,
    execute: Distribution,
    diff: Distribution,
    policy: Distribution,
    bind_and_store: Distribution,
    commit: Distribution,
}

#[derive(Debug)]
struct ParallelSummary {
    workers: usize,
    sequential_wall_ns: u64,
    parallel_wall_ns: u64,
    samples: Vec<Sample>,
}

impl ParallelSummary {
    #[allow(clippy::cast_precision_loss)]
    fn speedup(&self) -> f64 {
        self.sequential_wall_ns as f64 / self.parallel_wall_ns as f64
    }
}

#[derive(Debug)]
struct Arguments {
    iterations: usize,
    cold_iterations: usize,
    parallel_workers: usize,
    worker: PathBuf,
    output: PathBuf,
}

type NamedCase = (&'static str, CaseSummary);

#[derive(Clone, Copy)]
struct ReportInput<'a> {
    arguments: &'a Arguments,
    runtime_open_ns: u64,
    cold: Sample,
    repeated_cold: (Distribution, Distribution),
    large_fixture_ns: u64,
    large_runtime_open_ns: u64,
    cases: &'a [NamedCase],
    parallel: &'a ParallelSummary,
}

struct TempWorkspace {
    path: PathBuf,
}

impl TempWorkspace {
    fn create(label: &str) -> io::Result<Self> {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let timestamp = unix_duration().as_nanos();
        let path = std::env::temp_dir().join(format!(
            "vsh-native-benchmark-{label}-{}-{timestamp}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path)?;
        Ok(Self { path })
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn unix_duration() -> Duration {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
}

fn elapsed_ns(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

fn percentile(ordered: &[u64], numerator: usize) -> u64 {
    let index = ((ordered.len() - 1) * numerator + 50) / 100;
    ordered[index]
}

fn distribution(values: impl IntoIterator<Item = u64>) -> Distribution {
    let mut ordered: Vec<u64> = values.into_iter().collect();
    ordered.sort_unstable();
    Distribution {
        min: ordered[0],
        p50: percentile(&ordered, 50),
        p95: percentile(&ordered, 95),
        p99: percentile(&ordered, 99),
        max: ordered[ordered.len() - 1],
    }
}

fn summarize(samples: Vec<Sample>) -> CaseSummary {
    let wall = distribution(samples.iter().map(|sample| sample.wall_ns));
    let internal = distribution(samples.iter().map(|sample| sample.internal_ns));
    let api_envelope = distribution(samples.iter().map(|sample| sample.api_envelope_ns()));
    let changed_paths = distribution(samples.iter().map(|sample| sample.changed_paths));
    let stages = StageDistributions {
        snapshot: distribution(samples.iter().map(|sample| sample.stages.snapshot)),
        execute: distribution(samples.iter().map(|sample| sample.stages.execute)),
        diff: distribution(samples.iter().map(|sample| sample.stages.diff)),
        policy: distribution(samples.iter().map(|sample| sample.stages.policy)),
        bind_and_store: distribution(samples.iter().map(|sample| sample.stages.bind_and_store)),
        commit: distribution(samples.iter().map(|sample| sample.stages.commit)),
    };
    CaseSummary {
        samples,
        wall,
        internal,
        api_envelope,
        changed_paths,
        stages,
    }
}

fn sample(runtime: &Runtime, code: &str, intent: &str) -> Result<Sample, Box<dyn Error>> {
    let started = Instant::now();
    let receipt = runtime.preview(RunRequest::new(code).with_intent(intent))?;
    let wall_ns = elapsed_ns(started);
    if receipt.state == TransactionState::AutoApproved
        && !runtime.discard_preview(receipt.transaction)?
    {
        return Err(io::Error::other("runtime did not retain its auto-approved preview").into());
    }
    Ok(Sample {
        wall_ns,
        internal_ns: receipt.timings.total_ns,
        stages: receipt.timings.into(),
        state: receipt.state,
        changed_paths: u64::try_from(receipt.changed_paths).unwrap_or(u64::MAX),
    })
}

fn run_case(
    runtime: &Runtime,
    code: &str,
    name: &str,
    iterations: usize,
    expected_state: TransactionState,
) -> Result<CaseSummary, Box<dyn Error>> {
    let warmup = sample(runtime, code, &format!("{name}-warmup"))?;
    if warmup.state != expected_state {
        return Err(io::Error::other(format!(
            "{name} warmup returned {:?}, expected {expected_state:?}",
            warmup.state
        ))
        .into());
    }
    let samples = (0..iterations)
        .map(|index| sample(runtime, code, &format!("{name}-{index}")))
        .collect::<Result<Vec<_>, _>>()?;
    if let Some(sample) = samples.iter().find(|sample| sample.state != expected_state) {
        return Err(io::Error::other(format!(
            "{name} returned {:?}, expected {expected_state:?}",
            sample.state
        ))
        .into());
    }
    Ok(summarize(samples))
}

fn small_fixture(root: &Path) -> io::Result<()> {
    for index in 0..20 {
        fs::write(
            root.join(format!("input-{index:02}.txt")),
            format!("line-{index}\n").repeat(32),
        )?;
    }
    Ok(())
}

fn large_tree_fixture(root: &Path) -> io::Result<()> {
    let large_tree = root.join("large-tree");
    fs::create_dir(&large_tree)?;
    for directory_index in 0..100 {
        let directory = large_tree.join(format!("dir-{directory_index:03}"));
        fs::create_dir(&directory)?;
        for file_index in 0..100 {
            fs::File::create(directory.join(format!("file-{file_index:03}.txt")))?;
        }
    }
    Ok(())
}

fn read_ten_program() -> String {
    let reads = (0..10)
        .map(|index| format!("Path('/workspace/input-{index:02}.txt').read_bytes()"))
        .collect::<Vec<_>>()
        .join(",\n    ");
    format!("from pathlib import Path\nlen(b''.join([\n    {reads}\n]))")
}

fn edit_twenty_program() -> String {
    let writes = (0..20)
        .map(|index| {
            format!("Path('/workspace/output-{index:02}.txt').write_text('value-{index}')")
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("from pathlib import Path\n{writes}\n20")
}

fn search_ten_thousand_program() -> &'static str {
    concat!(
        "import os\n",
        "matches = 0\n",
        "root = '/workspace/large-tree'\n",
        "for directory in os.listdir(root):\n",
        "    for name in os.listdir(root + '/' + directory):\n",
        "        if name.endswith('7.txt'):\n",
        "            matches += 1\n",
        "matches\n",
    )
}

fn glob_ten_thousand_with_vsh_program() -> &'static str {
    "len(vsh_glob('**/*7.txt', path='/workspace/large-tree', max_results=1000))\n"
}

fn rename_subtree_program() -> &'static str {
    "import os\nos.rename('/workspace/large-tree/dir-000', '/workspace/moved-tree')\nNone\n"
}

fn delete_subtree_program() -> &'static str {
    concat!(
        "import os\n",
        "root = '/workspace/large-tree/dir-000'\n",
        "for name in os.listdir(root):\n",
        "    os.unlink(root + '/' + name)\n",
        "os.rmdir(root)\n",
        "None\n",
    )
}

fn delete_subtree_with_vsh_program() -> &'static str {
    "vsh_remove('/workspace/large-tree/dir-000', recursive=True)\nNone\n"
}

fn massive_delete_program() -> &'static str {
    concat!(
        "import os\n",
        "root = '/workspace/large-tree'\n",
        "for directory in sorted(os.listdir(root))[:50]:\n",
        "    subtree = root + '/' + directory\n",
        "    for name in os.listdir(subtree):\n",
        "        os.unlink(subtree + '/' + name)\n",
        "    os.rmdir(subtree)\n",
        "None\n",
    )
}

fn repeated_cold_case(
    iterations: usize,
    worker: &Path,
) -> Result<(Distribution, Distribution), Box<dyn Error>> {
    let mut opens = Vec::with_capacity(iterations);
    let mut first_calls = Vec::with_capacity(iterations);
    for index in 0..iterations {
        let workspace = TempWorkspace::create(&format!("cold-{index}"))?;
        let open_started = Instant::now();
        let runtime = open_runtime(&workspace.path, worker)?;
        opens.push(elapsed_ns(open_started));
        let first = sample(&runtime, "None", &format!("cold-{index}"))?;
        if first.state != TransactionState::AutoApproved {
            return Err(
                io::Error::other(format!("cold first call returned {:?}", first.state)).into(),
            );
        }
        first_calls.push(first.wall_ns);
    }
    Ok((distribution(opens), distribution(first_calls)))
}

fn open_runtime(root: &Path, worker: &Path) -> Result<Runtime, Box<dyn Error>> {
    Ok(Runtime::open(
        RuntimeConfig::new(root).with_worker_path(worker),
    )?)
}

fn parallel_case(worker_count: usize, worker: &Path) -> Result<ParallelSummary, Box<dyn Error>> {
    let workspaces = (0..worker_count)
        .map(|index| TempWorkspace::create(&format!("parallel-{index}")))
        .collect::<Result<Vec<_>, _>>()?;
    let runtimes = workspaces
        .iter()
        .map(|workspace| open_runtime(&workspace.path, worker).map(Arc::new))
        .collect::<Result<Vec<_>, _>>()?;

    for (index, runtime) in runtimes.iter().enumerate() {
        sample(runtime, "None", &format!("parallel-warmup-{index}"))?;
    }

    let sequential_started = Instant::now();
    for (index, runtime) in runtimes.iter().enumerate() {
        for transaction in 0..PARALLEL_TRANSACTIONS_PER_RUNTIME {
            sample(
                runtime,
                "sum(range(10000))",
                &format!("sequential-{index}-{transaction}"),
            )?;
        }
    }
    let sequential_wall_ns = elapsed_ns(sequential_started);

    let parallel_started = Instant::now();
    let handles = runtimes
        .iter()
        .enumerate()
        .map(|(index, runtime)| {
            let runtime = Arc::clone(runtime);
            thread::spawn(move || {
                (0..PARALLEL_TRANSACTIONS_PER_RUNTIME)
                    .map(|transaction| {
                        sample(
                            &runtime,
                            "sum(range(10000))",
                            &format!("parallel-{index}-{transaction}"),
                        )
                        .map_err(|error| error.to_string())
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
        })
        .collect::<Vec<_>>();
    let sample_groups = handles
        .into_iter()
        .map(|handle| {
            handle
                .join()
                .map_err(|_| io::Error::other("parallel benchmark thread panicked"))?
                .map_err(io::Error::other)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let samples = sample_groups.into_iter().flatten().collect();
    let parallel_wall_ns = elapsed_ns(parallel_started);

    Ok(ParallelSummary {
        workers: worker_count,
        sequential_wall_ns,
        parallel_wall_ns,
        samples,
    })
}

fn required_value(
    arguments: &mut impl Iterator<Item = std::ffi::OsString>,
    flag: &str,
) -> io::Result<std::ffi::OsString> {
    arguments.next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{flag} requires a value"),
        )
    })
}

fn parse_usize(value: &std::ffi::OsStr, flag: &str) -> io::Result<usize> {
    value.to_string_lossy().parse().map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid {flag} value: {error}"),
        )
    })
}

fn parse_arguments() -> io::Result<Arguments> {
    let mut parsed = Arguments {
        iterations: 100,
        cold_iterations: 30,
        parallel_workers: 4,
        worker: PathBuf::from("vsh-monty-worker"),
        output: PathBuf::from("benchmarks/results/local/native-rust.json"),
    };
    let mut arguments = std::env::args_os().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.to_string_lossy().as_ref() {
            "--iterations" => {
                let value = required_value(&mut arguments, "--iterations")?;
                parsed.iterations = parse_usize(&value, "--iterations")?;
            }
            "--cold-iterations" => {
                let value = required_value(&mut arguments, "--cold-iterations")?;
                parsed.cold_iterations = parse_usize(&value, "--cold-iterations")?;
            }
            "--parallel-workers" => {
                let value = required_value(&mut arguments, "--parallel-workers")?;
                parsed.parallel_workers = parse_usize(&value, "--parallel-workers")?;
            }
            "--worker" => {
                parsed.worker = PathBuf::from(required_value(&mut arguments, "--worker")?);
            }
            "--output" => {
                parsed.output = PathBuf::from(required_value(&mut arguments, "--output")?);
            }
            flag => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("unknown argument: {flag}"),
                ));
            }
        }
    }
    if parsed.iterations < 3 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "--iterations must be at least 3",
        ));
    }
    if parsed.parallel_workers < 2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "--parallel-workers must be at least 2",
        ));
    }
    if parsed.cold_iterations < 3 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "--cold-iterations must be at least 3",
        ));
    }
    Ok(parsed)
}

fn push_json_string(output: &mut String, value: &str) {
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if character.is_control() => {
                write!(output, "\\u{:04x}", u32::from(character))
                    .expect("writing to String cannot fail");
            }
            character => output.push(character),
        }
    }
    output.push('"');
}

fn push_distribution(output: &mut String, value: Distribution) {
    write!(
        output,
        "{{\"min\":{},\"p50\":{},\"p95\":{},\"p99\":{},\"max\":{}}}",
        value.min, value.p50, value.p95, value.p99, value.max
    )
    .expect("writing to String cannot fail");
}

fn push_stage_distributions(output: &mut String, value: StageDistributions) {
    output.push_str("{\"snapshot\":");
    push_distribution(output, value.snapshot);
    output.push_str(",\"execute\":");
    push_distribution(output, value.execute);
    output.push_str(",\"diff\":");
    push_distribution(output, value.diff);
    output.push_str(",\"policy\":");
    push_distribution(output, value.policy);
    output.push_str(",\"bind_and_store\":");
    push_distribution(output, value.bind_and_store);
    output.push_str(",\"commit\":");
    push_distribution(output, value.commit);
    output.push('}');
}

fn push_samples(output: &mut String, samples: &[Sample]) {
    output.push('[');
    for (index, sample) in samples.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        write!(
            output,
            "{{\"wall_ns\":{},\"internal_ns\":{},\"api_envelope_ns\":{},\"state\":",
            sample.wall_ns,
            sample.internal_ns,
            sample.api_envelope_ns()
        )
        .expect("writing to String cannot fail");
        push_json_string(output, transaction_state_name(sample.state));
        write!(output, ",\"changed_paths\":{}}}", sample.changed_paths)
            .expect("writing to String cannot fail");
    }
    output.push(']');
}

fn push_case(output: &mut String, value: &CaseSummary) {
    output.push_str("{\"samples\":");
    push_samples(output, &value.samples);
    output.push_str(",\"wall_ns\":");
    push_distribution(output, value.wall);
    output.push_str(",\"internal_ns\":");
    push_distribution(output, value.internal);
    output.push_str(",\"api_envelope_ns\":");
    push_distribution(output, value.api_envelope);
    output.push_str(",\"changed_paths\":");
    push_distribution(output, value.changed_paths);
    output.push_str(",\"state\":");
    push_json_string(output, transaction_state_name(value.samples[0].state));
    output.push_str(",\"stages_ns\":");
    push_stage_distributions(output, value.stages);
    output.push('}');
}

const fn transaction_state_name(state: TransactionState) -> &'static str {
    match state {
        TransactionState::Created => "created",
        TransactionState::Running => "running",
        TransactionState::VirtualComplete => "virtual_complete",
        TransactionState::Denied => "denied",
        TransactionState::AutoApproved => "auto_approved",
        TransactionState::PendingApproval => "pending_approval",
        TransactionState::Approved => "approved",
        TransactionState::Reserved => "reserved",
        TransactionState::Revalidating => "revalidating",
        TransactionState::Committing => "committing",
        TransactionState::Committed => "committed",
        TransactionState::Stale => "stale",
        TransactionState::Expired => "expired",
        TransactionState::RecoveryRequired => "recovery_required",
        TransactionState::Failed => "failed",
        _ => "unknown",
    }
}

fn json_report(report: ReportInput<'_>) -> String {
    let ReportInput {
        arguments,
        runtime_open_ns,
        cold,
        repeated_cold,
        large_fixture_ns,
        large_runtime_open_ns,
        cases,
        parallel,
    } = report;
    let mut output = String::with_capacity(32 * 1024);
    output.push_str("{\n  \"schema\":\"vsh-native-rust-benchmark-v2\",\n");
    write!(
        output,
        "  \"captured_at_unix_ms\":{},\n  \"environment\":{{\"vsh_version\":",
        unix_duration().as_millis()
    )
    .expect("writing to String cannot fail");
    push_json_string(&mut output, VERSION);
    output.push_str(",\"os\":");
    push_json_string(&mut output, std::env::consts::OS);
    output.push_str(",\"arch\":");
    push_json_string(&mut output, std::env::consts::ARCH);
    output.push_str(",\"worker\":");
    push_json_string(&mut output, &arguments.worker.to_string_lossy());
    write!(
        output,
        ",\"available_parallelism\":{}}},\n  \"iterations\":{},\n",
        thread::available_parallelism().map_or(1, std::num::NonZero::get),
        arguments.iterations
    )
    .expect("writing to String cannot fail");
    write!(
        output,
        "  \"cold\":{{\"runtime_open_ns\":{runtime_open_ns},\"first_call_wall_ns\":{},\"first_call_internal_ns\":{},\"first_call_api_envelope_ns\":{},\"repeated_samples\":{},\"repeated_runtime_open_ns\":",
        cold.wall_ns,
        cold.internal_ns,
        cold.api_envelope_ns(),
        arguments.cold_iterations
    )
    .expect("writing to String cannot fail");
    push_distribution(&mut output, repeated_cold.0);
    output.push_str(",\"repeated_first_call_wall_ns\":");
    push_distribution(&mut output, repeated_cold.1);
    write!(
        output,
        "}},\n  \"large_tree\":{{\"directories\":100,\"files\":10000,\"fixture_ns\":{large_fixture_ns},\"runtime_open_ns\":{large_runtime_open_ns}}},\n  \"cases\":{{"
    )
    .expect("writing to String cannot fail");
    for (index, (name, summary)) in cases.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        push_json_string(&mut output, name);
        output.push(':');
        push_case(&mut output, summary);
    }
    write!(
        output,
        "}},\n  \"parallel_independent_runtimes\":{{\"workers\":{},\"transactions_per_runtime\":{},\"total_transactions\":{},\"sequential_wall_ns\":{},\"parallel_wall_ns\":{},\"speedup\":{:.6},\"samples\":",
        parallel.workers,
        PARALLEL_TRANSACTIONS_PER_RUNTIME,
        parallel.workers * PARALLEL_TRANSACTIONS_PER_RUNTIME,
        parallel.sequential_wall_ns,
        parallel.parallel_wall_ns,
        parallel.speedup()
    )
    .expect("writing to String cannot fail");
    push_samples(&mut output, &parallel.samples);
    output.push_str("}\n}\n");
    output
}

fn markdown_report(
    cases: &[(&str, CaseSummary)],
    repeated_cold: (Distribution, Distribution),
    parallel: &ParallelSummary,
) -> String {
    let mut output = String::from(
        "# VSH native Rust benchmark\n\n| Case | state | changed paths | wall p50 ms | wall p99 ms | snapshot p50 ms | execute p50 ms | Rust API envelope p50 µs |\n|---|---|---:|---:|---:|---:|---:|---:|\n",
    );
    for (name, summary) in cases {
        #[allow(clippy::cast_precision_loss)]
        let wall_p50_ms = summary.wall.p50 as f64 / 1_000_000.0;
        #[allow(clippy::cast_precision_loss)]
        let wall_p99_ms = summary.wall.p99 as f64 / 1_000_000.0;
        #[allow(clippy::cast_precision_loss)]
        let envelope_p50_us = summary.api_envelope.p50 as f64 / 1_000.0;
        #[allow(clippy::cast_precision_loss)]
        let snapshot_p50_ms = summary.stages.snapshot.p50 as f64 / 1_000_000.0;
        #[allow(clippy::cast_precision_loss)]
        let execute_p50_ms = summary.stages.execute.p50 as f64 / 1_000_000.0;
        writeln!(
            output,
            "| {name} | {} | {} | {wall_p50_ms:.3} | {wall_p99_ms:.3} | {snapshot_p50_ms:.3} | {execute_p50_ms:.3} | {envelope_p50_us:.1} |",
            transaction_state_name(summary.samples[0].state),
            summary.changed_paths.p50
        )
        .expect("writing to String cannot fail");
    }
    write!(
        output,
        "\nRepeated cold runtime-open p50/p99: {:.3}/{:.3} ms; first-call p50/p99: {:.3}/{:.3} ms.\n\nIndependent-runtime parallel speedup: {:.2}x across {} workers.\n",
        ns_as_ms(repeated_cold.0.p50),
        ns_as_ms(repeated_cold.0.p99),
        ns_as_ms(repeated_cold.1.p50),
        ns_as_ms(repeated_cold.1.p99),
        parallel.speedup(),
        parallel.workers
    )
    .expect("writing to String cannot fail");
    output
}

#[allow(clippy::cast_precision_loss)]
fn ns_as_ms(value: u64) -> f64 {
    value as f64 / 1_000_000.0
}

fn benchmark_small(arguments: &Arguments) -> Result<(u64, Sample, Vec<NamedCase>), Box<dyn Error>> {
    let workspace = TempWorkspace::create("primary")?;
    small_fixture(&workspace.path)?;
    let open_started = Instant::now();
    let runtime = open_runtime(&workspace.path, &arguments.worker)?;
    let runtime_open_ns = elapsed_ns(open_started);
    let cold = sample(&runtime, "None", "cold-first-worker")?;
    let read_ten = read_ten_program();
    let edit_twenty = edit_twenty_program();
    let cases = vec![
        (
            "noop",
            run_case(
                &runtime,
                "None",
                "noop",
                arguments.iterations,
                TransactionState::AutoApproved,
            )?,
        ),
        (
            "read_10",
            run_case(
                &runtime,
                &read_ten,
                "read-10",
                arguments.iterations,
                TransactionState::AutoApproved,
            )?,
        ),
        (
            "edit_20",
            run_case(
                &runtime,
                &edit_twenty,
                "edit-20",
                arguments.iterations,
                TransactionState::AutoApproved,
            )?,
        ),
    ];
    Ok((runtime_open_ns, cold, cases))
}

fn benchmark_large(arguments: &Arguments) -> Result<(u64, u64, Vec<NamedCase>), Box<dyn Error>> {
    let large_workspace = TempWorkspace::create("large")?;
    let fixture_started = Instant::now();
    large_tree_fixture(&large_workspace.path)?;
    let large_fixture_ns = elapsed_ns(fixture_started);
    let large_open_started = Instant::now();
    let large_runtime = open_runtime(&large_workspace.path, &arguments.worker)?;
    let large_runtime_open_ns = elapsed_ns(large_open_started);
    let cases = vec![
        (
            "search_10k",
            run_case(
                &large_runtime,
                search_ten_thousand_program(),
                "search-10k",
                arguments.iterations,
                TransactionState::AutoApproved,
            )?,
        ),
        (
            "vsh_glob_10k",
            run_case(
                &large_runtime,
                glob_ten_thousand_with_vsh_program(),
                "vsh-glob-10k",
                arguments.iterations,
                TransactionState::AutoApproved,
            )?,
        ),
        (
            "rename_subtree_100",
            run_case(
                &large_runtime,
                rename_subtree_program(),
                "rename-subtree-100",
                arguments.iterations,
                TransactionState::PendingApproval,
            )?,
        ),
        (
            "delete_subtree_100",
            run_case(
                &large_runtime,
                delete_subtree_program(),
                "delete-subtree-100",
                arguments.iterations,
                TransactionState::PendingApproval,
            )?,
        ),
        (
            "vsh_remove_subtree_100",
            run_case(
                &large_runtime,
                delete_subtree_with_vsh_program(),
                "vsh-remove-subtree-100",
                arguments.iterations,
                TransactionState::PendingApproval,
            )?,
        ),
        (
            "massive_delete_5k",
            run_case(
                &large_runtime,
                massive_delete_program(),
                "massive-delete-5k",
                arguments.iterations,
                TransactionState::PendingApproval,
            )?,
        ),
    ];
    Ok((large_fixture_ns, large_runtime_open_ns, cases))
}

fn main() -> Result<(), Box<dyn Error>> {
    let arguments = parse_arguments()?;
    let (runtime_open_ns, cold, mut cases) = benchmark_small(&arguments)?;
    let repeated_cold = repeated_cold_case(arguments.cold_iterations, &arguments.worker)?;
    let (large_fixture_ns, large_runtime_open_ns, large_cases) = benchmark_large(&arguments)?;
    cases.extend(large_cases);

    let parallel = parallel_case(arguments.parallel_workers, &arguments.worker)?;

    if let Some(parent) = arguments.output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        &arguments.output,
        json_report(ReportInput {
            arguments: &arguments,
            runtime_open_ns,
            cold,
            repeated_cold,
            large_fixture_ns,
            large_runtime_open_ns,
            cases: &cases,
            parallel: &parallel,
        }),
    )?;
    let markdown = arguments.output.with_extension("md");
    fs::write(&markdown, markdown_report(&cases, repeated_cold, &parallel))?;
    println!("{}", markdown.display());
    Ok(())
}
