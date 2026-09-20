use serde::Serialize;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Running,
    Cancelling,
    Cancelled,
    Completed,
    Failed,
}

impl Status {
    pub fn active(self) -> bool {
        matches!(self, Self::Running | Self::Cancelling)
    }
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Directory,
    File,
    Link,
    Other,
}

#[derive(Clone, Debug, Serialize)]
pub struct ScanIssue {
    pub path: String,
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Summary {
    pub id: Uuid,
    pub root: String,
    pub status: Status,
    pub revision: u64,
    pub started_at: u64,
    pub finished_at: Option<u64>,
    pub files: u64,
    pub directories: u64,
    pub logical_bytes: u64,
    pub allocated_bytes: Option<u64>,
    pub complete: bool,
    pub issue_count: u64,
    pub issues: Vec<ScanIssue>,
}

#[derive(Clone, Debug, Serialize)]
pub struct EntryView {
    pub id: usize,
    pub parent_id: Option<usize>,
    pub name: String,
    pub kind: Kind,
    pub logical_bytes: u64,
    pub allocated_bytes: Option<u64>,
    pub modified_at: Option<u64>,
    pub enumerated: bool,
}

#[derive(Debug, Serialize)]
pub struct Detail {
    #[serde(flatten)]
    pub entry: EntryView,
    pub path: String,
    pub ancestors: Vec<EntryView>,
}

#[derive(Debug, Serialize)]
pub struct Page {
    pub revision: u64,
    pub total: usize,
    pub offset: usize,
    pub entries: Vec<EntryView>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum QueryError {
    NotFound,
    NotDirectory,
    StaleRevision,
}
