use super::{
    filesystem::{is_link, modified},
    Batch, Directory, Discovered,
};
use crate::{Kind, Scan, Status};
use std::{
    collections::VecDeque,
    fs,
    panic::{catch_unwind, AssertUnwindSafe},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Condvar, Mutex,
    },
    thread,
};

const BATCH_SIZE: usize = 256;

struct Work {
    pending: VecDeque<Directory>,
    active: usize,
}

struct Queue {
    work: Mutex<Work>,
    ready: Condvar,
    failed: AtomicBool,
}

impl Queue {
    fn new(root: PathBuf) -> Self {
        Self {
            work: Mutex::new(Work {
                pending: VecDeque::from([Directory { path: root, id: 0 }]),
                active: 0,
            }),
            ready: Condvar::new(),
            failed: AtomicBool::new(false),
        }
    }

    fn stopped(&self, scan: &Scan) -> bool {
        self.failed.load(Ordering::Relaxed) || scan.cancel.load(Ordering::Relaxed)
    }

    fn take(&self, scan: &Scan) -> Option<Directory> {
        let mut work = self.work.lock().unwrap();
        loop {
            if self.stopped(scan) {
                return None;
            }
            if let Some(directory) = work.pending.pop_front() {
                work.active += 1;
                return Some(directory);
            }
            // An empty queue alone is not completion: active workers may discover children.
            if work.active == 0 {
                return None;
            }
            work = self.ready.wait(work).unwrap();
        }
    }

    fn publish(&self, directories: Vec<Directory>, scan: &Scan) {
        if directories.is_empty() {
            return;
        }
        let mut work = self.work.lock().unwrap();
        if !self.stopped(scan) {
            work.pending.extend(directories);
            self.ready.notify_all();
        }
    }

    fn complete(&self) {
        let mut work = self.work.lock().unwrap();
        work.active -= 1;
        self.ready.notify_all();
    }

    fn abort(&self) {
        // Synchronize with the condvar check/wait to avoid a lost wakeup on failure.
        let mut work = self.work.lock().unwrap();
        self.failed.store(true, Ordering::Relaxed);
        work.pending.clear();
        self.ready.notify_all();
    }
}

impl Scan {
    pub(crate) fn run_parallel(&self, workers: usize) {
        self.run_pool(workers, &|_| {});
    }

    // The hook lets tests hold real workers at deterministic cancellation/failure boundaries.
    fn run_pool(&self, workers: usize, before_directory: &(impl Fn(usize) + Sync)) {
        let queue = Queue::new(self.root.clone());
        thread::scope(|scope| {
            for number in 1..workers {
                let queue = &queue;
                if let Err(error) = thread::Builder::new()
                    .name(format!("orion-directory-{number}"))
                    .spawn_scoped(scope, move || {
                        self.directory_worker(queue, before_directory)
                    })
                {
                    queue.abort();
                    self.issue(
                        &self.root,
                        "worker_failed",
                        format!("无法创建扫描线程：{error}"),
                    );
                    break;
                }
            }
            // Reuse the server's scan thread instead of adding an idle coordinator.
            self.directory_worker(&queue, before_directory);
        }); // All workers have exited before a terminal status becomes visible.
        self.finish(if queue.failed.load(Ordering::Relaxed) {
            Status::Failed
        } else {
            Status::Completed
        });
    }

    fn directory_worker(&self, queue: &Queue, before_directory: &(impl Fn(usize) + Sync)) {
        let result = catch_unwind(AssertUnwindSafe(|| {
            let mut batch = Batch::default();
            while let Some(directory) = queue.take(self) {
                before_directory(directory.id);
                if !queue.stopped(self) {
                    self.read_directory(&directory, queue, &mut batch);
                }
                queue.complete();
            }
        }));
        if result.is_err() {
            queue.abort();
            self.issue(&self.root, "worker_failed", "扫描线程异常退出。".into());
        }
    }

