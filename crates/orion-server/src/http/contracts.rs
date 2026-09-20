//! Wire-only request/response types. Scan result types remain the core's public views.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Serialize)]
pub struct ConnectionDocument {
    pub url: String,
    pub token: String,
    pub instance_id: Uuid,
}

#[derive(Serialize)]
pub(super) struct HealthResponse {
    pub name: &'static str,
    pub api_version: u32,
    pub instance_id: Uuid,
    pub version: &'static str,
    pub capabilities: Vec<&'static str>,
}

impl HealthResponse {
    pub fn new(instance_id: Uuid) -> Self {
        Self {
            name: "orion-server",
            api_version: 1,
            instance_id,
            version: env!("CARGO_PKG_VERSION"),
            capabilities: vec!["graceful_shutdown", "treemap"],
        }
    }
}

#[derive(Serialize)]
pub(super) struct ErrorBody {
    pub code: String,
    pub message: String,
    pub task_id: Option<Uuid>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ShutdownRequest {
    pub instance_id: Uuid,
    #[serde(default)]
    pub cancel_active: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StartScanRequest {
    pub root: String,
    pub request_id: Uuid,
}

#[derive(Default, Deserialize)]
pub(super) struct ListQuery {
    #[serde(default)]
    pub parent: usize,
    #[serde(default)]
    pub offset: usize,
    pub limit: Option<usize>,
    pub revision: Option<u64>,
}

#[derive(Default, Deserialize)]
pub(super) struct TreemapQuery {
    #[serde(default)]
    pub parent: usize,
    pub depth: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use orion_core::{
        Detail, EntryView, Kind, Page, ScanIssue, Status, Summary, Treemap, TreemapNode,
    };

    #[test]
    fn serialized_responses_match_the_shared_client_contract() {
        let id = Uuid::nil();
        let tasks: Vec<_> = [
            Status::Running,
            Status::Cancelling,
            Status::Cancelled,
            Status::Completed,
            Status::Failed,
        ]
        .into_iter()
        .map(|status| Summary {
            id,
            root: "C:\\data".into(),
            status,
            revision: 7,
            started_at: 1000,
            finished_at: (!status.active()).then_some(2000),
            files: 2,
            directories: 1,
            logical_bytes: 37,
            allocated_bytes: None,
            complete: status == Status::Completed,
            issue_count: 1,
            issues: vec![ScanIssue {
                path: "C:\\data\\link".into(),
                code: "link_skipped".into(),
                message: "fixture issue".into(),
            }],
        })
        .collect();
        let entries: Vec<_> = [Kind::Directory, Kind::File, Kind::Link, Kind::Other]
            .into_iter()
            .enumerate()
            .map(|(id, kind)| EntryView {
                id,
                parent_id: (id > 0).then_some(0),
                name: format!("entry-{id}"),
                kind,
                logical_bytes: 37,
                allocated_bytes: None,
                modified_at: (id > 0).then_some(1000),
                enumerated: kind == Kind::File,
            })
            .collect();
        let mut health = HealthResponse::new(id);
        health.version = "fixture-version";
        let value = serde_json::json!({
            "health": health,
            "connection": ConnectionDocument { url: "http://127.0.0.1:43120".into(), token: "fixture-token".into(), instance_id: id },
            "tasks": tasks,
            "page": Page { revision: 7, total: entries.len(), offset: 0, entries: entries.clone() },
            "detail": Detail { entry: entries[1].clone(), path: "C:\\data\\entry-1".into(), ancestors: vec![entries[0].clone()] },
            "treemap": Treemap { revision: 7, depth: 2, root: TreemapNode {
                entry: entries[0].clone(), child_count: 1, zero_count: 0, expanded: true, omitted_count: 0, omitted_bytes: 0,
                children: vec![TreemapNode { entry: entries[1].clone(), child_count: 0, zero_count: 0, expanded: false, omitted_count: 0, omitted_bytes: 0, children: vec![] }],
            }},
            "errors": [ErrorBody { code: "scan_in_progress".into(), message: "fixture conflict".into(), task_id: Some(id) }, ErrorBody { code: "stale_revision".into(), message: "fixture stale".into(), task_id: None }],
        });
        let expected: serde_json::Value =
            serde_json::from_str(include_str!("../../../../contracts/api-v1.json")).unwrap();
        assert_eq!(value, expected, "Update the shared fixture and run the TypeScript contract tests when the wire shape changes");
    }
}
