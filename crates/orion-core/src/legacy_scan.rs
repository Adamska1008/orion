use crate::{
    index::Entry,
    scanner::filesystem::{is_link, modified},
    *,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::Ordering,
};

#[doc(hidden)]
pub type BenchmarkRow = (PathBuf, Kind, u64, Option<u64>, bool);

impl Scan {
    /// Historical path-query implementation, compiled only for controlled A/B benchmarks.
    #[cfg(feature = "bench-internals")]
    #[doc(hidden)]
    pub fn run_legacy_metadata(&self) {
        self.run_with_metadata(|_, path| fs::symlink_metadata(path));
    }

    /// Frozen serial enumerator used before introducing directory workers and batching.
    #[cfg(feature = "bench-internals")]
    #[doc(hidden)]
    pub fn run_enumeration_metadata(&self) {
        self.run_with_metadata(|entry, _| entry.metadata());
    }

    #[cfg(any(feature = "bench-internals", test))]
    pub(crate) fn run_with_metadata(
        &self,
        metadata_for: impl Fn(&fs::DirEntry, &Path) -> std::io::Result<fs::Metadata>,
    ) {
        let mut pending = vec![(self.root.clone(), 0)];
        while let Some((path, parent)) = pending.pop() {
            if self.cancel.load(Ordering::Relaxed) {
                break;
            }
            // Recheck queued directories: they may have been replaced since discovery.
            let valid = fs::symlink_metadata(&path).is_ok_and(|m| m.is_dir() && !is_link(&m))
                && fs::canonicalize(&path).is_ok_and(|p| p.starts_with(&self.root));
            if !valid {
                self.issue(
                    &path,
                    "directory_changed",
                    "目录消失、变为链接或已越出扫描范围，已跳过。".into(),
                );
                if parent == 0 {
                    self.finish(Status::Failed);
                    return;
                }
                continue;
            }
            let reader = match fs::read_dir(&path) {
                Ok(reader) => reader,
                Err(e) => {
                    self.issue(&path, "read_directory", e.to_string());
                    if parent == 0 {
                        self.finish(Status::Failed);
                        return;
                    }
                    continue;
                }
            };
            let mut enumerated = true;
            for child in reader {
                if self.cancel.load(Ordering::Relaxed) {
                    enumerated = false;
                    break;
                }
                let child = match child {
                    Ok(child) => child,
                    Err(e) => {
                        self.issue(&path, "read_entry", e.to_string());
                        enumerated = false;
                        continue;
                    }
                };
                let child_path = child.path();
                let metadata = match metadata_for(&child, &child_path) {
                    Ok(metadata) => metadata,
                    Err(e) => {
                        self.issue(&child_path, "metadata", e.to_string());
                        enumerated = false;
                        continue;
                    }
                };
                let kind = if is_link(&metadata) {
                    Kind::Link
                } else if metadata.is_dir() {
                    Kind::Directory
                } else if metadata.is_file() {
                    Kind::File
                } else {
                    Kind::Other
                };
                let bytes = if kind == Kind::File {
                    metadata.len()
                } else {
                    0
                };
                let id = {
                    let mut index = self.index.write().unwrap();
                    let id = index.entries.len();
                    index.entries.push(Entry {
                        parent: Some(parent),
                        name: child.file_name(),
                        kind,
                        bytes,
                        modified: modified(&metadata),
                        enumerated: kind == Kind::File,
                        children: vec![],
                        children_revision: 0,
                    });
                    index.entries[parent].children.push(id);
                    index.entries[parent].children_revision = index.revision + 1;
                    if kind == Kind::File {
                        index.files += 1;
                    }
                    if kind == Kind::Directory {
                        index.directories += 1;
                    }
                    let mut ancestor = Some(parent);
                    while let Some(a) = ancestor {
                        index.entries[a].bytes = index.entries[a].bytes.saturating_add(bytes);
                        ancestor = index.entries[a].parent;
                        if let Some(parent) = ancestor {
                            index.entries[parent].children_revision = index.revision + 1;
                        }
                    }
                    index.revision += 1;
                    id
                };
                match kind {
                    Kind::Directory => pending.push((child_path, id)),
                    Kind::Link => {
                        self.issue(&child_path, "link_skipped", "未跟随链接或重解析点。".into())
                    }
                    Kind::Other => {
                        self.issue(&child_path, "special_skipped", "未统计特殊文件。".into())
                    }
                    Kind::File => {}
                }
            }
            let mut index = self.index.write().unwrap();
            index.entries[parent].enumerated = enumerated;
            index.revision += 1;
        }
        self.finish(Status::Completed);
    }

    /// Canonical, order-independent rows for benchmark correctness checks (outside timing).
    #[cfg(any(feature = "bench-internals", test))]
    #[doc(hidden)]
    pub fn benchmark_rows(&self) -> Vec<BenchmarkRow> {
        let index = self.index.read().unwrap();
        let mut rows = Vec::with_capacity(index.entries.len());
        for (id, entry) in index.entries.iter().enumerate() {
            let mut parts = vec![];
            let mut current = id;
            while let Some(parent) = index.entries[current].parent {
                parts.push(index.entries[current].name.as_os_str());
                current = parent;
            }
            let path: PathBuf = parts.into_iter().rev().collect();
            rows.push((
                path,
                entry.kind,
                entry.bytes,
                entry.modified,
                entry.enumerated,
            ));
        }
        rows.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        rows
    }
}
