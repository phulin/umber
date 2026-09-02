use std::sync::Arc;
use std::{collections::hash_map::DefaultHasher, hash::Hash, hash::Hasher};

use tex_state::{SourceId, World};

use super::{
    MalformedUnicodeRange, RegisteredSource, RegisteredSourceKind, SourceRegistration,
    SourceRegistrationError,
};
use crate::{CommandProfile, CommandState};

#[test]
fn unicode_registration_rejects_the_exact_malformed_range_before_allocation() {
    let mut state = CommandState::<()>::new(CommandProfile::unicode_extended(
        crate::CommandDialect::Tex82,
    ));
    let malformed = SourceRegistration::new(
        RegisteredSourceKind::World,
        Arc::<[u8]>::from(&b"ok\xf0\x28\x8c\x28"[..]),
    );

    assert_eq!(
        state.register_source(malformed),
        Err(SourceRegistrationError::MalformedUnicode(
            MalformedUnicodeRange { start: 2, end: 3 }
        ))
    );

    let id = state
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(&b"valid"[..]),
        ))
        .expect("failed registration must not consume an identity");
    assert_eq!(id, SourceId::new(0));
}

#[test]
fn incomplete_unicode_sequence_reports_through_end_of_backing() {
    let error = RegisteredSource::register(
        SourceId::new(4),
        CommandProfile::unicode_extended(crate::CommandDialect::Pdftex14029),
        SourceRegistration::new(
            RegisteredSourceKind::EditorFragment,
            Arc::<[u8]>::from(&b"a\xe2\x82"[..]),
        ),
    )
    .expect_err("incomplete UTF-8 must be rejected");

    assert_eq!(
        error,
        SourceRegistrationError::MalformedUnicode(MalformedUnicodeRange { start: 1, end: 3 })
    );
}

#[test]
fn unicode_registration_rejects_overlong_surrogate_and_stray_continuations() {
    let malformed: &[(&[u8], u64, u64)] = &[
        (b"\xc0\xaf", 0, 1),
        (b"\xed\xa0\x80", 0, 1),
        (b"ok\x80", 2, 3),
    ];

    for &(bytes, start, end) in malformed {
        let error = RegisteredSource::register(
            SourceId::new(4),
            CommandProfile::unicode_extended(crate::CommandDialect::Tex82),
            SourceRegistration::new(RegisteredSourceKind::Generated, Arc::<[u8]>::from(bytes)),
        )
        .expect_err("malformed UTF-8 must be rejected");
        assert_eq!(
            error,
            SourceRegistrationError::MalformedUnicode(MalformedUnicodeRange { start, end })
        );
    }
}

#[test]
fn exact_byte_registration_preserves_every_byte() {
    let bytes: Arc<[u8]> = (u8::MIN..=u8::MAX).collect::<Vec<_>>().into();
    let registered = RegisteredSource::register(
        SourceId::new(9),
        CommandProfile::PDFTEX14029,
        SourceRegistration::new(RegisteredSourceKind::ReadLine, Arc::clone(&bytes)),
    )
    .expect("exact-byte registration never decodes");

    assert_eq!(registered.bytes.as_ref(), bytes.as_ref());
    assert_eq!(registered.kind, RegisteredSourceKind::ReadLine);
}

#[test]
fn every_registration_kind_is_retained_without_changing_backing() {
    for (raw, kind) in [
        (0, RegisteredSourceKind::World),
        (1, RegisteredSourceKind::Generated),
        (2, RegisteredSourceKind::EditorFragment),
        (3, RegisteredSourceKind::ReadLine),
    ] {
        let bytes = Arc::<[u8]>::from([raw, 0xff]);
        let registered = RegisteredSource::register(
            SourceId::new(raw.into()),
            CommandProfile::TEX82,
            SourceRegistration::new(kind, Arc::clone(&bytes)),
        )
        .expect("already acquired exact backing must register");

        assert_eq!(registered.kind, kind);
        assert_eq!(registered.bytes.as_ref(), bytes.as_ref());
    }
}

