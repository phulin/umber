use super::{PrepareMagDiagnostic, Stores};
use crate::env::banks::{DimenParam, GlueParam, IntParam};
use crate::font::NULL_FONT;
use crate::glue::{GlueSpec, Order};
use crate::hyphenation::{ExceptionSpec, PatternSpec};
use crate::ids::{ArenaRef, GlueId, NodeListId, OriginListId};
use crate::macro_store::{MacroDefinitionProvenance, MacroMeaning};
use crate::math::{
    FractionThickness, MathChoice, MathField, MathFraction, MathListNode, MathNoad, MathStyle,
    NoadClass, NoadKind,
};
use crate::meaning::Meaning;
use crate::meaning::MeaningFlags;
use crate::node::{
    BoxNode, BoxNodeFields, DiscKind, GlueKind, KernKind, LeaderPayload, Node, Sign, UnsetKind,
    UnsetNode, UnsetNodeFields, Whatsit,
};
use crate::scaled::{GlueSetRatio, Scaled};
use crate::source_map::SourceDescriptor;
use crate::state_hash::StateHasher;
use crate::token::{Catcode, OriginId, Token, TracedTokenWord};
use crate::world::InputRecordId;
use crate::{
    input::SourceId,
    provenance::{
        InsertedOrigin, InsertedOriginKind, MacroInvocationOrigin, OriginRecord, SourceOrigin,
        SynthesizedOrigin, SynthesizedOriginKind, SyntheticOrigin, SyntheticOriginKind,
    },
};

#[test]
fn rollback_restores_env_and_interner_as_one_tuple() {
    let mut stores = Stores::new();
    let kept = stores.intern("kept");
    stores.set_meaning(kept, Meaning::Relax);
    let snapshot = stores.checkpoint();

    let temporary = stores.intern("temporary");
    stores.set_meaning(temporary, Meaning::CharGiven('x'));

    stores.rollback(&snapshot);

    assert_eq!(stores.resolve(kept), "kept");
    assert_eq!(stores.meaning(kept), Meaning::Relax);
    let reused = stores.intern("temporary");
    assert_eq!(reused.raw(), temporary.raw());
    assert_eq!(stores.meaning(reused), Meaning::Undefined);
}

#[test]
fn owned_and_borrowed_semantic_hash_paths_match_every_node_variant() {
    let mut stores = Stores::new();
    let empty = stores.freeze_node_list(&[]);
    let tokens = stores.intern_token_list(&[]);
    let box_node = BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(1),
        height: Scaled::from_raw(2),
        depth: Scaled::from_raw(3),
        shift: Scaled::from_raw(4),
        display: true,
        glue_set: GlueSetRatio::from_raw(5),
        glue_sign: Sign::Shrinking,
        glue_order: Order::Fill,
        children: empty,
    });
    let nodes = vec![
        Node::Char {
            font: NULL_FONT,
            ch: 'x',
        },
        Node::Lig {
            font: NULL_FONT,
            ch: 'f',
            orig: ('f', 'i'),
        },
        Node::Kern {
            amount: Scaled::from_raw(-6),
            kind: KernKind::Mu,
        },
        Node::Glue {
            spec: GlueId::ZERO,
            kind: GlueKind::Leaders,
            leader: Some(LeaderPayload::Rule {
                width: Some(Scaled::from_raw(7)),
                height: None,
                depth: Some(Scaled::from_raw(8)),
            }),
        },
        Node::Penalty(-9),
        Node::Rule {
            width: None,
            height: Some(Scaled::from_raw(10)),
            depth: None,
        },
        Node::HList(box_node),
        Node::VList(box_node),
        Node::Unset(UnsetNode::new(UnsetNodeFields {
            kind: UnsetKind::VBox,
            width: Scaled::from_raw(11),
            height: Scaled::from_raw(12),
            depth: Scaled::from_raw(13),
            span_count: 2,
            stretch: Scaled::from_raw(14),
            stretch_order: Order::Filll,
            shrink: Scaled::from_raw(15),
            shrink_order: Order::Fil,
            children: empty,
        })),
        Node::Disc {
            kind: DiscKind::AutomaticHyphen,
            pre: empty,
            post: empty,
            replace: empty,
        },
        Node::Mark { class: 3, tokens },
        Node::Ins {
            class: 4,
            size: Scaled::from_raw(16),
            split_top_skip: GlueId::ZERO,
            split_max_depth: Scaled::from_raw(17),
            floating_penalty: -18,
            content: empty,
        },
        Node::Whatsit(Whatsit::Language {
            language: 19,
            left_hyphen_min: 2,
            right_hyphen_min: 3,
        }),
        Node::MathOn(Scaled::from_raw(20)),
        Node::MathOff(Scaled::from_raw(21)),
        Node::MathNoad(MathNoad::new(
            NoadKind::Normal(NoadClass::Ord),
            MathField::SubMlist(empty),
        )),
        Node::FractionNoad(MathFraction {
            numerator: empty,
            denominator: empty,
            thickness: FractionThickness::Explicit(Scaled::from_raw(22)),
            left_delimiter: Some(23),
            right_delimiter: None,
        }),
        Node::MathStyle(MathStyle::ScriptScript),
        Node::MathChoice(MathChoice {
            display: empty,
            text: empty,
            script: empty,
            script_script: empty,
        }),
        Node::MathList(MathListNode {
            display: true,
            content: empty,
        }),
        Node::Nonscript,
        Node::Adjust(empty),
    ];
    let id = stores.freeze_node_list(&nodes);
    stores.testing_assert_owned_borrowed_node_hashes_equal(id);
}

#[test]
fn node_semantic_ids_are_canonical_and_compose_from_children() {
    fn nested(stores: &mut Stores, penalty: i32) -> (NodeListId, NodeListId) {
        let child = stores.freeze_node_list(&[Node::Penalty(penalty)]);
        let root = stores.freeze_node_list(&[Node::Adjust(child)]);
        (child, root)
    }

    let mut direct = Stores::new();
    let (direct_child, direct_root) = nested(&mut direct, 10);

    let mut shifted = Stores::new();
    let _unrelated = shifted.freeze_node_list(&[Node::Penalty(999)]);
    let (shifted_child, shifted_root) = nested(&mut shifted, 10);
    let (_, different_root) = nested(&mut shifted, 11);

    assert_ne!(direct_child, shifted_child, "runtime allocation differs");
    assert_eq!(
        direct.node_semantic_id(direct_child),
        shifted.node_semantic_id(shifted_child)
    );
    assert_eq!(
        direct.node_semantic_id(direct_root),
        shifted.node_semantic_id(shifted_root)
    );
    assert_ne!(
        shifted.node_semantic_id(shifted_root),
        shifted.node_semantic_id(different_root)
    );

    let mut builder = shifted.node_list_builder();
    builder.push(Node::Adjust(shifted_child));
    let built_root = shifted.finish_node_list(&mut builder);
    assert_eq!(
        shifted.node_semantic_id(built_root),
        shifted.node_semantic_id(shifted_root)
    );

    let mut fork = direct.clone();
    assert_eq!(
        fork.node_semantic_id(direct_root),
        direct.node_semantic_id(direct_root)
    );
    let (_, fork_root) = nested(&mut fork, 10);
    assert_eq!(
        fork.node_semantic_id(fork_root),
        direct.node_semantic_id(direct_root)
    );
}

#[test]
fn node_semantic_ids_follow_rollback_promotion_and_epoch_clone() {
    let mut stores = Stores::new();
    let snapshot = stores.checkpoint();
    let stale = stores.freeze_node_list(&[Node::Penalty(1)]);
    let stale_semantic_id = stores.node_semantic_id(stale);
    stores.rollback(&snapshot);

    let replacement = stores.freeze_node_list(&[Node::Penalty(2)]);
    assert_ne!(stale, replacement);
    assert_ne!(stale_semantic_id, stores.node_semantic_id(replacement));
    assert!(std::panic::catch_unwind(|| stores.node_semantic_id(stale)).is_err());

    let root = stores.freeze_node_list(&[Node::Adjust(replacement)]);
    let semantic_id = stores.node_semantic_id(root);
    stores.set_box_reg(0, root);
    let survivor = stores.box_reg(0).expect("box assignment promotes the list");
    assert_eq!(stores.node_semantic_id(survivor), semantic_id);

    let epoch_clone = stores.clone_node_list_to_epoch(survivor);
    assert_eq!(stores.node_semantic_id(epoch_clone), semantic_id);
}

#[test]
fn node_semantic_ids_exclude_token_provenance() {
    let mut stores = Stores::new();
    let token = Token::Char {
        ch: 'x',
        cat: Catcode::Other,
    };
    let first_origin = stores.synthetic_origin(SyntheticOriginKind::Test);
    let second_origin = stores.synthetic_origin(SyntheticOriginKind::Engine);
    let first_tokens =
        stores.finish_traced_token_list(&[TracedTokenWord::pack(token, first_origin)]);
    let second_tokens =
        stores.finish_traced_token_list(&[TracedTokenWord::pack(token, second_origin)]);
    assert_ne!(first_tokens.origin_list(), second_tokens.origin_list());

    let first = stores.freeze_node_list(&[Node::Mark {
        class: 0,
        tokens: first_tokens.token_list(),
    }]);
    let second = stores.freeze_node_list(&[Node::Mark {
        class: 0,
        tokens: second_tokens.token_list(),
    }]);
    assert_ne!(first, second);
    assert_eq!(
        stores.node_semantic_id(first),
        stores.node_semantic_id(second)
    );
}

