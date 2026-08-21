//! Generation-scoped fixtures for crate-internal command tests.

use tex_state::interner::InternerBudget;
use tex_state::token::{OriginId, Token, TracedTokenWord};
use tex_state::{GenerationBrand, Universe};

use crate::input::{ReplayTrace, RetirementBehavior, TokenBehavior, TokenPayload};
use crate::{CommandHostCapabilities, CommandHostContext, CommandProcessor, CommandState};

fn budget() -> InternerBudget {
    InternerBudget::new(16_384, 16_384, 1 << 20).expect("test interner budget")
}

/// Runs one test inside the fresh generation brand that owns every live id.
pub(crate) fn with_universe<R>(
    test: impl for<'id> FnOnce(&mut Universe<GenerationBrand<'id>>) -> R,
) -> R {
    tex_state::with_universe(budget(), |universe| {
        universe.set_interaction_mode(tex_state::InteractionMode::Nonstop);
        test(universe)
    })
    .expect("test universe allocation")
}

#[must_use]
pub(crate) fn traced(token: Token) -> TracedTokenWord {
    TracedTokenWord::pack(token, OriginId::UNKNOWN)
}

pub(crate) fn push<G>(command: &mut CommandState<G>, tokens: impl IntoIterator<Item = Token>) {
    command.push_token_level(
        TokenPayload::transient(tokens.into_iter().map(traced)),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
}

pub(crate) fn processor<'a, G>(
    command: &'a mut CommandState<G>,
    universe: &'a mut Universe<G>,
    capabilities: &'a mut CommandHostCapabilities,
) -> CommandProcessor<'a, 'a, G> {
    CommandProcessor::new(
        command,
        universe
            .command_context()
            .expect("admitted command context"),
        CommandHostContext::new(capabilities),
    )
}
