use tex_state::interner::ControlSequenceKind;
use tex_state::interner::InternerBudget;
use tex_state::meaning::ResolvedMeaning;

use super::*;

#[test]
fn nullfont_identifier_is_a_distinct_frozen_control_sequence() {
    crate::test_harness::with_universe(|universe| {
        install_tex82_unexpandable_primitives(universe);
        let mut context = universe.command_context().expect("fresh context");
        let frozen = context
            .font_identifier_symbol(tex_state::font::NULL_FONT)
            .expect("TeX82 §553 frozen nullfont identifier");
        let ordinary = context.symbol("nullfont").expect("ordinary primitive");
        assert_ne!(frozen, ordinary);
        assert_eq!(
            context.control_sequence_kind(frozen),
            ControlSequenceKind::Internal
        );
        assert_eq!(
            context.meaning(frozen),
            ResolvedMeaning::Static(Meaning::Font(tex_state::font::NULL_FONT))
        );

        context
            .assign_resolved_meaning(
                ordinary,
                ResolvedMeaning::Static(Meaning::Relax),
                tex_state::AssignmentScope::Global,
            )
            .expect("redefine ordinary nullfont");
        assert_eq!(
            context.meaning(frozen),
            ResolvedMeaning::Static(Meaning::Font(tex_state::font::NULL_FONT)),
            "the font identifier is independent of the ordinary primitive"
        );
    });
}

#[test]
fn format_registration_preserves_a_present_nullfont_identifier() {
    crate::test_harness::with_universe(|universe| {
        install_tex82_unexpandable_primitives(universe);
        let frozen = universe
            .command_context()
            .expect("fresh context")
            .font_identifier_symbol(tex_state::font::NULL_FONT)
            .expect("fresh frozen identity");

        register_tex82_unexpandable_primitives(universe).expect("valid format activation");
        assert_eq!(
            universe
                .command_context()
                .expect("restored context")
                .font_identifier_symbol(tex_state::font::NULL_FONT),
            Some(frozen),
            "re-registration keeps the initialized identity"
        );

        let selected = {
            let mut context = universe.command_context().expect("live context");
            let selected = context.intern_control_sequence("selected");
            context.set_font_identifier_symbol(tex_state::font::NULL_FONT, selected);
            selected
        };
        register_tex82_unexpandable_primitives(universe).expect("valid format activation");
        assert_eq!(
            universe
                .command_context()
                .expect("restored context")
                .font_identifier_symbol(tex_state::font::NULL_FONT),
            Some(selected),
            "a nonempty serialized identifier must be preserved"
        );
    });
}

fn format_budget() -> InternerBudget {
    InternerBudget::new(16_384, 16_384, 1 << 20).expect("format test interner budget")
}

#[test]
fn fresh_nullfont_identifier_survives_format_bytes_and_registration() {
    let image = crate::test_harness::with_universe(|universe| {
        install_tex82_unexpandable_primitives(universe);
        universe.capture_format_image().expect("fresh TeX82 format")
    });
    let image = tex_state::DetachedFormatImage::try_from_bytes(image.into_bytes())
        .expect("serialized TeX82 format");
    tex_state::with_materialized_format(
        format_budget(),
        tex_state::World::memory(),
        image,
        |universe| {
            let original = universe
                .command_context()
                .expect("loaded context")
                .font_identifier_symbol(tex_state::font::NULL_FONT)
                .expect("serialized nullfont identifier");
            register_tex82_unexpandable_primitives(universe).expect("valid format activation");
            let context = universe.command_context().expect("registered context");
            assert_eq!(
                context.font_identifier_symbol(tex_state::font::NULL_FONT),
                Some(original)
            );
            assert_eq!(
                context.control_sequence_kind(original),
                ControlSequenceKind::Internal
            );
            assert_eq!(
                context.meaning(original),
                ResolvedMeaning::Static(Meaning::Font(tex_state::font::NULL_FONT))
            );
        },
    )
    .expect("materialize TeX82 format");
}

#[test]
fn incomplete_format_is_rejected_before_primitive_registration() {
    // A generic state image may be captured before TeX primitive installation.
    // It is not a valid production TeX format until the nullfont id is present.
    let image = crate::test_harness::with_universe(|universe| {
        assert_eq!(
            universe
                .command_context()
                .expect("fresh context")
                .font_identifier_symbol(tex_state::font::NULL_FONT),
            None
        );
        universe
            .capture_format_image()
            .expect("generic state image")
    });
    let image = tex_state::DetachedFormatImage::try_from_bytes(image.into_bytes())
        .expect("serialized generic state image");
    tex_state::with_materialized_format(
        format_budget(),
        tex_state::World::memory(),
        image,
        |universe| {
            assert_eq!(
                universe
                    .command_context()
                    .expect("loaded context")
                    .font_identifier_symbol(tex_state::font::NULL_FONT),
                None
            );
            assert_eq!(universe.primitive_meaning("relax"), None);
            let error = register_tex82_unexpandable_primitives(universe)
                .expect_err("incomplete TeX format must be rejected");
            assert!(matches!(error, tex_state::FormatError::InvalidState(_)));
            assert!(error.to_string().contains("nullfont identifier"));
            let context = universe.command_context().expect("rejected context");
            assert_eq!(
                context.font_identifier_symbol(tex_state::font::NULL_FONT),
                None,
                "rejection does not heal the missing identifier"
            );
            drop(context);
            assert_eq!(universe.primitive_meaning("relax"), None);
        },
    )
    .expect("materialize generic state image");
}