#[test]
fn semantic_projection_visits_only_outer_nodes() {
    let mut stores = Stores::new();
    let mut nested = stores.freeze_node_list(&[Node::Penalty(1)]);
    for _ in 0..512 {
        nested = stores.freeze_node_list(&[Node::Adjust(nested)]);
    }

    let outer = [Node::Adjust(nested), Node::Penalty(2)];
    let mut hasher = StateHasher::new(0x6f75_7465_725f_6e64);
    let visits = stores.hash_node_slice_semantic(&outer, &mut hasher);
    assert_eq!(visits, outer.len());

    let mut equivalent = Stores::new();
    let mut equivalent_nested = equivalent.freeze_node_list(&[Node::Penalty(1)]);
    for _ in 0..512 {
        equivalent_nested = equivalent.freeze_node_list(&[Node::Adjust(equivalent_nested)]);
    }
    let mut equivalent_hasher = StateHasher::new(0x6f75_7465_725f_6e64);
    let equivalent_visits = equivalent.hash_node_slice_semantic(
        &[Node::Adjust(equivalent_nested), Node::Penalty(2)],
        &mut equivalent_hasher,
    );
    assert_eq!(equivalent_visits, outer.len());
    assert_eq!(hasher.finish(), equivalent_hasher.finish());
}

#[test]
fn semantic_hash_scratch_reuses_capacity_but_store_clone_does_not_copy_it() {
    let mut stores = Stores::new();
    let symbols = (0..64)
        .map(|index| stores.intern(&format!("hash-scratch-{index}")))
        .collect::<Vec<_>>();
    let cursor = stores.state_hash_cursor();
    for (index, symbol) in symbols.into_iter().enumerate() {
        stores.set_meaning(
            symbol,
            Meaning::CharGiven(char::from(b'a' + (index % 26) as u8)),
        );
    }
    let end = stores.checkpoint();
    let _ = stores.state_hash_slice(&cursor, &end);

    let retained = stores.semantic_hash_cache.testing_scratch_capacities();
    assert!(retained.0 > 0);
    assert!(retained.1 > 0);
    let cloned = stores.clone();
    assert_eq!(
        cloned.semantic_hash_cache.testing_scratch_capacities(),
        (0, 0)
    );
}

#[test]
fn semantic_hash_only_walks_hyphenation_after_root_changes() {
    let mut stores = Stores::new();
    let initial_cursor = stores.state_hash_cursor();
    let initial = stores.checkpoint();
    let _ = stores.state_hash_slice(&initial_cursor, &initial);
    assert_eq!(
        stores.semantic_hash_cache.testing_hyphenation_hash_calls(),
        1,
        "the first framed projection computes its discardable fingerprint"
    );

    stores.add_hyphenation_pattern(PatternSpec {
        letters: "alpha".chars().collect(),
        values: vec![0, 1, 0, 0, 0, 0],
    });
    let with_pattern = stores.checkpoint();
    let _ = stores.state_hash_slice(&initial_cursor, &with_pattern);
    assert_eq!(
        stores.semantic_hash_cache.testing_hyphenation_hash_calls(),
        2
    );

    let pattern_cursor = stores.state_hash_cursor_from_snapshot(&with_pattern);
    stores.set_count(0, 1);
    let unrelated_change = stores.checkpoint();
    let _ = stores.state_hash_slice(&pattern_cursor, &unrelated_change);
    assert_eq!(
        stores.semantic_hash_cache.testing_hyphenation_hash_calls(),
        2,
        "an unrelated state change must not rehash the retained hyphenation root"
    );

    stores.add_hyphenation_exception(ExceptionSpec {
        word: "hyphenation".to_owned(),
        positions: vec![2, 6],
    });
    let with_exception = stores.checkpoint();
    let _ = stores.state_hash_slice(
        &stores.state_hash_cursor_from_snapshot(&unrelated_change),
        &with_exception,
    );
    assert_eq!(
        stores.semantic_hash_cache.testing_hyphenation_hash_calls(),
        3
    );

    stores.rollback(&with_pattern);
    stores.set_count(0, 2);
    let after_rollback = stores.checkpoint();
    let _ = stores.state_hash_slice(&pattern_cursor, &after_rollback);
    assert_eq!(
        stores.semantic_hash_cache.testing_hyphenation_hash_calls(),
        4,
        "rollback clears derived projections and rebuilds the restored root canonically"
    );
}

#[test]
fn source_origin_direct_boundary_crossing_falls_back_to_one_span_arena() {
    let mut stores = Stores::new();
    stores.source_map.set_next_position_for_test(0x7fff_fffd);
    stores
        .register_source(
            SourceId::new(0),
            SourceDescriptor::world(InputRecordId::new(0), 4),
            [0usize].into(),
        )
        .expect("cross-boundary source registers");
    let before = stores.provenance_stats();

    let first = stores.source_token_origin(SourceId::new(0), 0, 1);
    let last_direct = stores.source_token_origin(SourceId::new(0), 1, 2);
    let first_wide = stores.source_token_origin(SourceId::new(0), 2, 3);
    let after = stores.provenance_stats();

    assert!(matches!(
        first.decode(),
        crate::token::OriginEncoding::DirectSource(_)
    ));
    assert!(matches!(
        last_direct.decode(),
        crate::token::OriginEncoding::DirectSource(_)
    ));
    assert!(matches!(
        first_wide.decode(),
        crate::token::OriginEncoding::Arena(_)
    ));
    assert!(matches!(
        stores.origin(first_wide),
        OriginRecord::SourceSpan(_)
    ));
    assert_eq!(after.origin_records(), before.origin_records() + 1);

    let list = stores.allocate_origin_list(&[first, last_direct, first_wide]);
    assert_eq!(stores.origin_list(list), &[first, last_direct, first_wide]);
}

#[test]
fn oversized_and_cumulative_sources_use_wide_fallback_without_narrowing_positions() {
    let mut oversized = Stores::new();
    oversized
        .register_source(
            SourceId::new(0),
            SourceDescriptor::world(InputRecordId::new(0), 0x8000_0001),
            [0usize].into(),
        )
        .expect("single oversized source registers in logical u64 space");
    let wide = oversized.source_token_origin(SourceId::new(0), 0x7fff_ffff, 0x8000_0000);
    let OriginRecord::SourceSpan(span) = oversized.origin(wide) else {
        panic!("wide position must use source-span fallback");
    };
    assert_eq!(
        span.lo(),
        oversized
            .source_position(SourceId::new(0), 0x7fff_ffff)
            .expect("wide logical position remains addressable")
    );

    let mut cumulative = Stores::new();
    cumulative
        .source_map
        .set_next_position_for_test(0x7fff_ff00);
    cumulative
        .register_source(
            SourceId::new(0),
            SourceDescriptor::world(InputRecordId::new(0), 0xff),
            [0usize].into(),
        )
        .expect("first source registers");
    cumulative
        .register_source(
            SourceId::new(1),
            SourceDescriptor::world(InputRecordId::new(1), 2),
            [0usize].into(),
        )
        .expect("second source registers beyond direct space");
    let fallback = cumulative.source_token_origin(SourceId::new(1), 0, 1);
    assert!(matches!(
        fallback.decode(),
        crate::token::OriginEncoding::Arena(_)
    ));
}

#[test]
fn direct_and_fallback_liveness_tracks_aggregate_rollback() {
    let mut stores = Stores::new();
    stores.source_map.set_next_position_for_test(0x7fff_fffe);
    let checkpoint = stores.checkpoint();
    stores
        .register_source(
            SourceId::new(4),
            SourceDescriptor::world(InputRecordId::new(0), 2),
            [0usize].into(),
        )
        .expect("source registers");
    let direct = stores.source_token_origin(SourceId::new(4), 0, 1);
    let fallback = stores.source_token_origin(SourceId::new(4), 1, 2);
    assert!(stores.origin_if_live(direct).is_some());
    assert!(stores.origin_if_live(fallback).is_some());

    stores.rollback(&checkpoint);
    assert!(stores.origin_if_live(direct).is_none());
    assert!(stores.origin_if_live(fallback).is_none());
    assert_eq!(stores.provenance_stats().origin_records(), 0);
}

#[test]
fn group_exit_restores_all_code_tables() {
    let mut stores = Stores::new();
    let ch = '@';
    let before = (
        stores.catcode(ch),
        stores.lccode(ch),
        stores.uccode(ch),
        stores.sfcode(ch),
        stores.mathcode(ch),
        stores.delcode(ch),
    );

    stores.enter_group();
    stores.set_catcode(ch, Catcode::Letter);
    stores.set_lccode(ch, 'a' as u32);
    stores.set_uccode(ch, 'A' as u32);
    stores.set_sfcode(ch, 777);
    stores.set_mathcode(ch, 1234);
    stores.set_delcode(ch, 5678);
    assert_eq!(stores.leave_group(), Vec::<Token>::new());

    assert_eq!(
        (
            stores.catcode(ch),
            stores.lccode(ch),
            stores.uccode(ch),
            stores.sfcode(ch),
            stores.mathcode(ch),
            stores.delcode(ch),
        ),
        before
    );
}

