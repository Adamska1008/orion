use crate::{
    index::{Entry, Index},
    scanner::filesystem::{is_link, modified},
    *,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, RwLock,
    },
};
use uuid::Uuid;

pub struct Scan {
    pub id: Uuid,
    pub(crate) root: PathBuf,
    pub(crate) started_at: u64,
    pub(crate) cancel: AtomicBool,
    pub(crate) index: RwLock<Index>,
    pub(crate) queries: std::sync::Mutex<crate::query::QueryCache>,
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
            queries: Default::default(),
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
                    children_revision: 0,
                }],
            }),
        }))
    }

    pub fn status(&self) -> Status {
        self.index.read().unwrap().status
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

    pub(crate) fn issue(&self, path: &Path, code: &str, message: String) {
        let mut index = self.index.write().unwrap();
        index.record_issue(ScanIssue {
            path: path.to_string_lossy().into(),
            code: code.into(),
            message,
        });
        index.revision += 1;
    }

    pub fn fail(&self, message: String) {
        self.issue(&self.root, "worker_failed", message);
        self.finish(Status::Failed);
    }

    pub(crate) fn finish(&self, status: Status) {
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
        self.run_with_workers(default_scan_workers());
    }

    /// Run bounded directory workers; values outside 1..=MAX_SCAN_WORKERS are clamped.
    pub fn run_with_workers(&self, workers: usize) {
        self.run_parallel(workers.clamp(1, MAX_SCAN_WORKERS));
    }

    pub(crate) fn commit_batch(
        &self,
        parent: usize,
        batch: &mut crate::scanner::Batch,
        enumerated: Option<bool>,
    ) -> Vec<crate::scanner::Directory> {
        self.index
            .write()
            .unwrap()
            .apply_batch(parent, batch, enumerated)
    }
}
