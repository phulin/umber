use std::sync::Arc;

use tex_state::SourceId;

use super::{
    MalformedUnicodeRange, RegisteredSource, RegisteredSourceKind, SourceRegistration,
    SourceRegistrationError,
};
use crate::{CommandProfile, CommandState};

#[test]
fn unicode_registration_rejects_the_exact_malformed_range_before_allocation() {
    let mut state = CommandState::new(CommandProfile::unicode_extended(
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
        CommandProfile::unicode_extended(crate::CommandDialect::Pdftex14027),
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
fn exact_byte_registration_preserves_every_byte() {
    let bytes: Arc<[u8]> = (u8::MIN..=u8::MAX).collect::<Vec<_>>().into();
    let registered = RegisteredSource::register(
        SourceId::new(9),
        CommandProfile::PDFTEX14027,
        SourceRegistration::new(RegisteredSourceKind::ReadLine, Arc::clone(&bytes)),
    )
    .expect("exact-byte registration never decodes");

    assert_eq!(registered.bytes, bytes);
    assert_eq!(registered.kind, RegisteredSourceKind::ReadLine);
}

#[test]
fn opening_requires_a_retained_registration() {
    let mut state = CommandState::new(CommandProfile::TEX82);

    let error = state
        .open_registered_source(SourceId::new(77))
        .expect_err("unknown source must not create a cursor");

    assert_eq!(error.source(), SourceId::new(77));
    assert!(state.input.levels.is_empty());
}
