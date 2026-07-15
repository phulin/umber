#![cfg(feature = "profiling-stats")]

use tex_expand::{ExpansionContext, get_x_token_with_context};
use tex_lex::{InputStack, MemoryInput, TokenListReplayKind};
use tex_state::Universe;
use tex_state::macro_store::MacroMeaning;
use tex_state::meaning::MeaningFlags;
use tex_state::token::{Catcode, Token};

#[test]
fn warmed_macro_loop_performs_no_permanent_list_publication() {
    const CALLS: usize = 10_000;

    let mut universe = Universe::new();
    let macro_name = universe.intern("identity");
    let parameters = universe.intern_token_list(&[Token::param(1)]);
    let replacement = universe.intern_token_list(&[Token::param(1)]);
    universe.set_macro_meaning(
        macro_name,
        MacroMeaning::new(MeaningFlags::EMPTY, parameters, replacement),
    );
    let value = Token::Char {
        ch: 'x',
        cat: Catcode::Letter,
    };
    let invocation = universe.intern_token_list(
        &(0..CALLS)
            .flat_map(|_| [Token::Cs(macro_name.symbol()), value])
            .collect::<Vec<_>>(),
    );
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(invocation, TokenListReplayKind::Inserted);
    let before_tokens = tex_state::measurement::token_store_measurement();
    let before_origins = universe.provenance_stats();
    let mut expansion = ExpansionContext::new("texput");
    let mut delivered = 0;

    while get_x_token_with_context(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut universe),
        &mut expansion,
    )
    .expect("macro loop expands")
    .is_some()
    {
        delivered += 1;
    }

    let after_tokens = tex_state::measurement::token_store_measurement();
    let origin_growth = universe.provenance_stats().saturating_sub(before_origins);
    assert_eq!(delivered, CALLS);
    assert_eq!(after_tokens.intern_calls, before_tokens.intern_calls);
    assert_eq!(origin_growth.origin_list_spans(), 0);
    assert_eq!(origin_growth.origin_list_entries(), 0);
}
