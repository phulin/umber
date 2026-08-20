use core::mem::needs_drop;
use core::num::{NonZeroU32, NonZeroUsize};

use crate::glue::{GlueSpec, Order};
use crate::ids::{GlueId, MacroDefinitionId, TokenListId};
use crate::macro_store::MacroParameterPattern;
use crate::meaning::MeaningFlags;
use crate::scaled::Scaled;
use crate::token::{Catcode, OriginId, Token};
use crate::token_store::TokenSemanticId;

use super::*;

fn capacity(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).expect("test capacity is nonzero")
}

fn token(ch: char) -> Token {
    Token::Char {
        ch,
        cat: Catcode::Other,
    }
}

fn root(token_offset: u32, origin: u32) -> RuntimeOriginEntry {
    RuntimeOriginEntry::new(token_offset, OriginId::from_raw(origin))
}

fn token_input<'a>(
    id: u32,
    tokens: &'a [Token],
    roots: &'a [RuntimeOriginEntry],
) -> RuntimeTokenListInput<'a> {
    RuntimeTokenListInput {
        id: TokenListId::testing_new(id),
        semantic_id: TokenSemanticId::testing(u64::from(id)),
        tokens,
        provenance: roots,
    }
}

#[test]
fn coordinates_are_copy_only_and_admission_borrows_exact_payloads() {
    assert!(!needs_drop::<RuntimeTokenListCoordinate>());
    assert!(!needs_drop::<RuntimeMacroCoordinate>());
    assert!(!needs_drop::<RuntimeGlueCoordinate>());

    let values = [token('a'), token('b')];
    let roots = [root(1, 7)];
    let mut candidate = RuntimeValueStore::new(capacity(16))
        .candidate()
        .expect("candidate exists");
    let tokens = candidate
        .append_token_list(token_input(4, &values, &roots))
        .expect("token list appends");
    let glue = candidate
        .append_glue(
            GlueId::testing_new(3),
            GlueSpec {
                width: Scaled::from_raw(12),
                stretch: Scaled::from_raw(3),
                stretch_order: Order::Fil,
                shrink: Scaled::from_raw(1),
                shrink_order: Order::Normal,
            },
        )
        .expect("glue appends");
    let accepted = candidate.accept().expect("candidate accepts");

    let token_view = accepted
        .admit_token_list(tokens)
        .expect("token region admits once");
    assert_eq!(token_view.coordinate(), tokens);
    assert_eq!(token_view.semantic_id(), TokenSemanticId::testing(4));
    assert_eq!(token_view.tokens(), values);
    assert_eq!(token_view.provenance().len(), 1);
    assert_eq!(
        token_view.traced_word(0).expect("word exists").origin(),
        OriginId::UNKNOWN
    );
    assert_eq!(
        token_view.traced_word(1).expect("word exists").origin(),
        OriginId::from_raw(7)
    );
    let glue_view = accepted.admit_glue(glue).expect("glue region admits once");
    assert_eq!(glue_view.coordinate(), glue);
    assert_eq!(glue_view.spec().width, Scaled::from_raw(12));
}

#[test]
fn oversized_token_bundle_stays_inside_one_region() {
    let values = (0..32).map(|_| token('x')).collect::<Vec<_>>();
    let mut candidate = RuntimeValueStore::new(capacity(4))
        .candidate()
        .expect("candidate exists");
    let tokens = candidate
        .append_token_list(token_input(1, &values, &[]))
        .expect("oversized token list appends atomically");
    let accepted = candidate.accept().expect("candidate accepts");
    let view = accepted
        .admit_token_list(tokens)
        .expect("complete oversized row remains resolvable");
    assert_eq!(view.tokens(), values);
    assert_eq!(accepted.accounting().live_regions, 1);
}

#[test]
fn sparse_provenance_must_be_sorted_unique_and_inside_the_token_span() {
    let values = [token('p')];
    let invalid = [root(1, 7)];
    let mut candidate = RuntimeValueStore::new(capacity(4))
        .candidate()
        .expect("candidate exists");
    let before = candidate.accounting();
    assert_eq!(
        candidate.append_token_list(token_input(2, &values, &invalid)),
        Err(RegionArenaError::OffsetOutOfBounds)
    );
    assert_eq!(candidate.accounting(), before);
}