#[test]
fn global_code_table_assignments_survive_groups_but_not_snapshot_rollback() {
    let mut stores = Stores::new();
    let ch = '@';
    let snapshot = stores.checkpoint();

    stores.enter_group();
    stores.set_catcode_global(ch, Catcode::Letter);
    stores.set_lccode_global(ch, 'a' as u32);
    stores.set_uccode_global(ch, 'A' as u32);
    stores.set_sfcode_global(ch, 777);
    stores.set_mathcode_global(ch, 1234);
    stores.set_delcode_global(ch, 5678);
    assert_eq!(stores.leave_group(), Vec::<Token>::new());

    assert_eq!(stores.catcode(ch), Catcode::Letter);
    assert_eq!(stores.lccode(ch), 'a' as u32);
    assert_eq!(stores.uccode(ch), 'A' as u32);
    assert_eq!(stores.sfcode(ch), 777);
    assert_eq!(stores.mathcode(ch), 1234);
    assert_eq!(stores.delcode(ch), 5678);

    stores.rollback(&snapshot);
    assert_eq!(stores.catcode(ch), Catcode::Other);
    assert_eq!(stores.lccode(ch), 0);
    assert_eq!(stores.uccode(ch), 0);
    assert_eq!(stores.sfcode(ch), 1000);
    assert_eq!(stores.mathcode(ch), ch as u32);
    assert_eq!(stores.delcode(ch), -1);
}

#[test]
fn rollback_restores_token_store_as_part_of_snapshot_tuple() {
    let mut stores = Stores::new();
    let snapshot = stores.checkpoint();
    let stale = stores.intern_token_list(&[crate::token::Token::param(1)]);

    stores.rollback(&snapshot);
    let reused = stores.intern_token_list(&[crate::token::Token::param(2)]);

    assert_eq!(reused.raw(), stale.raw());
    assert_ne!(reused, stale);
    assert_eq!(stores.tokens(reused), &[crate::token::Token::param(2)]);
}

#[test]
fn token_list_builder_finishes_through_stores_boundary() {
    let mut stores = Stores::new();
    let symbol = stores.intern("macro");
    let mut builder = stores.token_list_builder();
    builder.push(crate::token::Token::Cs(symbol.symbol()));
    builder.push(crate::token::Token::param(1));

    let id = stores.finish_token_list(&mut builder);

    assert!(builder.is_empty());
    assert_eq!(
        stores.tokens(id),
        &[
            crate::token::Token::Cs(symbol.symbol()),
            crate::token::Token::param(1)
        ]
    );

    builder.push(crate::token::Token::param(2));
    let reused = stores.finish_token_list(&mut builder);
    assert_eq!(stores.tokens(reused), &[crate::token::Token::param(2)]);
}

#[test]
fn builder_and_bulk_token_list_identities_match() {
    let mut stores = Stores::new();
    let symbol = stores.intern("macro");
    let tokens = [
        Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        },
        Token::Cs(symbol.symbol()),
        Token::param(1),
    ];

    let bulk = stores.intern_token_list(&tokens);
    let mut builder = stores.token_list_builder();
    for token in tokens {
        builder.push(token);
    }
    let built = stores.finish_token_list(&mut builder);

    assert_eq!(built, bulk);
    assert_eq!(
        stores.tokens.semantic_id(built),
        stores.tokens.semantic_id(bulk)
    );
}

#[test]
fn token_list_ingress_rejects_equal_slot_foreign_symbols_before_interning() {
    let mut foreign = Stores::new();
    let foreign_symbol = foreign.intern("foreign");
    let token = Token::Cs(foreign_symbol.symbol());
    let mut stores = Stores::new();
    let local = stores.intern("local");
    assert_eq!(foreign_symbol.raw(), local.raw());
    assert_ne!(foreign_symbol.symbol(), local.symbol());

    let rejected = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        stores.intern_token_list(&[token]);
    }));
    assert!(rejected.is_err());

    let accepted = stores.intern_token_list(&[Token::Cs(local.symbol())]);
    assert_eq!(
        accepted.raw(),
        1,
        "rejected ingress must not allocate a list"
    );
}

#[test]
fn token_list_builder_rejects_equal_slot_foreign_symbol_atomically() {
    let mut foreign = Stores::new();
    let foreign_symbol = foreign.intern("foreign");
    let mut stores = Stores::new();
    let local = stores.intern("local");
    assert_eq!(foreign_symbol.raw(), local.raw());
    assert_ne!(foreign_symbol.symbol(), local.symbol());
    let mut builder = stores.token_list_builder();
    builder.push(Token::Cs(foreign_symbol.symbol()));

    let rejected = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        stores.finish_token_list(&mut builder);
    }));
    assert!(rejected.is_err());
    assert_eq!(builder.len(), 1, "rejected builder must remain reusable");

    builder.clear();
    builder.push(Token::Cs(local.symbol()));
    let accepted = stores.finish_token_list(&mut builder);
    assert_eq!(
        accepted.raw(),
        1,
        "rejected builder must not allocate a list"
    );
    assert!(builder.is_empty());
}

#[test]
fn provenance_records_and_lists_round_trip_through_stores_boundary() {
    let mut stores = Stores::new();
    let symbol = stores.intern("m");
    let params = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[Token::Cs(symbol.symbol())]);
    let definition = stores.intern_macro(MacroMeaning::new(MeaningFlags::EMPTY, params, body));
    let source = stores.source_origin(SourceId::new(3), 40, 5, 2);
    let macro_origin =
        stores.macro_invocation_origin(definition, source, OriginId::UNKNOWN, OriginId::UNKNOWN);
    let inserted = stores.inserted_origin(
        InsertedOriginKind::Paragraph,
        Token::Char {
            ch: 'p',
            cat: Catcode::Letter,
        },
        macro_origin,
    );
    let synthesized = stores.synthesized_origin(SynthesizedOriginKind::Expansion, inserted);
    let synthetic = stores.synthetic_origin(SyntheticOriginKind::Engine);
    let list = stores.allocate_origin_list(&[source, macro_origin, inserted, synthesized]);

    assert_eq!(stores.bootstrap_origin(), OriginId::UNKNOWN);
    assert_eq!(
        stores.origin(source),
        OriginRecord::Source(SourceOrigin::new(SourceId::new(3), 40, 5, 2))
    );
    assert_eq!(
        stores.origin(macro_origin),
        OriginRecord::MacroInvocation(MacroInvocationOrigin::new(
            definition,
            source,
            OriginId::UNKNOWN,
            OriginId::UNKNOWN,
        ))
    );
    assert_eq!(
        stores.origin(inserted),
        OriginRecord::Inserted(InsertedOrigin::new(
            InsertedOriginKind::Paragraph,
            Token::Char {
                ch: 'p',
                cat: Catcode::Letter,
            },
            macro_origin,
        ))
    );
    assert_eq!(
        stores.origin(synthesized),
        OriginRecord::Synthesized(SynthesizedOrigin::new(
            SynthesizedOriginKind::Expansion,
            inserted,
        ))
    );
    assert_eq!(
        stores.origin(synthetic),
        OriginRecord::Synthetic(SyntheticOrigin::new(SyntheticOriginKind::Engine))
    );
    assert_eq!(
        stores.origin_list(list),
        &[source, macro_origin, inserted, synthesized]
    );
    assert_eq!(stores.origin_list(OriginListId::EMPTY), &[]);
}

#[test]
fn rollback_restores_provenance_as_part_of_snapshot_tuple() {
    let mut stores = Stores::new();
    let kept = stores.synthetic_origin(SyntheticOriginKind::Engine);
    let snapshot = stores.checkpoint();
    let stale = stores.synthetic_origin(SyntheticOriginKind::Primitive);
    let stale_list = stores.allocate_origin_list(&[kept, stale]);

    stores.rollback(&snapshot);
    let reused = stores.synthetic_origin(SyntheticOriginKind::Format);
    let reused_list = stores.allocate_origin_list(&[kept, reused]);

    assert_ne!(reused.raw(), stale.raw());
    assert_eq!(stores.origin_if_live(stale), None);
    assert_eq!(reused_list.raw(), stale_list.raw());
    assert_ne!(reused_list, stale_list);
    assert_eq!(
        stores.origin(reused),
        OriginRecord::Synthetic(SyntheticOrigin::new(SyntheticOriginKind::Format))
    );
    assert_eq!(stores.origin_list(reused_list), &[kept, reused]);
}

#[test]
fn macro_meaning_round_trips_through_stores_boundary() {
    let mut stores = Stores::new();
    let symbol = stores.intern("m");
    let params = stores.intern_token_list(&[Token::Char {
        ch: '#',
        cat: Catcode::Parameter,
    }]);
    let body = stores.intern_token_list(&[Token::param(1), Token::Cs(symbol.symbol())]);
    let macro_meaning = MacroMeaning::new(
        MeaningFlags::LONG | MeaningFlags::OUTER | MeaningFlags::PROTECTED,
        params,
        body,
    );

    stores.set_macro_meaning(symbol, macro_meaning);

    assert_eq!(stores.macro_meaning(symbol), Some(macro_meaning));
    let Meaning::Macro { flags, definition } = stores.meaning(symbol) else {
        panic!("expected macro meaning");
    };
    assert_eq!(flags, macro_meaning.flags());
    assert_eq!(stores.macro_definition(definition), macro_meaning);
}

