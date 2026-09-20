use reqwest::{blocking::Client, StatusCode, Url};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Mutex,
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Deserialize, Serialize)]
pub struct Connection {
    pub url: String,
    pub token: String,
    pub instance_id: String,
}

#[derive(Deserialize)]
struct Health {
    name: String,
    api_version: u32,
    instance_id: String,
    #[serde(default)]
    capabilities: Vec<String>,
}

#[derive(Deserialize)]
struct Task {
    status: String,
    files: u64,
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
    client: Client,
    session: Mutex<Session>,
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
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(2))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            runtime,
            server,
            external,
            client,
            session: Mutex::new(Session::default()),
        })
    }

    fn connection_path(&self) -> PathBuf {
        self.external
            .clone()
            .unwrap_or_else(|| self.runtime.join("connection.json"))
    }

    fn read_connection(&self) -> Result<Connection, String> {
        let bytes =
            fs::read(self.connection_path()).map_err(|_| "未找到后台连接信息。".to_string())?;
        let mut connection: Connection =
            serde_json::from_slice(&bytes).map_err(|_| "后台连接信息格式无效。".to_string())?;
        connection.url = validate_url(&connection.url)?;
        if connection.token.trim().is_empty() || connection.instance_id.is_empty() {
            return Err("后台连接信息不完整。".into());
        }
        Ok(connection)
    }

    fn health(&self, connection: &Connection) -> Result<Health, String> {
        let response = self
            .client
            .get(format!("{}/api/v1/health", connection.url))
            .bearer_auth(&connection.token)
            .send()
            .map_err(|_| "后台未响应。".to_string())?;
        let health: Health = response
            .error_for_status()
            .map_err(|_| "后台认证失败，请重新连接。".to_string())?
            .json()
            .map_err(|_| "后台返回了无效的状态。".to_string())?;
        if health.name != "orion-server"
            || health.api_version != 1
            || health.instance_id != connection.instance_id
            || !health.capabilities.iter().any(|c| c == "graceful_shutdown")
        {
            return Err("后台版本或实例不匹配，请退出旧版后台后重试。".into());
        }
        Ok(health)
    }

    pub fn ensure(&self) -> Result<Connection, String> {
        let mut session = self.session.lock().unwrap();
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
            if !self.server.is_file() {
                return Err(
                    "缺少 orion-server.exe。请将它与桌面程序放在同一目录；开发时先编译后端。"
                        .into(),
                );
            }
            let log = open_log(&self.runtime)?;
            let error_log = log.try_clone().map_err(|e| e.to_string())?;
            let mut command = Command::new(&self.server);
            command
                .env("ORION_RUNTIME_DIR", &self.runtime)
                .env(
                    "ORION_CONNECTION_FILE",
                    self.runtime.join("connection.json"),
                )
                .env("ORION_PORT", "0")
                .current_dir(&self.runtime)
                .stdin(Stdio::null())
                .stdout(log)
                .stderr(error_log);
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                command.creation_flags(0x08000000); // CREATE_NO_WINDOW
            }
            session.child = Some(command.spawn().map_err(|e| format!("无法启动后台：{e}"))?);
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

    /// Polling never restarts a crashed server or a server that is intentionally exiting.
    pub fn status(&self) -> String {
        let session = self.session.lock().unwrap();
        if session.stopping {
            return "正在停止后台…".into();
        }
        let Some(connection) = &session.connection else {
            return "后台尚未连接".into();
        };
        let result = self
            .client
            .get(format!("{}/api/v1/tasks", connection.url))
            .bearer_auth(&connection.token)
            .send()
            .and_then(|r| r.error_for_status())
            .and_then(|r| r.json::<Vec<Task>>());
        match result {
            Ok(tasks) => match tasks.first() {
                Some(task) if task.status == "running" => {
                    format!("正在扫描 · {} 个文件", task.files)
                }
                Some(task) if task.status == "cancelling" => "正在停止扫描…".into(),
                Some(task) if task.status == "completed" => {
                    format!("扫描完成 · {} 个文件", task.files)
                }
                Some(task) if task.status == "failed" => "扫描失败 · 打开窗口查看".into(),
                Some(_) => "扫描已取消".into(),
                None => "后台就绪 · 等待扫描".into(),
            },
            Err(_) => "后台已断开 · 打开窗口重连".into(),
        }
    }

    pub fn shutdown(&self, cancel_active: bool) -> Result<(), ExitError> {
        let mut session = self.session.lock().unwrap();
        // A failed connection attempt never grants ownership of an unknown server.
        if session.connection.is_none() && session.child.is_none() {
            session.stopping = true;
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
            return Ok(());
        };
        if !session.stopping {
            match self.client.post(format!("{}/api/v1/server/shutdown", connection.url))
                .bearer_auth(&connection.token)
                .json(&serde_json::json!({"instance_id": connection.instance_id, "cancel_active": cancel_active}))
                .send() {
                Ok(response) if response.status() == StatusCode::ACCEPTED => { session.stopping = true; }
                Ok(response) if response.status() == StatusCode::CONFLICT => {
                    let body: serde_json::Value = response.json().unwrap_or_default();
                    if body["code"] == "scan_in_progress" { return Err(ExitError::ActiveScan); }
                    return Err(ExitError::Failed("后台实例已变化，请重新连接后退出。".into()));
                }
                Ok(_) => return Err(ExitError::Failed("后台拒绝退出请求，请确认版本和连接状态。".into())),
                Err(error) if error.is_connect() => {
                    if session.child.as_mut().is_some_and(|child| child.try_wait().ok().flatten().is_none()) {
                        return Err(ExitError::Failed("后台进程仍在运行，但暂时无法连接，请稍后重试。".into()));
                    }
                    session.stopping = true;
                    return Ok(());
                }
                Err(_) => return Err(ExitError::Failed("退出请求未得到确认，请稍后重试。".into())),
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
                match self
                    .client
                    .get(format!("{}/api/v1/health", connection.url))
                    .bearer_auth(&connection.token)
                    .send()
                {
                    Err(error) if error.is_connect() => return Ok(()),
                    Ok(response) if response.status() == StatusCode::UNAUTHORIZED => return Ok(()),
                    _ => {}
                }
            }
            thread::sleep(Duration::from_millis(100));
        }
        Err(ExitError::Failed(
            "正在等待扫描工作线程退出，请稍后再次选择“退出 Orion”。".into(),
        ))
    }
}

