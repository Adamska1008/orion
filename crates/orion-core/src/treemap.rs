//! Bounded, coherent subtree views for space maps. No filesystem access.
use crate::{index::Index, EntryView, Kind, QueryError, Scan};
use serde::Serialize;
use std::{cmp::Reverse, collections::BinaryHeap};

const MAX_NODES: usize = 2048;
const MAX_CHILDREN: usize = 64;

#[derive(Debug, Serialize)]
pub struct Treemap {
    pub revision: u64,
    pub depth: usize,
    pub root: TreemapNode,
}

#[derive(Debug, Serialize)]
pub struct TreemapNode {
    #[serde(flatten)]
    pub entry: EntryView,
    pub child_count: usize,
    pub zero_count: usize,
    pub expanded: bool,
    pub omitted_count: usize,
    pub omitted_bytes: u64,
    pub children: Vec<TreemapNode>,
}

impl Scan {
    /// Root is depth zero. Child areas and parent totals come from the same revision.
    pub fn treemap(&self, parent: usize, depth: usize) -> Result<Treemap, QueryError> {
        let index = self.index.read().unwrap();
        let root = index.entries.get(parent).ok_or(QueryError::NotFound)?;
        if root.kind != Kind::Directory {
            return Err(QueryError::NotDirectory);
        }
        let depth = depth.clamp(1, 4);
        let mut budget = MAX_NODES - 1;
        Ok(Treemap {
            revision: index.revision,
            depth,
            root: subtree(&index, parent, depth, &mut budget),
        })
    }
}

