use std::collections::BTreeMap;
use std::sync::Arc;

use proptest::prelude::*;

use super::*;

fn generated_file(path: &str, bytes: &[u8], producer: u64) -> VirtualFile {
    VirtualFile::new(
        VirtualPath::user(path).expect("generated path"),
        Arc::<[u8]>::from(bytes),
        FileOrigin::Generated {
            producer: ProducerId::new(producer),
            build: BuildId::new(1),
            stage: StageId::new(1),
        },
    )
}

fn fs_with_accepted(entries: &[(&str, &[u8], u64)]) -> VirtualFs {
    let mut storage = LayeredFileStorage::new();
    for (path, bytes, producer) in entries {
        storage
            .insert(
                LayerKind::AcceptedGenerated,
                generated_file(path, bytes, *producer),
            )
            .expect("unique accepted file");
    }
    VirtualFs::from_storage(storage, VfsLimits::default()).expect("valid accepted VFS")
}

fn snapshot_bytes<'a>(snapshot: &'a VfsSnapshot, path: &str) -> Option<&'a [u8]> {
    snapshot
        .get(&VirtualPath::user(path).expect("path"))
        .expect("live snapshot")
        .map(VirtualFile::bytes)
}

#[test]
fn stage_writes_are_private_until_finish_and_discard_stales_its_snapshot() {
    let mut fs = fs_with_accepted(&[]);
    let mut build = fs.begin_build(BuildPlan::new(BuildId::new(2)));
    let stage_snapshot = {
        let mut stage = build.begin_stage(ProducerId::new(10)).expect("stage");
        let snapshot = stage.snapshot();
        stage
            .write(
                VirtualPath::user("main.aux").expect("path"),
                b"partial".to_vec(),
            )
            .expect("private write");
        assert_eq!(snapshot_bytes(&snapshot, "main.aux"), None);
        stage.discard();
        snapshot
    };
    assert!(stage_snapshot.is_stale());
    assert_eq!(snapshot_bytes(&build.snapshot(), "main.aux"), None);

    let mut stage = build.begin_stage(ProducerId::new(10)).expect("stage");
    stage
        .write(
            VirtualPath::user("main.aux").expect("path"),
            b"complete".to_vec(),
        )
        .expect("write");
    let input = stage.snapshot();
    let commit = stage.finish().expect("commit");
    assert_eq!(commit.logical_bytes, 8);
    assert!(input.is_stale());
    assert_eq!(
        snapshot_bytes(&build.snapshot(), "main.aux"),
        Some(&b"complete"[..])
    );
}

#[test]
fn later_stages_read_prior_commits_and_same_producer_may_rewrite() {
    let mut fs = fs_with_accepted(&[]);
    let mut build = fs.begin_build(BuildPlan::new(BuildId::new(2)));
    let mut first = build.begin_stage(ProducerId::new(10)).expect("stage");
    first
        .write(
            VirtualPath::user("main.aux").expect("path"),
            b"one".to_vec(),
        )
        .expect("write");
    first.finish().expect("commit");

    let mut second = build.begin_stage(ProducerId::new(10)).expect("stage");
    assert_eq!(
        snapshot_bytes(&second.snapshot(), "main.aux"),
        Some(&b"one"[..])
    );
    second
        .write(
            VirtualPath::user("main.aux").expect("path"),
            b"two".to_vec(),
        )
        .expect("rewrite");
    second.finish().expect("commit");
    assert_eq!(
        snapshot_bytes(&build.snapshot(), "main.aux"),
        Some(&b"two"[..])
    );
}

#[test]
fn cross_producer_collision_is_atomic_and_requires_exact_declaration() {
    let path = VirtualPath::user("main.bbl").expect("path");
    let mut fs = fs_with_accepted(&[]);
    let mut plan = BuildPlan::new(BuildId::new(2));
    plan.declare_replacement(path.clone(), ProducerId::new(1), ProducerId::new(3))
        .expect("job declaration");
    let mut build = fs.begin_build(plan);

    let mut first = build.begin_stage(ProducerId::new(1)).expect("stage");
    first.write(path.clone(), b"first".to_vec()).expect("write");
    first.finish().expect("commit");

    let mut collision = build.begin_stage(ProducerId::new(2)).expect("stage");
    collision
        .write(
            VirtualPath::user("a-unique.log").expect("path"),
            b"leak".to_vec(),
        )
        .expect("private write");
    collision
        .write(path.clone(), b"wrong".to_vec())
        .expect("private write");
    assert_eq!(
        collision.finish(),
        Err(TransactionError::UndeclaredCollision {
            path: path.clone(),
            previous: ProducerId::new(1),
            replacing: ProducerId::new(2),
        })
    );
    let after_failure = build.snapshot();
    assert_eq!(
        snapshot_bytes(&after_failure, "main.bbl"),
        Some(&b"first"[..])
    );
    assert_eq!(snapshot_bytes(&after_failure, "a-unique.log"), None);

    let mut replacement = build.begin_stage(ProducerId::new(3)).expect("stage");
    replacement
        .write(path, b"declared".to_vec())
        .expect("write");
    replacement.finish().expect("declared replacement");
    assert_eq!(
        snapshot_bytes(&build.snapshot(), "main.bbl"),
        Some(&b"declared"[..])
    );
}

