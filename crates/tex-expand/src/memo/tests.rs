use super::*;

use tex_lex::{InputStack, MemoryInput};
use tex_state::Universe;
use tex_state::macro_store::MacroMeaning;
use tex_state::meaning::MeaningFlags;
use tex_state::token::Catcode;

fn letter(ch: char) -> Token {
    Token::Char {
        ch,
        cat: Catcode::Letter,
    }
}

fn install_one_argument_macro(
    stores: &mut Universe,
    name: &str,
    prefix: char,
    suffix: char,
    body_origin: OriginId,
) {
    let symbol = stores.intern(name);
    let parameters = stores.intern_token_list(&[Token::param(1)]);
    let replacement = stores.intern_token_list(&[letter(prefix), Token::param(1), letter(suffix)]);
    let origins = stores.allocate_origin_list(&[body_origin, body_origin, body_origin]);
    stores.set_macro_meaning_with_provenance(
        symbol,
        MacroMeaning::new(MeaningFlags::EMPTY, parameters, replacement),
        MacroDefinitionProvenance::new(
            OriginId::UNKNOWN,
            tex_state::ids::OriginListId::EMPTY,
            origins,
        ),
    );
}

fn expand_all(
    stores: &mut Universe,
    expansion: &mut ExpansionContext<'_>,
    source: &str,
) -> Vec<TracedTokenWord> {
    let mut input = InputStack::new(MemoryInput::new(source));
    let mut output = Vec::new();
    loop {
        let token = crate::get_x_token_with_context(
            &mut input,
            &mut tex_state::ExpansionContext::new(stores),
            expansion,
        )
        .expect("memoized expansion");
        let Some(token) = token else { break };
        output.push(token);
    }
    output
}

#[test]
fn repeated_substitution_hits_and_rebinds_each_argument_origin() {
    let mut stores = Universe::new();
    crate::install_expandable_primitives(&mut stores);
    let body_origin = stores.source_origin(tex_state::SourceId::new(8), 80, 8, 8);
    install_one_argument_macro(&mut stores, "m", 'A', 'B', body_origin);
    let mut expansion = ExpansionContext::new("texput").memoizing(ExpansionMemoConfig::default());

    let output = expand_all(&mut stores, &mut expansion, "\\m{x}\\m{x}%");
    assert_eq!(
        output
            .iter()
            .filter_map(|word| word.token())
            .collect::<Vec<_>>(),
        vec![
            letter('A'),
            letter('x'),
            letter('B'),
            letter('A'),
            letter('x'),
            letter('B')
        ]
    );
    assert_eq!(output[0].origin(), body_origin);
    assert_eq!(output[3].origin(), body_origin);
    assert_ne!(output[1].origin(), OriginId::UNKNOWN);
    assert_ne!(output[1].origin(), output[4].origin());

    let stats = expansion.memo_stats().expect("memo stats enabled");
    assert_eq!(stats.substitution_lookups, 2);
    assert_eq!(stats.substitution_hits, 1);
    assert_eq!(stats.substitution_misses, 1);
    assert_eq!(stats.substituted_tokens_reused, 3);
    assert!(stats.retained_bytes > 0);
}

#[test]
fn equal_semantics_hit_across_universes_but_use_target_provenance() {
    let mut first = Universe::new();
    crate::install_expandable_primitives(&mut first);
    let first_origin = first.source_origin(tex_state::SourceId::new(1), 1, 1, 1);
    install_one_argument_macro(&mut first, "m", 'L', 'R', first_origin);

    let mut expansion = ExpansionContext::new("texput").memoizing(ExpansionMemoConfig::default());
    let first_output = expand_all(&mut first, &mut expansion, "\\m{a}%");
    assert_eq!(first_output[0].origin(), first_origin);

    let mut second = Universe::new();
    crate::install_expandable_primitives(&mut second);
    let _padding = second.source_origin(tex_state::SourceId::new(9), 90, 9, 9);
    let second_origin = second.source_origin(tex_state::SourceId::new(2), 2, 2, 2);
    install_one_argument_macro(&mut second, "m", 'L', 'R', second_origin);
    let second_output = expand_all(&mut second, &mut expansion, "\\m{a}%");
    assert_eq!(second_output[0].origin(), second_origin);
    assert_ne!(second_output[0].origin(), first_output[0].origin());
    assert_eq!(
        expansion
            .memo_stats()
            .expect("memo stats enabled")
            .substitution_hits,
        1
    );
}

#[test]
fn forced_candidate_collision_verifies_full_semantic_key() {
    let mut stores = Universe::new();
    crate::install_expandable_primitives(&mut stores);
    install_one_argument_macro(&mut stores, "left", 'L', '!', OriginId::UNKNOWN);
    install_one_argument_macro(&mut stores, "right", 'R', '?', OriginId::UNKNOWN);
    let mut expansion = ExpansionContext::new("texput").memoizing(ExpansionMemoConfig::default());
    expansion
        .memo
        .as_mut()
        .expect("memo cache enabled")
        .forced_candidate = Some(7);

    let output = expand_all(&mut stores, &mut expansion, "\\left{x}\\right{y}%");
    assert_eq!(
        output
            .iter()
            .filter_map(|word| word.token())
            .collect::<Vec<_>>(),
        vec![
            letter('L'),
            letter('x'),
            letter('!'),
            letter('R'),
            letter('y'),
            letter('?')
        ]
    );
    let stats = expansion.memo_stats().expect("memo stats enabled");
    assert_eq!(stats.substitution_misses, 2);
    assert_eq!(stats.substitution_hits, 0);
}

#[test]
fn entry_and_byte_budgets_evict_and_clear_to_baseline() {
    let mut stores = Universe::new();
    crate::install_expandable_primitives(&mut stores);
    install_one_argument_macro(&mut stores, "a", 'A', 'a', OriginId::UNKNOWN);
    install_one_argument_macro(&mut stores, "b", 'B', 'b', OriginId::UNKNOWN);
    let mut expansion = ExpansionContext::new("texput").memoizing(ExpansionMemoConfig {
        max_entries: 1,
        max_retained_bytes: 64 * 1024,
    });

    let _ = expand_all(&mut stores, &mut expansion, "\\a{x}\\b{x}%");
    let stats = expansion.memo_stats().expect("memo stats enabled");
    assert_eq!(stats.retained_entries, 1);
    assert_eq!(stats.evictions, 1);
    assert!(stats.retained_bytes > 0);
    expansion.clear_memoization();
    assert_eq!(
        expansion
            .memo_stats()
            .expect("memo stats enabled")
            .retained_entries,
        0
    );
    assert_eq!(
        expansion
            .memo_stats()
            .expect("memo stats enabled")
            .retained_bytes,
        0
    );
}
