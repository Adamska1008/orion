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
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let state = orion_server::AppState::new(token.clone());
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    let url = format!("http://{}", listener.local_addr()?);
    // Each instance gets its own credential file; concurrent servers cannot overwrite it.
    let directory = std::env::var_os("ORION_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| ".orion".into());
    private_directory(&directory)?;
    let connection = directory.join(format!("connection-{}.json", listener.local_addr()?.port()));
    fs::write(
        &connection,
        serde_json::to_vec(
            &serde_json::json!({"url":url,"token":token,"instance_id":state.instance_id}),
        )?,
    )?;
    println!("Orion Server listening at {url}");
    println!(
        "Connection file: {}",
        fs::canonicalize(&connection)?.display()
    );
    let result = axum::serve(listener, orion_server::router(state))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await;
    let _ = fs::remove_file(connection);
    result?;
    Ok(())
}
