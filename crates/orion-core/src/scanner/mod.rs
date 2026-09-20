pub(crate) mod filesystem;
mod parallel;

use crate::{Kind, ScanIssue};
use std::{ffi::OsString, path::PathBuf};

pub(crate) struct Directory {
    pub(crate) path: PathBuf,
    pub(crate) id: usize,
}

pub(crate) struct Discovered {
    pub(crate) name: OsString,
    pub(crate) kind: Kind,
    pub(crate) bytes: u64,
    pub(crate) modified: Option<u64>,
    pub(crate) directory_path: Option<PathBuf>,
}

#[derive(Default)]
pub(crate) struct Batch {
    pub(crate) entries: Vec<Discovered>,
    pub(crate) issues: Vec<ScanIssue>,
}

impl Batch {
    pub(crate) fn len(&self) -> usize {
        self.entries.len() + self.issues.len()
    }

    pub(crate) fn issue(&mut self, path: &std::path::Path, code: &str, message: String) {
        self.issues.push(ScanIssue {
            path: path.to_string_lossy().into(),
            code: code.into(),
            message,
        });
    }
}
