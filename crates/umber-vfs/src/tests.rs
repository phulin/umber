use std::path::Path;
use std::sync::Arc;

use proptest::prelude::*;

use super::{
    BuildId, DISTRIBUTION_LAYER_PRECEDENCE, FileKind, FileOrigin, FileRequestKey,
    ImmutableBindingError, InsertOutcome, JOB_LAYER_PRECEDENCE, LayerKind, LayeredFileStorage,
    ProducerId, StageId, VirtualFile, VirtualPath, VirtualPathError,
};

fn user_file(path: &str, bytes: &[u8]) -> VirtualFile {
    VirtualFile::new(
        VirtualPath::user(path).expect("user path"),
        Arc::<[u8]>::from(bytes),
        FileOrigin::User,
    )
}

fn resolved_file(path: &str, bytes: &[u8]) -> VirtualFile {
    let request = FileRequestKey::new(FileKind::TexInput, "plain.tex").expect("request");
    VirtualFile::new(
        VirtualPath::distribution(path).expect("distribution path"),
        Arc::<[u8]>::from(bytes),
        FileOrigin::Resolved(request),
    )
}

fn generated_file(path: &str, bytes: &[u8], producer: u64) -> VirtualFile {
    VirtualFile::new(
        VirtualPath::user(path).expect("generated path"),
        Arc::<[u8]>::from(bytes),
        FileOrigin::Generated {
            producer: ProducerId::new(producer),
            build: BuildId::new(7),
            stage: StageId::new(2),
        },
    )
}

fn assert_path(actual: Result<VirtualPath, VirtualPathError>, expected: &str) {
    let actual = actual.expect("valid virtual path");
    assert_eq!(actual.as_str(), expected);
    assert_eq!(actual.as_path(), Path::new(expected));
    assert_eq!(actual.to_string(), expected);
    assert_eq!(format!("{actual:?}"), format!("VirtualPath({expected:?})"));
}

fn assert_error(actual: Result<VirtualPath, VirtualPathError>, expected: &'static str) {
    let error = actual.expect_err("invalid virtual path");
    assert_eq!(error.message(), expected);
    assert_eq!(error.to_string(), expected);
    assert_eq!(error, VirtualPathError::new(expected));
    assert!(std::error::Error::source(&error).is_none());
}

#[test]
fn user_paths_preserve_the_existing_canonical_forms() {
    for (input, expected) in [
        ("main.tex", "/job/main.tex"),
        ("./parts//chapter.tex", "/job/parts/chapter.tex"),
        ("/job/main.tex", "/job/main.tex"),
        ("//job///./parts/./chapter.tex/", "/job/parts/chapter.tex"),
        ("job/main.tex", "/job/job/main.tex"),
        (".hidden", "/job/.hidden"),
        ("résumé/日本語.tex", "/job/résumé/日本語.tex"),
        ("e\u{301}.tex", "/job/e\u{301}.tex"),
        ("🦀.tex", "/job/🦀.tex"),
        ("a b/%23.tex", "/job/a b/%23.tex"),
    ] {
        assert_path(VirtualPath::user(input), expected);
    }
}

#[test]
fn distribution_paths_preserve_the_existing_canonical_forms() {
    for (input, expected) in [
        ("/texlive/plain.tex", "/texlive/plain.tex"),
        (
            "//texlive///tex/./plain//base/plain.tex/",
            "/texlive/tex/plain/base/plain.tex",
        ),
        ("/texlive/.hidden", "/texlive/.hidden"),
        ("/texlive/文書/é.tex", "/texlive/文書/é.tex"),
        ("/texlive/a b/%23.tex", "/texlive/a b/%23.tex"),
    ] {
        assert_path(VirtualPath::distribution(input), expected);
    }
}

#[test]
fn user_path_failures_retain_exact_error_categories_and_messages() {
    for (input, expected) in [
        ("", "path is empty"),
        ("/", "path does not name a file"),
        (".", "path does not name a file"),
        ("././", "path does not name a file"),
        ("/job", "path does not name a file"),
        ("/job///./", "path does not name a file"),
        ("..", "parent traversal is not allowed"),
        ("../secret.tex", "parent traversal is not allowed"),
        ("a/../secret.tex", "parent traversal is not allowed"),
        ("/job/../secret.tex", "parent traversal is not allowed"),
        (
            "/other/file.tex",
            "absolute path is outside its required virtual root",
        ),
        (
            "/Job/file.tex",
            "absolute path is outside its required virtual root",
        ),
        (
            "/texlive/file.tex",
            "absolute path is outside its required virtual root",
        ),
        (
            "https://example.test/a.tex",
            "NUL, backslash, colon, and URL-shaped paths are not allowed",
        ),
        (
            "C:/file.tex",
            "NUL, backslash, colon, and URL-shaped paths are not allowed",
        ),
        (
            "dir\\file.tex",
            "NUL, backslash, colon, and URL-shaped paths are not allowed",
        ),
        (
            "bad\0name.tex",
            "NUL, backslash, colon, and URL-shaped paths are not allowed",
        ),
    ] {
        assert_error(VirtualPath::user(input), expected);
    }
}

