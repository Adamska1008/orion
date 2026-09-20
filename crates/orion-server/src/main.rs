use std::{fs, path::Path, process::Command};

fn private_directory(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(path)?;
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Grant the current Windows SID access before writing a bearer credential.
        let user = Command::new("whoami.exe")
            .args(["/user", "/fo", "csv", "/nh"])
            .creation_flags(0x08000000)
            .output()?;
        let text = String::from_utf8_lossy(&user.stdout);
        let sid = text
            .split(',')
            .nth(1)
            .ok_or("cannot discover current user SID")?
            .trim()
            .trim_matches('"');
        if !sid.starts_with("S-1-") {
            return Err("invalid current user SID".into());
        }
        let status = Command::new("icacls.exe")
            .arg(path)
            .args(["/inheritance:r", "/grant:r", &format!("*{sid}:(OI)(CI)F")])
            .creation_flags(0x08000000)
            .output()?;
        if !status.status.success() {
            return Err("cannot restrict connection directory permissions".into());
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let port = std::env::var("ORION_PORT")
        .unwrap_or_else(|_| "43120".into())
        .parse::<u16>()?;
    let scan_workers = match std::env::var("ORION_SCAN_WORKERS") {
        Ok(value) => value.parse::<usize>()?,
        Err(std::env::VarError::NotPresent) => orion_core::default_scan_workers(),
        Err(error) => return Err(error.into()),
    };
    if !(1..=orion_core::MAX_SCAN_WORKERS).contains(&scan_workers) {
        return Err("ORION_SCAN_WORKERS must be between 1 and 16".into());
    }
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let state = orion_server::AppState::new(token.clone()).with_scan_workers(scan_workers);
    let directory = std::env::var_os("ORION_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| ".orion".into());
    private_directory(&directory)?;
    // Managed desktops use one stable discovery file even with a dynamically selected port.
    let managed_connection =
        std::env::var_os("ORION_CONNECTION_FILE").map(std::path::PathBuf::from);
    let _instance_lock = if let Some(path) = &managed_connection {
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        if fs::canonicalize(parent)? != fs::canonicalize(&directory)? {
            return Err("ORION_CONNECTION_FILE must be inside ORION_RUNTIME_DIR".into());
        }
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path.with_extension("lock"))?;
        file.try_lock()
            .map_err(|_| "another server owns this connection file")?;
        Some(file)
    } else {
        None
    };
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    let url = format!("http://{}", listener.local_addr()?);
    let connection = managed_connection.unwrap_or_else(|| {
        directory.join(format!(
            "connection-{}.json",
            listener.local_addr().unwrap().port()
        ))
    });
    let pending = connection.with_extension("pending");
    fs::write(
        &pending,
        serde_json::to_vec(
            &serde_json::json!({"url":url,"token":token,"instance_id":state.instance_id}),
        )?,
    )?;
    fs::rename(&pending, &connection)?;
    println!("Orion Server listening at {url}");
    println!("Scan workers: {scan_workers}");
    println!(
        "Connection file: {}",
        fs::canonicalize(&connection)?.display()
    );
    let shutdown_state = state.clone();
    let result = axum::serve(listener, orion_server::router(state))
        .with_graceful_shutdown(async move {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => { let _ = shutdown_state.begin_shutdown(true); }
                _ = shutdown_state.shutdown_requested() => {}
            }
            shutdown_state.wait_for_scan_exit().await;
        })
        .await;
    let _ = fs::remove_file(connection);
    result?;
    Ok(())
}
