use proptest::prelude::*;

use super::*;
use crate::{ProjectWorkspace, SnapshotRetention, VirtualFile, VirtualRoot};

fn workspace_with_accepted(entries: &[(&str, &[u8])]) -> ProjectWorkspace {
    let mut workspace = ProjectWorkspace::new(VfsLimits::default()).expect("limits");
    let mut generated = workspace.begin_generated();
    for (path, bytes) in entries {
        generated
            .write(VirtualPath::user(path).expect("path"), bytes.to_vec())
            .expect("generated file");
    }
    generated.accept().expect("accepted generated set");
    workspace
}

fn snapshot_bytes<'a>(snapshot: &'a VfsSnapshot, path: &str) -> Option<&'a [u8]> {
    snapshot
        .get(&VirtualPath::user(path).expect("path"))
        .expect("live snapshot")
        .map(VirtualFile::bytes)
}

#[test]
fn writes_are_private_and_candidate_snapshots_are_immutable() {
    let mut workspace = workspace_with_accepted(&[("old.aux", b"old")]);
    let accepted = workspace.snapshot();
    let mut generated = workspace.begin_generated();
    let before = generated.snapshot();
    generated
        .write(
            VirtualPath::user("new.aux").expect("path"),
            b"complete".to_vec(),
        )
        .expect("write");
    let after = generated.snapshot();

    assert_eq!(snapshot_bytes(&accepted, "new.aux"), None);
    assert_eq!(snapshot_bytes(&before, "new.aux"), None);
    assert_eq!(snapshot_bytes(&after, "new.aux"), Some(&b"complete"[..]));
    assert_eq!(snapshot_bytes(&after, "old.aux"), Some(&b"old"[..]));
}

#[test]
fn accept_replaces_the_whole_set_and_stales_candidate_snapshots() {
    let mut workspace = workspace_with_accepted(&[("old.aux", b"old")]);
    let candidate = {
        let mut generated = workspace.begin_generated();
        generated
            .write(
                VirtualPath::user("new.aux").expect("path"),
                b"accepted".to_vec(),
            )
            .expect("write");
        let candidate = generated.snapshot();
        assert_eq!(snapshot_bytes(&candidate, "old.aux"), Some(&b"old"[..]));
        assert_eq!(
            generated.accept().expect("accept"),
            AcceptedGenerated {
                generated_files: 1,
                logical_bytes: 8,
            }
        );
        candidate
    };

    assert!(candidate.is_stale());
    assert_eq!(snapshot_bytes(&workspace.snapshot(), "old.aux"), None);
    assert_eq!(
        snapshot_bytes(&workspace.snapshot(), "new.aux"),
        Some(&b"accepted"[..])
    );
}

#[test]
fn discard_and_drop_roll_back_without_staling_accepted_snapshots() {
    let mut workspace = workspace_with_accepted(&[("old.aux", b"old")]);
    let accepted = workspace.snapshot();
    let candidate = {
        let mut generated = workspace.begin_generated();
        generated
            .write(
                VirtualPath::user("new.aux").expect("path"),
                b"discard".to_vec(),
            )
            .expect("write");
        let candidate = generated.snapshot();
        generated.discard();
        candidate
    };
    assert!(candidate.is_stale());
    assert!(!accepted.is_stale());
    assert_eq!(snapshot_bytes(&accepted, "old.aux"), Some(&b"old"[..]));
    assert_eq!(snapshot_bytes(&workspace.snapshot(), "new.aux"), None);

    let dropped = {
        let mut generated = workspace.begin_generated();
        generated
            .write(
                VirtualPath::user("dropped.aux").expect("path"),
                b"drop".to_vec(),
            )
            .expect("write");
        generated.snapshot()
    };
    assert!(dropped.is_stale());
    assert_eq!(snapshot_bytes(&workspace.snapshot(), "dropped.aux"), None);
}

#[test]
fn replacement_and_failed_writes_are_atomic_and_exactly_accounted() {
    let limits = VfsLimits {
        stage_bytes: 5,
        generated_bytes: 5,
        one_file_bytes: 4,
        ..VfsLimits::default()
    };
    let mut workspace = ProjectWorkspace::new(limits).expect("limits");
    let mut generated = workspace.begin_generated();
    let path = VirtualPath::user("main.aux").expect("path");
    generated
        .write(path.clone(), b"four".to_vec())
        .expect("write");
    assert!(matches!(
        generated.write(path.clone(), b"overs".to_vec()),
        Err(TransactionError::Limit(VfsLimitError::LimitExceeded {
            kind: VfsLimitKind::OneFileBytes,
            ..
        }))
    ));
    assert_eq!(
        snapshot_bytes(&generated.snapshot(), "main.aux"),
        Some(&b"four"[..])
    );
    generated.write(path, b"two".to_vec()).expect("replacement");
    generated
        .write(VirtualPath::user("other").expect("path"), b"12".to_vec())
        .expect("exact total");
    assert!(matches!(
        generated.write(VirtualPath::user("too-much").expect("path"), vec![0]),
        Err(TransactionError::Limit(VfsLimitError::LimitExceeded {
            kind: VfsLimitKind::GeneratedBytes,
            ..
        }))
    ));
    assert_eq!(
        generated
            .snapshot()
            .list_root(VirtualRoot::Job, 8)
            .expect("list")
            .len(),
        2
    );
}

#[test]
fn candidate_precedence_and_retention_include_shared_accepted_bytes() {
    let mut workspace = workspace_with_accepted(&[("same.aux", b"accepted")]);
    workspace
        .register_user(
            VirtualPath::user("same.aux").expect("path"),
            b"user".to_vec(),
        )
        .expect("user");
    let accepted = workspace.snapshot();
    let accepted_bytes = accepted
        .get(&VirtualPath::user("same.aux").expect("path"))
        .expect("live")
        .expect("accepted")
        .shared_bytes();
    let mut generated = workspace.begin_generated();
    let before = generated.snapshot();
    let before_bytes = before
        .get(&VirtualPath::user("same.aux").expect("path"))
        .expect("live")
        .expect("accepted")
        .shared_bytes();
    assert!(tex_content::SharedBytes::ptr_eq(
        &accepted_bytes,
        &before_bytes
    ));
    assert_eq!(
        before.retention(),
        SnapshotRetention {
            bindings: 2,
            logical_bytes: 12,
            input_bytes: 4,
            generated_bytes: 8,
        }
    );
    generated
        .write(
            VirtualPath::user("same.aux").expect("path"),
            b"pending".to_vec(),
        )
        .expect("replacement");
    assert_eq!(
        snapshot_bytes(&generated.snapshot(), "same.aux"),
        Some(&b"pending"[..])
    );
}

proptest! {
    #[test]
    fn accepted_set_equals_the_last_write_per_path(
        writes in prop::collection::vec(
            ("[a-z]{1,6}\\.aux", prop::collection::vec(any::<u8>(), 0..64)),
            0..64,
        )
    ) {
        let expected = writes.iter().cloned().collect::<std::collections::BTreeMap<_, _>>();
        let mut workspace = ProjectWorkspace::new(VfsLimits::default()).expect("limits");
        let mut generated = workspace.begin_generated();
        for (path, bytes) in writes {
            generated.write(VirtualPath::user(&path).expect("path"), bytes).expect("bounded write");
        }
        generated.accept().expect("accept");
        let snapshot = workspace.snapshot();
        for (path, bytes) in expected {
            prop_assert_eq!(snapshot_bytes(&snapshot, &path), Some(bytes.as_slice()));
        }
    }
}
