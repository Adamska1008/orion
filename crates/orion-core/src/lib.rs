//! Read-only filesystem scanning. No HTTP, GUI, or process lifetime assumptions.
#[cfg(feature = "bench-internals")]
mod benchmark;
mod index;
#[cfg(any(feature = "bench-internals", test))]
mod legacy_scan;
mod model;
mod query;
mod scan;
mod scanner;
mod treemap;
#[cfg(any(feature = "bench-internals", test))]
pub use legacy_scan::BenchmarkRow;
pub use model::*;
pub use scan::Scan;
use std::time::{SystemTime, UNIX_EPOCH};
pub use treemap::{Treemap, TreemapNode};

/// Bounds concurrent filesystem requests, including the calling scan thread.
pub const MAX_SCAN_WORKERS: usize = 16;

pub fn default_scan_workers() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get().min(4))
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests;
