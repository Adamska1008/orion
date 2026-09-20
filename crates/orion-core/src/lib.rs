//! Read-only filesystem scanning. No HTTP, GUI, or process lifetime assumptions.
use serde::Serialize;
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, RwLock,
    },
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Running,
    Cancelling,
    Cancelled,
    Completed,
    Failed,
}

impl Status {
    pub fn active(self) -> bool {
        matches!(self, Self::Running | Self::Cancelling)
    }
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Directory,
    File,
    Link,
    Other,
}

#[derive(Clone, Debug, Serialize)]
pub struct ScanIssue {
    pub path: String,
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Summary {
    pub id: Uuid,
    pub root: String,
    pub status: Status,
    pub revision: u64,
    pub started_at: u64,
    pub finished_at: Option<u64>,
    pub files: u64,
    pub directories: u64,
    pub logical_bytes: u64,
    pub allocated_bytes: Option<u64>,
    pub complete: bool,
    pub issue_count: u64,
    pub issues: Vec<ScanIssue>,
}

#[derive(Clone, Debug, Serialize)]
pub struct EntryView {
    pub id: usize,
    pub parent_id: Option<usize>,
    pub name: String,
    pub kind: Kind,
    pub logical_bytes: u64,
    pub allocated_bytes: Option<u64>,
    pub modified_at: Option<u64>,
    pub enumerated: bool,
}

#[derive(Debug, Serialize)]
pub struct Detail {
    #[serde(flatten)]
    pub entry: EntryView,
    pub path: String,
    pub ancestors: Vec<EntryView>,
}

#[derive(Debug, Serialize)]
pub struct Page {
    pub revision: u64,
    pub total: usize,
    pub offset: usize,
    pub entries: Vec<EntryView>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum QueryError {
    NotFound,
    NotDirectory,
    StaleRevision,
}

struct Entry {
    parent: Option<usize>,
    name: OsString,
    kind: Kind,
    bytes: u64,
    modified: Option<u64>,
    enumerated: bool,
    children: Vec<usize>,
}

impl Entry {
    fn view(&self, id: usize) -> EntryView {
        EntryView {
            id,
            parent_id: self.parent,
            name: self.name.to_string_lossy().into(),
            kind: self.kind,
            logical_bytes: self.bytes,
            allocated_bytes: None,
            modified_at: self.modified,
            enumerated: self.enumerated,
        }
    }
}

struct Index {
    status: Status,
    revision: u64,
    finished_at: Option<u64>,
    files: u64,
    directories: u64,
    entries: Vec<Entry>,
    issue_count: u64,
    issues: Vec<ScanIssue>,
}

pub struct Scan {
    pub id: Uuid,
    root: PathBuf,
    started_at: u64,
    cancel: AtomicBool,
    index: RwLock<Index>,
}

#[cfg(feature = "bench-internals")]
#[doc(hidden)]
pub type BenchmarkRow = (PathBuf, Kind, u64, Option<u64>, bool);

fn modified(metadata: &fs::Metadata) -> Option<u64> {
    metadata
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|t| t.as_millis() as u64)
}

fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0 // All reparse points, including junctions.
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

impl Scan {
    pub fn new(path: &Path) -> Result<Arc<Self>, String> {
        if !path.is_absolute() {
            return Err("扫描范围必须是绝对路径。".into());
        }
        let metadata = fs::symlink_metadata(path).map_err(|e| format!("无法访问扫描范围：{e}"))?;
        if is_link(&metadata) || !metadata.is_dir() {
            return Err("请选择普通目录；本轮不跟随符号链接或重解析点。".into());
        }
        let root = fs::canonicalize(path).map_err(|e| format!("无法解析扫描范围：{e}"))?;
        Ok(Arc::new(Self {
            id: Uuid::new_v4(),
            root: root.clone(),
            started_at: now_ms(),
            cancel: AtomicBool::new(false),
            index: RwLock::new(Index {
                status: Status::Running,
                revision: 0,
                finished_at: None,
                files: 0,
                directories: 1,
                issue_count: 0,
                issues: vec![],
                entries: vec![Entry {
                    parent: None,
                    name: root.into_os_string(),
                    kind: Kind::Directory,
                    bytes: 0,
                    modified: modified(&metadata),
                    enumerated: false,
                    children: vec![],
                }],
            }),
        }))
    }