    fn read_directory(&self, directory: &Directory, queue: &Queue, batch: &mut Batch) {
        let path = &directory.path;
        // Discovery and execution may be far apart: revalidate every queued directory.
        let valid = fs::symlink_metadata(path).is_ok_and(|m| m.is_dir() && !is_link(&m))
            && fs::canonicalize(path).is_ok_and(|p| p.starts_with(&self.root));
        if !valid {
            self.issue(
                path,
                "directory_changed",
                "目录消失、变为链接或已越出扫描范围，已跳过。".into(),
            );
            if directory.id == 0 {
                queue.abort();
            }
            return;
        }
        let reader = match fs::read_dir(path) {
            Ok(reader) => reader,
            Err(error) => {
                self.issue(path, "read_directory", error.to_string());
                if directory.id == 0 {
                    queue.abort();
                }
                return;
            }
        };
        let mut enumerated = true;
        for child in reader {
            if queue.stopped(self) {
                enumerated = false;
                break;
            }
            match child {
                Err(error) => {
                    batch.issue(path, "read_entry", error.to_string());
                    enumerated = false;
                }
                Ok(child) => match child.metadata() {
                    Err(error) => {
                        batch.issue(&child.path(), "metadata", error.to_string());
                        enumerated = false;
                    }
                    Ok(metadata) => {
                        let kind = if is_link(&metadata) {
                            Kind::Link
                        } else if metadata.is_dir() {
                            Kind::Directory
                        } else if metadata.is_file() {
                            Kind::File
                        } else {
                            Kind::Other
                        };
                        batch.entries.push(Discovered {
                            name: child.file_name(),
                            kind,
                            bytes: if kind == Kind::File {
                                metadata.len()
                            } else {
                                0
                            },
                            modified: modified(&metadata),
                            directory_path: (kind == Kind::Directory).then(|| child.path()),
                        });
                        match kind {
                            Kind::Link => batch.issue(
                                &child.path(),
                                "link_skipped",
                                "未跟随链接或重解析点。".into(),
                            ),
                            Kind::Other => batch.issue(
                                &child.path(),
                                "special_skipped",
                                "未统计特殊文件。".into(),
                            ),
                            _ => {}
                        }
                    }
                },
            }
            if batch.len() >= BATCH_SIZE {
                self.publish_batch(directory.id, batch, None, queue);
            }
        }
        self.publish_batch(directory.id, batch, Some(enumerated), queue);
    }

    fn publish_batch(
        &self,
        parent: usize,
        batch: &mut Batch,
        enumerated: Option<bool>,
        queue: &Queue,
    ) {
        let directories = self.commit_batch(parent, batch, enumerated);
        // Never acquire the work queue while holding the index lock.
        queue.publish(directories, self);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{mpsc, Arc, Barrier},
        time::Duration,
    };

    fn tree(root: &std::path::Path) {
        fs::write(root.join("root-file"), [1; 37]).unwrap();
        for d in 0..32 {
            let directory = root.join(format!("中文-{d:02}"));
            fs::create_dir(&directory).unwrap();
            fs::create_dir(directory.join("empty")).unwrap();
            for f in 0..20 {
                fs::write(directory.join(format!("f-{f:02}")), vec![7; d + f]).unwrap();
            }
        }
    }

    #[test]
    fn parallel_and_serial_match_every_entry_and_ancestor() {
        let temp = tempfile::tempdir().unwrap();
        tree(temp.path());
        // Directory revalidation may refresh NTFS's initially stale creation timestamps.
        let warmup = Scan::new(temp.path()).unwrap();
        warmup.run_with_metadata(|entry, _| entry.metadata());
        let serial = Scan::new(temp.path()).unwrap();
        serial.run_with_metadata(|entry, _| entry.metadata());
        for workers in [1, 2, 4, 8] {
            let scan = Scan::new(temp.path()).unwrap();
            scan.run_with_workers(workers);
            assert!(scan.summary().complete);
            assert_eq!(
                scan.benchmark_rows(),
                serial.benchmark_rows(),
                "workers={workers}"
            );
            assert_eq!(scan.summary().files, 641);
            assert_eq!(scan.summary().directories, 65);
        }
    }

