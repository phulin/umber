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

fn expand_definition_body(
    stores: &mut Universe,
    expansion: &mut ExpansionContext<'_>,
    source: &str,
) -> (Vec<Token>, Vec<OriginId>) {
    let mut input = InputStack::new(MemoryInput::new(source));
    let scanned = crate::scan::scan_toks_expanded(
        &mut input,
        &mut tex_state::ExpansionContext::new(stores),
        MeaningFlags::EMPTY,
        TracedTokenWord::pack(letter('d'), OriginId::UNKNOWN),
        expansion,
    )
    .expect("expanded definition body");
    let replacement = scanned.meaning().replacement_text();
    let origins = scanned.provenance().replacement_origins();
    (
        stores.tokens(replacement).to_vec(),
        if origins == tex_state::ids::OriginListId::EMPTY {
            vec![OriginId::UNKNOWN; stores.tokens(replacement).len()]
        } else {
            stores.origin_list(origins).to_vec()
        },
    )
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

#[test]
fn expanded_replay_episode_hits_and_rebinds_input_ordinals() {
    let mut stores = Universe::new();
    crate::install_expandable_primitives(&mut stores);
    install_one_argument_macro(&mut stores, "m", 'A', 'B', OriginId::UNKNOWN);
    let mut expansion = ExpansionContext::new("texput").memoizing(ExpansionMemoConfig::default());

    let (first, first_origins) = expand_definition_body(&mut stores, &mut expansion, "{\\m{x}}%");
    let (second, second_origins) =
        expand_definition_body(&mut stores, &mut expansion, "   {\\m{x}}%");
    assert_eq!(first, vec![letter('A'), letter('x'), letter('B')]);
    assert_eq!(second, first);
    assert_ne!(first_origins[1], OriginId::UNKNOWN);
    assert_ne!(first_origins[1], second_origins[1]);
    let stats = expansion.memo_stats().expect("memo stats enabled");
    assert_eq!(stats.episode_lookups, 2);
    assert_eq!(stats.episode_hits, 1);
    assert_eq!(stats.episode_misses, 1);
    assert_eq!(stats.expanded_tokens_reused, 3);
}

#[test]
fn episode_dependencies_ignore_unrelated_mutation_and_invalidate_meaning_change() {
    let mut stores = Universe::new();
    crate::install_expandable_primitives(&mut stores);
    let symbol = stores.intern("m");
    let empty = stores.intern_token_list(&[]);
    let first_body = stores.intern_token_list(&[letter('A')]);
    stores.set_macro_meaning(
        symbol,
        MacroMeaning::new(MeaningFlags::EMPTY, empty, first_body),
    );
    let mut expansion = ExpansionContext::new("texput").memoizing(ExpansionMemoConfig::default());

    assert_eq!(
        expand_definition_body(&mut stores, &mut expansion, "{\\m}%").0,
        vec![letter('A')]
    );
    stores.set_count(17, 99);
    assert_eq!(
        expand_definition_body(&mut stores, &mut expansion, "{\\m}%").0,
        vec![letter('A')]
    );
    let second_body = stores.intern_token_list(&[letter('B')]);
    stores.set_macro_meaning(
        symbol,
        MacroMeaning::new(MeaningFlags::EMPTY, empty, second_body),
    );
    assert_eq!(
        expand_definition_body(&mut stores, &mut expansion, "{\\m}%").0,
        vec![letter('B')]
    );
    let stats = expansion.memo_stats().expect("memo stats enabled");
    assert_eq!(stats.episode_hits, 1);
    assert_eq!(stats.episode_invalidations, 1);
}

#[test]
fn episode_register_dependency_invalidates_only_the_observed_cell() {
    let mut stores = Universe::new();
    crate::install_expandable_primitives(&mut stores);
    let count = stores.intern("observedcount");
    stores.set_meaning(count, tex_state::meaning::Meaning::CountRegister(0));
    stores.set_count(0, 12);
    let mut expansion = ExpansionContext::new("texput").memoizing(ExpansionMemoConfig::default());

    assert_eq!(
        expand_definition_body(&mut stores, &mut expansion, "{\\the\\observedcount}%").0,
        vec![
            Token::Char {
                ch: '1',
                cat: Catcode::Other
            },
            Token::Char {
                ch: '2',
                cat: Catcode::Other
            }
        ]
    );
    stores.set_count(1, 77);
    let _ = expand_definition_body(&mut stores, &mut expansion, "{\\the\\observedcount}%");
    stores.set_count(0, 13);
    assert_eq!(
        expand_definition_body(&mut stores, &mut expansion, "{\\the\\observedcount}%").0,
        vec![
            Token::Char {
                ch: '1',
                cat: Catcode::Other
            },
            Token::Char {
                ch: '3',
                cat: Catcode::Other
            }
        ]
    );
    let stats = expansion.memo_stats().expect("memo stats enabled");
    assert_eq!(stats.episode_hits, 1);
    assert_eq!(stats.episode_invalidations, 1);
}

#[test]
fn expansion_episode_hits_across_allocation_distinct_universes() {
    let mut first = Universe::new();
    crate::install_expandable_primitives(&mut first);
    install_one_argument_macro(&mut first, "m", 'A', 'B', OriginId::UNKNOWN);
    let mut expansion = ExpansionContext::new("texput").memoizing(ExpansionMemoConfig::default());
    let first_output = expand_definition_body(&mut first, &mut expansion, "{\\m{x}}%");

    let mut second = Universe::new();
    crate::install_expandable_primitives(&mut second);
    install_one_argument_macro(&mut second, "m", 'A', 'B', OriginId::UNKNOWN);
    let second_output = expand_definition_body(&mut second, &mut expansion, "{\\m{x}}%");
    assert_eq!(first_output.0, second_output.0);
    assert_eq!(
        expansion
            .memo_stats()
            .expect("memo stats enabled")
            .episode_hits,
        1
    );
}

#[test]
fn relaxed_interning_is_an_episode_barrier() {
    let mut stores = Universe::new();
    crate::install_expandable_primitives(&mut stores);
    let mut expansion = ExpansionContext::new("texput").memoizing(ExpansionMemoConfig::default());

    assert_eq!(
        expand_definition_body(&mut stores, &mut expansion, "{\\csname made\\endcsname}%").0,
        vec![Token::Cs(stores.intern("made").symbol())]
    );
    let _ = expand_definition_body(&mut stores, &mut expansion, "{\\csname made\\endcsname}%");
    let stats = expansion.memo_stats().expect("memo stats enabled");
    assert_eq!(stats.episode_hits, 0);
    assert_eq!(stats.episode_barrier_rejections, 2);
}

#[test]
fn episode_collision_and_malformed_entry_fall_back_to_cold_expansion() {
    let mut stores = Universe::new();
    crate::install_expandable_primitives(&mut stores);
    let mut expansion = ExpansionContext::new("texput").memoizing(ExpansionMemoConfig::default());
    expansion
        .memo
        .as_mut()
        .expect("memo cache enabled")
        .forced_candidate = Some(11);

    assert_eq!(
        expand_definition_body(&mut stores, &mut expansion, "{A}%").0,
        vec![letter('A')]
    );
    assert_eq!(
        expand_definition_body(&mut stores, &mut expansion, "{B}%").0,
        vec![letter('B')]
    );
    expansion
        .memo
        .as_mut()
        .expect("memo cache enabled")
        .episodes
        .iter_mut()
        .find(|entry| entry.key.input == vec![letter('A')])
        .expect("cached A episode")
        .origins
        .clear();
    assert_eq!(
        expand_definition_body(&mut stores, &mut expansion, "{A}%").0,
        vec![letter('A')]
    );

    let stats = expansion.memo_stats().expect("memo stats enabled");
    assert_eq!(stats.episode_hits, 0);
    assert_eq!(stats.episode_invalidations, 1);
    assert_eq!(stats.episode_misses, 3);
}
