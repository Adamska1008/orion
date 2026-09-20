# Orion

Windows 优先的只读磁盘空间浏览器。独立 Rust Server 负责扫描与查询，Tauri / React 客户端负责桌面交互。

## 开发运行

需要 Rust / Cargo 1.92+、Node.js 22.12+（当前验证使用 24.12）、npm、Windows MSVC 构建工具和 WebView2。

首次在项目根目录安装前端依赖：

```powershell
npm ci
```

在项目根目录打开两个终端。终端一运行后端，保持终端打开：

```powershell
cargo run -p orion-server
```

终端二运行桌面客户端：

```powershell
npm run desktop
```

选择目录后即可扫描、按大小浏览、进入子目录、查看详情或取消。关闭 GUI 不停止后端；在后端终端按 Ctrl+C 停止服务。使用普通开发命令，不提供自动后台启动、停止进程或修改系统启动项的脚本。

服务默认只监听 `127.0.0.1:43120`，随机令牌保存在项目 `.orion/connection-43120.json`，该目录限制当前 Windows 用户访问且已被 Git 忽略。GUI 从工作目录及其上级查找连接文件。端口占用时明确报错，不结束占用它的进程。可通过 `ORION_PORT`、`ORION_RUNTIME_DIR`、`ORION_CONNECTION_FILE` 手动覆盖开发配置；连接文件不要提交或分享。

只看 Web 界面可运行 `npm run dev`，再在页面连接设置中填写本地连接文件的地址和令牌。浏览器预览需要手动输入目录路径；系统目录对话框与文件定位需在桌面客户端使用。

侧栏底部的“外观”支持跟随系统、浅色和暗色，默认跟随系统。选择会保存在当前客户端，重启后恢复；跟随系统时会随系统主题实时切换。

## 常规检查

```powershell
cargo test -p orion-core -p orion-server
cargo fmt --all --check
cargo clippy -p orion-core -p orion-server --all-targets -- -D warnings
npm test
npm run build
```

生成包含前端资源的本机调试程序：先 `npm run build`，再 `cargo build -p orion-desktop --features custom-protocol`。开发热更新使用前述 `npm run desktop`。

## Release 试用

在项目根目录运行 `npm run build`，然后执行：

```powershell
cargo build --locked --release -p orion-server -p orion-desktop --features orion-desktop/custom-protocol
```

产物是 `target/release/orion-server.exe` 和 `target/release/orion-desktop.exe`。两者均为 Release；界面资源已嵌入，不需要 Vite 开发服务。

测试前先结束旧扫描并关闭旧客户端，在旧 Server 的终端按 Ctrl+C 停止服务。在项目根目录的两个终端分别运行：

```powershell
.\target\release\orion-server.exe
```

```powershell
.\target\release\orion-desktop.exe
```

这份 Release 保留当前扫描算法，用于建立性能基线。多次扫描会受 Windows 文件系统缓存和同时运行的程序影响，比较时记录扫描范围、耗时、文件数和异常数量；不要仅凭一次先 Debug 后 Release 的结果归因。

## 当前边界

- 只读扫描，不删除或修改被扫描文件。一次运行一个扫描任务，当前只保留最近一次扫描；开始新扫描会替换旧结果。服务退出后结果丢失。
- 大小是各目录项的文件逻辑长度之和，硬链接会按目录项重复计数；实际分配空间未知。不是卷已用空间、可释放空间或一致的文件系统快照。
- 不跟随符号链接和 Windows 重解析点，跳过项与访问错误使结果标为不完整，最多返回前 100 条错误明细。
- 普通路径枚举会复查目录与链接，但不能在其他进程恶意并发替换路径时提供严格隔离保证。文件增删改动需要重新扫描。
- 目录列表按大小降序，每页 200 项并虚拟化渲染。扫描时每次查询是独立版本，跨页排序可能变化；GUI 不拼接不同版本的分页结果。
- 未实现持久化、清理、MFT / USN、完整 CLI、MCP 或 Skill。

扫描性能对比使用标准 `cargo bench`，方法、固定数据集与真实目录用法见 [Benchmark](docs/BENCHMARK.md)。

## 项目结构

- `crates/orion-core`：目录枚举、索引、统计、取消与部分结果。
- `crates/orion-server`：本地 API、认证、共享任务和重复请求处理。
- `apps/desktop`：React UI 与 Tauri 桌面集成。
- [Product Spec](docs/PRODUCT_SPEC.md)、[技术选型](docs/TECH_STACK.md)、[MVP 任务计划](docs/MVP_TASK_PLAN.md)、[API 说明](docs/API.md)。