fn validate_url(input: &str) -> Result<String, String> {
    let url = Url::parse(input).map_err(|_| "后台地址无效。".to_string())?;
    if url.scheme() != "http"
        || url.host_str() != Some("127.0.0.1")
        || url.port().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("后台地址必须是本机回环地址。".into());
    }
    Ok(url.origin().ascii_serialization())
}

fn open_log(runtime: &Path) -> Result<File, String> {
    let path = runtime.join("server.log");
    if fs::metadata(&path).is_ok_and(|m| m.len() > 2 * 1024 * 1024) {
        let previous = runtime.join("server.previous.log");
        if previous.exists() {
            fs::remove_file(&previous).map_err(|e| e.to_string())?;
        }
        fs::rename(&path, &previous).map_err(|e| e.to_string())?;
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("无法打开后台日志：{e}"))
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
    fn connection_urls_are_loopback_only_and_cannot_redirect_credentials() {
        assert_eq!(
            validate_url("http://127.0.0.1:43120/").unwrap(),
            "http://127.0.0.1:43120"
        );
        for value in [
            "https://example.com",
            "http://127.0.0.1.evil:80",
            "http://user@127.0.0.1:43120",
            "http://127.0.0.1:43120/?token=x",
            "http://127.0.0.1:43120/path",
        ] {
            assert!(validate_url(value).is_err(), "{value}");
        }
    }
}
