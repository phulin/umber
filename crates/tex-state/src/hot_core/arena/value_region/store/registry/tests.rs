use core::mem::needs_drop;
use core::num::NonZeroU32;

use crate::glue::GlueSpec;
use crate::macro_store::MacroParameterPattern;
use crate::meaning::MeaningFlags;
use crate::scaled::Scaled;
use crate::token::{Catcode, OriginId, Token};
use crate::token_store::TokenSemanticId;

use super::*;

fn capacity(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).expect("test capacity is nonzero")
}

fn semantic(value: u64) -> TokenSemanticId {
    TokenSemanticId::testing(value)
}

fn token(ch: char) -> Token {
    Token::Char {
        ch,
        cat: Catcode::Other,
    }
}

fn glue(width: i32) -> GlueSpec {
    GlueSpec {
        width: Scaled::from_raw(width),
        ..GlueSpec::ZERO
    }
}

fn token_input<'a>(value: u64, tokens: &'a [Token]) -> RuntimeTokenValueInput<'a> {
    RuntimeTokenValueInput {
        semantic_id: semantic(value),
        tokens,
        provenance: &[],
    }
}

fn macro_input<'a>(
    parameter_text: TokenListId,
    replacement_text: TokenListId,
    definition_origin: OriginId,
) -> RuntimeMacroValueInput<'a> {
    RuntimeMacroValueInput {
        flags: MeaningFlags::EMPTY,
        parameter_pattern: MacroParameterPattern::from_tokens(&[]),
        parameter_text,
        replacement_text,
        definition_origin,
        parameter_origins: &[],
        replacement_origins: &[],
        observation_width: 7,
    }
}

#[test]
fn migrated_value_families_have_no_per_value_ownership_compatibility() {
    let sources = [
        include_str!("../../../../../token_store.rs"),
        include_str!("../../../../../macro_store.rs"),
        include_str!("../../../../../glue.rs"),
        include_str!("../registry.rs"),
        include_str!("../../store.rs"),
    ];
    for forbidden in [
        "Weak<",
        "Arc<()>",
        "_liveness",
        "allocation_events",
        "ReachableValueRef",
        "ReachableValuePool",
        "PackedTokenPair",
        "PackedMacroChunkOwner",
    ] {
        assert!(
            sources.iter().all(|source| !source.contains(forbidden)),
            "migrated family source contains forbidden compatibility {forbidden}"
        );
    }
}

#[test]
fn cold_exact_lookup_is_collision_safe_without_owning_candidates() {
    let mut registry =
        RuntimeValueRegistry::new(capacity(32), semantic(0)).expect("registry initializes");
    let left = [token('a')];
    let right = [token('b')];
    let left_id = registry
        .intern_token_list(RuntimeTokenValueInput {
            semantic_id: semantic(99),
            tokens: &left,
            provenance: &[],
        })
        .expect("left token list interns");
    let same_id = registry
        .intern_token_list(RuntimeTokenValueInput {
            semantic_id: semantic(99),
            tokens: &left,
            provenance: &[],
        })
        .expect("exact token list reuses identity");
    let collision_id = registry
        .intern_token_list(RuntimeTokenValueInput {
            semantic_id: semantic(99),
            tokens: &right,
            provenance: &[],
        })
        .expect("semantic collision remains distinct");
    assert_eq!(same_id, left_id);
    assert_ne!(collision_id, left_id);

    let first_glue = registry.intern_glue(glue(17)).expect("glue interns");
    let same_glue = registry.intern_glue(glue(17)).expect("glue reuses");
    let distinct_glue = registry.intern_glue(glue(18)).expect("glue differs");
    assert_eq!(same_glue, first_glue);
    assert_ne!(distinct_glue, first_glue);
}