#[test]
fn separately_created_identical_macro_bodies_share_token_list_identity() {
    let mut stores = Stores::new();
    let a = stores.intern("a");
    let b = stores.intern("b");
    let first_body = stores.intern_token_list(&[Token::param(1), Token::Cs(a.symbol())]);
    let second_body = stores.intern_token_list(&[Token::param(1), Token::Cs(a.symbol())]);
    let params = stores.intern_token_list(&[]);

    assert_eq!(first_body, second_body);

    stores.set_macro_meaning(
        a,
        MacroMeaning::new(MeaningFlags::EMPTY, params, first_body),
    );
    stores.set_macro_meaning(
        b,
        MacroMeaning::new(MeaningFlags::EMPTY, params, second_body),
    );

    assert_eq!(
        stores.macro_meaning(a).map(MacroMeaning::replacement_text),
        stores.macro_meaning(b).map(MacroMeaning::replacement_text)
    );
}

#[test]
fn identical_macro_definitions_get_distinct_definition_identity() {
    let mut stores = Stores::new();
    let symbol = stores.intern("same");
    let params = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[Token::Cs(symbol.symbol())]);
    let macro_meaning = MacroMeaning::new(MeaningFlags::PROTECTED, params, body);

    let first = stores.intern_macro(macro_meaning);
    let second = stores.intern_macro(macro_meaning);

    assert_ne!(first, second);
    assert!(
        stores
            .macro_definition(first)
            .semantic_eq(stores.macro_definition(second))
    );
}

#[test]
fn identical_macro_definitions_keep_distinct_provenance() {
    let mut stores = Stores::new();
    let symbol = stores.intern("same");
    let params = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[Token::Cs(symbol.symbol())]);
    let macro_meaning = MacroMeaning::new(MeaningFlags::PROTECTED, params, body);
    let first_origin = stores.source_origin(SourceId::new(1), 10, 2, 3);
    let second_origin = stores.source_origin(SourceId::new(2), 20, 4, 5);
    let first_body_origins = stores.allocate_origin_list(&[first_origin]);
    let second_body_origins = stores.allocate_origin_list(&[second_origin]);

    let first = stores.intern_macro_with_provenance(
        macro_meaning,
        Some(MacroDefinitionProvenance::new(
            first_origin,
            OriginListId::EMPTY,
            first_body_origins,
        )),
    );
    let second = stores.intern_macro_with_provenance(
        macro_meaning,
        Some(MacroDefinitionProvenance::new(
            second_origin,
            OriginListId::EMPTY,
            second_body_origins,
        )),
    );

    assert_ne!(first, second);
    assert!(
        stores
            .macro_definition(first)
            .semantic_eq(stores.macro_definition(second))
    );
    assert_eq!(
        stores
            .macro_definition_provenance(first)
            .definition_origin(),
        first_origin
    );
    assert_eq!(
        stores
            .macro_definition_provenance(second)
            .replacement_origins(),
        second_body_origins
    );
}

#[test]
fn missing_macro_definition_provenance_degrades_to_unknown() {
    let mut stores = Stores::new();
    let params = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[]);
    let definition = stores.intern_macro(MacroMeaning::new(MeaningFlags::EMPTY, params, body));

    assert_eq!(
        stores.macro_definition_provenance(definition),
        MacroDefinitionProvenance::unknown()
    );
}

#[test]
fn rollback_restores_macro_store_as_part_of_snapshot_tuple() {
    let mut stores = Stores::new();
    let symbol = stores.intern("macro");
    let params = stores.intern_token_list(&[]);
    let kept_body = stores.intern_token_list(&[Token::param(1)]);
    let kept = stores.intern_macro(MacroMeaning::new(MeaningFlags::LONG, params, kept_body));
    let snapshot = stores.checkpoint();
    let stale_body = stores.intern_token_list(&[Token::param(2)]);
    let stale = stores.intern_macro(MacroMeaning::new(MeaningFlags::OUTER, params, stale_body));

    stores.rollback(&snapshot);
    let reused_body = stores.intern_token_list(&[Token::Cs(symbol.symbol())]);
    let reused = stores.intern_macro(MacroMeaning::new(
        MeaningFlags::PROTECTED,
        params,
        reused_body,
    ));

    assert_eq!(stores.macro_definition(kept).replacement_text(), kept_body);
    assert_eq!(reused.raw(), stale.raw());
    assert_ne!(reused, stale);
    assert!(!stores.macros.contains(stale));
    assert_eq!(
        stores.macro_definition(reused).replacement_text(),
        reused_body
    );
}

#[test]
fn rollback_restores_glue_store_as_part_of_snapshot_tuple() {
    let mut stores = Stores::new();
    let snapshot = stores.checkpoint();
    let stale = stores.intern_glue(glue_spec(1));

    stores.rollback(&snapshot);
    let reused = stores.intern_glue(glue_spec(2));

    assert_eq!(reused.raw(), stale.raw());
    assert_ne!(reused, stale);
    assert!(!stores.glue.contains(stale));
    assert_eq!(stores.glue(reused), glue_spec(2));
    assert_eq!(stores.glue(crate::ids::GlueId::ZERO), GlueSpec::ZERO);
}

#[test]
fn state_defaults_match_tex82_initex() {
    let stores = Stores::new();

    assert_eq!(stores.int_param(IntParam::PRETOLERANCE), 0);
    assert_eq!(stores.int_param(IntParam::TOLERANCE), 10_000);
    assert_eq!(stores.int_param(IntParam::MAG), 1000);
    assert_eq!(stores.int_param(IntParam::MAX_DEAD_CYCLES), 25);
    assert_eq!(stores.int_param(IntParam::HANG_AFTER), 1);
    assert_eq!(stores.int_param(IntParam::ESCAPE_CHAR), b'\\' as i32);
    assert_eq!(stores.int_param(IntParam::END_LINE_CHAR), 13);
    assert_eq!(stores.int_param(IntParam::DEFAULT_HYPHEN_CHAR), 0);
    assert_eq!(stores.int_param(IntParam::DEFAULT_SKEW_CHAR), 0);
    assert_eq!(stores.int_param(IntParam::FAM), 0);
    assert_eq!(stores.int_param(IntParam::UC_HYPH), 0);
    assert_eq!(stores.int_param(IntParam::LEFT_HYPHEN_MIN), 0);
    assert_eq!(stores.int_param(IntParam::RIGHT_HYPHEN_MIN), 0);
    assert_eq!(stores.dimen(0), scaled(0));
    assert_eq!(stores.dimen_param(DimenParam::OVERFULL_RULE), scaled(0));
    assert_eq!(stores.dimen_param(DimenParam::MAX_DEPTH), scaled(0));
    assert_eq!(
        stores.glue_param(GlueParam::BASELINE_SKIP),
        crate::ids::GlueId::ZERO
    );
    assert_eq!(
        stores.glue_param(GlueParam::PAR_FILL_SKIP),
        crate::ids::GlueId::ZERO
    );
}

#[test]
fn node_list_builder_finishes_through_stores_boundary() {
    let mut stores = Stores::new();
    let mut builder = stores.node_list_builder();
    builder.push(Node::MathOn(Scaled::from_raw(0)));
    builder.push(Node::MathOff(Scaled::from_raw(0)));

    let id = stores.finish_node_list(&mut builder);

    assert!(builder.is_empty());
    assert_eq!(
        stores.nodes(id),
        &[
            Node::MathOn(Scaled::from_raw(0)),
            Node::MathOff(Scaled::from_raw(0))
        ]
    );

    builder.push(Node::Char {
        font: NULL_FONT,
        ch: 'x',
    });
    let reused = stores.finish_node_list(&mut builder);
    assert_eq!(
        stores.nodes(reused),
        &[Node::Char {
            font: NULL_FONT,
            ch: 'x'
        }]
    );
}

#[test]
#[should_panic(expected = "glue id is not live in this Universe timeline")]
fn freeze_node_list_rejects_stale_rolled_back_glue_id() {
    let mut stores = Stores::new();
    let snapshot = stores.checkpoint();
    let stale = stores.intern_glue(glue_spec(1));

    stores.rollback(&snapshot);
    stores.freeze_node_list(&[Node::Glue {
        spec: stale,
        kind: crate::node::GlueKind::Normal,
        leader: None,
    }]);
}

#[test]
#[should_panic(expected = "glue id is not live in this Universe timeline")]
fn finish_node_list_rejects_foreign_glue_id() {
    let mut stores = Stores::new();
    let mut foreign = stores.clone();
    let foreign_glue = foreign.intern_glue(glue_spec(1));
    let mut builder = stores.node_list_builder();
    builder.push(Node::Glue {
        spec: foreign_glue,
        kind: crate::node::GlueKind::Normal,
        leader: None,
    });

    let _ = stores.finish_node_list(&mut builder);
}

