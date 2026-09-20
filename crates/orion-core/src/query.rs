use crate::*;
use std::{collections::VecDeque, sync::Arc};

const MAX_CACHED_DIRECTORIES: usize = 32;
const MAX_CACHED_IDS: usize = 1_000_000;

struct CachedDirectory {
    parent: usize,
    version: u64,
    ids: Arc<[usize]>,
}

/// LRU cache of sorted IDs only: at most 32 directories and 1M IDs (8 MiB on 64-bit).
#[derive(Default)]
pub(crate) struct QueryCache {
    directories: VecDeque<CachedDirectory>,
    ids: usize,
}

impl QueryCache {
    fn get(&mut self, parent: usize, version: u64) -> Option<Arc<[usize]>> {
        let position = self
            .directories
            .iter()
            .position(|entry| entry.parent == parent && entry.version == version)?;
        let entry = self.directories.remove(position)?;
        let ids = entry.ids.clone();
        self.directories.push_front(entry);
        Some(ids)
    }

    fn insert(&mut self, parent: usize, version: u64, ids: Arc<[usize]>) {
        if ids.len() > MAX_CACHED_IDS {
            return;
        }
        if let Some(position) = self
            .directories
            .iter()
            .position(|entry| entry.parent == parent)
        {
            if self.directories[position].version > version {
                return;
            }
            self.ids -= self.directories.remove(position).unwrap().ids.len();
        }
        while self.directories.len() >= MAX_CACHED_DIRECTORIES
            || self.ids + ids.len() > MAX_CACHED_IDS
        {
            self.ids -= self.directories.pop_back().unwrap().ids.len();
        }
        self.ids += ids.len();
        self.directories.push_front(CachedDirectory {
            parent,
            version,
            ids,
        });
    }
}

impl Scan {
    pub fn list(
        &self,
        parent: usize,
        offset: usize,
        limit: usize,
        revision: Option<u64>,
    ) -> Result<Page, QueryError> {
        self.list_with_cache(parent, offset, limit, revision, true)
    }

    #[cfg(feature = "bench-internals")]
    #[doc(hidden)]
    pub fn benchmark_list_uncached(
        &self,
        parent: usize,
        offset: usize,
        limit: usize,
    ) -> Result<Page, QueryError> {
        self.list_with_cache(parent, offset, limit, None, false)
    }

    fn list_with_cache(
        &self,
        parent: usize,
        offset: usize,
        limit: usize,
        revision: Option<u64>,
        use_cache: bool,
    ) -> Result<Page, QueryError> {
        let (revision, version, mut entries) = {
            let index = self.index.read().unwrap();
            if revision.is_some_and(|value| value != index.revision) {
                return Err(QueryError::StaleRevision);
            }
            let directory = index.entries.get(parent).ok_or(QueryError::NotFound)?;
            if directory.kind != Kind::Directory {
                return Err(QueryError::NotDirectory);
            }
            let cached = if use_cache {
                self.queries
                    .lock()
                    .unwrap()
                    .get(parent, directory.children_revision)
            } else {
                None
            };
            if let Some(ids) = cached {
                // IDs and all returned views are read under one index version.
                return Ok(Page {
                    revision: index.revision,
                    total: ids.len(),
                    offset,
                    entries: ids
                        .iter()
                        .skip(offset)
                        .take(limit.min(500))
                        .map(|&id| index.entries[id].view(id))
                        .collect(),
                });
            }
            let entries = directory
                .children
                .iter()
                .map(|&id| index.entries[id].view(id))
                .collect::<Vec<_>>();
            (index.revision, directory.children_revision, entries)
        };
        // A miss snapshots once, then sorts without blocking writers. Even if a batch is
        // published during the sort, this response remains coherent and that cache key expires.
        entries.sort_unstable_by(|a, b| {
            b.logical_bytes
                .cmp(&a.logical_bytes)
                .then_with(|| a.name.cmp(&b.name))
                .then(a.id.cmp(&b.id))
        });
        if use_cache && entries.len() <= MAX_CACHED_IDS {
            let ids = entries
                .iter()
                .map(|entry| entry.id)
                .collect::<Vec<_>>()
                .into();
            self.queries.lock().unwrap().insert(parent, version, ids);
        }
        Ok(Page {
            revision,
            total: entries.len(),
            offset,
            entries: entries
                .into_iter()
                .skip(offset)
                .take(limit.min(500))
                .collect(),
        })
    }

