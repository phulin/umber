//! Generation-scoped fixtures for crate-internal executor tests.

use tex_state::env::banks::IntParam;
use tex_state::env::{AssignmentScope, CodeTableKind};
use tex_state::interner::InternerBudget;
use tex_state::token::Catcode;
use tex_state::{
    CommandContext, GenerationBrand, GroupFrame, GroupKind, InteractionMode, StateError, Universe,
    World,
};

/// Generation-aware vocabulary shared by executor tests.
pub(crate) mod vocabulary {
    pub(crate) use tex_state::token::OriginId;
    pub(crate) use tex_state::{DependencyRegionError, ResolvedMeaning, TokenListId};
}

fn budget() -> InternerBudget {
    InternerBudget::new(16_384, 16_384, 1 << 20).expect("test interner budget")
}

/// Runs one test inside the fresh generation brand that owns every live id.
///
/// TeX82 §74 initializes interaction to `error_stop_mode`; this neutral
/// fixture preserves that production default.
pub(crate) fn with_universe<R>(
    test: impl for<'id> FnOnce(&mut Universe<GenerationBrand<'id>>) -> R,
) -> R {
    tex_state::with_universe(budget(), test).expect("test universe allocation")
}

/// Runs an unattended test with interaction explicitly set to Nonstop.
pub(crate) fn with_nonstop_universe<R>(
    test: impl for<'id> FnOnce(&mut Universe<GenerationBrand<'id>>) -> R,
) -> R {
    with_universe(|universe| {
        universe.set_interaction_mode(InteractionMode::Nonstop);
        test(universe)
    })
}

/// Runs one test with the production TeX82 INITEX profile installed.
///
/// This follows the same public primitive-installation path as
/// [`crate::MainControl::tex82_initex`]. The underlying [`with_universe`]
/// fixture deliberately remains neutral for state tests that exercise
/// pre-profile invariants.
pub(crate) fn with_tex82_universe<R>(
    test: impl for<'id> FnOnce(&mut Universe<GenerationBrand<'id>>) -> R,
) -> R {
    with_universe(|universe| {
        tex_command::install_tex82_expandable_primitives(universe);
        crate::install_unexpandable_primitives(universe);
        test(universe)
    })
}

/// Installs the TeX82 profile after explicitly selecting Nonstop interaction.
pub(crate) fn with_nonstop_tex82_universe<R>(
    test: impl for<'id> FnOnce(&mut Universe<GenerationBrand<'id>>) -> R,
) -> R {
    with_nonstop_universe(|universe| {
        tex_command::install_tex82_expandable_primitives(universe);
        crate::install_unexpandable_primitives(universe);
        test(universe)
    })
}

/// Runs one test with a caller-supplied host world inside the fresh brand.
pub(crate) fn with_world_universe<R>(
    world: World,
    test: impl for<'id> FnOnce(&mut Universe<GenerationBrand<'id>>) -> R,
) -> R {
    with_universe(|universe| {
        *universe.world_mut() = world;
        test(universe)
    })
}

/// Runs one test with an explicit in-memory host world inside the fresh brand.
pub(crate) fn with_memory_universe<R>(
    test: impl for<'id> FnOnce(&mut Universe<GenerationBrand<'id>>) -> R,
) -> R {
    with_world_universe(World::memory(), test)
}

/// Restores a format and tests it without allowing branded ids to escape.
///
/// The restoration operation is supplied by the format owner. This fixture
/// deliberately does not reconstruct the retired aggregate `Universe` format
/// constructor: restoration mutates the one freshly branded destination and
/// the test runs only after that operation succeeds.
pub(crate) fn with_restored_format_universe<E, R>(
    world: World,
    restore: impl for<'id> FnOnce(&mut Universe<GenerationBrand<'id>>) -> Result<(), E>,
    test: impl for<'id> FnOnce(&mut Universe<GenerationBrand<'id>>) -> R,
) -> Result<R, E> {
    with_world_universe(world, |universe| {
        restore(universe)?;
        Ok(test(universe))
    })
}

/// Runs test setup through one admitted command-state episode.
pub(crate) fn with_admitted<G, R>(
    universe: &mut Universe<G>,
    setup: impl FnOnce(&mut CommandContext<'_, G>) -> R,
) -> R {
    let mut context = universe.command_context().expect("command admission");
    setup(&mut context)
}

/// Assigns an integer parameter through the admitted state boundary.
pub(crate) fn assign_int_param<G>(
    universe: &mut Universe<G>,
    parameter: IntParam,
    value: i32,
    scope: AssignmentScope,
) -> Result<(), StateError> {
    with_admitted(universe, |context| {
        context.assign_int_param(parameter, value, scope)
    })
}

/// Runs related PDF fixture setup under one command admission.
pub(crate) fn with_pdf_setup<G, R>(
    universe: &mut Universe<G>,
    setup: impl FnOnce(&mut CommandContext<'_, G>) -> R,
) -> R {
    with_admitted(universe, setup)
}

/// Opens a group through admitted command state and returns its durable frame.
pub(crate) fn begin_group<G>(
    universe: &mut Universe<G>,
    kind: GroupKind,
    entered_line: u32,
) -> Result<GroupFrame, StateError> {
    with_admitted(universe, |context| context.begin_group(kind, entered_line))
}

/// Runs one test with plain.tex's printable category-code prelude installed.
pub(crate) fn with_plain_universe<R>(
    test: impl for<'id> FnOnce(&mut Universe<GenerationBrand<'id>>) -> R,
) -> R {
    with_tex82_universe(|universe| {
        install_plain_catcodes(universe);
        test(universe)
    })
}

/// Installs the Plain prelude after explicitly selecting Nonstop interaction.
pub(crate) fn with_nonstop_plain_universe<R>(
    test: impl for<'id> FnOnce(&mut Universe<GenerationBrand<'id>>) -> R,
) -> R {
    with_nonstop_tex82_universe(|universe| {
        install_plain_catcodes(universe);
        test(universe)
    })
}

