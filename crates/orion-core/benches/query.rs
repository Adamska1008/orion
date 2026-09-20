//! In-memory directory query benchmark. No files are created for the synthetic entries.
use orion_core::{Page, Scan};
use serde::Serialize;
use std::{
    hint::black_box,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Barrier,
    },
    time::Instant,
};

#[derive(Serialize)]
struct Timing {
    p50_ms: f64,
    p95_ms: f64,
    max_ms: f64,
    samples: usize,
    samples_ms: Vec<f64>,
}
fn distribution(samples_ms: Vec<f64>) -> Timing {
    let mut values = samples_ms.clone();
    values.sort_by(f64::total_cmp);
    Timing {
        p50_ms: values[values.len() / 2],
        p95_ms: values[(values.len() * 95 / 100).min(values.len() - 1)],
        max_ms: *values.last().unwrap(),
        samples: values.len(),
        samples_ms,
    }
}
fn time_page(f: impl FnOnce() -> Page) -> (Page, f64) {
    let begin = Instant::now();
    let page = black_box(f());
    (page, begin.elapsed().as_secs_f64() * 1000.0)
}

#[derive(Serialize)]
struct Dataset {
    entries: usize,
    cold_cached_ms: f64,
    uncached: Timing,
    cached: Timing,
    concurrent_queries: Timing,
    concurrent_publish_ms: f64,
    cancel_ms: f64,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if cfg!(debug_assertions) {
        return Err("Run with cargo bench".into());
    }
    let mut results = Vec::new();
    for count in [10_000, 100_000] {
        let directory = tempfile::tempdir().unwrap();
        let scan = Scan::new(directory.path()).unwrap();
        scan.benchmark_publish_files(count);
        let (first, cold_cached_ms) = time_page(|| scan.list(0, 0, 50, None).unwrap());
        let baseline = scan.benchmark_list_uncached(0, 0, 50).unwrap();
        assert_eq!(
            serde_json::to_value(first).unwrap(),
            serde_json::to_value(baseline).unwrap()
        );
        let mut cached = Vec::new();
        let mut uncached = Vec::new();
        for sample in 0..40 {
            let offset = sample * 50;
            // Alternate query order; both variants read the same immutable index.
            let mut run = |use_cache| {
                let (page, elapsed) = time_page(|| {
                    if use_cache {
                        scan.list(0, offset, 50, None).unwrap()
                    } else {
                        scan.benchmark_list_uncached(0, offset, 50).unwrap()
                    }
                });
                if use_cache {
                    cached.push(elapsed);
                } else {
                    uncached.push(elapsed);
                }
                page
            };
            let a = run(sample % 2 == 0);
            let b = run(sample % 2 != 0);
            assert_eq!(
                serde_json::to_value(a).unwrap(),
                serde_json::to_value(b).unwrap()
            );
        }
        let done = Arc::new(AtomicBool::new(false));
        let gate = Arc::new(Barrier::new(2));
        let writer_scan = scan.clone();
        let writer_done = done.clone();
        let writer_gate = gate.clone();
        let writer = std::thread::spawn(move || {
            writer_gate.wait();
            let begin = Instant::now();
            for _ in 0..40 {
                writer_scan.benchmark_publish_files(256);
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            writer_done.store(true, Ordering::Release);
            begin.elapsed().as_secs_f64() * 1000.0
        });
        gate.wait();
        let mut concurrent = Vec::new();
        while !done.load(Ordering::Acquire) || concurrent.is_empty() {
            let (_, elapsed) = time_page(|| scan.list(0, 0, 50, None).unwrap());
            concurrent.push(elapsed);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let concurrent_publish_ms = writer.join().unwrap();
        let begin = Instant::now();
        scan.cancel();
        let cancel_ms = begin.elapsed().as_secs_f64() * 1000.0;
        results.push(Dataset {
            entries: count,
            cold_cached_ms,
            uncached: distribution(uncached),
            cached: distribution(cached),
            concurrent_queries: distribution(concurrent),
            concurrent_publish_ms,
            cancel_ms,
        });
    }
    let report = serde_json::json!({
        "schema_version": 1,
        "timestamp_ms": orion_core::now_ms(),
        "environment": { "os": std::env::consts::OS, "arch": std::env::consts::ARCH, "logical_cpus": std::thread::available_parallelism().unwrap().get() },
        "method": "Optimized build; synthetic in-memory wide directory; 50-entry pages; 40 alternating uncached/cached comparisons with full JSON equality. Concurrent case adds 40 batches of 256 entries, paced by 1ms; reader also waits 1ms. Not filesystem scan, HTTP or UI latency. Cache limited to 32 directories/1M IDs; no full-process memory measurement.",
        "query_source_sha256": format!("{:x}", sha2::Sha256::digest(include_bytes!("../src/query.rs"))),
        "fixture_source_sha256": format!("{:x}", sha2::Sha256::digest(include_bytes!("../src/benchmark.rs"))),
        "benchmark_source_sha256": format!("{:x}", sha2::Sha256::digest(include_bytes!("query.rs"))),
        "datasets": results,
    });
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
    Ok(())
}

use sha2::Digest;