#[test]
fn distribution_path_failures_retain_exact_error_categories_and_messages() {
    for (input, expected) in [
        ("", "distribution paths must be absolute under /texlive"),
        (
            "plain.tex",
            "distribution paths must be absolute under /texlive",
        ),
        (
            "texlive/plain.tex",
            "distribution paths must be absolute under /texlive",
        ),
        ("/", "path does not name a file"),
        ("/texlive", "path does not name a file"),
        ("/texlive/./", "path does not name a file"),
        ("/../texlive/a.tex", "parent traversal is not allowed"),
        ("/texlive/a/../b.tex", "parent traversal is not allowed"),
        (
            "/job/file.tex",
            "absolute path is outside its required virtual root",
        ),
        (
            "/Texlive/file.tex",
            "absolute path is outside its required virtual root",
        ),
        (
            "/texlive/https://example.test/a.tex",
            "NUL, backslash, colon, and URL-shaped paths are not allowed",
        ),
        (
            "/texlive/dir\\file.tex",
            "NUL, backslash, colon, and URL-shaped paths are not allowed",
        ),
        (
            "/texlive/bad\0name.tex",
            "NUL, backslash, colon, and URL-shaped paths are not allowed",
        ),
    ] {
        assert_error(VirtualPath::distribution(input), expected);
    }
}

#[test]
fn ordering_and_cloning_use_canonical_bytes() {
    let canonical = VirtualPath::user("a//b.tex").expect("path");
    let equivalent = VirtualPath::user("./a/b.tex").expect("path");
    let later = VirtualPath::user("a/c.tex").expect("path");
    assert_eq!(canonical, equivalent);
    assert_eq!(canonical.clone(), canonical);
    assert!(canonical < later);
}

#[test]
fn virtual_files_share_bytes_and_separate_content_from_path_binding_identity() {
    let bytes = Arc::<[u8]>::from(&b"identical"[..]);
    let first = VirtualFile::new(
        VirtualPath::user("a.tex").expect("path"),
        Arc::clone(&bytes),
        FileOrigin::User,
    );
    let second = VirtualFile::new(
        VirtualPath::user("b.tex").expect("path"),
        Arc::clone(&bytes),
        FileOrigin::User,
    );

    assert!(Arc::ptr_eq(&first.shared_bytes(), &bytes));
    assert_eq!(first.bytes(), b"identical");
    assert_eq!(first.content_id(), second.content_id());
    assert_ne!(first.binding_id(), second.binding_id());
    assert_eq!(first.origin(), &FileOrigin::User);
    assert_eq!(first.path().as_str(), "/job/a.tex");
    assert_eq!(first.clone(), first);
}

#[test]
fn ownership_layers_and_lookup_precedence_are_explicit() {
    assert_eq!(
        JOB_LAYER_PRECEDENCE,
        [
            LayerKind::PendingGenerated,
            LayerKind::AcceptedGenerated,
            LayerKind::User
        ]
    );
    assert_eq!(DISTRIBUTION_LAYER_PRECEDENCE, [LayerKind::ResolvedResource]);

    let mut storage = LayeredFileStorage::new();
    assert_eq!(
        storage.insert(LayerKind::User, user_file("main.tex", b"user")),
        Ok(InsertOutcome::Inserted)
    );
    assert_eq!(
        storage.insert(
            LayerKind::ResolvedResource,
            resolved_file("/texlive/plain.tex", b"resource")
        ),
        Ok(InsertOutcome::Inserted)
    );
    assert_eq!(
        storage.insert(
            LayerKind::AcceptedGenerated,
            generated_file("main.aux", b"accepted", 1)
        ),
        Ok(InsertOutcome::Inserted)
    );
    assert_eq!(
        storage.insert(
            LayerKind::PendingGenerated,
            generated_file("main.aux", b"pending", 2)
        ),
        Ok(InsertOutcome::Inserted)
    );
    for kind in [
        LayerKind::User,
        LayerKind::ResolvedResource,
        LayerKind::AcceptedGenerated,
        LayerKind::PendingGenerated,
    ] {
        assert_eq!(storage.layer(kind).kind(), kind);
        assert_eq!(storage.layer(kind).len(), 1);
        assert!(!storage.layer(kind).is_empty());
    }
}

#[test]
fn layers_reject_wrong_roots_and_origins() {
    let mut storage = LayeredFileStorage::new();
    let wrong_origin = resolved_file("/texlive/plain.tex", b"plain");
    assert_eq!(
        storage.insert(LayerKind::User, wrong_origin),
        Err(ImmutableBindingError::WrongOrigin {
            layer: LayerKind::User,
            origin: FileOrigin::Resolved(
                FileRequestKey::new(FileKind::TexInput, "plain.tex").expect("request")
            ),
        })
    );

    let wrong_root = VirtualFile::new(
        VirtualPath::distribution("/texlive/plain.tex").expect("path"),
        Arc::<[u8]>::from(&b"plain"[..]),
        FileOrigin::User,
    );
    assert_eq!(
        storage.insert(LayerKind::User, wrong_root),
        Err(ImmutableBindingError::WrongRoot {
            layer: LayerKind::User,
            path: VirtualPath::distribution("/texlive/plain.tex").expect("path"),
        })
    );
}

