//! End-to-end scanner benchmark. Invoke with `cargo bench`, no server or launcher.
use orion_core::{default_scan_workers, Kind, Scan, Status, Summary, MAX_SCAN_WORKERS};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::Command,
    time::Instant,
};

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Variant {
    Legacy,
    Serial,
    Current,
    Enumerated,
}

#[derive(Debug)]
struct Options {
    runs: usize,
    warmups: usize,
    suite: String,
    root: Option<PathBuf>,
    output: Option<PathBuf>,
    label: String,
    candidate: Variant,
    baseline: Variant,
    workers: usize,
}

impl Options {
    fn parse() -> Result<Option<Self>, String> {
        let mut options = Self {
            runs: 8,
            warmups: 2,
            suite: "all".into(),
            root: None,
            output: None,
            label: "unlabelled".into(),
            candidate: Variant::Current,
            baseline: Variant::Serial,
            workers: default_scan_workers(),
        };
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            if arg == "--bench" {
                continue;
            } // Cargo's harness argument.
            if arg == "--help" || arg == "-h" {
                println!("scan benchmark: --suite all|wide|tree|deep|hardlink (default all) OR --root <absolute directory>\n  --baseline serial|legacy (default serial) --candidate current|enumerated (default current)\n  --workers <1..=16; current only; default min(CPUs,4)>\n  --runs <even number >=4, default 8> --warmups <>=1, default 2>\n  --label <experiment name> --output <JSON file>\nEach run pair alternates AB / BA. This measures warm scans, not cold-cache I/O.");
                return Ok(None);
            }
            let value = args
                .next()
                .ok_or_else(|| format!("Missing value for {arg}"))?;
            match arg.as_str() {
                "--runs" => options.runs = value.parse().map_err(|_| "Invalid runs")?,
                "--warmups" => options.warmups = value.parse().map_err(|_| "Invalid warmups")?,
                "--suite" => options.suite = value,
                "--root" => options.root = Some(value.into()),
                "--output" => options.output = Some(value.into()),
                "--label" => options.label = value,
                "--workers" => options.workers = value.parse().map_err(|_| "Invalid workers")?,
                "--baseline" => {
                    options.baseline = match value.as_str() {
                        "serial" => Variant::Serial,
                        "legacy" => Variant::Legacy,
                        _ => return Err("baseline must be serial or legacy".into()),
                    }
                }
                "--candidate" => {
                    options.candidate = match value.as_str() {
                        "current" => Variant::Current,
                        "enumerated" => Variant::Enumerated,
                        _ => return Err("candidate must be current or enumerated".into()),
                    }
                }
                _ => return Err(format!("Unknown argument: {arg}")),
            }
        }
        if options.runs < 4 || !options.runs.is_multiple_of(2) {
            return Err("runs must be even and >= 4".into());
        }
        if options.warmups == 0 {
            return Err("At least one warmup per variant is required".into());
        }
        if !(1..=MAX_SCAN_WORKERS).contains(&options.workers) {
            return Err("workers must be in 1..=16".into());
        }
        if !["all", "wide", "tree", "deep", "hardlink"].contains(&options.suite.as_str()) {
            return Err("Unknown suite".into());
        }
        if options.root.is_some() && options.suite != "all" {
            return Err("Use either --root or --suite".into());
        }
        if let Some(root) = &options.root {
            if !root.is_absolute() {
                return Err("--root must be absolute".into());
            }
        }
        if let Some(output) = &options.output {
            if output.is_relative() {
                // Cargo runs benches in the package directory; CLI paths use workspace root.
                let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
                options.output = Some(workspace.join(output));
            }
        }
        Ok(Some(options))
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
struct Counts {
    files: u64,
    directories: u64,
    logical_bytes: u64,
    issue_count: u64,
    complete: bool,
}

impl From<&Summary> for Counts {
    fn from(s: &Summary) -> Self {
        Self {
            files: s.files,
            directories: s.directories,
            logical_bytes: s.logical_bytes,
            issue_count: s.issue_count,
            complete: s.complete,
        }
    }
}

struct Dataset {
    name: String,
    root: PathBuf,
    expected: Option<Counts>,
    // TempDir deletes only this uniquely-created, canonicalized workspace child.
    _owner: Option<tempfile::TempDir>,
}

impl Dataset {
    fn generated(name: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let workspace = fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))?;
        let base = workspace.join("target/bench-fixtures");
        fs::create_dir_all(&base)?;
        let base = fs::canonicalize(base)?;
        if !base.starts_with(&workspace) || base == workspace {
            return Err("Fixture base escaped its workspace directory".into());
        }
        let owner = tempfile::Builder::new().prefix("scan-").tempdir_in(&base)?;
        let root = fs::canonicalize(owner.path())?;
        if !root.starts_with(&base) || root == base {
            return Err("Fixture escaped its workspace directory".into());
        }
        let mut dataset = Self {
            name: format!("{name}-v1"),
            root,
            expected: Some(Counts {
                files: 0,
                directories: 1,
                logical_bytes: 0,
                issue_count: 0,
                complete: true,
            }),
            _owner: Some(owner),
        };
        match name {
            "wide" => {
                for i in 0..12_000 {
                    dataset.file(
                        &dataset.root.clone(),
                        &format!("文件-{i:05}.bin"),
                        128 + i % 1024,
                    )?;
                }
            }
            "tree" => {
                let mut pending = vec![(dataset.root.clone(), 0)];
                while let Some((parent, depth)) = pending.pop() {
                    for i in 0..12 {
                        dataset.file(&parent, &format!("item-{i:02}"), 64 + i * 97)?;
                    }
                    if depth < 4 {
                        for i in 0..4 {
                            let child = parent.join(format!("d{i}"));
                            dataset.directory(&child)?;
                            pending.push((child, depth + 1));
                        }
                    }
                }
                for i in 0..4 {
                    dataset.file(
                        &dataset.root.clone(),
                        &format!("large-{i}.bin"),
                        4 * 1024 * 1024,
                    )?;
                }
            }
            "deep" => {
                let mut parent = dataset.root.clone();
                for depth in 0..32 {
                    for i in 0..16 {
                        dataset.file(&parent, &format!("file-{i:02}"), 256 + depth * 31 + i)?;
                    }
                    parent = parent.join(format!("d{depth:02}"));
                    dataset.directory(&parent)?;
                }
            }
            "hardlink" => {
                let original = dataset.root.join("original.bin");
                fs::write(&original, [7; 4096])?;
                fs::hard_link(&original, dataset.root.join("alias.bin"))?;
                // Do not query alias metadata before the candidate: it can refresh NTFS's cache.
                fs::write(&original, [9; 8192])?;
                let expected = dataset.expected.as_mut().unwrap();
                expected.files = 2;
                expected.logical_bytes = 16384;
            }
            _ => return Err("Unknown fixture".into()),
        }
        Ok(dataset)
    }

    fn directory(&mut self, path: &Path) -> io::Result<()> {
        fs::create_dir(path)?;
        self.expected.as_mut().unwrap().directories += 1;
        Ok(())
    }

    fn file(&mut self, parent: &Path, name: &str, bytes: u64) -> io::Result<()> {
        // Contents are irrelevant to a metadata scan; use deterministic logical lengths.
        fs::File::create(parent.join(name))?.set_len(bytes)?;
        let counts = self.expected.as_mut().unwrap();
        counts.files += 1;
        counts.logical_bytes += bytes;
        Ok(())
    }
}

