use std::sync::Arc;

use super::*;
use crate::{
    FileKind, FileRequestKey, ProjectWorkspace, ResolvedFile, SnapshotRetention, VfsLimits,
};

fn bytes<'a>(snapshot: &'a VfsSnapshot, path: &VirtualPath) -> Option<&'a [u8]> {
    snapshot
        .get(path)
        .expect("live snapshot")
        .map(VirtualFile::bytes)
}

fn workspace_with_users(entries: &[(&str, &[u8])]) -> ProjectWorkspace {
    let mut workspace = ProjectWorkspace::new(VfsLimits::default()).expect("VFS");
    for (path, data) in entries {
        workspace
            .register_user(VirtualPath::user(path).expect("path"), data.to_vec())
            .expect("user file");
    }
    workspace
}

#[test]
fn snapshot_clone_is_cheap_and_mutation_preserves_its_generation() {
    let mut workspace = workspace_with_users(&[("main.tex", b"old")]);
    let snapshot = workspace.snapshot();
    let clone = snapshot.clone();
    assert!(Arc::ptr_eq(&snapshot.generation, &clone.generation));
    assert!(Arc::ptr_eq(&snapshot.valid, &clone.valid));

    workspace
        .register_user(
            VirtualPath::user("later.tex").expect("path"),
            b"new".to_vec(),
        )
        .expect("new file");
    assert!(!Arc::ptr_eq(
        &snapshot.generation,
        &workspace.snapshot().generation
    ));
    assert_eq!(
        bytes(&snapshot, &VirtualPath::user("main.tex").expect("path")),
        Some(&b"old"[..])
    );
    assert!(
        !snapshot
            .contains(&VirtualPath::user("later.tex").expect("path"))
            .expect("live")
    );
}

#[test]
fn exact_lookup_obeys_root_specific_precedence() {
    let mut workspace = workspace_with_users(&[("same.aux", b"user")]);
    {
        let mut accepted = workspace.begin_generated();
        accepted
            .write(
                VirtualPath::user("same.aux").expect("path"),
                b"accepted".to_vec(),
            )
            .expect("write");
        accepted.accept().expect("accept");
    }
    let request = FileRequestKey::new(FileKind::TexInput, "plain.tex").expect("request");
    workspace
        .preload(ResolvedFile {
            request,
            virtual_path: "/texlive/plain.tex".into(),
            bytes: b"plain".to_vec().into(),
            expected_digest: None,
        })
        .expect("resource");
    let mut pending = workspace.begin_generated();
    pending
        .write(
            VirtualPath::user("same.aux").expect("path"),
            b"pending".to_vec(),
        )
        .expect("write");
    let snapshot = pending.snapshot();
    assert_eq!(
        bytes(&snapshot, &VirtualPath::user("same.aux").expect("path")),
        Some(&b"pending"[..])
    );
    assert_eq!(
        bytes(
            &snapshot,
            &VirtualPath::distribution("/texlive/plain.tex").expect("path")
        ),
        Some(&b"plain"[..])
    );
}

#[test]
fn lexical_enumeration_is_visible_unique_component_aware_and_bounded() {
    let mut workspace = workspace_with_users(&[
        ("z.tex", b"z"),
        ("dir/c.tex", b"c"),
        ("directory/no.tex", b"no"),
        ("dir/a.tex", b"a"),
    ]);
    let mut generated = workspace.begin_generated();
    generated
        .write(
            VirtualPath::user("dir/a.tex").expect("path"),
            b"shadow".to_vec(),
        )
        .expect("write");
    generated
        .write(
            VirtualPath::user("dir/b.tex").expect("path"),
            b"new".to_vec(),
        )
        .expect("write");
    let snapshot = generated.snapshot();
    let prefix = VirtualPath::user("dir").expect("prefix");
    assert_eq!(
        snapshot.list(&prefix, 3).expect("bound"),
        [
            VirtualPath::user("dir/a.tex").expect("path"),
            VirtualPath::user("dir/b.tex").expect("path"),
            VirtualPath::user("dir/c.tex").expect("path"),
        ]
    );
    assert_eq!(
        snapshot.list(&prefix, 2),
        Err(SnapshotError::EnumerationLimitExceeded { limit: 2 })
    );
}

#[test]
fn enumeration_and_identity_ignore_insertion_order_and_discarded_attempts() {
    let entries = [("c.tex", &b"c"[..]), ("a.tex", b"a"), ("b.tex", b"b")];
    let forward = workspace_with_users(&entries);
    let reverse = workspace_with_users(&entries.into_iter().rev().collect::<Vec<_>>());
    assert_eq!(
        forward.snapshot().list_root(VirtualRoot::Job, 8),
        reverse.snapshot().list_root(VirtualRoot::Job, 8)
    );
    assert_eq!(
        forward.snapshot().generation_identity(),
        reverse.snapshot().generation_identity()
    );

    let accepted = forward.snapshot();
    let mut workspace = forward;
    let mut attempt = workspace.begin_generated();
    attempt
        .write(
            VirtualPath::user("attempt.aux").expect("path"),
            b"discard".to_vec(),
        )
        .expect("write");
    attempt.discard();
    assert!(
        !accepted
            .contains(&VirtualPath::user("attempt.aux").expect("path"))
            .expect("live")
    );
}

#[test]
fn retention_counts_generation_bindings_and_stale_clones_fail_reads() {
    let mut workspace = workspace_with_users(&[("main.tex", b"1234")]);
    workspace
        .preload(ResolvedFile {
            request: FileRequestKey::new(FileKind::TexInput, "plain.tex").expect("request"),
            virtual_path: "/texlive/plain.tex".into(),
            bytes: b"123456".to_vec().into(),
            expected_digest: None,
        })
        .expect("resource");
    let snapshot = workspace.snapshot();
    assert_eq!(
        snapshot.retention(),
        SnapshotRetention {
            bindings: 2,
            logical_bytes: 10,
            input_bytes: 10,
            generated_bytes: 0
        }
    );
    let clone = snapshot.clone();
    let identity = snapshot.generation_identity();
    snapshot.invalidate();
    assert_eq!(
        clone.get(&VirtualPath::user("main.tex").expect("path")),
        Err(SnapshotError::Stale {
            generation: identity
        })
    );
    assert_eq!(clone.retention().logical_bytes, 10);
}
