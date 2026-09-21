# 调用参考

工具为 `../scripts/orion.py`，需要 Python 3.10+，无第三方依赖。以下命令均可从任意目录运行，将 `<skill-dir>` 和示例 ID 换成实际值。支持读取已运行的 Orion 本地 API v1；不启动、停止或取消后台任务，也不修改被扫描文件。

## 连接

```powershell
python "<skill-dir>/scripts/orion.py" status
python "<skill-dir>/scripts/orion.py" --connection-file "C:\OrionRuntime\connection.json" status
```

全局选项须放在子命令之前：

| 选项 | 含义 |
| --- | --- |
| `--connection-file PATH` | 显式指定连接文件，不猜测其他后台 |
| `--instance-id UUID` | 要求仍是此前输出的服务实例，防止跨实例继续工作 |
| `--timeout SECONDS` | 单次 HTTP 超时，默认 5 秒，允许 0.1–30 秒 |

未指定文件时，先使用 `ORION_CONNECTION_FILE`；未设置时依次查找 `%LOCALAPPDATA%\dev.orion.desktop\runtime\connection.json` 和当前工作目录的 `.orion/connection-43120.json`，选取第一个存在的文件。选定之后认证失败或后台不可达，不会回退到另一个文件。自定义端口或运行目录请显式指定文件。

仅接受带端口的 `http://127.0.0.1` 地址，禁用代理且不跟随重定向。每次命令先检查服务名称、API 版本和实例 ID。连接令牌不会出现在正常输出中；不要直接打印连接文件。

## 命令

```powershell
# 查看服务和已有扫描；复用返回的任务 ID。
python "<skill-dir>/scripts/orion.py" status

# 发起扫描。仅在原有结果确实可以被替换时，加 --replace-existing。
python "<skill-dir>/scripts/orion.py" scan --root "D:\资料"
python "<skill-dir>/scripts/orion.py" scan --root "D:\资料" --replace-existing

# 查询或有限等待；--seconds 默认 20，范围 1–60。
python "<skill-dir>/scripts/orion.py" task --task TASK_UUID
python "<skill-dir>/scripts/orion.py" wait --task TASK_UUID --seconds 20

# 从根目录 0 开始看概览，再用返回的目录 ID 深入。
python "<skill-dir>/scripts/orion.py" tree --task TASK_UUID --parent 0 --depth 2 --top 8
python "<skill-dir>/scripts/orion.py" entries --task TASK_UUID --parent 0 --limit 20
python "<skill-dir>/scripts/orion.py" detail --task TASK_UUID --entry ENTRY_ID
```

`status` 不发起扫描。`scan` 始终表示新建扫描请求：发起前若发现后台保留任务而未指定 `--replace-existing`，工具返回 `existing_scan` 和已有任务，供 Agent 复用或决定是否替换。这个检查不能锁定其他客户端；扫描状态仍以后台响应为准。即使带该选项，运行中的任务也由后台拒绝替换，工具不会自动取消它。

`tree` 的 `depth` 为 1–4，默认 2，查询根是第 0 层。后台最多返回 2,048 个节点，每个目录最多 64 个最大子项；工具进一步只展示每个目录前 `top` 个子项（默认 8，范围 1–64）。额外省略的直接子项也合并到 `omitted_count` / `omitted_bytes`，保留大小守恒。`zero_count` 是 0 大小条目数，`expanded: false` 表示未展开，不代表空目录。

`entries` 按大小降序返回一页，默认 `--offset 0 --limit 20`，上限 500。需拼接多页时，从第一页记录 `revision`，后续传入 `--revision`；`stale_revision` 时重新从第一页读取。扫描仍在运行时优先逐页观察，或者等任务结束再收集完整分页。

```powershell
python "<skill-dir>/scripts/orion.py" entries --task TASK_UUID --parent 0 --offset 20 --limit 20 --revision REVISION
```

## 输出与恢复

成功时 stdout 为一个 JSON 对象，退出码 0：

```json
{"ok": true, "server": {"url": "http://127.0.0.1:43120", "instance_id": "UUID"}, "data": {}}
```

`data` 对应查询结果；`status` 包含 `health` 和 `tasks`；`scan` 另含顶层 `request_id`；`wait` 的 `data` 包含 `finished` 和最后一次 `task`。`finished: false` 表示本次等待时间用尽，任务仍可继续查询；`finished: true` 也可能是取消或失败，仍需检查任务状态和覆盖信息。

运行错误为 `{"ok": false, "error": {"code": "...", "message": "..."}}`，退出码 1。参数使用错误由 argparse 输出帮助并返回 2。HTTP 错误保留状态码、服务端错误码和可能存在的 `task_id`，不自动重试或重扫。

| 情况 | 处理 |
| --- | --- |
| `connection_missing` / `unreachable` / `unauthorized` | 确认 Orion 已启动及连接文件位置；重新运行命令会重读该文件。不要输出令牌或改变系统配置 |
| `instance_changed` | 服务已换实例，丢弃旧任务/条目 ID，重新 `status` |
| `existing_scan` / `scan_in_progress` | 查看已有任务，能复用则复用；运行中的其他范围不要自行取消 |
| `stale_revision` | 分页版本已变，重新读第一页 |
| 任务不存在或结果已过期 | 先 `status`，确认是否被桌面或其他客户端替换；不要默默扩大范围重扫 |
| `treemap_unsupported` | 用 `entries` 和 `detail` 逐层查询 |

发起扫描的响应丢失时，错误输出会保留 `request_id`、`root` 和已验证的 `server`。先检查任务；确需重试时，**保持同一个服务实例、原始 root 字符串和 request_id**，例如：

```powershell
python "<skill-dir>/scripts/orion.py" --instance-id INSTANCE_UUID scan --root "D:\资料" --request-id REQUEST_UUID --replace-existing
```

同一实例内后台按请求 ID 去重。不要在未确认前生成新 ID，也不要将旧请求自动转发到重启后的新实例。大小是逻辑长度，`allocated_bytes: null` 表示未知；目录及文件的详细判断规则以 `SKILL.md` 为准。
