//! Synthetic in-memory fixtures for isolating query cost from filesystem enumeration.
use crate::{
    scanner::{Batch, Discovered},
    Kind, Scan,
};

impl Scan {
    #[doc(hidden)]
    pub fn benchmark_publish_files(&self, count: usize) {
        let start = self.index.read().unwrap().entries.len();
        for offset in (0..count).step_by(256) {
            let mut batch = Batch::default();
            for number in offset..(offset + 256).min(count) {
                let id = start + number;
                batch.entries.push(Discovered {
                    name: format!("file-{id:08}-中文.bin").into(),
                    kind: Kind::File,
                    bytes: ((id * 7919) % 1_000_003) as u64,
                    modified: Some(1000),
                    directory_path: None,
                });
            }
            self.commit_batch(0, &mut batch, Some(true));
        }
    }
}
