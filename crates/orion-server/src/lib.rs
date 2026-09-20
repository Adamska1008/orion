mod http;
pub mod tasks;

pub use http::contracts::ConnectionDocument;
pub use http::router;
use std::sync::Arc;
pub use tasks::StartScan;
use tasks::{TaskCoordinator, TaskError};
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    token: Arc<String>,
    pub instance_id: Uuid,
    coordinator: Arc<TaskCoordinator>,
}

impl AppState {
    pub fn new(token: String) -> Self {
        Self {
            token: Arc::new(token),
            instance_id: Uuid::new_v4(),
            coordinator: Arc::new(TaskCoordinator::new(orion_core::default_scan_workers())),
        }
    }

    pub fn with_scan_workers(self, workers: usize) -> Self {
        self.coordinator.set_scan_workers(workers);
        self
    }

    pub fn begin_shutdown(&self, cancel_active: bool) -> Result<(), TaskError> {
        self.coordinator.begin_shutdown(cancel_active)
    }
    pub async fn shutdown_requested(&self) {
        self.coordinator.shutdown_requested().await;
    }
    pub async fn wait_for_scan_exit(&self) {
        self.coordinator.wait_for_scan_exit().await;
    }
}