    pub fn summary(&self) -> Summary {
        let index = self.index.read().unwrap();
        Summary {
            id: self.id,
            root: self.root.to_string_lossy().into(),
            status: index.status,
            revision: index.revision,
            started_at: self.started_at,
            finished_at: index.finished_at,
            files: index.files,
            directories: index.directories,
            logical_bytes: index.entries[0].bytes,
            allocated_bytes: None,
            complete: index.status == Status::Completed && index.issue_count == 0,
            issue_count: index.issue_count,
            issues: index.issues.clone(),
        }
    }

    pub fn cancel(&self) {
        let mut index = self.index.write().unwrap();
        if index.status.active() {
            self.cancel.store(true, Ordering::Relaxed);
            index.status = Status::Cancelling;
            index.revision += 1;
        }
    }

    fn issue(&self, path: &Path, code: &str, message: String) {
        let mut index = self.index.write().unwrap();
        index.issue_count += 1;
        // Keep the API response and memory bounded even for inaccessible trees.
        if index.issues.len() < 100 {
            index.issues.push(ScanIssue {
                path: path.to_string_lossy().into(),
                code: code.into(),
                message,
            });
        }
        index.revision += 1;
    }

    pub fn fail(&self, message: String) {
        self.issue(&self.root, "worker_failed", message);
        self.finish(Status::Failed);
    }

    fn finish(&self, status: Status) {
        let mut index = self.index.write().unwrap();
        // Resolve cancellation while holding the same lock as cancel().
        index.status = if status != Status::Failed && self.cancel.load(Ordering::Relaxed) {
            Status::Cancelled
        } else {
            status
        };
        index.finished_at = Some(now_ms());
        index.revision += 1;
    }

    /// Called on a dedicated worker; filesystem I/O never holds the index lock.
    pub fn run(&self) {
        // Enumerated metadata can retain stale sizes for other names of a hard link.
        self.run_with_metadata(|_, path| fs::symlink_metadata(path));
    }

    /// Historical path-query implementation, compiled only for controlled A/B benchmarks.
    #[cfg(feature = "bench-internals")]
    #[doc(hidden)]
    pub fn run_legacy_metadata(&self) {
        self.run_with_metadata(|_, path| fs::symlink_metadata(path));
    }

    /// Experimental enumeration cache; may return stale NTFS hard-link information.
    #[cfg(feature = "bench-internals")]
    #[doc(hidden)]
    pub fn run_enumeration_metadata(&self) {
        self.run_with_metadata(|entry, _| entry.metadata());
    }

    fn run_with_metadata(
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
                    });
                    index.entries[parent].children.push(id);
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
    #[cfg(feature = "bench-internals")]
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

    pub fn list(
        &self,
        parent: usize,
        offset: usize,
        limit: usize,
        revision: Option<u64>,
    ) -> Result<Page, QueryError> {
        let (revision, mut entries) = {
            let index = self.index.read().unwrap();
            if revision.is_some_and(|v| v != index.revision) {
                return Err(QueryError::StaleRevision);
            }
            let entry = index.entries.get(parent).ok_or(QueryError::NotFound)?;
            if entry.kind != Kind::Directory {
                return Err(QueryError::NotDirectory);
            }
            let entries = entry
                .children
                .iter()
                .map(|&id| index.entries[id].view(id))
                .collect::<Vec<_>>();
            (index.revision, entries)
        };
        // Sort outside the lock; a response is a coherent snapshot of one revision.
        entries.sort_unstable_by(|a, b| {
            b.logical_bytes
                .cmp(&a.logical_bytes)
                .then_with(|| a.name.cmp(&b.name))
                .then(a.id.cmp(&b.id))
        });
        let total = entries.len();
        Ok(Page {
            revision,
            total,
            offset,
            entries: entries
                .into_iter()
                .skip(offset)
                .take(limit.min(500))
                .collect(),
        })
    }

