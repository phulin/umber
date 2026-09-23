//! Generation-scoped fixtures for crate-internal command tests.

use tex_state::interner::InternerBudget;
use tex_state::token::{OriginId, Token, TracedTokenWord};
use tex_state::{CommandContext, GenerationBrand, Universe};

use crate::input::{PackedTokenSpanHandle, ReplayTrace, RetirementBehavior, TokenBehavior};
use crate::{
    CommandFuelLedger, CommandHostCapabilities, CommandHostContext, CommandProcessor, CommandState,
    CurrentCommand, DeliveryStatus,
};

/// Assert a successful raw delivery at tests that inspect its command value.
pub(crate) fn expect_raw_command<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
) -> CurrentCommand<G> {
    let mut destination = None;
    assert_eq!(
        processor
            .get_next_into(&mut destination)
            .expect("raw delivery"),
        DeliveryStatus::Command
    );
    destination.expect("raw delivery filled the caller slot")
}

/// Assert a successful source-token delivery at tests that inspect its value.
pub(crate) fn expect_token_command<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
) -> CurrentCommand<G> {
    let mut destination = None;
    assert_eq!(
        processor
            .get_token_into(&mut destination)
            .expect("token delivery"),
        DeliveryStatus::Command
    );
    destination.expect("token delivery filled the caller slot")
}

/// Assert a successful expanded delivery at tests that inspect its command.
pub(crate) fn expect_expanded_command<G>(
    processor: &mut CommandProcessor<'_, '_, G>,
) -> CurrentCommand<G> {
    let mut destination = None;
    assert_eq!(
        processor
            .get_x_token_into(&mut destination)
            .expect("expanded delivery"),
        DeliveryStatus::Command
    );
    destination.expect("expanded delivery filled the caller slot")
}

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
        PackedTokenSpanHandle::transient(tokens.into_iter().map(traced)),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
}

pub(crate) fn processor<'episode, 'admission, G>(
    command: &'episode mut CommandState<G>,
    context: &'episode mut CommandContext<'admission, G>,
    capabilities: &'episode mut CommandHostCapabilities,
    fuel: &'episode mut CommandFuelLedger,
    diagnostic_effects: &'episode mut tex_state::diagnostic::DiagnosticEffects,
) -> CommandProcessor<'episode, 'admission, G> {
    CommandProcessor::new(
        command,
        context,
        CommandHostContext::new(capabilities),
        fuel.fuel_mut(),
        None,
        diagnostic_effects,
    )
}
