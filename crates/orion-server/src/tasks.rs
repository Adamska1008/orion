//! Application rules for the single retained scan. No HTTP types or filesystem I/O under the task lock.
use orion_core::{Scan, Summary};
use std::{
    collections::HashMap,
    path::Path,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc, Condvar, Mutex,
    },
};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskError {
    ShuttingDown,
    RequestConflict,
    ResultReplaced,
    ScanInProgress(Option<Uuid>),
    SessionLimit,
    InvalidRoot(String),
    WorkerStartFailed,
    TaskNotFound,
    Internal,
}

pub struct StartScan {
    pub root: String,
    pub request_id: Uuid,
}

struct Pending {
    request: StartScan,
    result: Mutex<Option<Result<Arc<Scan>, TaskError>>>,
    ready: Condvar,
}

impl Pending {
    fn wait(&self) -> Result<Arc<Scan>, TaskError> {
        let mut result = self.result.lock().unwrap();
        while result.is_none() {
            result = self.ready.wait(result).unwrap();
        }
        result.as_ref().unwrap().clone()
    }

    fn complete(&self, result: Result<Arc<Scan>, TaskError>) {
        *self.result.lock().unwrap() = Some(result);
        self.ready.notify_all();
    }
}

#[derive(Default)]
struct Tasks {
    latest: Option<Arc<Scan>>,
    requests: HashMap<Uuid, (String, Uuid)>,
    pending: Option<Arc<Pending>>,
    shutting_down: bool,
}

pub struct TaskCoordinator {
    tasks: Mutex<Tasks>,
    workers: AtomicUsize,
    shutdown: tokio::sync::Notify,
}

impl TaskCoordinator {
    pub fn new(workers: usize) -> Self {
        Self {
            tasks: Mutex::default(),
            workers: AtomicUsize::new(workers.clamp(1, orion_core::MAX_SCAN_WORKERS)),
            shutdown: tokio::sync::Notify::new(),
        }
    }

    pub(crate) fn set_scan_workers(&self, workers: usize) {
        self.workers.store(
            workers.clamp(1, orion_core::MAX_SCAN_WORKERS),
            Ordering::Relaxed,
        );
    }

    pub fn latest(&self) -> Option<Arc<Scan>> {
        self.tasks.lock().unwrap().latest.clone()
    }

    pub fn task(&self, id: Uuid) -> Result<Arc<Scan>, TaskError> {
        self.latest()
            .filter(|scan| scan.id == id)
            .ok_or(TaskError::TaskNotFound)
    }

    pub fn summaries(&self) -> Vec<Summary> {
        self.latest().iter().map(|scan| scan.summary()).collect()
    }

    pub fn begin_shutdown(&self, cancel_active: bool) -> Result<(), TaskError> {
        let scan = {
            let mut tasks = self.tasks.lock().unwrap();
            if tasks.shutting_down {
                return Ok(());
            }
            let active = tasks
                .latest
                .as_ref()
                .filter(|scan| scan.status().active())
                .cloned();
            if !cancel_active && (active.is_some() || tasks.pending.is_some()) {
                return Err(TaskError::ScanInProgress(
                    active.as_ref().map(|scan| scan.id),
                ));
            }
            tasks.shutting_down = true;
            active
        };
        if let Some(scan) = scan {
            scan.cancel();
        }
        self.shutdown.notify_one();
        Ok(())
    }

    pub async fn shutdown_requested(&self) {
        self.shutdown.notified().await;
    }