#[test]
#[should_panic(expected = "token list is not live in this Universe timeline")]
fn freeze_node_list_rejects_stale_rolled_back_mark_token_list() {
    let mut stores = Stores::new();
    let snapshot = stores.checkpoint();
    let stale = stores.intern_token_list(&[crate::token::Token::param(1)]);

    stores.rollback(&snapshot);
    stores.freeze_node_list(&[Node::Mark {
        class: 0,
        tokens: stale,
    }]);
}

#[test]
#[should_panic(expected = "token list is not live in this Universe timeline")]
fn finish_node_list_rejects_foreign_whatsit_token_list() {
    let mut stores = Stores::new();
    let mut foreign = stores.clone();
    let foreign_tokens = foreign.intern_token_list(&[crate::token::Token::param(1)]);
    let mut builder = stores.node_list_builder();
    builder.push(Node::Whatsit(crate::node::Whatsit::DeferredWrite {
        sink: crate::world::PrintSink::TerminalAndLog,
        tokens: foreign_tokens,
    }));

    let _ = stores.finish_node_list(&mut builder);
}

#[test]
#[should_panic(expected = "child node-list id is not live in this Universe timeline")]
fn freeze_node_list_rejects_stale_rolled_back_child_node_list() {
    let mut stores = Stores::new();
    let snapshot = stores.checkpoint();
    let stale = one_char(&mut stores, 'x');

    stores.rollback(&snapshot);
    stores.freeze_node_list(&[Node::Penalty(1), Node::Penalty(2)]);
    stores.freeze_node_list(&[Node::Adjust(stale)]);
}

#[test]
#[should_panic(expected = "node list is not live in this Universe timeline")]
fn aggregate_read_rejects_stale_epoch_list_after_covering_reallocation() {
    let mut stores = Stores::new();
    let snapshot = stores.checkpoint();
    let stale = one_char(&mut stores, 'x');

    stores.rollback(&snapshot);
    stores.freeze_node_list(&[Node::Penalty(1), Node::Penalty(2)]);
    let _ = stores.nodes(stale);
}

#[test]
#[should_panic(expected = "node list is not live in this Universe timeline")]
fn box_write_rejects_stale_epoch_list_after_equal_reallocation() {
    let mut stores = Stores::new();
    let snapshot = stores.checkpoint();
    let stale = one_char(&mut stores, 'x');

    stores.rollback(&snapshot);
    let _replacement = one_char(&mut stores, 'y');
    stores.set_box_reg(0, stale);
}

#[test]
#[should_panic(expected = "child node-list id is not live in this Universe timeline")]
fn finish_node_list_rejects_foreign_child_node_list() {
    let mut stores = Stores::new();
    let mut foreign = Stores::new();
    let foreign_child = one_char(&mut foreign, 'x');
    let mut builder = stores.node_list_builder();
    builder.push(Node::HList(BoxNode::new(BoxNodeFields {
        width: scaled(10),
        height: scaled(7),
        depth: scaled(3),
        shift: scaled(0),
        display: false,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: foreign_child,
    })));

    let _ = stores.finish_node_list(&mut builder);
}

#[test]
#[should_panic(expected = "Stores snapshots are invalidated by exiting a group that encloses them")]
fn rollback_rejects_snapshot_taken_inside_exited_group() {
    let mut stores = Stores::new();
    stores.enter_group();
    let snapshot = stores.checkpoint();

    assert_eq!(stores.leave_group(), Vec::<Token>::new());

    stores.rollback(&snapshot);
}

#[test]
fn rollback_allows_snapshot_before_balanced_inner_group() {
    let mut stores = Stores::new();
    let symbol = stores.intern("kept");
    let snapshot = stores.checkpoint();

    stores.enter_group();
    stores.set_meaning(symbol, Meaning::CharGiven('x'));
    assert_eq!(stores.leave_group(), Vec::<Token>::new());

    stores.rollback(&snapshot);
    assert_eq!(stores.meaning(symbol), Meaning::Undefined);
}

#[test]
#[should_panic(expected = "Stores snapshot belongs to a different Stores instance")]
fn rollback_rejects_snapshot_from_different_store() {
    let mut first = Stores::new();
    let mut second = Stores::new();
    let snapshot = first.checkpoint();

    second.rollback(&snapshot);
}

#[test]
#[should_panic(expected = "Stores snapshot belongs to a different Stores instance")]
fn rollback_rejects_snapshot_from_cloned_store() {
    let mut first = Stores::new();
    let mut second = first.clone();
    let snapshot = first.checkpoint();

    second.rollback(&snapshot);
}

#[test]
#[should_panic(expected = "token list is not live in this Universe timeline")]
fn stale_rolled_back_token_list_cannot_mutate_toks_register() {
    let mut stores = Stores::new();
    let snapshot = stores.checkpoint();
    let stale = stores.intern_token_list(&[crate::token::Token::param(1)]);

    stores.rollback(&snapshot);
    stores.set_toks(0, stale);
}

#[test]
#[should_panic(expected = "macro definition id is not live in this Universe timeline")]
fn stale_rolled_back_macro_definition_cannot_mutate_meaning() {
    let mut stores = Stores::new();
    let symbol = stores.intern("macro");
    let params = stores.intern_token_list(&[]);
    let snapshot = stores.checkpoint();
    let body = stores.intern_token_list(&[Token::param(1)]);
    let stale = stores.intern_macro(MacroMeaning::new(MeaningFlags::EMPTY, params, body));

    stores.rollback(&snapshot);
    stores.set_meaning(
        symbol,
        Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition: stale,
        },
    );
}

#[test]
#[should_panic(expected = "glue id is not live in this Universe timeline")]
fn stale_rolled_back_glue_cannot_mutate_skip_register() {
    let mut stores = Stores::new();
    let snapshot = stores.checkpoint();
    let stale = stores.intern_glue(glue_spec(1));

    stores.rollback(&snapshot);
    stores.set_skip(0, stale);
}

#[test]
#[should_panic(expected = "glue id is not live in this Universe timeline")]
fn stale_rolled_back_glue_cannot_mutate_muskip_register() {
    let mut stores = Stores::new();
    let snapshot = stores.checkpoint();
    let stale = stores.intern_glue(glue_spec(1));

    stores.rollback(&snapshot);
    stores.set_muskip(0, stale);
}

#[test]
fn checkpoint_rollback_restores_muskip_register_and_glue_tuple() {
    let mut stores = Stores::new();
    let original = stores.intern_glue(glue_spec(1));
    stores.set_muskip(7, original);
    let snapshot = stores.checkpoint();
    let replacement = stores.intern_glue(glue_spec(2));

    stores.set_muskip(7, replacement);
    stores.rollback(&snapshot);

    assert_eq!(stores.muskip(7), original);
    assert_eq!(stores.glue(stores.muskip(7)), glue_spec(1));
}

#[test]
fn rollback_discards_aftergroup_payloads_pushed_after_snapshot() {
    let mut stores = Stores::new();
    stores.enter_group();
    let snapshot = stores.checkpoint();

    stores.push_aftergroup(Token::Char {
        ch: 'x',
        cat: Catcode::Letter,
    });
    stores.rollback(&snapshot);

    assert_eq!(stores.leave_group(), Vec::<Token>::new());
}

#[test]
fn rollback_restores_afterassignment_slot() {
    let mut stores = Stores::new();
    let original = Token::Char {
        ch: 'a',
        cat: Catcode::Letter,
    };
    let replacement = Token::Char {
        ch: 'b',
        cat: Catcode::Letter,
    };
    stores.set_afterassignment(original);
    let snapshot = stores.checkpoint();

    stores.set_afterassignment(replacement);
    stores.rollback(&snapshot);

    assert_eq!(stores.take_afterassignment(), Some(original));
    assert_eq!(stores.take_afterassignment(), None);
}

#[test]
fn equal_slot_foreign_aftergroup_token_preserves_payload_order() {
    let mut foreign = Stores::new();
    let foreign_symbol = foreign.intern("foreign");
    let mut stores = Stores::new();
    let local = stores.intern("local");
    assert_eq!(foreign_symbol.raw(), local.raw());
    assert_ne!(foreign_symbol.symbol(), local.symbol());
    let first = Token::param(1);
    let last = Token::Cs(local.symbol());
    stores.enter_group();
    stores.push_aftergroup(first);

    let rejected = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        stores.push_aftergroup(Token::Cs(foreign_symbol.symbol()));
    }));
    assert!(rejected.is_err());

    stores.push_aftergroup(last);
    assert_eq!(stores.leave_group(), vec![first, last]);
}

#[test]
fn equal_slot_foreign_afterassignment_token_preserves_previous_payload() {
    let mut foreign = Stores::new();
    let foreign_symbol = foreign.intern("foreign");
    let mut stores = Stores::new();
    let local = stores.intern("local");
    assert_eq!(foreign_symbol.raw(), local.raw());
    assert_ne!(foreign_symbol.symbol(), local.symbol());
    let original = Token::Cs(local.symbol());
    stores.set_afterassignment(original);

    let rejected = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        stores.set_afterassignment(Token::Cs(foreign_symbol.symbol()));
    }));
    assert!(rejected.is_err());
    assert_eq!(stores.take_afterassignment(), Some(original));
}

