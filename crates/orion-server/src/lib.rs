use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderValue, Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use orion_core::{QueryError, Scan, Summary};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tower_http::cors::CorsLayer;
use uuid::Uuid;

#[derive(Default)]
struct Tasks {
    latest: Option<Arc<Scan>>,
    requests: HashMap<Uuid, (String, Uuid)>,
}

#[derive(Clone)]
pub struct AppState {
    token: Arc<String>,
    pub instance_id: Uuid,
    tasks: Arc<Mutex<Tasks>>,
    scan_workers: usize,
}

impl AppState {
    pub fn new(token: String) -> Self {
        Self {
            token: Arc::new(token),
            instance_id: Uuid::new_v4(),
            tasks: Default::default(),
            scan_workers: orion_core::default_scan_workers(),
        }
    }

    pub fn with_scan_workers(mut self, workers: usize) -> Self {
        self.scan_workers = workers.clamp(1, orion_core::MAX_SCAN_WORKERS);
        self
    }

    fn task(&self, id: Uuid) -> Result<Arc<Scan>, ApiError> {
        self.tasks
            .lock()
            .unwrap()
            .latest
            .as_ref()
            .filter(|s| s.id == id)
            .cloned()
            .ok_or_else(|| {
                ApiError::new(
                    StatusCode::NOT_FOUND,
                    "task_not_found",
                    "任务不存在或已被新扫描替换；Server 重启不会保留结果。",
                )
            })
    }

    fn start(&self, request: StartScan) -> Result<Summary, ApiError> {
        let mut tasks = self.tasks.lock().unwrap();
        if let Some((root, id)) = tasks.requests.get(&request.request_id) {
            if root != &request.root {
                return Err(ApiError::new(
                    StatusCode::CONFLICT,
                    "request_id_conflict",
                    "相同 request_id 不能用于不同扫描范围。",
                ));
            }
            return tasks
                .latest
                .as_ref()
                .filter(|s| s.id == *id)
                .map(|s| s.summary())
                .ok_or_else(|| {
                    ApiError::new(
                        StatusCode::GONE,
                        "result_replaced",
                        "该请求已执行，结果已被后续扫描替换。",
                    )
                });
        }
        if let Some(task) = tasks
            .latest
            .as_ref()
            .filter(|s| s.summary().status.active())
        {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "scan_in_progress",
                "已有扫描正在运行，请查询或取消该任务。",
            )
            .with_task(task.id));
        }
        if tasks.requests.len() >= 1024 {
            return Err(ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "session_limit",
                "本次服务已处理 1024 个扫描请求，请重启服务。",
            ));
        }
        let scan = Scan::new(&PathBuf::from(&request.root))
            .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, "invalid_root", &e))?;
        let worker = scan.clone();
        let workers = self.scan_workers;
        std::thread::Builder::new()
            .name("orion-scan".into())
            .spawn(move || {
                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    worker.run_with_workers(workers)
                }))
                .is_err()
                {
                    worker.fail("扫描工作线程意外退出。".into());
                }
            })
            .map_err(|_| {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "worker_start_failed",
                    "无法启动扫描线程。",
                )
            })?;
        tasks
            .requests
            .insert(request.request_id, (request.root, scan.id));
        let summary = scan.summary();
        tasks.latest = Some(scan);
        Ok(summary)
    }
}

#[derive(Serialize)]
struct ErrorBody {
    code: String,
    message: String,
    task_id: Option<Uuid>,
}

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
    fn with_task(mut self, id: Uuid) -> Self {
        self.body.task_id = Some(id);
        self
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(self.body)).into_response()
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

async fn health(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(
        serde_json::json!({ "name": "orion-server", "api_version": 1, "instance_id": state.instance_id }),
    )
}

async fn tasks(State(state): State<AppState>) -> Json<Vec<Summary>> {
    Json(
        state
            .tasks
            .lock()
            .unwrap()
            .latest
            .iter()
            .map(|s| s.summary())
            .collect(),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartScan {
    pub root: String,
    pub request_id: Uuid,
}

async fn start(
    State(state): State<AppState>,
    body: Result<Json<StartScan>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<Summary>, ApiError> {
    let Json(request) = body.map_err(|_| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "请提供 root 和 UUID 格式的 request_id。",
        )
    })?;
    tokio::task::spawn_blocking(move || state.start(request))
        .await
        .map_err(|_| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "请求处理失败。",
            )
        })?
        .map(Json)
}

async fn task(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Summary>, ApiError> {
    Ok(Json(state.task(id)?.summary()))
}

async fn cancel(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Summary>, ApiError> {
    let task = state.task(id)?;
    task.cancel();
    Ok(Json(task.summary()))
}

#[derive(Default, Deserialize)]
struct ListQuery {
    #[serde(default)]
    parent: usize,
    #[serde(default)]
    offset: usize,
    limit: Option<usize>,
    revision: Option<u64>,
}

async fn entries(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(query): Query<ListQuery>,
) -> Result<Json<orion_core::Page>, ApiError> {
    let scan = state.task(id)?;
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
    state.task(id)?.detail(entry).map(Json).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

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
