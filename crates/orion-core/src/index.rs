use crate::{
    scanner::{Batch, Directory},
    EntryView, Kind, ScanIssue, Status,
};
use std::ffi::OsString;

pub(crate) struct Entry {
    pub(crate) parent: Option<usize>,
    pub(crate) name: OsString,
    pub(crate) kind: Kind,
    pub(crate) bytes: u64,
    pub(crate) modified: Option<u64>,
    pub(crate) enumerated: bool,
    pub(crate) children: Vec<usize>,
    pub(crate) children_revision: u64,
}

impl Entry {
    pub(crate) fn view(&self, id: usize) -> EntryView {
        EntryView {
            id,
            parent_id: self.parent,
            name: self.name.to_string_lossy().into(),
            kind: self.kind,
            logical_bytes: self.bytes,
            allocated_bytes: None,
            modified_at: self.modified,
            enumerated: self.enumerated,
        }
    }
}

pub(crate) struct Index {
    pub(crate) status: Status,
    pub(crate) revision: u64,
    pub(crate) finished_at: Option<u64>,
    pub(crate) files: u64,
    pub(crate) directories: u64,
    pub(crate) entries: Vec<Entry>,
    pub(crate) issue_count: u64,
    pub(crate) issues: Vec<ScanIssue>,
}

impl Index {
    pub(crate) fn record_issue(&mut self, issue: ScanIssue) {
        self.issue_count += 1;
        // Keep a deterministic bounded sample regardless of worker completion order.
        let position = self.issues.partition_point(|existing| {
            (&existing.path, &existing.code) <= (&issue.path, &issue.code)
        });
        if position < 100 {
            self.issues.insert(position, issue);
            self.issues.truncate(100);
        }
    }
}

impl Index {
    pub(crate) fn apply_batch(
        &mut self,
        parent: usize,
        batch: &mut Batch,
        enumerated: Option<bool>,
    ) -> Vec<Directory> {
        let mut directories = Vec::new();
        let mut bytes = 0u64;
        self.entries.reserve(batch.entries.len());
        if !batch.entries.is_empty() {
            self.entries[parent].children_revision = self.revision + 1;
        }
        for discovered in batch.entries.drain(..) {
            let id = self.entries.len();
            if let Some(path) = discovered.directory_path {
                directories.push(Directory { path, id });
            }
            self.files += u64::from(discovered.kind == Kind::File);
            self.directories += u64::from(discovered.kind == Kind::Directory);
            bytes = bytes.saturating_add(discovered.bytes);
            self.entries.push(Entry {
                parent: Some(parent),
                name: discovered.name,
                kind: discovered.kind,
                bytes: discovered.bytes,
                modified: discovered.modified,
                enumerated: discovered.kind == Kind::File,
                children: vec![],
                children_revision: 0,
            });
            self.entries[parent].children.push(id);
        }
        let mut ancestor = Some(parent);
        while let Some(id) = ancestor {
            self.entries[id].bytes = self.entries[id].bytes.saturating_add(bytes);
            ancestor = self.entries[id].parent;
            if bytes > 0 {
                if let Some(parent) = ancestor {
                    self.entries[parent].children_revision = self.revision + 1;
                }
            }
        }
        for issue in batch.issues.drain(..) {
            self.record_issue(issue);
        }
        if let Some(enumerated) = enumerated {
            self.entries[parent].enumerated = enumerated;
        }
        self.revision += 1;
        directories
    }
}