#[test]
fn post_reuse_symbol_token_is_rejected_at_every_scoped_ingress() {
    let mut stores = Stores::new();
    let snapshot = stores.checkpoint();
    let stale = stores.intern("stale");
    stores.rollback(&snapshot);
    let replacement = stores.intern("replacement");
    assert_eq!(stale.raw(), replacement.raw());
    assert_ne!(stale.symbol(), replacement.symbol());
    let token = Token::Cs(stale.symbol());
    stores.enter_group();

    for ingress in ["intern", "builder", "aftergroup", "afterassignment"] {
        let rejected = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match ingress {
            "intern" => {
                stores.intern_token_list(&[token]);
            }
            "builder" => {
                let mut builder = stores.token_list_builder();
                builder.push(token);
                stores.finish_token_list(&mut builder);
            }
            "aftergroup" => stores.push_aftergroup(token),
            "afterassignment" => stores.set_afterassignment(token),
            _ => unreachable!(),
        }));
        assert!(rejected.is_err(), "{ingress} accepted a rolled-back symbol");
    }

    assert_eq!(stores.take_afterassignment(), None);
    let replacement_token = Token::Cs(replacement.symbol());
    let accepted = stores.intern_token_list(&[replacement_token]);
    assert_eq!(
        accepted.raw(),
        1,
        "rejections must not allocate token lists"
    );
    stores.push_aftergroup(replacement_token);
    assert_eq!(stores.leave_group(), vec![replacement_token]);
    stores.set_afterassignment(replacement_token);
    assert_eq!(stores.take_afterassignment(), Some(replacement_token));
}

#[test]
#[should_panic(expected = "symbol is not live in this Universe timeline")]
fn stale_rolled_back_symbol_cannot_write_reused_meaning_cell() {
    let mut stores = Stores::new();
    let snapshot = stores.checkpoint();
    let stale = stores.intern("rolled-back");

    stores.rollback(&snapshot);
    stores.set_meaning(stale, Meaning::Relax);
}

#[test]
fn same_epoch_list_stored_twice_promotes_to_independent_roots() {
    let mut stores = Stores::new();
    let list = one_char(&mut stores, 'a');

    stores.set_box_reg(0, list);
    stores.set_box_reg(1, list);

    let first = stores.box_reg(0).expect("box 0 should be non-void");
    let second = stores.box_reg(1).expect("box 1 should be non-void");
    assert_ne!(first.arena(), second.arena());
    assert_eq!(stores.testing_live_survivor_slot_count(), 2);
    assert_eq!(stores.testing_survivor_refcount(first), 1);
    assert_eq!(stores.testing_survivor_refcount(second), 1);
}

#[test]
fn survivor_fork_keeps_inherited_roots_and_separates_new_roots() {
    let mut parent = Stores::new();
    let inherited_epoch = one_char(&mut parent, 'i');
    let inherited = parent.prepare_box_value(inherited_epoch);
    let mut child = parent.clone();

    assert_eq!(
        parent.nodes(inherited).to_vec(),
        child.nodes(inherited).to_vec()
    );

    let parent_epoch = one_char(&mut parent, 'p');
    let parent_only = parent.prepare_box_value(parent_epoch);
    let child_epoch = one_char(&mut child, 'c');
    let child_only = child.prepare_box_value(child_epoch);

    assert_ne!(parent_only.arena(), child_only.arena());
    assert!(parent.survivors.contains(parent_only));
    assert!(!parent.survivors.contains(child_only));
    assert!(child.survivors.contains(child_only));
    assert!(!child.survivors.contains(parent_only));
    assert!(std::panic::catch_unwind(|| parent.nodes(child_only)).is_err());
    assert!(std::panic::catch_unwind(|| child.nodes(parent_only)).is_err());
}

#[test]
fn released_survivor_key_stays_stale_when_its_storage_is_recycled() {
    let mut stores = Stores::new();
    let old_epoch = one_char(&mut stores, 'o');
    let stale = stores.prepare_box_value(old_epoch);
    stores.dec_survivor_ref(stale);

    let new_epoch = one_char(&mut stores, 'n');
    let replacement = stores.prepare_box_value(new_epoch);

    assert_ne!(stale.arena(), replacement.arena());
    assert!(!stores.survivors.contains(stale));
    assert!(stores.survivors.contains(replacement));
    assert_eq!(stores.testing_survivor_recycled_buffer_uses(), 1);
    assert!(std::panic::catch_unwind(|| stores.nodes(stale)).is_err());
}

#[test]
fn repeated_survivor_replacement_recycles_buffers_without_reviving_stale_ids() {
    const REPLACEMENTS: usize = 20_000;

    let mut stores = Stores::new();
    let first = one_char(&mut stores, 'a');
    let mut live = stores.prepare_box_value(first);
    let stale = live;

    for index in 1..REPLACEMENTS {
        let replacement = one_char(&mut stores, if index % 2 == 0 { 'a' } else { 'b' });
        let replacement = stores.prepare_box_value(replacement);
        stores.dec_survivor_ref(live);
        live = replacement;
    }

    assert!(!stores.survivors.contains(stale));
    assert_ne!(stale.arena(), live.arena());
    assert_eq!(stores.testing_live_survivor_slot_count(), 1);
    assert_eq!(stores.testing_survivor_root_slot_count(), REPLACEMENTS);
    // Two buffers cover the old and replacement roots; every later
    // promotion reuses one of them.
    assert_eq!(
        stores.testing_survivor_recycled_buffer_uses(),
        REPLACEMENTS - 2
    );
}

#[test]
fn survivor_recycling_carries_word_and_box_rule_sidecars_together() {
    let mut stores = Stores::new();
    let mut stale = None;
    let mut live = None;

    for raw in 0..32 {
        let child = stores.freeze_node_list(&[Node::Rule {
            width: Some(Scaled::from_raw(raw)),
            height: None,
            depth: Some(Scaled::from_raw(-raw)),
        }]);
        let root = stores.freeze_node_list(&[Node::HList(BoxNode::new(BoxNodeFields {
            width: Scaled::from_raw(raw),
            height: Scaled::from_raw(0),
            depth: Scaled::from_raw(0),
            shift: Scaled::from_raw(0),
            display: false,
            glue_set: GlueSetRatio::ZERO,
            glue_sign: Sign::Normal,
            glue_order: Order::Normal,
            children: child,
        }))]);
        let promoted = stores.prepare_box_value(root);
        if let Some(previous) = live.replace(promoted) {
            stale.get_or_insert(previous);
            stores.dec_survivor_ref(previous);
        }
    }

    let live = live.expect("one survivor remains");
    let Some(crate::node_arena::NodeRef::HList(box_node)) = stores.nodes(live).first() else {
        panic!("survivor root should retain its box sidecar")
    };
    assert_eq!(box_node.width, Scaled::from_raw(31));
    assert_eq!(
        stores.nodes(box_node.children),
        &[Node::Rule {
            width: Some(Scaled::from_raw(31)),
            height: None,
            depth: Some(Scaled::from_raw(-31)),
        }]
    );
    assert!(
        !stores
            .survivors
            .contains(stale.expect("a stale root exists"))
    );
    assert!(stores.testing_survivor_recycled_buffer_uses() > 0);
}

#[test]
fn coalesced_box_replacements_roll_back_to_the_checkpoint_owner() {
    const REPLACEMENTS: usize = 20_000;

    let mut stores = Stores::new();
    let baseline = one_char(&mut stores, 'o');
    stores.set_box_reg(0, baseline);
    let baseline = stores.box_reg(0).expect("baseline box should be stored");
    let snapshot = stores.checkpoint();
    let mut stale = None;

    for index in 0..REPLACEMENTS {
        let replacement = one_char(&mut stores, if index % 2 == 0 { 'a' } else { 'b' });
        stores.set_box_reg(0, replacement);
        stale.get_or_insert_with(|| stores.box_reg(0).expect("replacement should be stored"));
    }

    let stale = stale.expect("at least one replacement should be stored");
    assert!(!stores.survivors.contains(stale));
    assert_eq!(stores.testing_live_survivor_slot_count(), 2);
    assert_eq!(
        stores.testing_survivor_recycled_buffer_uses(),
        REPLACEMENTS - 2
    );

    stores.rollback(&snapshot);
    assert_eq!(stores.box_reg(0), Some(baseline));
    assert_eq!(stores.testing_live_survivor_slot_count(), 1);
    assert_eq!(stores.testing_survivor_refcount(baseline), 1);
}

#[test]
fn storing_survivor_in_second_register_shares_refcount_until_release() {
    let mut stores = Stores::new();
    let list = one_char(&mut stores, 'a');

    stores.set_box_reg(0, list);
    let survivor = stores.box_reg(0).expect("box should be non-void");
    stores.set_box_reg(1, survivor);

    assert_eq!(stores.testing_live_survivor_slot_count(), 1);
    assert_eq!(stores.testing_survivor_refcount(survivor), 2);

    assert_eq!(stores.take_box_reg(0), Some(survivor));
    // Register 1 and the take journal entry both hold the survivor until a
    // rollback/commit boundary drops the journal record.
    assert_eq!(stores.testing_survivor_refcount(survivor), 2);

    let replacement = one_char(&mut stores, 'b');
    stores.set_box_reg(1, replacement);
    assert_eq!(stores.testing_live_survivor_slot_count(), 2);
}