fn subtree(index: &Index, id: usize, depth: usize, budget: &mut usize) -> TreemapNode {
    let entry = &index.entries[id];
    let mut node = TreemapNode {
        entry: entry.view(id),
        child_count: entry.children.len(),
        zero_count: 0,
        expanded: false,
        omitted_count: 0,
        omitted_bytes: 0,
        children: vec![],
    };
    if entry.kind != Kind::Directory || depth == 0 || *budget == 0 {
        return node;
    }
    node.expanded = true;
    let limit = MAX_CHILDREN.min(*budget);
    // The heap contains only the largest children; even a million-item directory
    // needs O(limit) temporary memory. Ties use names then IDs for stable layout.
    let mut largest = BinaryHeap::with_capacity(limit);
    let mut positive = 0;
    for &child in &entry.children {
        let item = &index.entries[child];
        if item.bytes == 0 {
            node.zero_count += 1;
            continue;
        }
        positive += 1;
        let rank = (Reverse(item.bytes), item.name.as_os_str(), child);
        if largest.len() < limit {
            largest.push(rank);
        } else if largest.peek().is_some_and(|worst| rank < *worst) {
            *largest.peek_mut().unwrap() = rank;
        }
    }
    let selected = largest.into_sorted_vec();
    // Reserve siblings before descending so a deep first child cannot consume them.
    *budget -= selected.len();
    node.omitted_count = positive - selected.len();
    let included = selected.iter().fold(0u64, |sum, item| {
        sum.saturating_add(index.entries[item.2].bytes)
    });
    node.omitted_bytes = entry.bytes.saturating_sub(included);
    node.children = selected
        .into_iter()
        .map(|(_, _, child)| subtree(index, child, depth - 1, budget))
        .collect();
    node
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::{Batch, Discovered};

    fn add(scan: &Scan, parent: usize, names: Vec<(String, Kind, u64)>) -> usize {
        let first = scan.index.read().unwrap().entries.len();
        let mut batch = Batch {
            entries: names
                .into_iter()
                .map(|(name, kind, bytes)| Discovered {
                    name: name.into(),
                    kind,
                    bytes,
                    modified: None,
                    directory_path: None,
                })
                .collect(),
            issues: vec![],
        };
        scan.commit_batch(parent, &mut batch, Some(true));
        first
    }

    fn verify(node: &TreemapNode) -> usize {
        if node.expanded {
            assert_eq!(
                node.child_count,
                node.children.len() + node.zero_count + node.omitted_count
            );
            assert_eq!(
                node.entry.logical_bytes,
                node.children
                    .iter()
                    .map(|n| n.entry.logical_bytes)
                    .sum::<u64>()
                    + node.omitted_bytes
            );
        }
        assert!(node.children.len() <= MAX_CHILDREN);
        1 + node.children.iter().map(verify).sum::<usize>()
    }

    #[test]
    fn depth_is_relative_and_zero_size_entries_do_not_distort_area() {
        let temp = tempfile::tempdir().unwrap();
        let scan = Scan::new(temp.path()).unwrap();
        let a = add(
            &scan,
            0,
            vec![
                ("a".into(), Kind::Directory, 0),
                ("zero".into(), Kind::File, 0),
                ("link".into(), Kind::Link, 0),
            ],
        );
        let b = add(&scan, a, vec![("b".into(), Kind::Directory, 0)]);
        let c = add(&scan, b, vec![("c".into(), Kind::Directory, 0)]);
        let file = add(&scan, c, vec![("file".into(), Kind::File, 37)]);
        for depth in 1..=4 {
            let tree = scan.treemap(0, depth).unwrap();
            assert_eq!(tree.revision, scan.summary().revision);
            assert_eq!(tree.root.entry.logical_bytes, 37);
            assert_eq!(tree.root.zero_count, 2);
            let mut node = &tree.root;
            for _ in 0..depth {
                node = &node.children[0];
            }
            assert!(node.children.is_empty());
            assert!(!node.expanded);
            assert_eq!(verify(&tree.root), depth + 1);
        }
        assert_eq!(
            scan.treemap(b, 2).unwrap().root.children[0].children[0]
                .entry
                .id,
            file
        );
        assert!(matches!(
            scan.treemap(file, 2),
            Err(QueryError::NotDirectory)
        ));
        assert!(matches!(scan.treemap(999, 2), Err(QueryError::NotFound)));
    }

    #[test]
    fn wide_deep_trees_are_bounded_and_omitted_bytes_are_preserved() {
        let temp = tempfile::tempdir().unwrap();
        let scan = Scan::new(temp.path()).unwrap();
        let first = add(
            &scan,
            0,
            (0..90)
                .map(|i| (format!("dir-{i:03}"), Kind::Directory, 0))
                .collect(),
        );
        for parent in first..first + 90 {
            add(
                &scan,
                parent,
                (1..=100)
                    .map(|i| (format!("file-{i:03}"), Kind::File, i))
                    .collect(),
            );
        }
        let tree = scan.treemap(0, 4).unwrap();
        assert!(verify(&tree.root) <= MAX_NODES);
        assert_eq!(tree.root.children.len(), MAX_CHILDREN);
        assert_eq!(tree.root.omitted_count, 26);
        assert_eq!(tree.root.omitted_bytes, 26 * 5050);
        assert_eq!(tree.root.children[0].entry.name, "dir-000");
        assert_eq!(tree.root.children[0].children[0].entry.logical_bytes, 100);
        assert!(tree.root.children.iter().any(|child| !child.expanded));
    }

    #[test]
    fn empty_directories_and_new_scan_batches_remain_coherent() {
        let temp = tempfile::tempdir().unwrap();
        let scan = Scan::new(temp.path()).unwrap();
        let before = scan.treemap(0, 2).unwrap();
        assert_eq!(before.root.entry.logical_bytes, 0);
        assert!(before.root.children.is_empty());
        let directory = add(&scan, 0, vec![("growing".into(), Kind::Directory, 0)]);
        for batch in 0..10 {
            add(
                &scan,
                directory,
                vec![(format!("batch-{batch}"), Kind::File, 10)],
            );
            let tree = scan.treemap(0, 2).unwrap();
            assert!(tree.revision > before.revision);
            assert_eq!(tree.root.entry.logical_bytes, (batch + 1) * 10);
            verify(&tree.root);
        }
    }
}