#[test]
fn generated_descriptor_is_retained_without_changing_source_semantics() {
    let bytes = Arc::<[u8]>::from(vec![b'x'; 64 * 1024]);
    let registered = RegisteredSource::register(
        SourceId::new(11),
        CommandProfile::TEX82,
        SourceRegistration::new(RegisteredSourceKind::Generated, Arc::clone(&bytes)),
    )
    .expect("generated source registers");
    let clone = registered.clone();

    let first = registered.source_descriptor();
    let second = registered.source_descriptor();
    let expected = tex_state::source_map::SourceDescriptor::generated(Arc::clone(&bytes));
    assert_eq!(first, expected);
    assert_eq!(second, expected);
    assert_eq!(registered, clone);

    let semantic_hash = |source: &RegisteredSource| {
        let mut hasher = DefaultHasher::new();
        source.hash(&mut hasher);
        hasher.finish()
    };
    assert_eq!(semantic_hash(&registered), semantic_hash(&clone));
}

#[test]
fn named_generated_descriptor_survives_editor_rebinding() {
    let bytes = Arc::<[u8]>::from(&b"first"[..]);
    let registered = RegisteredSource::register(
        SourceId::new(12),
        CommandProfile::TEX82,
        SourceRegistration::new(RegisteredSourceKind::Generated, bytes).with_name("/job/main.tex"),
    )
    .expect("named generated source registers");
    assert_eq!(
        registered.source_descriptor(),
        tex_state::source_map::SourceDescriptor::named_generated(
            "/job/main.tex",
            Arc::from(&b"first"[..]),
        )
    );

    let rebound = registered
        .rebind_generated(SourceId::new(12), (&b"second"[..]).into())
        .expect("named generated source rebinds");
    assert_eq!(
        rebound.source_descriptor(),
        tex_state::source_map::SourceDescriptor::editor_revision(
            Some("/job/main.tex"),
            Arc::from(&b"second"[..]),
        )
    );
}

#[test]
fn world_registration_retains_the_selected_input_record_for_provenance() {
    let mut world = World::memory();
    world
        .set_memory_file("child.tex", b"child")
        .expect("memory file is seeded");
    let content = world.read_file("child.tex").expect("world read succeeds");
    let record = content.record();
    let registered = RegisteredSource::register(
        SourceId::new(5),
        CommandProfile::TEX82,
        SourceRegistration::world(content),
    )
    .expect("world backing registers");

    assert!(matches!(
        registered.source_descriptor(),
        tex_state::source_map::SourceDescriptor::World {
            input_record,
            byte_len: 5,
        } if input_record == record
    ));
}

#[test]
fn world_registration_retains_file_enquiry_modification_metadata() {
    let mut world = World::memory();
    let date = tex_state::FileModificationDate::with_offset(
        tex_state::JobClock {
            year: 2026,
            month: 8,
            day: 2,
            time: 12 * 60 + 34,
            second: 56,
        },
        -300,
    );
    world
        .set_memory_file("child.tex", b"child")
        .expect("memory file is seeded");
    world
        .set_memory_file_modification_date("child.tex", date)
        .expect("date is seeded");
    let content = world.read_file("child.tex").expect("world read succeeds");

    let registration = SourceRegistration::world(content);

    assert_eq!(registration.modification_date(), Some(date));
}

#[test]
fn with_name_survives_registration_into_the_backing() {
    // §537's `a_make_name_string` name has to reach the opened level, not
    // just the registration that requested it.
    let registration = SourceRegistration::new(RegisteredSourceKind::Generated, b"x".to_vec())
        .with_name("child.tex");
    assert_eq!(registration.name(), Some("child.tex"));

    let registered = RegisteredSource::register(
        SourceId::new(1),
        CommandProfile::TEX82,
        registration.clone(),
    )
    .expect("named registration registers");
    assert_eq!(registered.name.as_deref(), Some("child.tex"));

    let unnamed = RegisteredSource::register(
        SourceId::new(2),
        CommandProfile::TEX82,
        SourceRegistration::new(RegisteredSourceKind::Generated, b"y".to_vec()),
    )
    .expect("unnamed registration registers");
    assert_eq!(unnamed.name, None);
    assert_ne!(registered, unnamed);
}

#[test]
fn opening_requires_a_retained_registration() {
    let mut state = CommandState::<()>::new(CommandProfile::TEX82);

    let error = state
        .open_registered_source(SourceId::new(77))
        .expect_err("unknown source must not create a cursor");

    assert_eq!(error.source(), SourceId::new(77));
    assert!(state.input.levels.is_empty());
}
