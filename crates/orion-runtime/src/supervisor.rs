use crate::{
    discovery,
    http_client::{LocalClient, ShutdownResult},
    process, Connection,
};
use std::{
    fs::{self, OpenOptions},
    path::PathBuf,
    process::Child,
    sync::Mutex,
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendStatus {
    Starting,
    NotConnected,
    Ready,
    Scanning(u64),
    Cancelling,
    Completed(u64),
    Failed,
    Cancelled,
    Stopping,
    Disconnected,
    Unknown,
}

#[derive(Clone)]
enum ConnectionState {
    Starting,
    Disconnected,
    Connected(Connection),
    Stopping,
}
impl ConnectionState {
    fn status(&self) -> BackendStatus {
        match self {
            Self::Starting => BackendStatus::Starting,
            Self::Disconnected => BackendStatus::NotConnected,
            Self::Stopping => BackendStatus::Stopping,
            Self::Connected(_) => BackendStatus::Disconnected, // Reconnected while an older poll was in flight.
        }
    }
}

#[derive(Debug)]
pub enum ExitError {
    ActiveScan,
    Failed(String),
}

impl std::fmt::Display for ExitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ActiveScan => f.write_str("后台仍在扫描，请确认停止扫描后退出。"),
            Self::Failed(message) => f.write_str(message),
        }
    }
}

#[derive(Default)]
struct Session {
    connection: Option<Connection>,
    child: Option<Child>,
    stopping: bool,
}

pub struct Backend {
    runtime: PathBuf,
    server: PathBuf,
    external: Option<PathBuf>,
    client: LocalClient,
    operation: Mutex<Session>,
    state: Mutex<ConnectionState>,
}

impl Backend {
    pub fn new(
        runtime: PathBuf,
        server: PathBuf,
        external: Option<PathBuf>,
    ) -> Result<Self, String> {
        let current = std::env::current_dir().map_err(|e| e.to_string())?;
        let runtime = if runtime.is_absolute() {
            runtime
        } else {
            current.join(runtime)
        };
        let external = external.map(|path| {
            if path.is_absolute() {
                path
            } else {
                current.join(path)
            }
        });
        let client = LocalClient::new()?;
        Ok(Self {
            runtime,
            server,
            external,
            client,
            operation: Mutex::new(Session::default()),
            state: Mutex::new(ConnectionState::Disconnected),
        })
    }

    fn connection_path(&self) -> PathBuf {
        self.external
            .clone()
            .unwrap_or_else(|| self.runtime.join("connection.json"))
    }

    fn read_connection(&self) -> Result<Connection, String> {
        discovery::read_connection(&self.connection_path())
    }
    fn health(&self, connection: &Connection) -> Result<(), String> {
        self.client.health(connection)
    }
    fn publish(&self, state: ConnectionState) {
        *self.state.lock().unwrap() = state;
    }

    pub fn ensure(&self) -> Result<Connection, String> {
        let mut session = self.operation.lock().unwrap();
        if session.stopping {
            return Err("后台正在退出。".into());
        }
        self.publish(ConnectionState::Starting);
        let result = self.ensure_session(&mut session);
        self.publish(match &result {
            Ok(connection) => ConnectionState::Connected(connection.clone()),
            Err(_) => ConnectionState::Disconnected,
        });
        result
    }