#[derive(Clone, Serialize, PartialEq, Eq)]
struct Identity {
    counts: Counts,
    entry_digest: String,
    // Diagnostic only: full equality above still includes every modification timestamp.
    structure_digest: String,
    issue_digest: String,
}

#[derive(Serialize)]
struct Sample {
    variant: Variant,
    pair: usize,
    position: usize,
    phase: Phase,
    elapsed_ms: f64,
    entries_per_second: f64,
    status: Status,
    identity: Identity,
}

#[derive(Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Preflight,
    Warmup,
    Measured,
}

fn sample(
    dataset: &Dataset,
    variant: Variant,
    pair: usize,
    position: usize,
    phase: Phase,
    workers: usize,
) -> Result<Sample, Box<dyn std::error::Error>> {
    // Include root validation, traversal and aggregation; exclude validation, output and drop.
    let begin = Instant::now();
    let scan = Scan::new(&dataset.root).map_err(io::Error::other)?;
    match variant {
        Variant::Legacy => scan.run_legacy_metadata(),
        Variant::Current => scan.run_with_workers(workers),
        Variant::Serial | Variant::Enumerated => scan.run_enumeration_metadata(),
    }
    let summary = std::hint::black_box(scan.summary());
    let elapsed = begin.elapsed();
    let counts = Counts::from(&summary);
    let rows = scan.benchmark_rows();
    let mut digest = Sha256::new();
    let mut structure = Sha256::new();
    for (path, kind, bytes, modified, enumerated) in rows {
        // Hash lossless native path units, not to_string_lossy(). Include a length boundary.
        #[cfg(windows)]
        let path_bytes: Vec<u8> = {
            use std::os::windows::ffi::OsStrExt;
            path.as_os_str()
                .encode_wide()
                .flat_map(u16::to_le_bytes)
                .collect()
        };
        #[cfg(unix)]
        let path_bytes = {
            use std::os::unix::ffi::OsStrExt;
            path.as_os_str().as_bytes().to_vec()
        };
        #[cfg(not(any(windows, unix)))]
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();
        digest.update((path_bytes.len() as u64).to_le_bytes());
        structure.update((path_bytes.len() as u64).to_le_bytes());
        digest.update(&path_bytes);
        structure.update(&path_bytes);
        let kind_byte = match kind {
            Kind::Directory => 0,
            Kind::File => 1,
            Kind::Link => 2,
            Kind::Other => 3,
        };
        digest.update([kind_byte]);
        structure.update([kind_byte]);
        digest.update(bytes.to_le_bytes());
        structure.update(bytes.to_le_bytes());
        digest.update([u8::from(modified.is_some())]);
        digest.update(modified.unwrap_or_default().to_le_bytes());
        digest.update([u8::from(enumerated)]);
        structure.update([u8::from(enumerated)]);
    }
    // Error messages can be localized; compare normalized paths + codes and the total count.
    let mut issues: Vec<_> = summary
        .issues
        .iter()
        .map(|issue| {
            (
                Path::new(&issue.path)
                    .strip_prefix(Path::new(&summary.root))
                    .unwrap_or(Path::new(&issue.path))
                    .to_string_lossy()
                    .into_owned(),
                &issue.code,
            )
        })
        .collect();
    issues.sort_unstable();
    let identity = Identity {
        counts,
        entry_digest: format!("{:x}", digest.finalize()),
        structure_digest: format!("{:x}", structure.finalize()),
        issue_digest: format!("{:x}", Sha256::digest(serde_json::to_vec(&issues)?)),
    };
    Ok(Sample {
        variant,
        pair,
        position,
        phase,
        elapsed_ms: elapsed.as_secs_f64() * 1000.0,
        entries_per_second: (summary.files + summary.directories) as f64 / elapsed.as_secs_f64(),
        status: summary.status,
        identity,
    })
}