    pub async fn wait_for_scan_exit(&self) {
        while self.latest().is_some_and(|scan| scan.status().active()) {
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }

    pub fn start(&self, request: StartScan) -> Result<Summary, TaskError> {
        self.start_with(request, Scan::new)
            .map(|scan| scan.summary())
    }

    fn start_with(
        &self,
        request: StartScan,
        prepare: impl FnOnce(&Path) -> Result<Arc<Scan>, String>,
    ) -> Result<Arc<Scan>, TaskError> {
        let pending = {
            let mut tasks = self.tasks.lock().unwrap();
            if tasks.shutting_down {
                return Err(TaskError::ShuttingDown);
            }
            if let Some((root, id)) = tasks.requests.get(&request.request_id) {
                if root != &request.root {
                    return Err(TaskError::RequestConflict);
                }
                return tasks
                    .latest
                    .as_ref()
                    .filter(|scan| scan.id == *id)
                    .cloned()
                    .ok_or(TaskError::ResultReplaced);
            }
            if let Some(pending) = &tasks.pending {
                if pending.request.request_id != request.request_id {
                    return Err(TaskError::ScanInProgress(None));
                }
                if pending.request.root != request.root {
                    return Err(TaskError::RequestConflict);
                }
                let pending = pending.clone();
                drop(tasks);
                return pending.wait();
            }
            if let Some(scan) = tasks.latest.as_ref().filter(|scan| scan.status().active()) {
                return Err(TaskError::ScanInProgress(Some(scan.id)));
            }
            if tasks.requests.len() >= 1024 {
                return Err(TaskError::SessionLimit);
            }
            let pending = Arc::new(Pending {
                request,
                result: Mutex::new(None),
                ready: Condvar::new(),
            });
            tasks.pending = Some(pending.clone());
            pending
        };

        // This guard also releases duplicate callers if validation unexpectedly panics.
        let reservation = Reservation {
            coordinator: self,
            pending: pending.clone(),
            completed: false,
        };
        let result = self.prepare_and_commit(&pending, prepare);
        reservation.complete(result.clone());
        result
    }

    fn prepare_and_commit(
        &self,
        pending: &Pending,
        prepare: impl FnOnce(&Path) -> Result<Arc<Scan>, String>,
    ) -> Result<Arc<Scan>, TaskError> {
        let scan = prepare(Path::new(&pending.request.root)).map_err(TaskError::InvalidRoot)?;
        let worker = scan.clone();
        let workers = self.workers.load(Ordering::Relaxed);
        let (release, ready) = mpsc::channel();
        std::thread::Builder::new()
            .name("orion-scan".into())
            .spawn(move || {
                // The worker cannot begin before the commit/shutdown decision under the task lock.
                if ready.recv() != Ok(()) {
                    return;
                }
                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    worker.run_with_workers(workers)
                }))
                .is_err()
                {
                    worker.fail("扫描工作线程意外退出。".into());
                }
            })
            .map_err(|_| TaskError::WorkerStartFailed)?;
        let previous = {
            let mut tasks = self.tasks.lock().unwrap();
            if tasks.shutting_down {
                return Err(TaskError::ShuttingDown);
            }
            // Sending on an unbounded channel cannot block. No I/O or old index destruction here.
            release.send(()).map_err(|_| TaskError::WorkerStartFailed)?;
            tasks.requests.insert(
                pending.request.request_id,
                (pending.request.root.clone(), scan.id),
            );
            tasks.latest.replace(scan.clone())
        };
        drop(previous);
        Ok(scan)
    }

    #[cfg(test)]
    pub(crate) fn set_scan_for_test(&self, scan: Arc<Scan>) {
        self.tasks.lock().unwrap().latest = Some(scan);
    }

    #[cfg(test)]
    pub(crate) fn is_shutting_down(&self) -> bool {
        self.tasks.lock().unwrap().shutting_down
    }
}

struct Reservation<'a> {
    coordinator: &'a TaskCoordinator,
    pending: Arc<Pending>,
    completed: bool,
}