#[test]
fn build_discard_preserves_accepted_and_accept_replaces_the_whole_layer() {
    let mut fs = fs_with_accepted(&[("old.aux", b"old", 1)]);
    {
        let mut build = fs.begin_build(BuildPlan::new(BuildId::new(2)));
        let mut stage = build.begin_stage(ProducerId::new(2)).expect("stage");
        stage
            .write(
                VirtualPath::user("new.aux").expect("path"),
                b"discard".to_vec(),
            )
            .expect("write");
        stage.finish().expect("commit pending");
        build.discard();
    }
    assert_eq!(snapshot_bytes(&fs.snapshot(), "old.aux"), Some(&b"old"[..]));
    assert_eq!(snapshot_bytes(&fs.snapshot(), "new.aux"), None);

    let mut plan = BuildPlan::new(BuildId::new(3));
    plan.invalidate_accepted(VirtualPath::user("old.aux").expect("path"))
        .expect("job invalidation");
    let accepted = {
        let mut build = fs.begin_build(plan);
        assert_eq!(snapshot_bytes(&build.snapshot(), "old.aux"), None);
        let mut stage = build.begin_stage(ProducerId::new(2)).expect("stage");
        stage
            .write(
                VirtualPath::user("new.aux").expect("path"),
                b"accepted".to_vec(),
            )
            .expect("write");
        stage.finish().expect("commit pending");
        build.accept().expect("accept build")
    };
    assert_eq!(accepted.generated_files, 1);
    assert_eq!(accepted.logical_bytes, 8);
    assert_eq!(snapshot_bytes(&fs.snapshot(), "old.aux"), None);
    assert_eq!(
        snapshot_bytes(&fs.snapshot(), "new.aux"),
        Some(&b"accepted"[..])
    );
    assert!(fs.storage().layer(LayerKind::PendingGenerated).is_empty());
}

#[test]
fn stage_and_build_limits_fail_without_publication() {
    let stage_limits = VfsLimits {
        stage_bytes: 3,
        ..VfsLimits::default()
    };
    let mut fs = VirtualFs::new(stage_limits).expect("limits");
    let mut build = fs.begin_build(BuildPlan::new(BuildId::new(1)));
    let mut stage = build.begin_stage(ProducerId::new(1)).expect("stage");
    assert!(matches!(
        stage.write(
            VirtualPath::user("large.aux").expect("path"),
            b"four".to_vec()
        ),
        Err(TransactionError::Limit(VfsLimitError::LimitExceeded {
            kind: VfsLimitKind::StageBytes,
            ..
        }))
    ));
    stage.discard();
    assert_eq!(snapshot_bytes(&build.snapshot(), "large.aux"), None);
    build.discard();

    let build_limits = VfsLimits {
        stage_files: 2,
        generated_files: 1,
        ..VfsLimits::default()
    };
    let mut fs = VirtualFs::new(build_limits).expect("limits");
    let mut build = fs.begin_build(BuildPlan::new(BuildId::new(1)));
    let mut stage = build.begin_stage(ProducerId::new(1)).expect("stage");
    stage
        .write(VirtualPath::user("a").expect("path"), vec![])
        .expect("write");
    stage
        .write(VirtualPath::user("b").expect("path"), vec![])
        .expect("write");
    assert!(matches!(
        stage.finish(),
        Err(TransactionError::Limit(VfsLimitError::LimitExceeded {
            kind: VfsLimitKind::GeneratedFiles,
            ..
        }))
    ));
    assert!(
        build
            .snapshot()
            .list_root(crate::VirtualRoot::Job, 2)
            .expect("list")
            .is_empty()
    );
}

proptest! {
    #[test]
    fn accepted_build_equals_the_last_same_producer_write_per_path(
        writes in prop::collection::vec(
            ("[a-z]{1,6}\\.aux", prop::collection::vec(any::<u8>(), 0..64)),
            0..64,
        )
    ) {
        let expected = writes.iter().cloned().collect::<BTreeMap<_, _>>();
        let mut fs = VirtualFs::new(VfsLimits::default()).expect("VFS");
        let mut build = fs.begin_build(BuildPlan::new(BuildId::new(7)));
        for (path, bytes) in writes {
            let mut stage = build.begin_stage(ProducerId::new(9)).expect("stage");
            stage.write(VirtualPath::user(&path).expect("path"), bytes).expect("bounded write");
            stage.finish().expect("same-producer commit");
        }
        build.accept().expect("accept");
        let snapshot = fs.snapshot();
        for (path, bytes) in expected {
            prop_assert_eq!(snapshot_bytes(&snapshot, &path), Some(bytes.as_slice()));
        }
    }

    #[test]
    fn arbitrary_stage_byte_limits_are_exact_and_atomic(
        limit in 0usize..512,
        bytes in prop::collection::vec(any::<u8>(), 0..1024),
    ) {
        let limits = VfsLimits { stage_bytes: limit, ..VfsLimits::default() };
        let mut fs = VirtualFs::new(limits).expect("limits");
        let mut build = fs.begin_build(BuildPlan::new(BuildId::new(1)));
        let mut stage = build.begin_stage(ProducerId::new(1)).expect("stage");
        let result = stage.write(VirtualPath::user("out.aux").expect("path"), bytes.clone());
        prop_assert_eq!(result.is_ok(), bytes.len() <= limit);
        stage.discard();
        let snapshot = build.snapshot();
        prop_assert_eq!(snapshot_bytes(&snapshot, "out.aux"), None);
    }
}