#[test]
fn exact_duplicate_is_idempotent_and_every_immutable_conflict_fails() {
    let mut storage = LayeredFileStorage::new();
    let exact = user_file("main.tex", b"one");
    assert_eq!(
        storage.insert(LayerKind::User, exact.clone()),
        Ok(InsertOutcome::Inserted)
    );
    assert_eq!(
        storage.insert(LayerKind::User, exact),
        Ok(InsertOutcome::AlreadyPresent)
    );

    let different_bytes = user_file("main.tex", b"two");
    assert!(matches!(
        storage.insert(LayerKind::User, different_bytes),
        Err(ImmutableBindingError::Conflict {
            layer: LayerKind::User,
            ..
        })
    ));

    let different_origin = VirtualFile::new(
        VirtualPath::user("main.tex").expect("path"),
        Arc::<[u8]>::from(&b"one"[..]),
        FileOrigin::Generated {
            producer: ProducerId::new(1),
            build: BuildId::new(1),
            stage: StageId::new(1),
        },
    );
    assert_eq!(
        storage.insert(LayerKind::User, different_origin),
        Err(ImmutableBindingError::WrongOrigin {
            layer: LayerKind::User,
            origin: FileOrigin::Generated {
                producer: ProducerId::new(1),
                build: BuildId::new(1),
                stage: StageId::new(1),
            },
        })
    );

    let mut generated = LayeredFileStorage::new();
    generated
        .insert(
            LayerKind::AcceptedGenerated,
            generated_file("main.aux", b"one", 1),
        )
        .expect("first producer");
    assert!(matches!(
        generated.insert(
            LayerKind::AcceptedGenerated,
            generated_file("main.aux", b"one", 2)
        ),
        Err(ImmutableBindingError::OriginConflict {
            layer: LayerKind::AcceptedGenerated,
            ..
        })
    ));
}

#[test]
fn storage_identity_covers_layers_and_provenance() {
    let mut user = LayeredFileStorage::new();
    user.insert(LayerKind::User, user_file("same.tex", b"same"))
        .expect("insert user");

    let mut pending = LayeredFileStorage::new();
    pending
        .insert(
            LayerKind::PendingGenerated,
            generated_file("same.tex", b"same", 1),
        )
        .expect("insert generated");
    assert_ne!(user.identity(), pending.identity());

    let mut other_producer = LayeredFileStorage::new();
    other_producer
        .insert(
            LayerKind::PendingGenerated,
            generated_file("same.tex", b"same", 2),
        )
        .expect("insert generated");
    assert_ne!(pending.identity(), other_producer.identity());
    assert_eq!(pending.identity(), pending.clone().identity());
}

proptest! {
    #[test]
    fn every_accepted_user_path_is_canonical_and_confined(input in any::<String>()) {
        if let Ok(path) = VirtualPath::user(&input) {
            prop_assert!(path.as_str().starts_with("/job/"));
            prop_assert!(!path.as_str().contains("//"));
            prop_assert!(!path.as_str().split('/').any(|part| part == "." || part == ".."));
            prop_assert!(!path.as_str().contains(['\0', '\\', ':']));
            prop_assert_eq!(VirtualPath::user(path.as_str()), Ok(path));
        }
    }

    #[test]
    fn every_accepted_distribution_path_is_canonical_and_confined(input in any::<String>()) {
        if let Ok(path) = VirtualPath::distribution(&input) {
            prop_assert!(path.as_str().starts_with("/texlive/"));
            prop_assert!(!path.as_str().contains("//"));
            prop_assert!(!path.as_str().split('/').any(|part| part == "." || part == ".."));
            prop_assert!(!path.as_str().contains(['\0', '\\', ':']));
            prop_assert_eq!(VirtualPath::distribution(path.as_str()), Ok(path));
        }
    }

    #[test]
    fn storage_identity_ignores_allocation_and_insertion_order(
        entries in prop::collection::btree_map("[a-z]{1,8}\\.tex", prop::collection::vec(any::<u8>(), 0..64), 0..32)
    ) {
        let mut forward = LayeredFileStorage::new();
        let mut reverse = LayeredFileStorage::new();
        for (path, bytes) in &entries {
            forward.insert(LayerKind::User, user_file(path, bytes)).expect("unique insert");
        }
        for (path, bytes) in entries.iter().rev() {
            let independently_allocated = bytes.clone();
            reverse.insert(LayerKind::User, user_file(path, &independently_allocated)).expect("unique insert");
        }
        prop_assert_eq!(forward.identity(), reverse.identity());
    }
}
