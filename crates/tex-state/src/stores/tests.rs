use super::{PrepareMagDiagnostic, Stores};
use crate::env::banks::{DimenParam, GlueParam, IntParam};
use crate::font::NULL_FONT;
use crate::glue::{GlueSpec, Order};
use crate::ids::{ArenaRef, NodeListId, OriginListId};
use crate::macro_store::{MacroDefinitionProvenance, MacroMeaning};
use crate::meaning::Meaning;
use crate::meaning::MeaningFlags;
use crate::node::{BoxNode, BoxNodeFields, Node, Sign};
use crate::scaled::{GlueSetRatio, Scaled};
use crate::token::{Catcode, OriginId, Token};
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
    assert_eq!(stores.tokens(reused), &[crate::token::Token::param(2)]);
}

#[test]
fn token_list_builder_finishes_through_stores_boundary() {
    let mut stores = Stores::new();
    let symbol = stores.intern("macro");
    let mut builder = stores.token_list_builder();
    builder.push(crate::token::Token::Cs(symbol));
    builder.push(crate::token::Token::param(1));

    let id = stores.finish_token_list(&mut builder);

    assert!(builder.is_empty());
    assert_eq!(
        stores.tokens(id),
        &[
            crate::token::Token::Cs(symbol),
            crate::token::Token::param(1)
        ]
    );

    builder.push(crate::token::Token::param(2));
    let reused = stores.finish_token_list(&mut builder);
    assert_eq!(stores.tokens(reused), &[crate::token::Token::param(2)]);
}

#[test]
fn provenance_records_and_lists_round_trip_through_stores_boundary() {
    let mut stores = Stores::new();
    let symbol = stores.intern("m");
    let params = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[Token::Cs(symbol)]);
    let definition = stores.intern_macro(MacroMeaning::new(MeaningFlags::EMPTY, params, body));
    let source = stores.source_origin(SourceId::new(3), 40, 5, 2);
    let macro_origin = stores.macro_invocation_origin(definition, source, OriginId::UNKNOWN);
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
            OriginId::UNKNOWN
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

    assert_eq!(reused.raw(), stale.raw());
    assert_eq!(reused_list.raw(), stale_list.raw());
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
    let body = stores.intern_token_list(&[Token::param(1), Token::Cs(symbol)]);
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
    let first_body = stores.intern_token_list(&[Token::param(1), Token::Cs(a)]);
    let second_body = stores.intern_token_list(&[Token::param(1), Token::Cs(a)]);
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
    let body = stores.intern_token_list(&[Token::Cs(symbol)]);
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
    let body = stores.intern_token_list(&[Token::Cs(symbol)]);
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
    let reused_body = stores.intern_token_list(&[Token::Cs(symbol)]);
    let reused = stores.intern_macro(MacroMeaning::new(
        MeaningFlags::PROTECTED,
        params,
        reused_body,
    ));

    assert_eq!(stores.macro_definition(kept).replacement_text(), kept_body);
    assert_eq!(reused.raw(), stale.raw());
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
    assert_eq!(stores.glue(reused), glue_spec(2));
    assert_eq!(stores.glue(crate::ids::GlueId::ZERO), GlueSpec::ZERO);
}

#[test]
fn paragraph_layout_defaults_match_plain_tex_format() {
    let stores = Stores::new();

    assert_eq!(stores.int_param(IntParam::PRETOLERANCE), 100);
    assert_eq!(stores.int_param(IntParam::TOLERANCE), 200);
    assert_eq!(stores.dimen_param(DimenParam::OVERFULL_RULE), scaled_pt(5));
    assert_eq!(stores.dimen_param(DimenParam::MAX_DEPTH), scaled_pt(4));
    assert_eq!(
        stores.glue(stores.glue_param(GlueParam::BASELINE_SKIP)),
        GlueSpec {
            width: scaled_pt(12),
            stretch: scaled(0),
            stretch_order: Order::Normal,
            shrink: scaled(0),
            shrink_order: Order::Normal,
        }
    );
    assert_eq!(
        stores.glue(stores.glue_param(GlueParam::PAR_FILL_SKIP)),
        GlueSpec {
            width: scaled(0),
            stretch: scaled_pt(1),
            stretch_order: Order::Fil,
            shrink: scaled(0),
            shrink_order: Order::Normal,
        }
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
    stores.freeze_node_list(&[Node::Adjust(stale)]);
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
    let [Node::VList(outer_box)] = stores.nodes(promoted_outer) else {
        panic!("outer survivor list should contain one vlist");
    };
    assert_same_root(promoted_outer, outer_box.children);
    let [Node::HList(middle_box)] = stores.nodes(outer_box.children) else {
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
        let [Node::HList(box_node)] = stores.nodes(promoted) else {
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

fn scaled_pt(points: i32) -> Scaled {
    Scaled::from_raw(points * Scaled::UNITY)
}
