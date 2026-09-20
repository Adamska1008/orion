// Exercise the desktop's real process manager against Cargo's freshly built server,
// without opening a Tauri window or touching the user's running Orion instance.
#[path = "../../../apps/desktop/src-tauri/src/backend.rs"]
mod backend;

use backend::{Backend, Connection};
use reqwest::blocking::Client;
use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

struct ManagedBackend(Backend);

impl Drop for ManagedBackend {
    fn drop(&mut self) {
        let _ = self.0.shutdown(true);
    }
}

fn manager(runtime: &Path) -> ManagedBackend {
    ManagedBackend(
        Backend::new(
            runtime.into(),
            env!("CARGO_BIN_EXE_orion-server").into(),
            None,
        )
        .unwrap(),
    )
}

fn client() -> Client {
    Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap()
}

#[test]
fn reconnect_after_server_crash_replaces_stale_discovery() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("connection.json");
    let mut command = Command::new(env!("CARGO_BIN_EXE_orion-server"));
    command
        .env("ORION_PORT", "0")
        .env("ORION_RUNTIME_DIR", directory.path())
        .env("ORION_CONNECTION_FILE", &path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    struct TestProcess(std::process::Child);
    impl Drop for TestProcess {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut process = TestProcess(command.spawn().unwrap());
    let deadline = Instant::now() + Duration::from_secs(10);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "test server did not publish discovery"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    let desktop = manager(directory.path());
    let previous = desktop.0.ensure().unwrap();
    let quitting = manager(directory.path());
    quitting.0.ensure().unwrap();
    process.0.kill().unwrap();
    process.0.wait().unwrap();
    assert!(path.exists(), "crash fixture should leave stale discovery");
    // A desktop that only attached to the crashed process must still be able to quit.
    quitting.0.shutdown(false).unwrap();
    let restarted = desktop.0.ensure().unwrap();
    assert_ne!(previous.instance_id, restarted.instance_id);
    desktop.0.shutdown(false).unwrap();
}

#[test]
fn desktop_starts_reuses_and_stops_server_then_restarts_cleanly() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = directory.path().join("runtime with spaces");
    fs::create_dir(&runtime).unwrap();
    // A stale discovery file from an interrupted process must be replaced atomically.
    fs::write(runtime.join("connection.json"), b"stale").unwrap();
    let owner = manager(&runtime);
    let first = owner.0.ensure().unwrap();
    assert!(owner.0.status().contains("就绪"));
    assert_eq!(owner.0.ensure().unwrap().instance_id, first.instance_id);

    let reopened = manager(&runtime);
    assert_eq!(reopened.0.ensure().unwrap().instance_id, first.instance_id);
    let client = client();
    let root = directory.path().join("scan");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("data"), [0; 37]).unwrap();
    client
        .post(format!("{}/api/v1/scans", first.url))
        .bearer_auth(&first.token)
        .json(&serde_json::json!({"root":root,"request_id":uuid::Uuid::new_v4()}))
        .send()
        .unwrap()
        .error_for_status()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let tasks: serde_json::Value = client
            .get(format!("{}/api/v1/tasks", first.url))
            .bearer_auth(&first.token)
            .send()
            .unwrap()
            .json()
            .unwrap();
        if tasks[0]["status"] == "completed" {
            assert_eq!(tasks[0]["logical_bytes"], 37);
            break;
        }
        assert!(Instant::now() < deadline, "fixture scan timed out");
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(reopened.0.status().contains("扫描完成"));
    reopened.0.shutdown(false).unwrap();
    assert!(!runtime.join("connection.json").exists());
    assert!(
        reopened.0.ensure().is_err(),
        "an exiting desktop must not restart its server"
    );

    let next = owner.0.ensure().unwrap();
    assert_ne!(next.instance_id, first.instance_id);
    assert!(next.token != first.token);
    owner.0.shutdown(false).unwrap();
    assert!(!runtime.join("connection.json").exists());
}

#[test]
fn duplicate_server_cannot_replace_live_connection_credentials() {
    let directory = tempfile::tempdir().unwrap();
    let owner = manager(directory.path());
    let connection = owner.0.ensure().unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_orion-server"));
    command
        .env("ORION_PORT", "0")
        .env("ORION_RUNTIME_DIR", directory.path())
        .env(
            "ORION_CONNECTION_FILE",
            directory.path().join("connection.json"),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut duplicate = command.spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = duplicate.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = duplicate.kill();
            let _ = duplicate.wait();
            panic!("duplicate server failed to reject the occupied lock");
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    assert!(!status.success());
    let saved: Connection =
        serde_json::from_slice(&fs::read(directory.path().join("connection.json")).unwrap())
            .unwrap();
    assert_eq!(saved.instance_id, connection.instance_id);
    // Assert only the boolean so a failed assertion cannot print a credential.
    assert!(saved.token == connection.token);
    assert!(owner.0.status().contains("就绪"));
    owner.0.shutdown(false).unwrap();
}