#[test]
fn group_exit_and_rollback_restore_box_refs_once() {
    let mut stores = Stores::new();
    let outer = one_char(&mut stores, 'o');
    stores.set_box_reg(0, outer);
    let baseline = stores.box_reg(0).expect("outer box should be stored");
    let snapshot = stores.checkpoint();

    stores.enter_group();
    let inner = one_char(&mut stores, 'i');
    stores.set_box_reg(0, inner);
    assert_eq!(stores.testing_live_survivor_slot_count(), 2);

    assert_eq!(stores.leave_group(), Vec::<Token>::new());
    assert_eq!(stores.box_reg(0), Some(baseline));
    assert_eq!(stores.testing_live_survivor_slot_count(), 1);
    assert_eq!(stores.testing_survivor_refcount(baseline), 1);

    stores.rollback(&snapshot);
    assert_eq!(stores.box_reg(0), Some(baseline));
    assert_eq!(stores.testing_live_survivor_slot_count(), 1);
    assert_eq!(stores.testing_survivor_refcount(baseline), 1);
}

#[test]
fn global_box_assignment_survives_group_and_journal_owner_survives_rollback() {
    let mut stores = Stores::new();
    let outer = one_char(&mut stores, 'o');
    stores.set_box_reg(0, outer);
    let baseline = stores.box_reg(0).expect("outer box should be stored");
    let snapshot = stores.checkpoint();

    stores.enter_group();
    let global = one_char(&mut stores, 'g');
    stores.set_box_reg_global(0, global);
    let global = stores.box_reg(0).expect("global box should be stored");

    assert_eq!(stores.leave_group(), Vec::<Token>::new());
    assert_eq!(stores.box_reg(0), Some(global));
    assert_eq!(stores.testing_survivor_refcount(global), 1);
    assert_eq!(stores.testing_survivor_refcount(baseline), 1);

    stores.rollback(&snapshot);
    assert_eq!(stores.box_reg(0), Some(baseline));
    assert_eq!(stores.testing_live_survivor_slot_count(), 1);
    assert_eq!(stores.testing_survivor_refcount(baseline), 1);
}

#[test]
fn same_value_global_box_adds_only_journal_owner() {
    let mut stores = Stores::new();
    let list = one_char(&mut stores, 'a');
    stores.set_box_reg(0, list);
    let survivor = stores.box_reg(0).expect("box should be stored");
    let snapshot = stores.checkpoint();

    stores.enter_group();
    stores.set_box_reg_global(0, survivor);
    assert_eq!(stores.testing_survivor_refcount(survivor), 2);
    assert_eq!(stores.leave_group(), Vec::<Token>::new());
    assert_eq!(stores.testing_survivor_refcount(survivor), 2);

    stores.rollback(&snapshot);
    assert_eq!(stores.box_reg(0), Some(survivor));
    assert_eq!(stores.testing_survivor_refcount(survivor), 1);
}

#[test]
fn same_value_local_box_assignment_preserves_live_register_owner() {
    let mut stores = Stores::new();
    let list = one_char(&mut stores, 'a');
    stores.set_box_reg(0, list);
    let survivor = stores.box_reg(0).expect("box should be stored");

    stores.set_box_reg(0, survivor);

    assert_eq!(stores.box_reg(0), Some(survivor));
    assert_eq!(stores.testing_survivor_refcount(survivor), 1);
    assert_eq!(
        stores.nodes(survivor),
        &[Node::Char {
            font: NULL_FONT,
            ch: 'a'
        }]
    );
}

#[test]
fn local_box_after_global_drops_local_survivor_on_group_exit() {
    let mut stores = Stores::new();
    let outer = one_char(&mut stores, 'o');
    stores.set_box_reg(0, outer);
    let baseline = stores.box_reg(0).expect("outer box should be stored");
    let snapshot = stores.checkpoint();

    stores.enter_group();
    let global = one_char(&mut stores, 'g');
    stores.set_box_reg_global(0, global);
    let global = stores.box_reg(0).expect("global box should be stored");
    let local = one_char(&mut stores, 'l');
    stores.set_box_reg(0, local);
    assert_eq!(stores.testing_live_survivor_slot_count(), 3);

    assert_eq!(stores.leave_group(), Vec::<Token>::new());
    assert_eq!(stores.box_reg(0), Some(global));
    assert_eq!(stores.testing_live_survivor_slot_count(), 2);
    assert_eq!(stores.testing_survivor_refcount(global), 1);
    assert_eq!(stores.testing_survivor_refcount(baseline), 1);

    stores.rollback(&snapshot);
    assert_eq!(stores.box_reg(0), Some(baseline));
    assert_eq!(stores.testing_live_survivor_slot_count(), 1);
    assert_eq!(stores.testing_survivor_refcount(baseline), 1);
}

#[test]
fn promoted_nested_box_remaps_children_to_same_survivor_root() {
    let mut stores = Stores::new();
    let inner = one_char(&mut stores, 'x');
    let middle = stores.freeze_node_list(&[Node::HList(BoxNode::new(BoxNodeFields {
        width: scaled(10),
        height: scaled(7),
        depth: scaled(3),
        shift: scaled(0),
        display: false,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: inner,
    }))]);
    let outer = stores.freeze_node_list(&[Node::VList(BoxNode::new(BoxNodeFields {
        width: scaled(20),
        height: scaled(9),
        depth: scaled(4),
        shift: scaled(0),
        display: false,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: middle,
    }))]);

    stores.set_box_reg(0, outer);
    let promoted_outer = stores.box_reg(0).expect("box should be promoted");
    let Some(crate::node_arena::NodeRef::VList(outer_box)) = stores.nodes(promoted_outer).first()
    else {
        panic!("outer survivor list should contain one vlist");
    };
    assert_same_root(promoted_outer, outer_box.children);
    let Some(crate::node_arena::NodeRef::HList(middle_box)) =
        stores.nodes(outer_box.children).first()
    else {
        panic!("middle survivor list should contain one hlist");
    };
    assert_same_root(promoted_outer, middle_box.children);
    assert_eq!(
        stores.nodes(middle_box.children),
        &[Node::Char {
            font: NULL_FONT,
            ch: 'x'
        }]
    );
}

#[test]
fn promotion_canonicalizes_shared_survivor_children_into_new_root() {
    let mut stores = Stores::new();
    let child = one_char(&mut stores, 'x');
    stores.set_box_reg(0, child);
    let child = stores.box_reg(0).expect("child box should be promoted");
    let fields = BoxNodeFields {
        width: scaled(10),
        height: scaled(7),
        depth: scaled(3),
        shift: scaled(0),
        display: false,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: child,
    };
    let outer = stores.freeze_node_list(&[
        Node::HList(BoxNode::new(fields)),
        Node::VList(BoxNode::new(fields)),
    ]);

    stores.set_box_reg(255, outer);
    let promoted = stores.box_reg(255).expect("outer box should be promoted");
    let nodes = stores.nodes(promoted);
    let (
        Some(crate::node_arena::NodeRef::HList(first)),
        Some(crate::node_arena::NodeRef::VList(second)),
    ) = (nodes.get(0), nodes.get(1))
    else {
        panic!("promoted root should preserve both wrapper boxes");
    };

    assert_same_root(promoted, first.children);
    assert_eq!(
        first.children, second.children,
        "shared child is copied once"
    );
    assert_eq!(
        stores.nodes(first.children),
        &[Node::Char {
            font: NULL_FONT,
            ch: 'x'
        }]
    );
}

#[test]
fn promotion_patches_every_child_bearing_compact_row() {
    let mut stores = Stores::new();
    let child = one_char(&mut stores, 'c');
    let box_node = BoxNode::new(BoxNodeFields {
        width: scaled(1),
        height: scaled(2),
        depth: scaled(3),
        shift: scaled(4),
        display: false,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: child,
    });
    let noad = MathNoad {
        kind: NoadKind::Normal(NoadClass::Ord),
        nucleus: MathField::SubBox(child),
        subscript: MathField::SubMlist(child),
        superscript: MathField::SubBox(child),
    };
    let root = stores.freeze_node_list(&[
        Node::HList(box_node),
        Node::VList(box_node),
        Node::Unset(UnsetNode::new(UnsetNodeFields {
            kind: UnsetKind::HBox,
            width: scaled(5),
            height: scaled(6),
            depth: scaled(7),
            span_count: 2,
            stretch: scaled(8),
            stretch_order: Order::Fil,
            shrink: scaled(9),
            shrink_order: Order::Fill,
            children: child,
        })),
        Node::Glue {
            spec: GlueId::ZERO,
            kind: GlueKind::Leaders,
            leader: Some(LeaderPayload::HList(box_node)),
        },
        Node::Disc {
            kind: DiscKind::Discretionary,
            pre: child,
            post: child,
            replace: child,
        },
        Node::Ins {
            class: 1,
            size: scaled(10),
            split_top_skip: GlueId::ZERO,
            split_max_depth: scaled(11),
            floating_penalty: 12,
            content: child,
        },
        Node::MathNoad(noad),
        Node::FractionNoad(MathFraction {
            numerator: child,
            denominator: child,
            thickness: FractionThickness::Default,
            left_delimiter: None,
            right_delimiter: None,
        }),
        Node::MathChoice(MathChoice {
            display: child,
            text: child,
            script: child,
            script_script: child,
        }),
        Node::MathList(MathListNode {
            display: false,
            content: child,
        }),
        Node::Adjust(child),
    ]);

    stores.set_box_reg(17, root);
    let promoted = stores.box_reg(17).expect("root should be promoted");
    let mut child_count = 0;
    for node in stores.nodes(promoted) {
        let mut children = Vec::new();
        node.to_owned().child_lists(&mut children);
        for child in children {
            assert_same_root(promoted, child);
            assert_eq!(
                stores.nodes(child),
                &[Node::Char {
                    font: NULL_FONT,
                    ch: 'c'
                }]
            );
            child_count += 1;
        }
    }
    assert_eq!(child_count, 19);
}

