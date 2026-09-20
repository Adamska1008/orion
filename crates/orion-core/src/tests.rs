use super::*;
use std::{fs, path::Path};

#[test]
fn nested_totals_sorted_pages_and_unicode_paths() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("中文")).unwrap();
    fs::write(temp.path().join("中文/a"), [0; 29]).unwrap();
    fs::write(temp.path().join("b"), [0; 11]).unwrap();
    fs::write(temp.path().join("c"), []).unwrap();
    let scan = Scan::new(temp.path()).unwrap();
    scan.run();
    let summary = scan.summary();
    assert!(summary.complete);
    assert_eq!(
        (summary.files, summary.directories, summary.logical_bytes),
        (3, 2, 40)
    );
    assert_eq!(summary.allocated_bytes, None);
    let page = scan.list(0, 0, 1, None).unwrap();
    assert_eq!(page.total, 3);
    assert_eq!(page.entries[0].name, "中文");
    let child = scan.list(page.entries[0].id, 0, 5, None).unwrap();
    assert!(Path::new(&scan.detail(child.entries[0].id).unwrap().path).ends_with("中文/a"));
    assert_eq!(
        scan.list(0, 1, 1, Some(summary.revision)).unwrap().entries[0].logical_bytes,
        11
    );
    assert!(matches!(
        scan.list(0, 0, 1, Some(0)),
        Err(QueryError::StaleRevision)
    ));
}

#[test]
fn cancellation_is_terminal_and_keeps_partial_data() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("file"), [0; 10]).unwrap();
    let scan = Scan::new(temp.path()).unwrap();
    scan.cancel();
    scan.cancel();
    scan.run();
    assert_eq!(scan.summary().status, Status::Cancelled);
    assert!(!scan.summary().complete);
    assert!(scan.summary().finished_at.is_some());
    scan.cancel();
    assert_eq!(scan.summary().status, Status::Cancelled);
}

#[test]
fn vanished_root_is_reported_and_relative_paths_rejected() {
    assert!(Scan::new(Path::new("relative")).is_err());
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("gone");
    fs::create_dir(&path).unwrap();
    let scan = Scan::new(&path).unwrap();
    fs::remove_dir(&path).unwrap();
    scan.run();
    assert!(!scan.summary().complete);
    assert_eq!(scan.summary().issue_count, 1);
}

#[test]
fn path_queries_read_updated_hard_link_size() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("中文目录");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("data.bin"), [7; 4096]).unwrap();
    fs::write(dir.join("empty.bin"), []).unwrap();
    fs::hard_link(dir.join("data.bin"), temp.path().join("hard-link.bin")).unwrap();
    // A linked name may still carry stale directory-entry information after this resize.
    fs::write(dir.join("data.bin"), [9; 8192]).unwrap();
    // Preserve the strict path-query regression as the benchmark control.
    let legacy = Scan::new(temp.path()).unwrap();
    legacy.run_with_metadata(|_, path| fs::symlink_metadata(path));
    assert_eq!(legacy.summary().logical_bytes, 16384);
    assert_eq!(legacy.summary().files, 3);
    assert!(legacy.summary().complete);
    let alias = legacy
        .list(0, 0, 10, None)
        .unwrap()
        .entries
        .into_iter()
        .find(|entry| entry.name == "hard-link.bin")
        .unwrap();
    assert_eq!(alias.logical_bytes, 8192);
}

#[test]
fn hard_links_are_counted_per_directory_entry() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("中文目录");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("data.bin"), [7; 8192]).unwrap();
    fs::hard_link(dir.join("data.bin"), temp.path().join("hard-link.bin")).unwrap();
    let scan = Scan::new(temp.path()).unwrap();
    scan.run();
    assert_eq!(scan.summary().logical_bytes, 16384);
    assert_eq!((scan.summary().files, scan.summary().directories), (2, 2));
    assert!(scan.summary().complete);
    let entries = scan.list(0, 0, 10, None).unwrap().entries;
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().all(|entry| entry.logical_bytes == 8192));
}

#[cfg(windows)]
#[test]
fn junction_is_not_followed() {
    use std::process::Command;
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("target");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("file"), [0; 50]).unwrap();
    let root = temp.path().join("root");
    fs::create_dir(&root).unwrap();
    let link = root.join("junction");
    let output = Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(&link)
        .arg(&target)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "junction fixture failed: {:?}",
        output
    );
    let scan = Scan::new(&root).unwrap();
    scan.run();
    assert_eq!(scan.summary().logical_bytes, 0);
    assert_eq!(scan.summary().issues[0].code, "link_skipped");
    #[cfg(feature = "bench-internals")]
    {
        let legacy = Scan::new(&root).unwrap();
        legacy.run_legacy_metadata();
        assert_eq!(scan.benchmark_rows(), legacy.benchmark_rows());
        assert_eq!(scan.summary().issue_count, legacy.summary().issue_count);
    }
    // Remove only the test junction using a native directory operation.
    fs::remove_dir(link).unwrap();
    assert!(target.join("file").exists());
}
