pub(crate) mod contracts;
use crate::{
    tasks::{StartScan, TaskError},
    AppState,
};
use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderValue, Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use contracts::{ErrorBody, HealthResponse, ListQuery, ShutdownRequest, StartScanRequest};
use orion_core::{QueryError, Summary};
use tower_http::cors::CorsLayer;
use uuid::Uuid;

pub struct ApiError {
    status: StatusCode,
    body: ErrorBody,
}

impl ApiError {
    fn new(status: StatusCode, code: &str, message: &str) -> Self {
        Self {
            status,
            body: ErrorBody {
                code: code.into(),
                message: message.into(),
                task_id: None,
            },
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(self.body)).into_response()
    }
}

impl From<TaskError> for ApiError {
    fn from(error: TaskError) -> Self {
        match error {
            TaskError::ShuttingDown => Self::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "server_shutting_down",
                "后台正在退出，无法开始新扫描。",
            ),
            TaskError::RequestConflict => Self::new(
                StatusCode::CONFLICT,
                "request_id_conflict",
                "相同 request_id 不能用于不同扫描范围。",
            ),
            TaskError::ResultReplaced => Self::new(
                StatusCode::GONE,
                "result_replaced",
                "该请求已执行，结果已被后续扫描替换。",
            ),
            TaskError::ScanInProgress(task) => {
                let mut error = Self::new(
                    StatusCode::CONFLICT,
                    "scan_in_progress",
                    "已有扫描正在运行或准备启动，请查询或取消该任务。",
                );
                error.body.task_id = task;
                error
            }
            TaskError::SessionLimit => Self::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "session_limit",
                "本次服务已处理 1024 个扫描请求，请重启服务。",
            ),
            TaskError::InvalidRoot(message) => {
                Self::new(StatusCode::BAD_REQUEST, "invalid_root", &message)
            }
            TaskError::WorkerStartFailed => Self::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "worker_start_failed",
                "无法启动扫描线程。",
            ),
            TaskError::TaskNotFound => Self::new(
                StatusCode::NOT_FOUND,
                "task_not_found",
                "任务不存在或已被新扫描替换；Server 重启不会保留结果。",
            ),
            TaskError::Internal => Self::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "请求处理失败。",
            ),
        }
    }
}

impl From<QueryError> for ApiError {
    fn from(error: QueryError) -> Self {
        match error {
            QueryError::NotFound => {
                Self::new(StatusCode::NOT_FOUND, "entry_not_found", "未找到该条目。")
            }
            QueryError::NotDirectory => {
                Self::new(StatusCode::BAD_REQUEST, "not_directory", "该条目不是目录。")
            }
            QueryError::StaleRevision => Self::new(
                StatusCode::CONFLICT,
                "stale_revision",
                "扫描结果已更新，请重新查询第一页。",
            ),
        }
    }
}

async fn authenticate(
    State(state): State<AppState>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let expected = format!("Bearer {}", state.token);
    if request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        != Some(expected.as_str())
    {
        return ApiError::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "连接令牌无效，请重新连接服务。 ",
        )
        .into_response();
    }
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

pub fn router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin([
            HeaderValue::from_static("http://localhost:1420"),
            HeaderValue::from_static("http://127.0.0.1:1420"),
            HeaderValue::from_static("http://tauri.localhost"),
            HeaderValue::from_static("https://tauri.localhost"),
            HeaderValue::from_static("tauri://localhost"),
        ])
        .allow_methods([Method::GET, Method::POST])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE]);
    Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/server/shutdown", post(shutdown))
        .route("/api/v1/tasks", get(tasks))
        .route("/api/v1/scans", post(start))
        .route("/api/v1/tasks/{id}", get(task))
        .route("/api/v1/tasks/{id}/cancel", post(cancel))
        .route("/api/v1/scans/{id}/entries", get(entries))
        .route("/api/v1/scans/{id}/entries/{entry}", get(detail))
        .fallback(|| async {
            ApiError::new(StatusCode::NOT_FOUND, "route_not_found", "未知 API 路径。")
        })
        .layer(middleware::from_fn_with_state(state.clone(), authenticate))
        .layer(cors)
        .with_state(state)
}

async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse::new(state.instance_id))
}

async fn shutdown(
    State(state): State<AppState>,
    Json(request): Json<ShutdownRequest>,
) -> Result<StatusCode, ApiError> {
    if request.instance_id != state.instance_id {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "instance_changed",
            "后台实例已变化，请重新连接后再退出。",
        ));
    }
    state.begin_shutdown(request.cancel_active)?;
    Ok(StatusCode::ACCEPTED)
}

async fn tasks(State(state): State<AppState>) -> Json<Vec<Summary>> {
    Json(state.coordinator.summaries())
}

async fn start(
    State(state): State<AppState>,
    body: Result<Json<StartScanRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<Summary>, ApiError> {
    let Json(request) = body.map_err(|_| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "请提供 root 和 UUID 格式的 request_id。",
        )
    })?;
    tokio::task::spawn_blocking(move || {
        state.coordinator.start(StartScan {
            root: request.root,
            request_id: request.request_id,
        })
    })
    .await
    .map_err(|_| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "请求处理失败。",
        )
    })?
    .map(Json)
    .map_err(Into::into)
}