#[test]
fn promotion_copies_overlapping_source_spans_independently() {
    let mut stores = Stores::new();
    let whole = stores.freeze_node_list(&[Node::Penalty(10), Node::Penalty(20)]);
    let suffix = stores.nodes.testing_subspan(whole, 1, 1);
    let fields = |children| {
        BoxNode::new(BoxNodeFields {
            width: scaled(1),
            height: scaled(1),
            depth: scaled(0),
            shift: scaled(0),
            display: false,
            glue_set: GlueSetRatio::ZERO,
            glue_sign: Sign::Normal,
            glue_order: Order::Normal,
            children,
        })
    };
    let root = stores.freeze_node_list(&[Node::HList(fields(whole)), Node::HList(fields(suffix))]);

    stores.set_box_reg(18, root);
    let promoted = stores.box_reg(18).expect("root should be promoted");
    let nodes = stores.nodes(promoted);
    let (
        Some(crate::node_arena::NodeRef::HList(whole)),
        Some(crate::node_arena::NodeRef::HList(suffix)),
    ) = (nodes.get(0), nodes.get(1))
    else {
        panic!("wrapper nodes should survive promotion");
    };
    assert_ne!(whole.children.start(), suffix.children.start());
    assert_eq!(
        stores.nodes(whole.children),
        &[Node::Penalty(10), Node::Penalty(20)]
    );
    assert_eq!(stores.nodes(suffix.children), &[Node::Penalty(20)]);
}

#[test]
fn mag_parameter_defaults_and_rolls_back_through_stores() {
    let mut stores = Stores::new();
    assert_eq!(stores.mag(), 1000);
    assert_eq!(stores.int_param(IntParam::MAG), 1000);

    let snapshot = stores.checkpoint();
    stores.set_mag(2000);
    assert_eq!(stores.mag(), 2000);

    stores.rollback(&snapshot);
    assert_eq!(stores.mag(), 1000);
}

#[test]
fn prepare_mag_coerces_illegal_values_and_rolls_back_freeze() {
    let mut stores = Stores::new();
    let snapshot = stores.checkpoint();
    stores.set_mag(0);

    let (prepared, diagnostic) = stores.prepare_mag();

    assert_eq!(prepared, 1000);
    assert_eq!(stores.mag(), 1000);
    assert_eq!(stores.prepared_mag(), Some(1000));
    assert_eq!(
        diagnostic,
        Some(PrepareMagDiagnostic::IllegalMagnification { attempted: 0 })
    );

    stores.rollback(&snapshot);
    assert_eq!(stores.mag(), 1000);
    assert_eq!(stores.prepared_mag(), None);
}

#[test]
fn prepare_mag_retain_first_job_magnification() {
    let mut stores = Stores::new();
    stores.set_mag(1200);
    assert_eq!(stores.prepare_mag(), (1200, None));

    stores.set_mag(2000);
    let (prepared, diagnostic) = stores.prepare_mag();

    assert_eq!(prepared, 1200);
    assert_eq!(stores.mag(), 1200);
    assert_eq!(stores.prepared_mag(), Some(1200));
    assert_eq!(
        diagnostic,
        Some(PrepareMagDiagnostic::IncompatibleMagnification {
            attempted: 2000,
            retained: 1200
        })
    );
}

#[test]
fn promotion_handles_pathologically_deep_box_nesting() {
    let mut stores = Stores::new();
    let mut current = one_char(&mut stores, 'x');
    for _ in 0..4096 {
        current = stores.freeze_node_list(&[Node::HList(BoxNode::new(BoxNodeFields {
            width: scaled(1),
            height: scaled(1),
            depth: scaled(0),
            shift: scaled(0),
            display: false,
            glue_set: GlueSetRatio::ZERO,
            glue_sign: Sign::Normal,
            glue_order: Order::Normal,
            children: current,
        }))]);
    }

    stores.set_box_reg(0, current);
    let mut promoted = stores.box_reg(0).expect("box should be promoted");
    for _ in 0..4096 {
        let Some(crate::node_arena::NodeRef::HList(box_node)) = stores.nodes(promoted).first()
        else {
            panic!("deep promoted chain should remain hlist nodes");
        };
        assert_same_root(promoted, box_node.children);
        promoted = box_node.children;
    }
    assert_eq!(
        stores.nodes(promoted),
        &[Node::Char {
            font: NULL_FONT,
            ch: 'x'
        }]
    );
}

#[test]
fn survivor_clone_to_epoch_is_iterative_and_child_first() {
    const DEPTH: usize = 8_192;
    let mut stores = Stores::new();
    let mut current = one_char(&mut stores, 'd');
    for _ in 0..DEPTH {
        current = stores.freeze_node_list(&[Node::HList(BoxNode::new(BoxNodeFields {
            width: scaled(1),
            height: scaled(1),
            depth: scaled(0),
            shift: scaled(0),
            display: false,
            glue_set: GlueSetRatio::ZERO,
            glue_sign: Sign::Normal,
            glue_order: Order::Normal,
            children: current,
        }))]);
    }
    stores.set_box_reg(19, current);
    let survivor = stores.box_reg(19).expect("deep graph should be promoted");

    let mut cloned = stores.clone_node_list_to_epoch(survivor);
    assert!(matches!(cloned.arena(), ArenaRef::Epoch));
    for _ in 0..DEPTH {
        let parent = cloned;
        let Some(crate::node_arena::NodeRef::HList(value)) = stores.nodes(parent).first() else {
            panic!("deep clone should retain hlist shape");
        };
        assert!(matches!(value.children.arena(), ArenaRef::Epoch));
        let child_span = stores.nodes.span(value.children).expect("child is live");
        let parent_span = stores.nodes.span(parent).expect("parent is live");
        assert!(child_span.start + child_span.len <= parent_span.start);
        cloned = value.children;
    }
    assert_eq!(
        stores.nodes(cloned),
        &[Node::Char {
            font: NULL_FONT,
            ch: 'd'
        }]
    );
}

#[test]
fn mixed_epoch_survivor_clone_memoizes_shared_exact_spans() {
    let mut stores = Stores::new();
    let child = one_char(&mut stores, 's');
    stores.set_box_reg(20, child);
    let survivor = stores.box_reg(20).expect("child should be promoted");
    let fields = BoxNodeFields {
        width: scaled(1),
        height: scaled(1),
        depth: scaled(0),
        shift: scaled(0),
        display: false,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: survivor,
    };
    let mixed = stores.freeze_node_list(&[
        Node::HList(BoxNode::new(fields)),
        Node::VList(BoxNode::new(fields)),
    ]);

    let cloned = stores.clone_node_list_to_epoch(mixed);
    let nodes = stores.nodes(cloned);
    let (
        Some(crate::node_arena::NodeRef::HList(first)),
        Some(crate::node_arena::NodeRef::VList(second)),
    ) = (nodes.get(0), nodes.get(1))
    else {
        panic!("mixed clone should retain wrapper nodes");
    };
    assert_eq!(first.children, second.children, "shared span cloned once");
    assert!(matches!(first.children.arena(), ArenaRef::Epoch));
    let child_span = stores.nodes.span(first.children).expect("child is live");
    let parent_span = stores.nodes.span(cloned).expect("parent is live");
    assert!(child_span.start + child_span.len <= parent_span.start);
    assert_eq!(
        stores.nodes(first.children),
        &[Node::Char {
            font: NULL_FONT,
            ch: 's'
        }]
    );
}

fn glue_spec(width: i32) -> GlueSpec {
    GlueSpec {
        width: Scaled::from_raw(width),
        stretch: Scaled::from_raw(2),
        stretch_order: Order::Fil,
        shrink: Scaled::from_raw(3),
        shrink_order: Order::Fill,
    }
}

fn one_char(stores: &mut Stores, ch: char) -> NodeListId {
    stores.freeze_node_list(&[Node::Char {
        font: NULL_FONT,
        ch,
    }])
}

fn assert_same_root(a: NodeListId, b: NodeListId) {
    let (ArenaRef::Survivor(a), ArenaRef::Survivor(b)) = (a.arena(), b.arena()) else {
        panic!("expected survivor ids");
    };
    assert_eq!(a, b);
}

fn scaled(raw: i32) -> Scaled {
    Scaled::from_raw(raw)
}
