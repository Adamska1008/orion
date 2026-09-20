#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod tray;

use orion_runtime::{Backend, Connection, ExitError};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager,
};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

struct Desktop {
    backend: Arc<Backend>,
    quitting: AtomicBool,
}

fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

// Desktop integration only. Scanning and task state live in orion-server.
#[tauri::command]
async fn server_connection(app: AppHandle) -> Result<Connection, String> {
    let backend = app.state::<Desktop>().backend.clone();
    tauri::async_runtime::spawn_blocking(move || backend.ensure())
        .await
        .map_err(|_| "后台连接任务意外退出。".to_string())?
}

fn quit(app: &AppHandle) {
    let state = app.state::<Desktop>();
    if state.quitting.swap(true, Ordering::SeqCst) {
        return;
    }
    let backend = state.backend.clone();
    let app = app.clone();
    // Native dialogs and HTTP waits must not block the window/tray event loop.
    std::thread::spawn(move || {
        let mut result = backend.shutdown(false);
        if matches!(result, Err(ExitError::ActiveScan)) {
            let confirmed = app.dialog()
                .message("扫描仍在进行。退出会停止扫描并关闭后台，当前扫描结果不会保存。关闭窗口可继续在后台扫描。")
                .title("退出 Orion？")
                .kind(MessageDialogKind::Warning)
                .buttons(MessageDialogButtons::OkCancelCustom("停止扫描并退出".into(), "继续后台扫描".into()))
                .blocking_show();
            if !confirmed {
                app.state::<Desktop>()
                    .quitting
                    .store(false, Ordering::SeqCst);
                return;
            }
            result = backend.shutdown(true);
        }
        match result {
            Ok(()) => app.exit(0),
            Err(error) => {
                app.dialog()
                    .message(error.to_string())
                    .title("Orion 尚未退出")
                    .kind(MessageDialogKind::Error)
                    .blocking_show();
                app.state::<Desktop>()
                    .quitting
                    .store(false, Ordering::SeqCst);
            }
        }
    });
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            show_window(app)
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let runtime = std::env::var_os("ORION_RUNTIME_DIR")
                .map(std::path::PathBuf::from)
                .unwrap_or(app.path().app_local_data_dir()?.join("runtime"));
            let executable = std::env::current_exe()?;
            let server = executable
                .parent()
                .ok_or("cannot locate executable directory")?
                .join(format!("orion-server{}", std::env::consts::EXE_SUFFIX));
            let backend = Arc::new(
                Backend::new(
                    runtime,
                    server,
                    std::env::var_os("ORION_CONNECTION_FILE").map(Into::into),
                )
                .map_err(std::io::Error::other)?,
            );
            app.manage(Desktop {
                backend: backend.clone(),
                quitting: AtomicBool::new(false),
            });

            let open = MenuItem::with_id(app, "open", "打开 Orion", true, None::<&str>)?;
            let status = MenuItem::with_id(app, "status", "正在启动后台…", false, None::<&str>)?;
            let separator = PredefinedMenuItem::separator(app)?;
            let exit = MenuItem::with_id(app, "quit", "退出 Orion", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &status, &separator, &exit])?;
            let tray = TrayIconBuilder::with_id("orion")
                .icon(
                    app.default_window_icon()
                        .ok_or("missing tray icon")?
                        .clone(),
                )
                .tooltip("Orion · 正在启动后台…")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => show_window(app),
                    "quit" => quit(app),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if matches!(
                        event,
                        TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        }
                    ) {
                        show_window(tray.app_handle());
                    }
                })
                .build(app)?;
            std::thread::spawn(move || {
                let _ = backend.ensure();
                let mut previous = String::new();
                loop {
                    let current = tray::status_text(backend.status());
                    if current != previous {
                        let _ = status.set_text(&current);
                        let _ = tray.set_tooltip(Some(format!("Orion · {current}")));
                        previous = current;
                    }
                    std::thread::sleep(Duration::from_secs(2));
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![server_connection])
        .run(tauri::generate_context!())
        .expect("failed to run Orion desktop");
}