#[test]
fn fixed_mark_is_copy_only_and_all_families_allocate_and_read() {
    assert!(!needs_drop::<RuntimeValueRegistryMark>());
    let mut registry =
        RuntimeValueRegistry::new(capacity(32), semantic(0)).expect("registry initializes");
    let unknown = OriginId::UNKNOWN;
    let values = [token('a'), token('b')];
    let tokens = registry
        .allocate_token_list(token_input(1, &values))
        .expect("token list allocates");
    let definition = registry
        .allocate_macro(macro_input(TokenListId::EMPTY, tokens, unknown))
        .expect("macro allocates");
    let glue = registry
        .allocate_glue(GlueSpec {
            width: Scaled::from_raw(13),
            ..GlueSpec::ZERO
        })
        .expect("glue allocates");

    assert_eq!(
        registry.token_list(tokens).expect("tokens read").tokens(),
        values
    );
    assert_eq!(
        registry
            .macro_definition(definition)
            .expect("macro reads")
            .replacement_text()
            .tokens(),
        values
    );
    assert_eq!(
        registry.glue(glue).expect("glue reads").spec().width,
        Scaled::from_raw(13)
    );
}

#[test]
fn published_roots_restore_before_reject_and_reused_slots_stay_stale() {
    let mut registry =
        RuntimeValueRegistry::new(capacity(16), semantic(0)).expect("registry initializes");
    let unknown = OriginId::UNKNOWN;
    let mut roots = registry.empty_published_store();
    let roots_mark = roots.publication_mark().expect("root mark fits");
    let mark = registry.mark().expect("registry mark exists");
    let values = [token('x')];
    let rejected_tokens = registry
        .allocate_token_list(token_input(2, &values))
        .expect("attempt token allocates");
    let rejected_macro = registry
        .allocate_macro(macro_input(TokenListId::EMPTY, rejected_tokens, unknown))
        .expect("attempt macro allocates");
    let rejected_glue = registry
        .allocate_glue(GlueSpec::ZERO)
        .expect("attempt glue allocates");
    registry
        .publish_into(&mut roots)
        .expect("attempt publishes");

    assert!(
        registry.rollback(mark).is_err(),
        "published owner must block unseal"
    );
    roots
        .restore_publication(roots_mark)
        .expect("destination roots restore first");
    registry.rollback(mark).expect("attempt rejects");

    let replacement_tokens = registry
        .allocate_token_list(token_input(3, &values))
        .expect("replacement token allocates");
    let replacement_macro = registry
        .allocate_macro(macro_input(TokenListId::EMPTY, replacement_tokens, unknown))
        .expect("replacement macro allocates");
    let replacement_glue = registry
        .allocate_glue(GlueSpec::ZERO)
        .expect("replacement glue allocates");
    assert_eq!(rejected_tokens.raw(), replacement_tokens.raw());
    assert_eq!(rejected_macro.raw(), replacement_macro.raw());
    assert_eq!(rejected_glue.raw(), replacement_glue.raw());
    assert_ne!(rejected_tokens, replacement_tokens);
    assert_ne!(rejected_macro, replacement_macro);
    assert_ne!(rejected_glue, replacement_glue);
    assert_eq!(
        registry.token_list(rejected_tokens).err(),
        Some(RuntimeValueRegistryError::UnknownTokenList)
    );
    assert_eq!(
        registry.macro_definition(rejected_macro).err(),
        Some(RuntimeValueRegistryError::UnknownMacroDefinition)
    );
    assert_eq!(
        registry.glue(rejected_glue).err(),
        Some(RuntimeValueRegistryError::UnknownGlue)
    );
}