    pub fn detail(&self, id: usize) -> Result<Detail, QueryError> {
        let index = self.index.read().unwrap();
        let entry = index.entries.get(id).ok_or(QueryError::NotFound)?;
        let mut ids = vec![];
        let mut parent = entry.parent;
        while let Some(p) = parent {
            ids.push(p);
            parent = index.entries[p].parent;
        }
        ids.reverse();
        let mut path = self.root.clone();
        for &p in ids.iter().skip(1) {
            path.push(&index.entries[p].name);
        }
        if id != 0 {
            path.push(&entry.name);
        }
        Ok(Detail {
            entry: entry.view(id),
            path: path.to_string_lossy().into(),
            ancestors: ids.into_iter().map(|p| index.entries[p].view(p)).collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::{Batch, Discovered};

    fn add(scan: &Scan, parent: usize, name: &str, kind: Kind, bytes: u64) -> usize {
        let id = scan.index.read().unwrap().entries.len();
        let mut batch = Batch {
            entries: vec![Discovered {
                name: name.into(),
                kind,
                bytes,
                modified: None,
                directory_path: None,
            }],
            issues: vec![],
        };
        scan.commit_batch(parent, &mut batch, Some(true));
        id
    }

    #[test]
    fn cached_pages_invalidate_when_a_descendant_changes_parent_order() {
        let temp = tempfile::tempdir().unwrap();
        let scan = Scan::new(temp.path()).unwrap();
        let a = add(&scan, 0, "a", Kind::Directory, 0);
        let b = add(&scan, 0, "b", Kind::Directory, 0);
        add(&scan, a, "first", Kind::File, 10);
        let before = scan.list(0, 0, 1, None).unwrap();
        assert_eq!(before.entries[0].id, a);
        assert_eq!(scan.list(0, 1, 1, None).unwrap().entries[0].id, b);
        let a_page = scan.list(a, 0, 10, None).unwrap();
        let version = scan.index.read().unwrap().entries[a].children_revision;
        let cached = scan.queries.lock().unwrap().get(a, version).unwrap();
        add(&scan, b, "larger", Kind::File, 20);
        let after = scan.list(0, 0, 1, None).unwrap();
        assert_eq!(after.entries[0].id, b);
        assert_eq!(after.entries[0].logical_bytes, 20);
        assert_eq!(scan.list(0, 1, 1, None).unwrap().entries[0].id, a);
        assert!(matches!(
            scan.list(0, 0, 1, Some(before.revision)),
            Err(QueryError::StaleRevision)
        ));
        let unchanged = scan.list(a, 0, 10, None).unwrap();
        assert!(unchanged.revision > a_page.revision);
        assert!(Arc::ptr_eq(
            &cached,
            &scan.queries.lock().unwrap().get(a, version).unwrap()
        ));
        assert_eq!(unchanged.entries[0].logical_bytes, 10);
    }

    #[test]
    fn cached_order_still_reads_current_entry_flags_and_new_entries() {
        let temp = tempfile::tempdir().unwrap();
        let scan = Scan::new(temp.path()).unwrap();
        let a = add(&scan, 0, "a", Kind::Directory, 0);
        assert!(!scan.list(0, 0, 15, None).unwrap().entries[0].enumerated);
        scan.commit_batch(a, &mut Batch::default(), Some(true));
        assert!(scan.list(0, 0, 15, None).unwrap().entries[0].enumerated);
        add(&scan, 0, "file", Kind::File, 100);
        let page = scan.list(0, 0, 15, None).unwrap();
        assert_eq!(page.total, 2);
        assert_eq!(page.entries[0].name, "file");
    }

    #[test]
    fn cache_limits_memory_and_evicts_least_recently_used_directories() {
        let mut cache = QueryCache::default();
        for parent in 0..MAX_CACHED_DIRECTORIES {
            cache.insert(parent, 1, vec![parent].into());
        }
        cache.get(0, 1).unwrap();
        cache.insert(100, 1, vec![100].into());
        assert!(cache.get(0, 1).is_some());
        assert!(cache.get(1, 1).is_none());
        cache.insert(200, 1, vec![0; MAX_CACHED_IDS].into());
        assert_eq!(cache.ids, MAX_CACHED_IDS);
        assert_eq!(cache.directories.len(), 1);
        cache.insert(201, 1, vec![0; MAX_CACHED_IDS + 1].into());
        assert_eq!(cache.ids, MAX_CACHED_IDS);
    }
}