#[derive(Serialize)]
struct Distribution {
    min_ms: f64,
    median_ms: f64,
    max_ms: f64,
    median_absolute_deviation_ms: f64,
}

fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    (sorted[(sorted.len() - 1) / 2] + sorted[sorted.len() / 2]) / 2.0
}

fn distribution(values: &[f64]) -> Distribution {
    let middle = median(values);
    Distribution {
        min_ms: values.iter().copied().fold(f64::INFINITY, f64::min),
        median_ms: middle,
        max_ms: values.iter().copied().fold(0.0, f64::max),
        median_absolute_deviation_ms: median(
            &values
                .iter()
                .map(|x| (x - middle).abs())
                .collect::<Vec<_>>(),
        ),
    }
}

#[derive(Serialize)]
struct Comparison {
    baseline: Distribution,
    candidate: Distribution,
    median_paired_speedup: f64,
    median_paired_reduction_percent: f64,
}

#[derive(Serialize)]
struct DatasetReport {
    name: String,
    root: String,
    expected: Option<Counts>,
    valid: bool,
    invalid_reason: Option<String>,
    samples: Vec<Sample>,
    comparison: Option<Comparison>,
}

fn run_dataset(
    dataset: Dataset,
    options: &Options,
) -> Result<DatasetReport, Box<dyn std::error::Error>> {
    println!("\n{}: {}", dataset.name, dataset.root.display());
    let mut samples = vec![];
    // A baseline path query can refresh cached metadata. Check the candidate first,
    // before any warmup; keep failures in the report even if later scans agree.
    for (position, variant) in [options.candidate, options.baseline]
        .into_iter()
        .enumerate()
    {
        let result = sample(
            &dataset,
            variant,
            0,
            position,
            Phase::Preflight,
            options.workers,
        )?;
        println!(
            "  preflight {:?}: {} files / {} dirs / {} bytes / {} issues",
            variant,
            result.identity.counts.files,
            result.identity.counts.directories,
            result.identity.counts.logical_bytes,
            result.identity.counts.issue_count
        );
        samples.push(result);
    }
    for pair in 0..options.warmups + options.runs {
        let warmup = pair < options.warmups;
        let round = if warmup { pair } else { pair - options.warmups };
        let order = if round % 2 == 0 {
            [options.baseline, options.candidate]
        } else {
            [options.candidate, options.baseline]
        };
        for (position, variant) in order.into_iter().enumerate() {
            let phase = if warmup {
                Phase::Warmup
            } else {
                Phase::Measured
            };
            let result = sample(&dataset, variant, round, position, phase, options.workers)?;
            println!(
                "  {} {:02} {:?}: {:8.2} ms | {} files / {} dirs / {} issues",
                if warmup { "warmup" } else { "sample" },
                round + 1,
                variant,
                result.elapsed_ms,
                result.identity.counts.files,
                result.identity.counts.directories,
                result.identity.counts.issue_count
            );
            io::stdout().flush()?;
            samples.push(result);
        }
    }
    let reference = &samples[0].identity;
    let valid = samples.iter().all(|s| {
        &s.identity == reference
            && s.status == Status::Completed
            && dataset
                .expected
                .as_ref()
                .is_none_or(|expected| &s.identity.counts == expected)
    });
    let comparison = if valid {
        let baseline: Vec<_> = samples
            .iter()
            .filter(|s| s.phase == Phase::Measured && s.variant == options.baseline)
            .map(|s| s.elapsed_ms)
            .collect();
        let candidate: Vec<_> = samples
            .iter()
            .filter(|s| s.phase == Phase::Measured && s.variant == options.candidate)
            .map(|s| s.elapsed_ms)
            .collect();
        let speedup = median(
            &baseline
                .iter()
                .zip(&candidate)
                .map(|(a, b)| a / b)
                .collect::<Vec<_>>(),
        );
        let reduction = median(
            &baseline
                .iter()
                .zip(&candidate)
                .map(|(a, b)| (1.0 - b / a) * 100.0)
                .collect::<Vec<_>>(),
        );
        let a = distribution(&baseline);
        let b = distribution(&candidate);
        println!(
            "  median: baseline {:.2} ms -> candidate {:.2} ms | paired {:.2}x / {:.1}% less time",
            a.median_ms, b.median_ms, speedup, reduction
        );
        Some(Comparison {
            baseline: a,
            candidate: b,
            median_paired_speedup: speedup,
            median_paired_reduction_percent: reduction,
        })
    } else {
        eprintln!(
            "  INVALID: metadata/coverage differs, expected totals failed, or directory changed; no speedup reported."
        );
        None
    };
    Ok(DatasetReport {
        name: dataset.name,
        root: dataset.root.to_string_lossy().into_owned(),
        expected: dataset.expected,
        valid,
        invalid_reason: (!valid).then(|| "Entry metadata/coverage differs, expected totals failed, scan failed, or directory changed (including preflight).".into()),
        samples,
        comparison,
    })
}