#[test]
fn macro_record_root_and_provenance_rows_publish_as_one_composite() {
    let parameter_tokens = [token('#'), Token::param(1)];
    let replacement_tokens = [token('z')];
    let definition_origin = OriginId::UNKNOWN;
    let parameter_origins = [root(1, 8)];
    let replacement_origins = [root(0, 9)];
    let mut candidate = RuntimeValueStore::new(capacity(64))
        .candidate()
        .expect("candidate exists");
    let parameter = candidate
        .append_token_list(token_input(10, &parameter_tokens, &[]))
        .expect("parameter text appends");
    let replacement = candidate
        .append_token_list(token_input(11, &replacement_tokens, &[]))
        .expect("replacement text appends");
    let definition = MacroDefinitionId::testing_new(7);
    let macro_coordinate = candidate
        .append_macro(RuntimeMacroInput {
            definition,
            flags: MeaningFlags::LONG,
            parameter_pattern: MacroParameterPattern::from_tokens(&[
                Token::Char {
                    ch: '#',
                    cat: Catcode::Parameter,
                },
                Token::param(1),
            ]),
            parameter_text: parameter,
            replacement_text: replacement,
            definition_origin,
            parameter_origins: &parameter_origins,
            replacement_origins: &replacement_origins,
            observation_operand: -19,
            allocation_serial: 23,
        })
        .expect("macro composite appends");
    assert_eq!(macro_coordinate.owner(), parameter.owner());
    let accepted = candidate.accept().expect("candidate accepts");

    let view = accepted
        .admit_macro(macro_coordinate)
        .expect("macro closure admits");
    assert_eq!(view.coordinate().id(), definition);
    assert_eq!(view.meaning().parameter_text(), parameter.id());
    assert_eq!(view.meaning().replacement_text(), replacement.id());
    assert_eq!(view.parameter_text().tokens(), parameter_tokens);
    assert_eq!(view.replacement_text().tokens(), replacement_tokens);
    assert_eq!(view.definition_origin(), OriginId::UNKNOWN);
    assert_eq!(view.parameter_origins().len(), 1);
    assert_eq!(view.replacement_origins().len(), 1);
    assert_eq!(view.observation_operand(), -19);
    assert_eq!(view.allocation_serial(), 23);
}

#[test]
fn counted_macro_root_set_retains_and_releases_region_multiplicity() {
    let values = [token('q')];
    let unknown = OriginId::UNKNOWN;
    let mut candidate = RuntimeValueStore::new(capacity(32))
        .candidate()
        .expect("candidate exists");
    let parameter = candidate
        .append_token_list(token_input(20, &[], &[]))
        .expect("parameter appends");
    let replacement = candidate
        .append_token_list(token_input(21, &values, &[]))
        .expect("replacement appends");
    let definition = candidate
        .append_macro(RuntimeMacroInput {
            definition: MacroDefinitionId::testing_new(2),
            flags: MeaningFlags::EMPTY,
            parameter_pattern: MacroParameterPattern::from_tokens(&[]),
            parameter_text: parameter,
            replacement_text: replacement,
            definition_origin: unknown,
            parameter_origins: &[],
            replacement_origins: &[],
            observation_operand: 2,
            allocation_serial: 2,
        })
        .expect("macro appends");
    let accepted = candidate.accept().expect("candidate accepts");
    let mut roots = accepted.empty_root_set();

    roots
        .retain_macro_from(&accepted, definition, NonZeroUsize::MIN)
        .expect("macro closure retains");
    assert_eq!(roots.testing_uses(definition.owner()), 3);
    roots
        .release_macro(definition, NonZeroUsize::MIN)
        .expect("macro closure releases");
    assert_eq!(roots.testing_uses(definition.owner()), 0);
    assert!(roots.admit_macro(definition).is_err());
}

#[test]
fn macro_root_set_retains_token_children_across_region_boundaries() {
    let values = [token('s')];
    let unknown = OriginId::UNKNOWN;
    let mut candidate = RuntimeValueStore::new(capacity(2))
        .candidate()
        .expect("candidate exists");
    let parameter = candidate
        .append_token_list(token_input(40, &[], &[]))
        .expect("parameter appends");
    let replacement = candidate
        .append_token_list(token_input(41, &values, &[]))
        .expect("replacement appends");
    let definition = candidate
        .append_macro(RuntimeMacroInput {
            definition: MacroDefinitionId::testing_new(5),
            flags: MeaningFlags::EMPTY,
            parameter_pattern: MacroParameterPattern::from_tokens(&[]),
            parameter_text: parameter,
            replacement_text: replacement,
            definition_origin: unknown,
            parameter_origins: &[],
            replacement_origins: &[],
            observation_operand: 5,
            allocation_serial: 5,
        })
        .expect("macro appends");
    assert_ne!(parameter.owner(), replacement.owner());
    assert_ne!(replacement.owner(), definition.owner());
    let accepted = candidate.accept().expect("candidate accepts");
    let mut roots = accepted.empty_root_set();

    roots
        .retain_macro_from(&accepted, definition, NonZeroUsize::MIN)
        .expect("cross-region macro closure retains");
    assert_eq!(roots.testing_uses(parameter.owner()), 1);
    assert_eq!(roots.testing_uses(replacement.owner()), 1);
    assert_eq!(roots.testing_uses(definition.owner()), 1);
    assert_eq!(
        roots
            .admit_macro(definition)
            .expect("retained closure admits")
            .replacement_text()
            .tokens(),
        values
    );
}

#[test]
fn rollback_rejects_published_coordinates_before_slot_reuse() {
    let values = [token('r')];
    let mut candidate = RuntimeValueStore::new(capacity(8))
        .candidate()
        .expect("candidate exists");
    let mark = candidate.mark().expect("mark exists");
    let rejected = candidate
        .append_token_list(token_input(30, &values, &[]))
        .expect("attempt appends");
    candidate.truncate(mark).expect("attempt rolls back");
    let replacement = candidate
        .append_token_list(token_input(31, &values, &[]))
        .expect("replacement appends");
    assert_ne!(rejected.owner(), replacement.owner());
    let accepted = candidate.accept().expect("replacement accepts");
    assert!(accepted.admit_token_list(rejected).is_err());
    assert_eq!(
        accepted
            .admit_token_list(replacement)
            .expect("replacement resolves")
            .tokens(),
        values
    );
}