    pub fn detail(&self, id: usize) -> Result<Detail, QueryError> {
        let index = self.index.read().unwrap();
        let entry = index.entries.get(id).ok_or(QueryError::NotFound)?;
        let mut ids = vec![];
        let mut parent = entry.parent;
        while let Some(p) = parent {
            ids.push(p);
            parent = index.entries[p].parent;
        }
        ids.reverse();
        let mut path = self.root.clone();
        for &p in ids.iter().skip(1) {
            path.push(&index.entries[p].name);
        }
        if id != 0 {
            path.push(&entry.name);
        }
        Ok(Detail {
            entry: entry.view(id),
            path: path.to_string_lossy().into(),
            ancestors: ids.into_iter().map(|p| index.entries[p].view(p)).collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_totals_sorted_pages_and_unicode_paths() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join("中文")).unwrap();
        fs::write(temp.path().join("中文/a"), [0; 29]).unwrap();
        fs::write(temp.path().join("b"), [0; 11]).unwrap();
        fs::write(temp.path().join("c"), []).unwrap();
        let scan = Scan::new(temp.path()).unwrap();
        scan.run();
        let summary = scan.summary();
        assert!(summary.complete);
        assert_eq!(
            (summary.files, summary.directories, summary.logical_bytes),
            (3, 2, 40)
        );
        assert_eq!(summary.allocated_bytes, None);
        let page = scan.list(0, 0, 1, None).unwrap();
        assert_eq!(page.total, 3);
        assert_eq!(page.entries[0].name, "中文");
        let child = scan.list(page.entries[0].id, 0, 5, None).unwrap();
        assert!(Path::new(&scan.detail(child.entries[0].id).unwrap().path).ends_with("中文/a"));
        assert_eq!(
            scan.list(0, 1, 1, Some(summary.revision)).unwrap().entries[0].logical_bytes,
            11
        );
        assert!(matches!(
            scan.list(0, 0, 1, Some(0)),
            Err(QueryError::StaleRevision)
        ));
    }

    #[test]
    fn cancellation_is_terminal_and_keeps_partial_data() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("file"), [0; 10]).unwrap();
        let scan = Scan::new(temp.path()).unwrap();
        scan.cancel();
        scan.cancel();
        scan.run();
        assert_eq!(scan.summary().status, Status::Cancelled);
        assert!(!scan.summary().complete);
        assert!(scan.summary().finished_at.is_some());
        scan.cancel();
        assert_eq!(scan.summary().status, Status::Cancelled);
    }

    #[test]
    fn vanished_root_is_reported_and_relative_paths_rejected() {
        assert!(Scan::new(Path::new("relative")).is_err());
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("gone");
        fs::create_dir(&path).unwrap();
        let scan = Scan::new(&path).unwrap();
        fs::remove_dir(&path).unwrap();
        scan.run();
        assert!(!scan.summary().complete);
        assert_eq!(scan.summary().issue_count, 1);
    }

    #[test]
    fn production_reads_updated_hard_link_size() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("中文目录");
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("data.bin"), [7; 4096]).unwrap();
        fs::write(dir.join("empty.bin"), []).unwrap();
        fs::hard_link(dir.join("data.bin"), temp.path().join("hard-link.bin")).unwrap();
        // A linked name may still carry stale directory-entry information after this resize.
        fs::write(dir.join("data.bin"), [9; 8192]).unwrap();
        let current = Scan::new(temp.path()).unwrap();
        current.run();
        assert_eq!(current.summary().logical_bytes, 16384);
        assert_eq!(current.summary().files, 3);
        assert!(current.summary().complete);
        let alias = current
            .list(0, 0, 10, None)
            .unwrap()
            .entries
            .into_iter()
            .find(|entry| entry.name == "hard-link.bin")
            .unwrap();
        assert_eq!(alias.logical_bytes, 8192);
    }

    #[cfg(windows)]
    #[test]
    fn junction_is_not_followed() {
        use std::process::Command;
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("file"), [0; 50]).unwrap();
        let root = temp.path().join("root");
        fs::create_dir(&root).unwrap();
        let link = root.join("junction");
        let output = Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&link)
            .arg(&target)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "junction fixture failed: {:?}",
            output
        );
        let scan = Scan::new(&root).unwrap();
        scan.run();
        assert_eq!(scan.summary().logical_bytes, 0);
        assert_eq!(scan.summary().issues[0].code, "link_skipped");
        #[cfg(feature = "bench-internals")]
        {
            let candidate = Scan::new(&root).unwrap();
            candidate.run_enumeration_metadata();
            assert_eq!(scan.benchmark_rows(), candidate.benchmark_rows());
            assert_eq!(scan.summary().issue_count, candidate.summary().issue_count);
        }
        // Remove only the test junction using a native directory operation.
        fs::remove_dir(link).unwrap();
        assert!(target.join("file").exists());
    }
}
