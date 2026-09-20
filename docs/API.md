# Orion · 本地 API v1

Server 默认监听 `http://127.0.0.1:43120`，所有接口需要 `Authorization: Bearer <token>`。连接信息写入启动输出指明的本地文件；令牌每次启动重新生成。不要把令牌写入 URL 或提交到仓库。

## 接口

| 方法 | 路径（前缀 `/api/v1`） | 行为 |
| --- | --- | --- |
| GET | `/health` | 服务名称、API 版本、`instance_id`；服务重启后实例标识变化 |
| GET | `/tasks` | 当前保留的扫描任务数组，长度为 0 或 1 |
| POST | `/scans` | 接收 `{ "root": "C:\\Data", "request_id": "UUID" }`，返回扫描任务 |
| GET | `/tasks/{id}` | 查询任务、进度、完整性与错误摘要 |
| POST | `/tasks/{id}/cancel` | 请求取消，空对象 `{}` 即可；取消已结束任务无副作用 |
| GET | `/scans/{id}/entries` | 目录直接子项，参数 `parent`、`offset`、`limit`、可选 `revision` |
| GET | `/scans/{id}/entries/{entry}` | 对象详情，包含完整路径、父项与祖先链 |

`root` 必须是现存的绝对目录路径，扫描根目录的条目标识为 `0`。目录枚举在固定数量的工作线程上运行，默认最多 4 个，可由 Server 的 `ORION_SCAN_WORKERS` 配置为 1–16；请求返回不意味着扫描已完成。

## 任务与重复请求

- 状态：`running`、`cancelling`、`cancelled`、`completed`、`failed`。`complete` 仅在枚举结束且没有跳过项或错误时为 `true`；`completed` 仍可能只有部分覆盖。
- `files` / `directories` 是已发现数量，目录数包含根目录；`logical_bytes` 是已统计文件逻辑长度。总工作量未知，不提供百分比或预计剩余时间。
- `allocated_bytes` 当前总是 `null`，表示未取得分配空间，不能解释为零。
- `started_at`、`finished_at`、`modified_at` 使用 Unix 毫秒；未知或未完成的时间为 `null`。
- 一个服务同时只运行一个扫描。冲突请求返回 HTTP 409、`scan_in_progress` 和现有 `task_id`。
- 超时重试必须复用 `request_id`。同一个键和同一个原始 `root` 字符串返回原任务；换范围返回 409。旧结果被新扫描替换后，同一个键返回 410，不再次执行。
- 只保留最近一次扫描索引。新扫描替换旧结果，但本次服务的请求标识仍保留，最多 1024 个；达到限制需要重启。客户端关闭不取消扫描；服务退出丢失所有任务与请求标识。

## 查询与完整性

列表按逻辑大小降序，同大小按名称及条目标识排序。`offset` 默认 0，`limit` 默认 200，限制为 1–500。响应包含 `revision`、`total`、`offset`、`entries`。

每个列表响应来自同一索引版本。指定的 `revision` 已过期时返回 409 `stale_revision`；需要拼接多页的客户端应重新取第一页。GUI 展示独立页，扫描中轮询刷新，不拼接不同版本。条目详情的 `enumerated` 对目录仅表示它的直接子项枚举结束，不表示所有后代都完整。

扫描按批次发布条目及祖先大小，每批最多 256 个条目；`revision` 是索引版本，不能当作已扫描文件数。条目标识在一个任务内稳定，不同任务的并发完成顺序可能不同。取消时保留已发布或已读入当前批次的部分结果，所有工作线程退出后才进入终态；单次阻塞的文件系统调用无法强制中断。

符号链接、Windows 重解析点和特殊文件不会被跟随或计入大小；观察到的文件消失、目录变更和访问失败会记录异常。`issue_count` 是总数，`issues` 按路径和错误代码排序后保留最多 100 条，包含路径、错误代码和原因；选择结果不受线程完成顺序影响。

扫描默认复用目录枚举提供的大小和修改时间。Windows 的 NTFS 硬链接可能存在旧的目录项信息，即使写入发生在扫描开始前，部分名称的大小与时间也可能滞后。`complete` 只表示枚举覆盖情况，不保证元信息新鲜度；列表和详情都读取扫描索引，不会在查询时刷新磁盘元信息。文件系统并非冻结快照，需要新的枚举结果时可明确发起重扫，但重扫不能保证操作系统缓存已更新。

业务错误返回 `{ "code": "...", "message": "...", "task_id": null }`。畸形 URL 参数或不支持的请求格式可能由 HTTP 框架直接返回 4xx；客户端不能假定所有错误体均为 JSON。

## 本地调用示例

在项目根目录的普通 PowerShell 终端中调用已启动的 Server：

```powershell
$orionConnection = Get-Content .orion/connection-43120.json -Raw | ConvertFrom-Json
$orionHeaders = @{ Authorization = "Bearer $($orionConnection.token)" }
Invoke-RestMethod "$($orionConnection.url)/api/v1/health" -Headers $orionHeaders
Invoke-RestMethod "$($orionConnection.url)/api/v1/tasks" -Headers $orionHeaders
```

浏览器跨域仅允许当前 Vite 开发地址与 Tauri 本地来源；其他调用者仍必须携带令牌。CORS 与回环监听不替代认证。
