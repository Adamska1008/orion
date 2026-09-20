# Orion

Windows 优先的只读磁盘空间浏览器。独立 Rust Server 负责扫描与查询，Tauri / React 客户端负责桌面交互。

## 开发运行

需要 Rust / Cargo 1.92+、Node.js 22.12+（当前验证使用 24.12）、npm、Windows MSVC 构建工具和 WebView2。

首次在项目根目录安装前端依赖：

```powershell
npm ci
```

在项目根目录先编译后端，再启动桌面开发模式：

```powershell
cargo build -p orion-server
```

桌面会自动启动同目录下的后端程序：

```powershell
npm run desktop
```

选择目录后即可扫描、按大小浏览、进入子目录、查看详情或取消。点击窗口 X 会隐藏到系统托盘，扫描继续；单击托盘图标或再次打开应用会恢复窗口。右键托盘选择“退出 Orion”才会关闭后台；扫描中会确认是否停止扫描并退出。托盘菜单和提示显示后台状态。

桌面管理的后台只监听 `127.0.0.1`，使用系统分配的空闲端口。连接信息和日志放在 `%LOCALAPPDATA%\dev.orion.desktop\runtime`（`connection.json` / `server.log`），凭据目录限制当前 Windows 用户访问。桌面异常退出后，再打开会重连存活的后台；后台异常退出后，可在连接设置中点击“重新连接后台”。连接文件不要提交或分享。

独立调试后端仍可运行 `cargo run -p orion-server`，默认端口为 43120，连接文件为项目 `.orion/connection-43120.json`，在其终端按 Ctrl+C 停止。GUI 默认管理自己的后台；如需连接手动启动的 Server，在启动 GUI 的终端设置 `ORION_CONNECTION_FILE` 为该连接文件的绝对路径。此模式只连接已有后台，托盘退出也会停止它。Server 必须使用支持退出 API 的新版。

`ORION_RUNTIME_DIR` 可隔离开发数据目录，`ORION_SCAN_WORKERS` 配置扫描线程；独立 Server 还支持 `ORION_PORT`。Server 设置 `ORION_CONNECTION_FILE` 时，该文件必须在 `ORION_RUNTIME_DIR` 内。没有自定义启动脚本、开机启动项或 Windows 服务。开发热重载可能重连现有后台；修改后端代码后，需先从托盘退出、重新编译，再启动桌面。

扫描默认使用最多 4 个目录工作线程（不超过可用 CPU 并行度），每批最多 256 个条目更新索引。可在启动 Server 前设置 `ORION_SCAN_WORKERS` 为 1–16；例如 `$env:ORION_SCAN_WORKERS = '8'`，重启 Server 后生效，启动输出会显示实际配置。设置为 1 仍保留批量更新，适合比较并发本身的收益。

只看 Web 界面可运行 `npm run dev`，再在页面连接设置中填写本地连接文件的地址和令牌。浏览器预览需要手动输入目录路径；系统目录对话框与文件定位需在桌面客户端使用。

顶部的外观选择支持跟随系统、浅色和暗色，默认跟随系统。选择会保存在当前客户端，重启后恢复；跟随系统时会随系统主题实时切换。

Windows 桌面客户端支持 `Ctrl+-` 缩小、`Ctrl++`（或 `Ctrl+=`）放大、`Ctrl+0` 恢复默认缩放，也支持 `Ctrl+滚轮` 调整界面大小。正文默认约 14px，辅助信息约 11–13px。

## 常规检查

```powershell
cargo test -p orion-core -p orion-server -p orion-desktop
cargo fmt --all --check
cargo clippy -p orion-core -p orion-server -p orion-desktop --all-targets -- -D warnings
npm test
npm run build
```

生成包含前端资源的本机调试程序：先 `npm run build`，再 `cargo build -p orion-server -p orion-desktop --features orion-desktop/custom-protocol`。开发热更新使用前述 `npm run desktop`。生命周期集成测试使用隔离临时目录和临时端口，不操作正在使用的 Orion。

## Release 试用

在项目根目录运行 `npm run build`，然后执行：

```powershell
cargo build --locked --release -p orion-server -p orion-desktop --features orion-desktop/custom-protocol
```

产物是 `target/release/orion-server.exe` 和 `target/release/orion-desktop.exe`。两者放在同一目录，双击 `orion-desktop.exe` 即可，后端会自动启动且不弹出控制台。界面资源已嵌入，不需要 Vite 开发服务；目前是便携版，仍需系统具备 WebView2。

升级前先从托盘退出旧版；没有托盘的旧版需关闭客户端，并在旧 Server 终端按 Ctrl+C。然后双击新版桌面程序，或运行：

```powershell
.\target\release\orion-desktop.exe
```

当前扫描已默认复用 Windows 目录枚举提供的 metadata，减少逐文件路径查询。重新构建并重启 Server 后生效，已运行的旧进程不会自动更新。多次扫描会受 Windows 文件系统缓存和同时运行的程序影响，比较时记录扫描范围、耗时、文件数和异常数量。

## 当前边界

- 只读扫描，不删除或修改被扫描文件。一次运行一个扫描任务，当前只保留最近一次扫描；开始新扫描会替换旧结果。服务退出后结果丢失。
- 大小是各目录项的文件逻辑长度之和，硬链接会按目录项重复计数；实际分配空间未知。不是卷已用空间、可释放空间或一致的文件系统快照。
- 大小和修改时间取自枚举结果；NTFS 硬链接的部分名称可能保留旧值，汇总也可能因此滞后。详情目前读取同一内存索引，不会另做即时查询；重扫也不能保证枚举缓存已更新。
- 不跟随符号链接和 Windows 重解析点，跳过项与访问错误使结果标为不完整。错误总数完整累计，明细按路径/代码稳定选择最多 100 条。
- 普通路径枚举会复查目录与链接，但不能在其他进程恶意并发替换路径时提供严格隔离保证。文件增删改动需要重新扫描。
- 目录列表按大小降序，默认每页 15 项，可切换为 50 / 100 项，支持上一页、下一页和页码跳转；每页在固定高度列表内虚拟化渲染。扫描时每次查询是独立版本，跨页排序可能变化；GUI 不拼接不同版本的分页结果。
- 未实现持久化、清理、MFT / USN、完整 CLI、MCP 或 Skill。

扫描性能对比使用标准 `cargo bench`，方法、固定数据集与真实目录用法见 [Benchmark](docs/BENCHMARK.md)。

## 项目结构

- `crates/orion-core`：目录枚举、索引、统计、取消与部分结果。
- `crates/orion-server`：本地 API、认证、共享任务和重复请求处理。
- `apps/desktop`：React UI 与 Tauri 桌面集成。
- [Product Spec](docs/PRODUCT_SPEC.md)、[技术选型](docs/TECH_STACK.md)、[MVP 任务计划](docs/MVP_TASK_PLAN.md)、[API 说明](docs/API.md)。