    fn ensure_session(&self, session: &mut Session) -> Result<Connection, String> {
        if session.stopping {
            return Err("后台正在退出。".into());
        }
        if let Some(child) = session.child.as_mut() {
            if child.try_wait().map_err(|e| e.to_string())?.is_some() {
                session.child = None;
            }
        }
        if let Some(connection) = &session.connection {
            if self.health(connection).is_ok() {
                return Ok(connection.clone());
            }
        }
        if let Ok(connection) = self.read_connection() {
            if self.health(&connection).is_ok() {
                session.connection = Some(connection.clone());
                return Ok(connection);
            }
        }
        if self.external.is_some() {
            let connection = self.read_connection()?;
            self.health(&connection)?;
            session.connection = Some(connection.clone());
            return Ok(connection);
        }
        fs::create_dir_all(&self.runtime).map_err(|e| format!("无法创建后台数据目录：{e}"))?;
        // Only the server holds this lock for its entire lifetime. A locked but unhealthy
        // server must not be replaced or killed; wait for startup or let the user retry.
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.runtime.join("connection.lock"))
            .map_err(|e| format!("无法访问后台锁：{e}"))?;
        let can_start = lock.try_lock().is_ok();
        drop(lock);
        if let Some(child) = session.child.as_mut() {
            if child.try_wait().map_err(|e| e.to_string())?.is_some() {
                session.child = None;
            }
        }
        if can_start && session.child.is_none() {
            session.child = Some(process::launch(&self.server, &self.runtime)?);
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if let Ok(connection) = self.read_connection() {
                if self.health(&connection).is_ok() {
                    session.connection = Some(connection.clone());
                    return Ok(connection);
                }
            }
            if let Some(child) = session.child.as_mut() {
                if child.try_wait().map_err(|e| e.to_string())?.is_some() {
                    return Err(format!(
                        "后台启动失败，请查看日志：{}",
                        self.runtime.join("server.log").display()
                    ));
                }
            }
            thread::sleep(Duration::from_millis(100));
        }
        Err(format!(
            "后台启动超时。请重试，或查看日志：{}",
            self.runtime.join("server.log").display()
        ))
    }

    /// Polling performs HTTP without holding either lifecycle or published-state locks.
    pub fn status(&self) -> BackendStatus {
        let state = self.state.lock().unwrap().clone();
        let ConnectionState::Connected(connection) = state else {
            return state.status();
        };
        let result = self.client.tasks(&connection);
        let current = self.state.lock().unwrap();
        if !matches!(&*current, ConnectionState::Connected(next) if next == &connection) {
            return current.status();
        }
        match result {
            Ok(tasks) => match tasks.first() {
                Some(task) => match task.status.as_str() {
                    "running" => BackendStatus::Scanning(task.files),
                    "cancelling" => BackendStatus::Cancelling,
                    "completed" => BackendStatus::Completed(task.files),
                    "failed" => BackendStatus::Failed,
                    "cancelled" => BackendStatus::Cancelled,
                    _ => BackendStatus::Unknown,
                },
                None => BackendStatus::Ready,
            },
            Err(_) => BackendStatus::Disconnected,
        }
    }

    pub fn shutdown(&self, cancel_active: bool) -> Result<(), ExitError> {
        let mut session = self.operation.lock().unwrap();
        // A failed connection attempt never grants ownership of an unknown server.
        if session.connection.is_none() && session.child.is_none() {
            session.stopping = true;
            self.publish(ConnectionState::Stopping);
            return Ok(());
        }
        let stopped = if let Some(child) = session.child.as_mut() {
            child
                .try_wait()
                .map_err(|e| ExitError::Failed(e.to_string()))?
                .is_some()
        } else {
            !self.connection_path().exists()
                || (self.external.is_none()
                    && OpenOptions::new()
                        .read(true)
                        .write(true)
                        .open(self.runtime.join("connection.lock"))
                        .is_ok_and(|lock| lock.try_lock().is_ok()))
        };
        if stopped {
            // Also prevents a concurrent startup request from launching a child after quit.
            session.stopping = true;
            self.publish(ConnectionState::Stopping);
            return Ok(());
        }
        let connection = session
            .connection
            .clone()
            .or_else(|| self.read_connection().ok());
        let Some(connection) = connection else {
            if session
                .child
                .as_mut()
                .is_some_and(|child| child.try_wait().ok().flatten().is_none())
            {
                return Err(ExitError::Failed(
                    "后台尚未就绪，暂时无法正常停止，请稍后重试。".into(),
                ));
            }
            session.stopping = true;
            self.publish(ConnectionState::Stopping);
            return Ok(());
        };
        if !session.stopping {
            match self.client.shutdown(&connection, cancel_active)? {
                ShutdownResult::Accepted => {
                    session.stopping = true;
                    self.publish(ConnectionState::Stopping);
                }
                ShutdownResult::Unreachable => {
                    if session
                        .child
                        .as_mut()
                        .is_some_and(|child| child.try_wait().ok().flatten().is_none())
                    {
                        return Err(ExitError::Failed(
                            "后台进程仍在运行，但暂时无法连接，请稍后重试。".into(),
                        ));
                    }
                    session.stopping = true;
                    self.publish(ConnectionState::Stopping);
                    return Ok(());
                }
            }
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if let Some(child) = session.child.as_mut() {
                if child
                    .try_wait()
                    .map_err(|e| ExitError::Failed(e.to_string()))?
                    .is_some()
                {
                    return Ok(());
                }
            } else {
                // The server removes discovery information only after requests have
                // drained. TCP refusal can instead appear as a timeout on Windows.
                if !self.connection_path().exists() {
                    return Ok(());
                }
                if self.client.gone(&connection) {
                    return Ok(());
                }
            }
            thread::sleep(Duration::from_millis(100));
        }
        Err(ExitError::Failed(
            "正在等待扫描工作线程退出，请稍后再次选择“退出 Orion”。".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quit_before_startup_prevents_a_late_server_launch() {
        let directory = tempfile::tempdir().unwrap();
        let backend = Backend::new(
            directory.path().into(),
            directory.path().join("missing-server"),
            None,
        )
        .unwrap();
        backend.shutdown(false).unwrap();
        assert!(backend.ensure().err().unwrap().contains("正在退出"));
        assert!(!directory.path().join("connection.lock").exists());
    }

    #[test]
    fn status_does_not_wait_for_a_lifecycle_operation() {
        let directory = tempfile::tempdir().unwrap();
        let backend = std::sync::Arc::new(
            Backend::new(
                directory.path().into(),
                directory.path().join("missing"),
                None,
            )
            .unwrap(),
        );
        let operation = backend.operation.lock().unwrap();
        backend.publish(ConnectionState::Starting);
        let poller = backend.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker = thread::spawn(move || sender.send(poller.status()).unwrap());
        let result = receiver.recv_timeout(Duration::from_secs(1));
        drop(operation);
        worker.join().unwrap();
        assert_eq!(result.unwrap(), BackendStatus::Starting);
    }
}
