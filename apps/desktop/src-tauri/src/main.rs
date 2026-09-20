#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[derive(serde::Serialize, serde::Deserialize)]
struct Connection {
    url: String,
    token: String,
}

// Desktop integration only. Scanning and task state live in orion-server.
#[tauri::command]
fn server_connection() -> Result<Connection, String> {
    let path = std::env::var_os("ORION_CONNECTION_FILE")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            let current = std::env::current_dir().ok()?;
            current
                .ancestors()
                .take(4)
                .map(|dir| dir.join(".orion/connection-43120.json"))
                .find(|path| path.is_file())
        })
        .ok_or("未找到本地服务连接文件，请先在项目根目录运行 cargo run -p orion-server。")?;
    let content =
        std::fs::read(path).map_err(|_| "无法读取服务连接文件，请确认 Server 已启动。")?;
    serde_json::from_slice(&content).map_err(|_| "服务连接文件格式无效。".into())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![server_connection])
        .run(tauri::generate_context!())
        .expect("failed to run Orion desktop");
}