#[derive(Serialize)]
struct Report {
    schema_version: u32,
    label: String,
    timestamp_ms: u64,
    environment: BTreeMap<String, String>,
    method: String,
    runs_per_variant: usize,
    warmups_per_variant: usize,
    core_source_sha256: String,
    benchmark_source_sha256: String,
    parallel_source_sha256: String,
    candidate: Variant,
    baseline: Variant,
    workers: usize,
    valid: bool,
    datasets: Vec<DatasetReport>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(options) = Options::parse().map_err(io::Error::other)? else {
        return Ok(());
    };
    if cfg!(debug_assertions) {
        return Err("Use cargo bench with its optimized bench profile, not a debug build".into());
    }
    let rustc = Command::new("rustc").arg("--version").output()?;
    let mut environment = BTreeMap::from([
        ("os".into(), std::env::consts::OS.into()),
        ("arch".into(), std::env::consts::ARCH.into()),
        (
            "rustc".into(),
            String::from_utf8_lossy(&rustc.stdout).trim().into(),
        ),
        (
            "logical_cpus".into(),
            std::thread::available_parallelism()?.get().to_string(),
        ),
        ("profile".into(), "bench (optimized)".into()),
        (
            "filesystem_cache".into(),
            "warm; not evicted; fixture creation is excluded".into(),
        ),
    ]);
    if let Ok(cpu) = std::env::var("PROCESSOR_IDENTIFIER") {
        environment.insert("cpu".into(), cpu);
    }
    let git = Command::new("git").args(["rev-parse", "HEAD"]).output()?;
    environment.insert(
        "git_head".into(),
        String::from_utf8_lossy(&git.stdout).trim().into(),
    );
    let mut report = Report {
        schema_version: 4,
        label: options.label.clone(),
        timestamp_ms: orion_core::now_ms(),
        environment,
        method: "Candidate-first correctness preflight, then same-process AB/BA pairs after warmup; new Scan each sample. Timer includes new/run/summary; excludes fixture setup, checksum, serialization and drop. Equality includes preflight and uses SHA-256 of sorted paths/types/sizes/mtime/enumeration plus counts/errors. No HTTP or GUI polling. Not cold-cache or whole-volume performance.".into(),
        runs_per_variant: options.runs,
        warmups_per_variant: options.warmups,
        candidate: options.candidate,
        baseline: options.baseline,
        workers: options.workers,
        parallel_source_sha256: format!("{:x}", Sha256::digest(include_bytes!("../src/scanner/parallel.rs"))),
        benchmark_source_sha256: format!("{:x}", Sha256::digest(include_bytes!("scan.rs"))),
        core_source_sha256: core_source_digest(),
        valid: true,
        datasets: vec![],
    };
    if let Some(root) = &options.root {
        report.datasets.push(run_dataset(
            Dataset {
                name: "real-directory".into(),
                root: root.clone(),
                expected: None,
                _owner: None,
            },
            &options,
        )?);
    } else {
        let suites = if options.suite == "all" {
            vec!["wide", "tree", "deep"]
        } else {
            vec![options.suite.as_str()]
        };
        for suite in suites {
            report
                .datasets
                .push(run_dataset(Dataset::generated(suite)?, &options)?);
        }
    }
    report.valid = report.datasets.iter().all(|d| d.valid);
    if let Some(output) = &options.output {
        if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        // Explicit output only, written after all scans; never mix report writes into timing.
        let file = fs::File::create(output)?;
        serde_json::to_writer_pretty(file, &report)?;
        println!("\nReport: {}", output.display());
    }
    if !report.valid {
        return Err(
            "Benchmark invalid: filesystem changed or implementations disagree (see JSON)".into(),
        );
    }
    Ok(())
}

fn core_source_digest() -> String {
    let mut digest = Sha256::new();
    for source in [
        include_bytes!("../src/lib.rs").as_slice(),
        include_bytes!("../src/model.rs").as_slice(),
        include_bytes!("../src/index.rs").as_slice(),
        include_bytes!("../src/scan.rs").as_slice(),
        include_bytes!("../src/query.rs").as_slice(),
        include_bytes!("../src/legacy_scan.rs").as_slice(),
        include_bytes!("../src/scanner/mod.rs").as_slice(),
        include_bytes!("../src/scanner/parallel.rs").as_slice(),
        include_bytes!("../src/scanner/filesystem.rs").as_slice(),
    ] {
        digest.update((source.len() as u64).to_le_bytes());
        digest.update(source);
    }
    format!("{:x}", digest.finalize())
}
