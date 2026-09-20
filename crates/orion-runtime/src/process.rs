use std::{
    fs::{self, File, OpenOptions},
    path::Path,
    process::{Child, Command, Stdio},
};

pub(crate) fn launch(server: &Path, runtime: &Path) -> Result<Child, String> {
    if !server.is_file() {
        return Err(
            "缺少 orion-server.exe。请将它与桌面程序放在同一目录；开发时先编译后端。".into(),
        );
    }
    let log = open_log(runtime)?;
    let error_log = log.try_clone().map_err(|e| e.to_string())?;
    let mut command = Command::new(server);
    command
        .env("ORION_RUNTIME_DIR", runtime)
        .env("ORION_CONNECTION_FILE", runtime.join("connection.json"))
        .env("ORION_PORT", "0")
        .current_dir(runtime)
        .stdin(Stdio::null())
        .stdout(log)
        .stderr(error_log);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    command.spawn().map_err(|e| format!("无法启动后台：{e}"))
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