async fn task(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Summary>, ApiError> {
    Ok(Json(state.coordinator.task(id)?.summary()))
}

async fn cancel(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Summary>, ApiError> {
    let task = state.coordinator.task(id)?;
    task.cancel();
    Ok(Json(task.summary()))
}

async fn entries(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(query): Query<ListQuery>,
) -> Result<Json<orion_core::Page>, ApiError> {
    let scan = state.coordinator.task(id)?;
    tokio::task::spawn_blocking(move || {
        scan.list(
            query.parent,
            query.offset,
            query.limit.unwrap_or(200).clamp(1, 500),
            query.revision,
        )
    })
    .await
    .map_err(|_| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "查询处理失败。",
        )
    })?
    .map(Json)
    .map_err(Into::into)
}

async fn detail(
    State(state): State<AppState>,
    Path((id, entry)): Path<(Uuid, usize)>,
) -> Result<Json<orion_core::Detail>, ApiError> {
    state
        .coordinator
        .task(id)?
        .detail(entry)
        .map(Json)
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use orion_core::Scan;
    use tower::ServiceExt;

    async fn request_shutdown(
        state: &AppState,
        instance: Uuid,
        cancel: bool,
        authenticated: bool,
    ) -> Response {
        let mut request = Request::builder()
            .method("POST")
            .uri("/api/v1/server/shutdown")
            .header("Content-Type", "application/json");
        if authenticated {
            request = request.header("Authorization", "Bearer test-token");
        }
        router(state.clone())
            .oneshot(
                request
                    .body(Body::from(
                        serde_json::json!({"instance_id": instance, "cancel_active": cancel})
                            .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn shutdown_requires_credentials_and_the_current_instance() {
        let state = AppState::new("test-token".into());
        assert_eq!(
            request_shutdown(&state, state.instance_id, true, false)
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            request_shutdown(&state, Uuid::new_v4(), true, true)
                .await
                .status(),
            StatusCode::CONFLICT
        );
        assert!(!state.coordinator.is_shutting_down());
        assert_eq!(
            request_shutdown(&state, state.instance_id, false, true)
                .await
                .status(),
            StatusCode::ACCEPTED
        );
        assert_eq!(
            request_shutdown(&state, state.instance_id, false, true)
                .await
                .status(),
            StatusCode::ACCEPTED
        );
        // The notification must survive a request arriving before the shutdown future polls.
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            state.shutdown_requested(),
        )
        .await
        .unwrap();
        state.wait_for_scan_exit().await;
        let error = state
            .coordinator
            .start(StartScan {
                root: "unused".into(),
                request_id: Uuid::new_v4(),
            })
            .err()
            .unwrap();
        assert_eq!(error, TaskError::ShuttingDown);
    }

    #[tokio::test]
    async fn active_scan_needs_confirmation_and_shutdown_waits_for_workers() {
        let directory = tempfile::tempdir().unwrap();
        let scan = Scan::new(directory.path()).unwrap();
        let state = AppState::new("test-token".into());
        // Delay starting the worker so this exercises the cancellation race deterministically.
        state.coordinator.set_scan_for_test(scan.clone());
        assert_eq!(
            request_shutdown(&state, state.instance_id, false, true)
                .await
                .status(),
            StatusCode::CONFLICT
        );
        assert!(!state.coordinator.is_shutting_down());
        assert_eq!(scan.summary().status, orion_core::Status::Running);
        assert_eq!(
            request_shutdown(&state, state.instance_id, true, true)
                .await
                .status(),
            StatusCode::ACCEPTED
        );
        assert_eq!(scan.summary().status, orion_core::Status::Cancelling);
        assert!(tokio::time::timeout(
            std::time::Duration::from_millis(50),
            state.wait_for_scan_exit()
        )
        .await
        .is_err());
        tokio::task::spawn_blocking(move || scan.run())
            .await
            .unwrap();
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            state.wait_for_scan_exit(),
        )
        .await
        .unwrap();
        assert_eq!(
            state
                .coordinator
                .latest()
                .as_ref()
                .unwrap()
                .summary()
                .status,
            orion_core::Status::Cancelled
        );
    }

    #[tokio::test]
    async fn authentication_and_cors_are_enforced() {
        let app = router(AppState::new("test-token".into()));
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/tasks")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/health")
                    .header("Authorization", "Bearer test-token")
                    .header("Origin", "https://untrusted.example")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(!response
            .headers()
            .contains_key("access-control-allow-origin"));
        let response = app
            .oneshot(
                Request::builder()
                    .method("OPTIONS")
                    .uri("/api/v1/scans")
                    .header("Origin", "http://localhost:1420")
                    .header("Access-Control-Request-Method", "POST")
                    .header(
                        "Access-Control-Request-Headers",
                        "authorization,content-type",
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.headers()["access-control-allow-origin"],
            "http://localhost:1420"
        );
    }

    #[tokio::test]
    async fn api_scan_retries_and_new_clients_share_one_task() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("data"), [0; 37]).unwrap();
        let app = router(AppState::new("test-token".into()));
        let payload = serde_json::json!({ "root": temp.path(), "request_id": Uuid::new_v4() });
        let mut responses = vec![];
        for _ in 0..2 {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/v1/scans")
                        .header("Authorization", "Bearer test-token")
                        .header("Content-Type", "application/json")
                        .body(Body::from(payload.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let body = response.into_body().collect().await.unwrap().to_bytes();
            responses.push(serde_json::from_slice::<serde_json::Value>(&body).unwrap());
        }
        assert_eq!(responses[0]["id"], responses[1]["id"]);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/tasks")
                    .header("Authorization", "Bearer test-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let list: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(list[0]["id"], responses[0]["id"]);
    }
}
