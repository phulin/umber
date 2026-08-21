//! Generation-scoped fixtures for public-boundary executor tests.

use tex_state::env::{AssignmentScope, CodeTableKind};
use tex_state::interner::InternerBudget;
use tex_state::token::Catcode;
use tex_state::{GenerationBrand, InteractionMode, Universe};

fn budget() -> InternerBudget {
    InternerBudget::new(16_384, 16_384, 1 << 20).expect("test interner budget")
}

pub(crate) fn with_universe<R>(
    test: impl for<'id> FnOnce(&mut Universe<GenerationBrand<'id>>) -> R,
) -> R {
    tex_state::with_universe(budget(), |universe| {
        universe.set_interaction_mode(InteractionMode::Nonstop);
        test(universe)
    })
    .expect("test universe allocation")
}

pub(crate) fn with_plain_universe<R>(
    test: impl for<'id> FnOnce(&mut Universe<GenerationBrand<'id>>) -> R,
) -> R {
    with_universe(|universe| {
        let mut context = universe.command_context().expect("command admission");
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
        drop(context);
        test(universe)
    })
}