impl Reservation<'_> {
    fn complete(mut self, result: Result<Arc<Scan>, TaskError>) {
        self.pending.complete(result);
        self.completed = true;
    }
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        if !self.completed {
            self.pending.complete(Err(TaskError::Internal));
        }
        let mut tasks = self.coordinator.tasks.lock().unwrap();
        if tasks
            .pending
            .as_ref()
            .is_some_and(|pending| Arc::ptr_eq(pending, &self.pending))
        {
            tasks.pending = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::atomic::{AtomicUsize, Ordering},
        time::Duration,
    };

    fn request(root: &Path, request_id: Uuid) -> StartScan {
        StartScan {
            root: root.to_string_lossy().into(),
            request_id,
        }
    }

    #[test]
    fn slow_validation_does_not_block_queries_or_shutdown_and_cannot_launch_after_exit() {
        let directory = tempfile::tempdir().unwrap();
        let coordinator = Arc::new(TaskCoordinator::new(1));
        let scan = Scan::new(directory.path()).unwrap();
        let prepared = scan.clone();
        let (entered, entered_rx) = mpsc::channel();
        let (release, release_rx) = mpsc::channel();
        let starter = coordinator.clone();
        let request = request(directory.path(), Uuid::new_v4());
        let worker = std::thread::spawn(move || {
            starter.start_with(request, |_| {
                entered.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok(prepared)
            })
        });
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let controller = coordinator.clone();
        let (answer, answer_rx) = mpsc::channel();
        let control = std::thread::spawn(move || {
            assert!(controller.summaries().is_empty());
            assert_eq!(
                controller.begin_shutdown(false),
                Err(TaskError::ScanInProgress(None))
            );
            answer.send(controller.begin_shutdown(true)).unwrap();
        });
        let response = answer_rx.recv_timeout(Duration::from_secs(2));
        release.send(()).unwrap(); // Always release the fixture before making assertions.
        control.join().unwrap();
        assert_eq!(response.unwrap(), Ok(()));
        assert_eq!(worker.join().unwrap().err(), Some(TaskError::ShuttingDown));
        assert!(coordinator.latest().is_none());
        assert_eq!(scan.summary().revision, 0);
    }

    #[test]
    fn concurrent_duplicates_share_preparation_and_request_identity() {
        let directory = tempfile::tempdir().unwrap();
        let coordinator = Arc::new(TaskCoordinator::new(1));
        let calls = Arc::new(AtomicUsize::new(0));
        let id = Uuid::new_v4();
        let (entered, entered_rx) = mpsc::channel();
        let (release, release_rx) = mpsc::channel();
        let starter = coordinator.clone();
        let count = calls.clone();
        let first = request(directory.path(), id);
        let worker = std::thread::spawn(move || {
            starter.start_with(first, |path| {
                count.fetch_add(1, Ordering::SeqCst);
                entered.send(()).unwrap();
                release_rx.recv().unwrap();
                Scan::new(path)
            })
        });
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(
            coordinator.start(request(Path::new("different"), id)).err(),
            Some(TaskError::RequestConflict)
        );
        let duplicate = coordinator.clone();
        let count = calls.clone();
        let second = request(directory.path(), id);
        let retry = std::thread::spawn(move || {
            duplicate.start_with(second, |path| {
                count.fetch_add(1, Ordering::SeqCst);
                Scan::new(path)
            })
        });
        release.send(()).unwrap();
        assert_eq!(
            worker.join().unwrap().unwrap().id,
            retry.join().unwrap().unwrap().id
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn validation_failure_and_panic_release_the_reservation() {
        let directory = tempfile::tempdir().unwrap();
        let coordinator = TaskCoordinator::new(1);
        let id = Uuid::new_v4();
        assert_eq!(
            coordinator
                .start_with(request(directory.path(), id), |_| Err("fixture".into()))
                .err(),
            Some(TaskError::InvalidRoot("fixture".into()))
        );
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ =
                coordinator.start_with(request(directory.path(), id), |_| panic!("fixture panic"));
        }))
        .is_err());
        assert!(coordinator.tasks.lock().unwrap().pending.is_none());
        assert!(coordinator.start(request(directory.path(), id)).is_ok());
    }
}
