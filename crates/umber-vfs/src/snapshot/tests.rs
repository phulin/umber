use std::sync::Arc;

use super::*;
use crate::{FileKind, FileOrigin, FileRequestKey, InsertOutcome, VirtualFile};

fn user_file(path: &str, bytes: &[u8]) -> VirtualFile {
    VirtualFile::new(
        VirtualPath::user(path).expect("user path"),
        Arc::<[u8]>::from(bytes),
        FileOrigin::User,
    )
}

fn resolved_file(path: &str, bytes: &[u8]) -> VirtualFile {
    VirtualFile::new(
        VirtualPath::distribution(path).expect("distribution path"),
        Arc::<[u8]>::from(bytes),
        FileOrigin::Resolved(
            FileRequestKey::new(
                FileKind::TexInput,
                path.strip_prefix("/texlive/").expect("distribution root"),
            )
            .expect("resource request"),
        ),
    )
}

fn generated_file(path: &str, bytes: &[u8]) -> VirtualFile {
    VirtualFile::new(
        VirtualPath::user(path).expect("generated path"),
        Arc::<[u8]>::from(bytes),
        FileOrigin::Generated,
    )
}

fn bytes<'a>(snapshot: &'a VfsSnapshot, path: &VirtualPath) -> Option<&'a [u8]> {
    snapshot
        .get(path)
        .expect("live snapshot")
        .map(VirtualFile::bytes)
}

#[test]
fn snapshot_clone_is_cheap_and_mutation_preserves_its_generation() {
    let mut storage = LayeredFileStorage::new();
    storage
        .insert(LayerKind::User, user_file("main.tex", b"old"))
        .expect("insert old file");
    let snapshot = storage.snapshot();
    let clone = snapshot.clone();
    assert!(Arc::ptr_eq(&snapshot.generation, &clone.generation));
    assert!(Arc::ptr_eq(&snapshot.valid, &clone.valid));

    storage
        .insert(LayerKind::User, user_file("later.tex", b"new"))
        .expect("insert new file");
    let current = storage.snapshot();
    assert!(!Arc::ptr_eq(&snapshot.generation, &current.generation));
    assert_eq!(
        bytes(&snapshot, &VirtualPath::user("main.tex").expect("path")),
        Some(&b"old"[..])
    );
    assert!(
        !snapshot
            .contains(&VirtualPath::user("later.tex").expect("path"))
            .expect("live snapshot")
    );
    assert!(
        storage
            .snapshot()
            .contains(&VirtualPath::user("later.tex").expect("path"))
            .expect("live snapshot")
    );
}

#[test]
fn exact_lookup_obeys_root_specific_precedence() {
    let mut storage = LayeredFileStorage::new();
    for (kind, file) in [
        (LayerKind::User, user_file("same.aux", b"user")),
        (
            LayerKind::AcceptedGenerated,
            generated_file("same.aux", b"accepted"),
        ),
        (
            LayerKind::PendingGenerated,
            generated_file("same.aux", b"pending"),
        ),
        (
            LayerKind::ResolvedResource,
            resolved_file("/texlive/plain.tex", b"plain"),
        ),
    ] {
        assert_eq!(storage.insert(kind, file), Ok(InsertOutcome::Inserted));
    }
    let path = VirtualPath::user("same.aux").expect("path");
    assert_eq!(bytes(&storage.snapshot(), &path), Some(&b"pending"[..]));
    assert_eq!(
        bytes(
            &storage.snapshot(),
            &VirtualPath::distribution("/texlive/plain.tex").expect("path")
        ),
        Some(&b"plain"[..])
    );
}

#[test]
fn lexical_enumeration_is_visible_unique_component_aware_and_bounded() {
    let mut storage = LayeredFileStorage::new();
    for path in ["z.tex", "dir/c.tex", "directory/no.tex", "dir/a.tex"] {
        storage
            .insert(LayerKind::User, user_file(path, path.as_bytes()))
            .expect("unique user file");
    }
    storage
        .insert(
            LayerKind::AcceptedGenerated,
            generated_file("dir/a.tex", b"accepted"),
        )
        .expect("accepted shadow");
    storage
        .insert(
            LayerKind::PendingGenerated,
            generated_file("dir/b.tex", b"pending"),
        )
        .expect("pending file");

    let snapshot = storage.snapshot();
    let prefix = VirtualPath::user("dir").expect("prefix");
    let listed = snapshot.list(&prefix, 3).expect("exact bound");
    assert_eq!(
        listed,
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
    assert_eq!(
        snapshot
            .list(&VirtualPath::user("missing").expect("prefix"), 0)
            .expect("empty enumeration"),
        Vec::<VirtualPath>::new()
    );
}

#[test]
fn enumeration_and_reads_ignore_insertion_order_and_discarded_attempts() {
    let entries = [("c.tex", b"c"), ("a.tex", b"a"), ("b.tex", b"b")];
    let mut forward = LayeredFileStorage::new();
    let mut reverse = LayeredFileStorage::new();
    for (path, data) in entries {
        forward
            .insert(LayerKind::User, user_file(path, data))
            .expect("forward insert");
    }
    for (path, data) in entries.into_iter().rev() {
        reverse
            .insert(LayerKind::User, user_file(path, data))
            .expect("reverse insert");
    }
    assert_eq!(
        forward.snapshot().list_root(VirtualRoot::Job, 8),
        reverse.snapshot().list_root(VirtualRoot::Job, 8)
    );
    assert_eq!(forward.identity(), reverse.identity());

    let accepted = forward.snapshot();
    let mut attempt = forward.clone();
    attempt
        .insert(
            LayerKind::PendingGenerated,
            generated_file("attempt.aux", b"discard me"),
        )
        .expect("pending attempt");
    drop(attempt);
    assert!(
        !accepted
            .contains(&VirtualPath::user("attempt.aux").expect("path"))
            .expect("live accepted snapshot")
    );
}

#[test]
fn retention_counts_all_generation_bindings_and_stale_clones_fail_reads() {
    let mut storage = LayeredFileStorage::new();
    storage
        .insert(LayerKind::User, user_file("main.tex", b"1234"))
        .expect("user");
    storage
        .insert(
            LayerKind::ResolvedResource,
            resolved_file("/texlive/plain.tex", b"123456"),
        )
        .expect("resource");
    let snapshot = storage.snapshot();
    assert_eq!(
        snapshot.retention(),
        SnapshotRetention {
            bindings: 2,
            logical_bytes: 10,
            input_bytes: 10,
            generated_bytes: 0,
        }
    );

    let clone = snapshot.clone();
    let identity = snapshot.generation_identity();
    snapshot.invalidate();
    assert!(snapshot.is_stale());
    assert!(clone.is_stale());
    let path = VirtualPath::user("main.tex").expect("path");
    assert_eq!(
        clone.get(&path),
        Err(SnapshotError::Stale {
            generation: identity
        })
    );
    assert_eq!(
        clone.list(&path, 1),
        Err(SnapshotError::Stale {
            generation: identity
        })
    );
    assert_eq!(clone.retention().logical_bytes, 10);
}