fn install_plain_catcodes<G>(universe: &mut Universe<G>) {
    with_admitted(universe, |context| {
        for (character, catcode) in [
            ('{', Catcode::BeginGroup),
            ('}', Catcode::EndGroup),
            ('$', Catcode::MathShift),
            ('&', Catcode::AlignmentTab),
            ('#', Catcode::Parameter),
            ('^', Catcode::Superscript),
            ('_', Catcode::Subscript),
        ] {
            context
                .assign_code(
                    CodeTableKind::Catcode,
                    character,
                    i64::from(catcode as u8),
                    AssignmentScope::Global,
                )
                .expect("plain category code installs");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use tex_state::meaning::InternalInteger;

    fn assert_typed_vocabulary<G>(
        _tokens: Option<vocabulary::TokenListId<G>>,
        _origin: Option<vocabulary::OriginId>,
        _meaning: Option<vocabulary::ResolvedMeaning<G>>,
        _dependency_error: Option<vocabulary::DependencyRegionError>,
    ) {
    }

    #[test]
    fn shared_vocabulary_preserves_the_generation_brand() {
        with_universe(|_| assert_typed_vocabulary::<GenerationBrand<'_>>(None, None, None, None));
    }

    #[test]
    fn fresh_and_unattended_fixtures_choose_interaction_explicitly() {
        // TeX82 §74 initializes a fresh runnable engine in ErrorStop mode.
        with_universe(|universe| {
            assert_eq!(universe.interaction_mode(), InteractionMode::ErrorStop);
        });
        with_plain_universe(|universe| {
            assert_eq!(universe.interaction_mode(), InteractionMode::ErrorStop);
        });

        with_nonstop_universe(|universe| {
            assert_eq!(universe.interaction_mode(), InteractionMode::Nonstop);
        });
        with_nonstop_plain_universe(|universe| {
            assert_eq!(universe.interaction_mode(), InteractionMode::Nonstop);
        });
    }

    #[test]
    fn tex82_fixture_installs_the_profile_while_the_base_fixture_stays_neutral() {
        with_universe(|universe| {
            with_admitted(universe, |context| {
                assert_eq!(context.int_param(IntParam::ESCAPE_CHAR), 0);
            });
        });
        with_tex82_universe(|universe| {
            with_admitted(universe, |context| {
                assert_eq!(context.int_param(IntParam::ESCAPE_CHAR), i32::from(b'\\'));
            });
        });
    }

    #[test]
    fn supplied_memory_world_and_admitted_setup_preserve_values() {
        with_memory_universe(|universe| {
            assign_int_param(
                universe,
                IntParam::TRACING_ONLINE,
                7,
                AssignmentScope::Global,
            )
            .expect("integer fixture assignment");
            with_pdf_setup(universe, |context| context.set_pdf_return_value(19));
            begin_group(universe, GroupKind::Simple, 23).expect("group fixture setup");

            with_admitted(universe, |context| {
                assert_eq!(context.int_param(IntParam::TRACING_ONLINE), 7);
                assert_eq!(
                    context.internal_integer(InternalInteger::PdfReturnValue),
                    Some(19)
                );
                assert_eq!(context.execution_group_depth(), 1);
                assert_eq!(context.group_frames()[0].entered_line(), 23);
            });
        });
    }

    #[test]
    fn restored_fixture_completes_before_the_test_callback() {
        with_restored_format_universe(
            World::memory(),
            |universe| assign_int_param(universe, IntParam::YEAR, 2026, AssignmentScope::Global),
            |universe| {
                with_admitted(universe, |context| {
                    assert_eq!(context.int_param(IntParam::YEAR), 2026);
                });
            },
        )
        .expect("fixture restoration");
    }

    #[test]
    fn plain_fixture_installs_category_codes_under_admission() {
        with_plain_universe(|universe| {
            with_admitted(universe, |context| {
                assert_eq!(
                    context
                        .code(CodeTableKind::Catcode, '{')
                        .expect("plain catcode"),
                    i64::from(Catcode::BeginGroup as u8)
                );
            });
        });
    }
}