    #[test]
    fn cancellation_preserves_consistent_data_and_joins_active_workers() {
        let temp = tempfile::tempdir().unwrap();
        tree(temp.path());
        let scan = Scan::new(temp.path()).unwrap();
        let worker_scan = scan.clone();
        let gate = Arc::new(Barrier::new(2));
        let worker_gate = gate.clone();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            let held = AtomicBool::new(false);
            worker_scan.run_pool(4, &|id| {
                if id != 0 && !held.swap(true, Ordering::Relaxed) {
                    entered_tx.send(()).unwrap();
                    worker_gate.wait();
                }
            });
            done_tx.send(()).unwrap();
        });
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        scan.cancel();
        assert_eq!(scan.summary().status, Status::Cancelling);
        assert!(scan.summary().finished_at.is_none());
        assert!(scan.list(0, 0, 100, None).is_ok());
        {
            let index = scan.index.read().unwrap();
            for entry in &index.entries {
                if entry.kind == Kind::Directory {
                    let bytes = entry
                        .children
                        .iter()
                        .fold(0u64, |sum, &id| sum.saturating_add(index.entries[id].bytes));
                    assert_eq!(entry.bytes, bytes);
                }
            }
        }
        gate.wait();
        done_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        handle.join().unwrap();
        let summary = scan.summary();
        assert_eq!(summary.status, Status::Cancelled);
        assert!(!summary.complete);
        assert!(summary.finished_at.is_some());
        let revision = summary.revision;
        scan.cancel();
        assert_eq!(scan.summary().revision, revision);
    }

    #[test]
    fn worker_panic_wakes_waiters_and_fails_after_join() {
        let temp = tempfile::tempdir().unwrap();
        tree(temp.path());
        let scan = Scan::new(temp.path()).unwrap();
        let once = AtomicBool::new(false);
        scan.run_pool(4, &|id| {
            if id != 0 && !once.swap(true, Ordering::Relaxed) {
                panic!("injected worker failure");
            }
        });
        assert_eq!(scan.summary().status, Status::Failed);
        assert!(!scan.summary().complete);
        assert!(scan
            .summary()
            .issues
            .iter()
            .any(|issue| issue.code == "worker_failed"));
    }

    #[test]
    fn error_sample_is_bounded_and_independent_of_arrival_order() {
        let temp = tempfile::tempdir().unwrap();
        let a = Scan::new(temp.path()).unwrap();
        let b = Scan::new(temp.path()).unwrap();
        for i in 0..150 {
            a.issue(
                &temp.path().join(format!("{i:03}")),
                "read_directory",
                "test".into(),
            );
        }
        for i in (0..150).rev() {
            b.issue(
                &temp.path().join(format!("{i:03}")),
                "read_directory",
                "test".into(),
            );
        }
        let a = a.summary();
        let b = b.summary();
        assert_eq!(a.issue_count, 150);
        assert_eq!(b.issue_count, 150);
        assert_eq!(a.issues.len(), 100);
        assert_eq!(
            a.issues.iter().map(|i| &i.path).collect::<Vec<_>>(),
            b.issues.iter().map(|i| &i.path).collect::<Vec<_>>()
        );
    }

    #[cfg(windows)]
    #[test]
    fn queued_directory_replaced_by_junction_is_skipped() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        let target = temp.path().join("outside");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&target).unwrap();
        fs::write(target.join("untouched"), [7; 31]).unwrap();
        let queued = root.join("queued");
        fs::create_dir(&queued).unwrap();
        let scan = Scan::new(&root).unwrap();
        scan.run_pool(4, &|id| {
            if id != 0 {
                fs::remove_dir(&queued).unwrap();
                let output = std::process::Command::new("cmd")
                    .args(["/c", "mklink", "/J"])
                    .arg(&queued)
                    .arg(&target)
                    .output()
                    .unwrap();
                assert!(output.status.success());
            }
        });
        assert_eq!(scan.summary().status, Status::Completed);
        assert_eq!(scan.summary().logical_bytes, 0);
        assert_eq!(scan.summary().files, 0);
        assert_eq!(scan.summary().issue_count, 1);
        assert_eq!(scan.summary().issues[0].code, "directory_changed");
        fs::remove_dir(queued).unwrap();
        assert!(target.join("untouched").exists());
    }
}
