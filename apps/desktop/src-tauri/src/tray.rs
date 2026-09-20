use orion_runtime::BackendStatus;

pub fn status_text(status: BackendStatus) -> String {
    match status {
        BackendStatus::Starting => "正在启动后台…".into(),
        BackendStatus::NotConnected => "后台尚未连接".into(),
        BackendStatus::Ready => "后台就绪 · 等待扫描".into(),
        BackendStatus::Scanning(files) => format!("正在扫描 · {files} 个文件"),
        BackendStatus::Cancelling => "正在停止扫描…".into(),
        BackendStatus::Completed(files) => format!("扫描完成 · {files} 个文件"),
        BackendStatus::Failed => "扫描失败 · 打开窗口查看".into(),
        BackendStatus::Cancelled => "扫描已取消".into(),
        BackendStatus::Stopping => "正在停止后台…".into(),
        BackendStatus::Disconnected => "后台已断开 · 打开窗口重连".into(),
        BackendStatus::Unknown => "未知任务状态 · 打开窗口查看".into(),
    }
}