#[test]
fn fork_shares_inherited_rows_and_rejects_foreign_suffix_ids() {
    let mut parent =
        RuntimeValueRegistry::new(capacity(16), semantic(0)).expect("registry initializes");
    let inherited_values = [token('i')];
    let inherited = parent
        .allocate_token_list(token_input(4, &inherited_values))
        .expect("inherited token allocates");
    let unknown = OriginId::UNKNOWN;
    let inherited_macro = parent
        .allocate_macro(macro_input(TokenListId::EMPTY, inherited, unknown))
        .expect("inherited macro allocates");
    let inherited_glue = parent
        .allocate_glue(glue(17))
        .expect("inherited glue allocates");
    let mut child = parent.fork().expect("cold fork succeeds");
    let parent_values = [token('p')];
    let child_values = [token('c')];
    let parent_only = parent
        .allocate_token_list(token_input(5, &parent_values))
        .expect("parent suffix allocates");
    let child_only = child
        .allocate_token_list(token_input(6, &child_values))
        .expect("child suffix allocates");

    assert_eq!(
        parent
            .token_list(inherited)
            .expect("parent inherits")
            .tokens(),
        inherited_values
    );
    assert_eq!(
        child
            .token_list(inherited)
            .expect("child inherits")
            .tokens(),
        inherited_values
    );
    assert_eq!(
        child
            .macro_definition(inherited_macro)
            .expect("child inherits macro")
            .meaning()
            .replacement_text(),
        inherited
    );
    assert_eq!(
        *child
            .glue(inherited_glue)
            .expect("child inherits glue")
            .spec(),
        glue(17)
    );
    assert!(child.token_list(parent_only).is_err());
    assert!(parent.token_list(child_only).is_err());
}

#[test]
fn all_live_growth_is_exact_in_locations_identities_and_region_values() {
    let mut registry =
        RuntimeValueRegistry::new(capacity(256), semantic(0)).expect("registry initializes");
    let unknown = OriginId::UNKNOWN;
    let before = registry.accounting();
    for cycle in 0..32_u64 {
        let values = [token('g')];
        let tokens = registry
            .allocate_token_list(token_input(10 + cycle, &values))
            .expect("token allocates");
        registry
            .allocate_macro(macro_input(TokenListId::EMPTY, tokens, unknown))
            .expect("macro allocates");
        registry
            .allocate_glue(GlueSpec::ZERO)
            .expect("glue allocates");
    }
    let after = registry.accounting();
    assert_eq!(after.token_locations - before.token_locations, 32);
    assert_eq!(after.macro_locations - before.macro_locations, 32);
    assert_eq!(after.glue_locations - before.glue_locations, 32);
    assert_eq!(after.identity_slots - before.identity_slots, 96);
    assert_eq!(
        after.regions.logical_values - before.regions.logical_values,
        192
    );
}

#[test]
fn ten_thousand_bounded_retries_reuse_retained_storage() {
    let mut registry =
        RuntimeValueRegistry::new(capacity(16), semantic(0)).expect("registry initializes");
    let unknown = OriginId::UNKNOWN;
    let mark = registry.mark().expect("retry mark exists");
    let values = [token('r')];
    let warm_tokens = registry
        .allocate_token_list(token_input(20, &values))
        .expect("warm token allocates");
    registry
        .allocate_macro(macro_input(TokenListId::EMPTY, warm_tokens, unknown))
        .expect("warm macro allocates");
    registry
        .allocate_glue(GlueSpec::ZERO)
        .expect("warm glue allocates");
    registry.rollback(mark).expect("warm retry rejects");
    let warm_tokens = registry
        .allocate_token_list(token_input(21, &values))
        .expect("second warm token allocates");
    registry
        .allocate_macro(macro_input(TokenListId::EMPTY, warm_tokens, unknown))
        .expect("second warm macro allocates");
    registry
        .allocate_glue(GlueSpec::ZERO)
        .expect("second warm glue allocates");
    registry.rollback(mark).expect("second warm retry rejects");
    let plateau = registry.accounting();

    for cycle in 0..10_000_u64 {
        let tokens = registry
            .allocate_token_list(token_input(100 + cycle, &values))
            .expect("retry token allocates");
        registry
            .allocate_macro(macro_input(TokenListId::EMPTY, tokens, unknown))
            .expect("retry macro allocates");
        registry
            .allocate_glue(GlueSpec::ZERO)
            .expect("retry glue allocates");
        registry.rollback(mark).expect("retry rejects");
    }

    assert_eq!(registry.accounting(), plateau);
}
