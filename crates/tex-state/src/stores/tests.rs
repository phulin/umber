use super::{PrepareMagDiagnostic, Stores};
use crate::cell::{BankTag, CellId};
use crate::env::banks::{DimenParam, GlueParam, IntParam};
use crate::font::NULL_FONT;
use crate::glue::{GlueSpec, Order};
use crate::hyphenation::{ExceptionSpec, PatternSpec};
use crate::ids::{ArenaRef, MacroDefinitionId, NodeListId};
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
use crate::node_arena::NodeListRef;
use crate::scaled::{GlueSetRatio, Scaled};
use crate::source_map::SourceDescriptor;
use crate::state_hash::StateHasher;
use crate::stores::EngineStackUsage;
use crate::token::{Catcode, OriginId, RootedTracedTokenBuffer, RootedTracedTokenWord, Token};
use crate::world::InputRecordId;
use crate::{
    input::SourceId,
    provenance::{
        InsertedOrigin, InsertedOriginKind, MacroInvocationOrigin, OriginRecord, SourceOrigin,
        SynthesizedOrigin, SynthesizedOriginKind, SyntheticOrigin, SyntheticOriginKind,
    },
};

trait StructuralBoxTestExt {
    fn install_box(&mut self, index: u16, root: NodeListRef);
    fn install_box_global(&mut self, index: u16, root: NodeListRef);
    fn box_owner(&self, index: u16) -> Option<NodeListRef>;
    fn take_box_owner(&mut self, index: u16) -> Option<NodeListRef>;
    fn take_box_owner_same_level(&mut self, index: u16) -> Option<NodeListRef>;
}

impl StructuralBoxTestExt for Stores {
    fn install_box(&mut self, index: u16, root: NodeListRef) {
        let _ = self.write_box_reg_ref(index, Some(root), false);
    }

    fn install_box_global(&mut self, index: u16, root: NodeListRef) {
        let _ = self.write_box_reg_ref(index, Some(root), true);
    }

    fn box_owner(&self, index: u16) -> Option<NodeListRef> {
        self.box_reg_ref(index)
    }

    fn take_box_owner(&mut self, index: u16) -> Option<NodeListRef> {
        self.take_box_reg_ref_with_receipt(index).0
    }

    fn take_box_owner_same_level(&mut self, index: u16) -> Option<NodeListRef> {
        self.take_box_reg_ref_same_level_with_receipt(index).0
    }
}

#[test]
fn recursive_box_copy_below_existing_coordinates_does_not_raise_extents() {
    let mut stores = Stores::new();
    let root = stores.freeze_node_list(&[Node::Char {
        font: NULL_FONT,
        ch: 'A',
        origin: crate::provenance::OriginRef::unknown(),
    }]);
    stores.install_box(0, root);
    let root = stores.box_owner(0).expect("promoted box root");
    stores.observe_main_memory();

    // A copy operation only changes §1334's retained coordinates when its
    // concurrent operation peak exceeds them. This is the phase/lifetime
    // negative exercised by e-TRIP's six copies: their owners are real, but
    // all of their composed peaks remain below the prior allocator record.
    stores.memory_low_extent = 2_021;
    stores.memory_high_extent = 1_296;
    stores.observe_main_memory_box_copy(&root, 0);
    assert_eq!(stores.memory_low_extent, 2_021);
    assert_eq!(stores.memory_high_extent, 1_296);
}

#[test]
fn engine_stack_usage_merges_runtime_high_water_by_owner() {
    let mut stores = Stores::new();
    stores.record_engine_stack_usage(EngineStackUsage {
        input_stack: 7,
        nest_stack: 3,
        parameter_stack: 9,
        buffer_stack: 20,
        save_stack: 5,
    });
    stores.record_engine_stack_usage(EngineStackUsage {
        input_stack: 2,
        nest_stack: 8,
        parameter_stack: 1,
        buffer_stack: 11,
        save_stack: 13,
    });

    let usage = stores.engine_usage_statistics();
    assert_eq!(usage.input_stack, 7);
    assert_eq!(usage.nest_stack, 8);
    assert_eq!(usage.parameter_stack, 9);
    assert_eq!(usage.buffer_stack, 20);
    assert_eq!(usage.save_stack, 13);
}

#[test]
fn string_pool_checkpoint_marks_recycled_strings_without_cloning_them() {
    let mut stores = Stores::new();
    stores.string_pool.recycling_enabled = true;
    stores.remember_pool_string("format-name");
    let journal_before = stores.string_pool_recycled_journal.len();
    let owners_before = std::sync::Arc::strong_count(
        stores
            .string_pool
            .recycled
            .get("format-name")
            .expect("retained format name"),
    );
    let snapshot = stores.checkpoint();

    // Web2C tex.ch [29.517] searches the retained pool without changing it.
    // Aggregate operation checkpoints therefore retain only the append mark,
    // not a cloned membership tree or another owner for every pooled string.
    assert_eq!(snapshot.string_pool_recycled_mark, journal_before);
    assert_eq!(
        std::sync::Arc::strong_count(
            stores
                .string_pool
                .recycled
                .get("format-name")
                .expect("retained format name"),
        ),
        owners_before
    );
    let baseline = stores.engine_usage_statistics();
    stores.slow_make_pool_string("format-name");
    assert_eq!(stores.engine_usage_statistics(), baseline);
    assert_eq!(stores.string_pool_recycled_journal.len(), journal_before);

    stores.slow_make_pool_string("runtime-name");
    assert_eq!(
        stores.string_pool_recycled_journal.len(),
        journal_before + 1
    );
    assert_eq!(stores.engine_usage_statistics().strings, 1);

    stores.rollback(&snapshot);
    assert_eq!(stores.string_pool_recycled_journal.len(), journal_before);
    assert!(!stores.string_pool.recycled.contains("runtime-name"));
    assert_eq!(stores.string_pool.used_strings(), 0);
    assert_eq!(stores.string_pool.used_characters(), 0);
    stores.slow_make_pool_string("runtime-name");
    assert_eq!(stores.string_pool.used_strings(), 1);
    assert_eq!(stores.string_pool.used_characters(), "runtime-name".len());
}

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
    let empty = crate::node_arena::NodeListRef::empty();
    let tokens = stores.intern_token_list(&[]);
    let box_node = BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(1),
        height: Scaled::from_raw(2),
        depth: Scaled::from_raw(3),
        shift: Scaled::from_raw(4),
        box_lr: crate::node::BoxLr::DList,
        glue_set: GlueSetRatio::from_raw(5),
        glue_sign: Sign::Shrinking,
        glue_order: Order::Fill,
        children: empty.clone(),
    });
    let nodes = vec![
        Node::Char {
            font: NULL_FONT,
            ch: 'x',
            origin: crate::provenance::OriginRef::unknown(),
        },
        Node::Char {
            font: NULL_FONT,
            ch: 'y',
            origin: crate::provenance::OriginRef::unknown(),
        },
        Node::Lig {
            font: NULL_FONT,
            ch: 'f',
            orig: vec!['f', 'i'],
            origins: vec![crate::provenance::OriginRef::unknown(); 2],
            left_hit: true,
            right_hit: true,
        },
        Node::Kern {
            amount: Scaled::from_raw(-6),
            kind: KernKind::Mu,
        },
        Node::MarginKern {
            amount: Scaled::from_raw(-7),
            side: crate::node::MarginKernSide::Right,
            font: NULL_FONT,
            ch: b'x',
        },
        Node::Glue {
            spec: crate::glue::testing_zero_glue_ref(),
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
        Node::HList(box_node.clone()),
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
            children: empty.clone(),
        })),
        Node::Disc {
            kind: DiscKind::AutomaticHyphen,
            pre: empty.clone(),
            post: empty.clone(),
            replace: empty.clone(),
            physical_replace_count: 0,
        },
        Node::Mark {
            class: 3,
            tokens: stores.token_list_ref(tokens),
        },
        Node::Ins {
            class: 4,
            size: Scaled::from_raw(16),
            split_top_skip: crate::glue::testing_zero_glue_ref(),
            split_max_depth: Scaled::from_raw(17),
            floating_penalty: -18,
            content: empty.clone(),
        },
        Node::Whatsit(Whatsit::Language {
            language: 19,
            left_hyphen_min: 2,
            right_hyphen_min: 3,
        }),
        Node::MathOn(Scaled::from_raw(20)),
        Node::MathOff(Scaled::from_raw(21)),
        Node::Direction(crate::node::Direction::EndR),
        Node::MathNoad(MathNoad::new(
            NoadKind::Normal(NoadClass::Ord),
            MathField::SubMlist(empty.clone()),
        )),
        Node::FractionNoad(MathFraction {
            numerator: empty.clone(),
            denominator: empty.clone(),
            thickness: FractionThickness::Explicit(Scaled::from_raw(22)),
            left_delimiter: Some(23),
            right_delimiter: None,
        }),
        Node::MathStyle(MathStyle::ScriptScript),
        Node::MathChoice(MathChoice {
            display: empty.clone(),
            text: empty.clone(),
            script: empty.clone(),
            script_script: empty.clone(),
        }),
        Node::MathList(MathListNode {
            display: true,
            content: empty.clone(),
        }),
        Node::Nonscript,
        Node::Adjust(crate::node::AdjustNode::ordinary(empty)),
    ];
    let id = stores.freeze_node_list(&nodes);
    assert_eq!(id.semantic_id(), stores.compute_node_semantic_id(&nodes));
}

#[test]
fn node_semantic_ids_are_canonical_and_compose_from_children() {
    fn nested(stores: &mut Stores, penalty: i32) -> (NodeListRef, NodeListRef) {
        let child = stores.freeze_node_list(&[Node::Penalty(penalty)]);
        let child_ref = child.clone();
        let root =
            stores.freeze_node_list(&[Node::Adjust(crate::node::AdjustNode::ordinary(child_ref))]);
        (child, root)
    }

    let mut direct = Stores::new();
    let (direct_child, direct_root) = nested(&mut direct, 10);

    let mut shifted = Stores::new();
    let _unrelated = shifted.freeze_node_list(&[Node::Penalty(999)]);
    let (shifted_child, shifted_root) = nested(&mut shifted, 10);
    let (_, different_root) = nested(&mut shifted, 11);

    assert_ne!(
        direct_child.id(),
        shifted_child.id(),
        "runtime allocation differs"
    );
    assert_eq!(direct_child.semantic_id(), shifted_child.semantic_id());
    assert_eq!(direct_root.semantic_id(), shifted_root.semantic_id());
    assert_ne!(shifted_root.semantic_id(), different_root.semantic_id());

    let mut builder = shifted.node_list_builder();
    builder.push(Node::Adjust(crate::node::AdjustNode::ordinary(
        shifted_child,
    )));
    let built_root = shifted.finish_node_list(&mut builder);
    assert_eq!(built_root.semantic_id(), shifted_root.semantic_id());

    let mut fork = direct.clone();
    let (_, fork_root) = nested(&mut fork, 10);
    assert_eq!(fork_root.semantic_id(), direct_root.semantic_id());
}

#[test]
fn box_lr_is_part_of_canonical_node_semantic_identity() {
    let mut stores = Stores::new();
    let empty = crate::node_arena::NodeListRef::empty();
    let mut identities = Vec::new();
    for box_lr in [
        crate::node::BoxLr::Normal,
        crate::node::BoxLr::Reversed,
        crate::node::BoxLr::DList,
    ] {
        let list = stores.freeze_node_list(&[Node::HList(BoxNode::new(BoxNodeFields {
            width: scaled(0),
            height: scaled(0),
            depth: scaled(0),
            shift: scaled(0),
            box_lr,
            glue_set: GlueSetRatio::ZERO,
            glue_sign: Sign::Normal,
            glue_order: Order::Normal,
            children: empty.clone(),
        }))]);
        identities.push(list.semantic_id());
    }
    assert_ne!(identities[0], identities[1]);
    assert_ne!(identities[0], identities[2]);
    assert_ne!(identities[1], identities[2]);
}

#[test]
fn owned_node_freeze_reuses_the_source_vector_and_preserves_identity() {
    let mut borrowed = Stores::new();
    let expected = vec![
        Node::Penalty(17),
        Node::Whatsit(Whatsit::Special {
            class: "owned".into(),
            payload: vec![1, 2, 3, 4],
        }),
        Node::Penalty(23),
    ];
    let borrowed_id = borrowed.freeze_node_list(&expected);

    let mut owned = Stores::new();
    let mut nodes = Vec::with_capacity(8);
    nodes.extend(expected.clone());
    let capacity = nodes.capacity();
    let owned_id = owned.freeze_node_list_owned(&mut nodes);

    assert!(nodes.is_empty());
    assert_eq!(nodes.capacity(), capacity);
    assert_eq!(borrowed_id.semantic_id(), owned_id.semantic_id());
    assert_eq!(owned_id.nodes().to_vec(), expected);
}

#[test]
fn frozen_font_semantics_exclude_mutable_identifier_names() {
    let mut stores = Stores::new();
    let snapshot = stores.checkpoint();
    let list = stores.freeze_node_list(&[Node::Char {
        font: NULL_FONT,
        ch: 'x',
        origin: crate::provenance::OriginRef::unknown(),
    }]);
    let semantic_id = list.semantic_id();
    let late = stores.intern("late-font-name");

    stores.set_font_identifier_symbol(NULL_FONT, late);
    assert_eq!(list.semantic_id(), semantic_id);

    let mut fork = stores.clone();
    let later = fork.intern("later-font-name");
    fork.set_font_identifier_symbol(NULL_FONT, later);
    assert_eq!(list.semantic_id(), semantic_id);

    stores.rollback(&snapshot);
    assert_eq!(stores.font_identifier_symbol(NULL_FONT), None);
    let late = stores.intern("late-font-name");
    stores.set_font_identifier_symbol(NULL_FONT, late);
    assert_eq!(stores.font_identifier_symbol(NULL_FONT), Some(late));
}

#[test]
fn node_semantic_ids_and_owners_survive_store_rollback() {
    let mut stores = Stores::new();
    let snapshot = stores.checkpoint();
    let retained = stores.freeze_node_list(&[Node::Penalty(1)]);
    let retained_semantic_id = retained.semantic_id();
    stores.rollback(&snapshot);

    let replacement = stores.freeze_node_list(&[Node::Penalty(2)]);
    assert_ne!(retained, replacement);
    assert_ne!(retained_semantic_id, replacement.semantic_id());
    assert_eq!(retained.semantic_id(), retained_semantic_id);
    assert_eq!(retained.to_vec(), [Node::Penalty(1)]);

    let replacement_ref = replacement;
    let root = stores.freeze_node_list(&[Node::Adjust(crate::node::AdjustNode::ordinary(
        replacement_ref,
    ))]);
    let semantic_id = root.semantic_id();
    stores.install_box(0, root);
    let stored = stores
        .box_reg_ref(0)
        .expect("box assignment retains the list owner");
    assert_eq!(stored.semantic_id(), semantic_id);
}

#[test]
fn compact_and_survivor_nodes_own_mark_tokens_until_rollback() {
    let mut stores = Stores::new();
    let snapshot = stores.checkpoint();
    let tokens = stores.intern_token_list_ref_in_domain(&[Token::param(1)], None);
    let token_id = tokens.id();
    let mut nodes = vec![Node::Mark {
        class: 7,
        tokens: tokens.clone(),
    }];

    let compact = stores.freeze_node_list_owned(&mut nodes);
    assert!(nodes.is_empty(), "owned freeze must move the token root");
    drop(tokens);
    let crate::node_arena::NodeRef::Mark { class, tokens } =
        compact.nodes().first().expect("compact mark")
    else {
        panic!("compact node must retain the mark sidecar")
    };
    assert_eq!(class, 7);
    assert_eq!(tokens.id(), token_id);

    stores.install_box(0, compact);
    let expected_tokens = stores.tokens.owner(token_id).expect("survivor node root");
    let survivor_node = stores
        .box_reg_ref(0)
        .expect("box assignment owns the list")
        .get(0)
        .expect("survivor mark");
    assert_eq!(
        survivor_node,
        Node::Mark {
            class: 7,
            tokens: expected_tokens,
        }
    );
    drop(survivor_node);

    stores.rollback(&snapshot);
    assert!(
        stores.tokens.owner(token_id).is_none(),
        "final typed node owner must release the token value"
    );
}

#[test]
fn node_semantic_ids_exclude_token_provenance() {
    let mut stores = Stores::new();
    let token = Token::Char {
        ch: 'x',
        cat: Catcode::Other,
    };
    let first_origin = stores.synthetic_origin_ref(SyntheticOriginKind::Test);
    let second_origin = stores.synthetic_origin_ref(SyntheticOriginKind::Engine);
    let first_buffer =
        RootedTracedTokenBuffer::new([RootedTracedTokenWord::new(token, first_origin)]);
    let second_buffer =
        RootedTracedTokenBuffer::new([RootedTracedTokenWord::new(token, second_origin)]);
    let first_tokens = stores.finish_rooted_traced_token_list_in_domain(&first_buffer, None);
    let second_tokens = stores.finish_rooted_traced_token_list_in_domain(&second_buffer, None);
    assert_ne!(first_tokens.origin_list(), second_tokens.origin_list());

    let first = stores.freeze_node_list(&[Node::Mark {
        class: 0,
        tokens: first_tokens.token_ref().clone(),
    }]);
    let second = stores.freeze_node_list(&[Node::Mark {
        class: 0,
        tokens: second_tokens.token_ref().clone(),
    }]);
    assert!(first.shares_payload(&second));
    assert_eq!(first.semantic_id(), second.semantic_id());
}

#[test]
fn character_origins_are_retained_but_excluded_from_node_semantics() {
    let mut stores = Stores::new();
    let first_origin = stores.synthetic_origin_ref(SyntheticOriginKind::Test);
    let second_origin = stores.synthetic_origin_ref(SyntheticOriginKind::Engine);
    let first = stores.freeze_node_list(&[Node::Char {
        font: NULL_FONT,
        ch: 'x',
        origin: first_origin.clone(),
    }]);
    let second = stores.freeze_node_list(&[Node::Char {
        font: NULL_FONT,
        ch: 'x',
        origin: second_origin.clone(),
    }]);

    assert_eq!(first.semantic_id(), second.semantic_id());
    let Some(crate::node_arena::NodeRef::Char {
        origin: retained_first,
        ..
    }) = first.nodes().first()
    else {
        panic!("first character")
    };
    let Some(crate::node_arena::NodeRef::Char {
        origin: retained_second,
        ..
    }) = second.nodes().first()
    else {
        panic!("second character")
    };
    assert_eq!(retained_first, first_origin.id());
    assert_eq!(retained_second, second_origin.id());

    let first_math = stores.freeze_node_list(&[Node::MathNoad(MathNoad::new(
        NoadKind::Normal(NoadClass::Ord),
        MathField::MathChar(crate::math::MathChar {
            family: 1,
            character: 'x',
            origin: first_origin.id(),
        }),
    ))]);
    let second_math = stores.freeze_node_list(&[Node::MathNoad(MathNoad::new(
        NoadKind::Normal(NoadClass::Ord),
        MathField::MathChar(crate::math::MathChar {
            family: 1,
            character: 'x',
            origin: second_origin.id(),
        }),
    ))]);
    assert_eq!(first_math.semantic_id(), second_math.semantic_id());
}

#[test]
fn semantic_projection_visits_only_outer_nodes() {
    let mut stores = Stores::new();
    let mut nested = stores.freeze_node_list(&[Node::Penalty(1)]);
    for _ in 0..512 {
        let nested_ref = nested;
        nested =
            stores.freeze_node_list(&[Node::Adjust(crate::node::AdjustNode::ordinary(nested_ref))]);
    }

    let nested_ref = nested;
    let outer = [
        Node::Adjust(crate::node::AdjustNode::ordinary(nested_ref)),
        Node::Penalty(2),
    ];
    let mut hasher = StateHasher::new(0x6f75_7465_725f_6e64);
    let visits = stores.hash_node_slice_semantic(&outer, &mut hasher);
    assert_eq!(visits, outer.len());

    let mut equivalent = Stores::new();
    let mut equivalent_nested = equivalent.freeze_node_list(&[Node::Penalty(1)]);
    for _ in 0..512 {
        let nested_ref = equivalent_nested;
        equivalent_nested = equivalent
            .freeze_node_list(&[Node::Adjust(crate::node::AdjustNode::ordinary(nested_ref))]);
    }
    let mut equivalent_hasher = StateHasher::new(0x6f75_7465_725f_6e64);
    let equivalent_visits = equivalent.hash_node_slice_semantic(
        &[
            Node::Adjust(crate::node::AdjustNode::ordinary(equivalent_nested)),
            Node::Penalty(2),
        ],
        &mut equivalent_hasher,
    );
    assert_eq!(equivalent_visits, outer.len());
    assert_eq!(hasher.finish(), equivalent_hasher.finish());
}

#[test]
fn adjustment_pre_marker_is_semantic_state() {
    let mut stores = Stores::new();
    let content = stores.freeze_node_list(&[Node::Penalty(17)]);
    let content_ref = content.clone();
    let ordinary = stores.freeze_node_list(&[Node::Adjust(crate::node::AdjustNode::ordinary(
        content_ref.clone(),
    ))]);
    let pre = stores.freeze_node_list(&[Node::Adjust(crate::node::AdjustNode {
        content: content_ref,
        pre: true,
    })]);

    assert_ne!(ordinary.semantic_id(), pre.semantic_id());
    assert!(matches!(
        pre.get(0),
        Some(Node::Adjust(adjust)) if adjust.pre && adjust.content.semantic_id() == content.semantic_id()
    ));
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
    let mut end = stores.checkpoint();
    let _ = stores.state_hash_slice(&cursor, &mut end);

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
fn exact_environment_identity_updates_distinct_journal_cells_and_rolls_back() {
    let mut stores = Stores::new();
    let baseline_cursor = stores.state_hash_cursor();
    let mut baseline = stores.checkpoint();
    let _ = stores.state_hash_slice(&baseline_cursor, &mut baseline);
    let baseline_identity = stores.exact_env_identity();
    let baseline_updates = stores.testing_exact_env_updates();

    stores.set_count(7, 1);
    stores.set_count(7, 2);
    stores.set_count(7, 3);
    let mut changed = stores.checkpoint();
    let _ = stores.state_hash_slice(&baseline_cursor, &mut changed);
    let changed_identity = stores.exact_env_identity();
    assert_ne!(changed_identity, baseline_identity);
    assert_eq!(
        stores.testing_exact_env_updates(),
        baseline_updates + 1,
        "one journal slice must apply one canonical delta per distinct dirty cell"
    );

    let mut rebuilt = stores.clone();
    rebuilt.initialize_exact_env_identity();
    assert_eq!(rebuilt.exact_env_identity(), changed_identity);

    stores.rollback(&baseline);
    assert_eq!(stores.exact_env_identity(), baseline_identity);
    stores.set_count(7, 3);
    let mut replayed = stores.checkpoint();
    let _ = stores.state_hash_slice(&baseline_cursor, &mut replayed);
    assert_eq!(stores.exact_env_identity(), changed_identity);
}

#[test]
fn exact_environment_identity_raw_scalar_restore_is_atomic_and_noop_is_free() {
    let mut stores = Stores::new();
    let cell = crate::cell::CellId::new(crate::cell::BankTag::Count, 7);
    let baseline_cursor = stores.state_hash_cursor();
    let baseline = stores.exact_env_identity();
    let baseline_updates = stores.testing_exact_env_updates();

    let receipt = stores.testing_restore_env_word(cell, 17);
    assert!(receipt.changed());
    assert_ne!(stores.exact_env_identity(), baseline);
    assert_eq!(
        stores.exact_env_identity(),
        stores.testing_recomputed_exact_env_identity()
    );
    assert_eq!(stores.testing_exact_env_updates(), baseline_updates + 1);

    let updates = stores.testing_exact_env_updates();
    let mut checkpoint = stores.checkpoint();
    let _ = stores.state_hash_slice(&baseline_cursor, &mut checkpoint);
    assert_eq!(
        stores.testing_exact_env_updates(),
        updates,
        "journal hashing must not fold the atomically synchronized raw write twice"
    );

    let updates = stores.testing_exact_env_updates();
    let receipt = stores.testing_restore_env_word(cell, 17);
    assert!(!receipt.changed());
    assert_eq!(stores.testing_exact_env_updates(), updates);
    assert_eq!(
        stores.exact_env_identity(),
        stores.testing_recomputed_exact_env_identity()
    );
}

#[test]
fn exact_environment_identity_raw_restore_and_journal_rollback_have_one_owner() {
    let mut stores = Stores::new();
    let baseline_cursor = stores.state_hash_cursor();
    let mut baseline = stores.checkpoint();
    let _ = stores.state_hash_slice(&baseline_cursor, &mut baseline);
    let baseline_identity = stores.exact_env_identity();

    let raw = crate::cell::CellId::new(crate::cell::BankTag::Count, 9);
    let _ = stores.testing_restore_env_word(raw, 23);
    let raw_identity = stores.exact_env_identity();
    assert_ne!(raw_identity, baseline_identity);
    assert_eq!(raw_identity, stores.testing_recomputed_exact_env_identity());

    stores.enter_group();
    let _ = stores.set_count(7, 11);
    let _ = stores.set_count_global(8, 13);
    let _ = stores.leave_group();
    let mut changed = stores.checkpoint();
    let _ = stores.state_hash_slice(&baseline_cursor, &mut changed);
    assert_eq!(
        stores.exact_env_identity(),
        stores.testing_recomputed_exact_env_identity(),
        "group restoration and global retention must fold from the journal once"
    );

    stores.rollback(&baseline);
    assert_eq!(stores.exact_env_identity(), baseline_identity);
    assert_eq!(
        stores.exact_env_identity(),
        stores.testing_recomputed_exact_env_identity(),
        "aggregate rollback restores the snapshot-owned accumulator"
    );
}

#[test]
fn exact_environment_identity_rekeys_font_banks_when_identifier_changes() {
    let mut stores = Stores::new();
    let baseline = stores.checkpoint();
    let unnamed = stores.exact_env_identity();
    let first = stores.intern("nullfont-first");

    stores.set_font_identifier_symbol(NULL_FONT, first);
    let named = stores.exact_env_identity();
    assert_ne!(named, unnamed);
    assert_eq!(named, stores.testing_recomputed_exact_env_identity());

    let updates = stores.testing_exact_env_updates();
    stores.set_font_identifier_symbol(NULL_FONT, first);
    assert_eq!(stores.exact_env_identity(), named);
    assert_eq!(
        stores.testing_exact_env_updates(),
        updates,
        "an identical identifier assignment must not rebuild the Env accumulator"
    );

    let second = stores.intern("nullfont-second");
    stores.set_font_identifier_symbol(NULL_FONT, second);
    assert_ne!(stores.exact_env_identity(), named);
    assert_eq!(
        stores.exact_env_identity(),
        stores.testing_recomputed_exact_env_identity()
    );

    stores.rollback(&baseline);
    assert_eq!(stores.font_identifier_symbol(NULL_FONT), None);
    assert_eq!(stores.exact_env_identity(), unnamed);
    assert_eq!(
        stores.exact_env_identity(),
        stores.testing_recomputed_exact_env_identity()
    );
}

#[test]
fn exact_environment_identity_excludes_empty_save_stack_representation() {
    let mut stores = Stores::new();
    let baseline = stores.exact_env_identity();

    stores.enter_group();
    let _ = stores.leave_group();
    let _ = stores.checkpoint();

    assert_eq!(stores.exact_env_identity(), baseline);
    assert_eq!(
        stores.exact_env_identity(),
        stores.testing_recomputed_exact_env_identity(),
        "group markers, epochs, and checkpoint baselines are representation metadata"
    );
}

#[test]
fn exact_environment_identity_nested_snapshots_restore_isolated_deltas() {
    let mut stores = Stores::new();
    stores.set_count(7, 11);
    let outer = stores.checkpoint();
    let outer_identity = stores.exact_env_identity();

    stores.set_count(7, 13);
    stores.set_dimen(9, Scaled::from_raw(17));
    let inner = stores.checkpoint();
    let inner_identity = stores.exact_env_identity();
    assert_ne!(inner_identity, outer_identity);

    stores.set_count(7, 19);
    stores.set_dimen(10, Scaled::from_raw(23));
    let _latest = stores.checkpoint();
    stores.rollback(&inner);
    assert_eq!(stores.count(7), 13);
    assert_eq!(stores.dimen(9), Scaled::from_raw(17));
    assert_eq!(stores.dimen(10), Scaled::from_raw(0));
    assert_eq!(stores.exact_env_identity(), inner_identity);
    assert_eq!(
        stores.exact_env_identity(),
        stores.testing_recomputed_exact_env_identity()
    );

    stores.set_count(8, 29);
    let _diverged = stores.checkpoint();
    stores.rollback(&outer);
    assert_eq!(stores.count(7), 11);
    assert_eq!(stores.count(8), 0);
    assert_eq!(stores.dimen(9), Scaled::from_raw(0));
    assert_eq!(stores.exact_env_identity(), outer_identity);
    assert_eq!(
        stores.exact_env_identity(),
        stores.testing_recomputed_exact_env_identity()
    );
}

#[test]
fn initex_string_pool_counts_one_unfinished_current_string_across_exceptions() {
    // TeX82 §38 exposes exactly one current string at pool_ptr. Section 934
    // makes each exception word (including its language byte), but multiple
    // exceptions cannot each own another unfinished current-string byte.
    let mut stores = Stores::new();
    let before = stores.engine_usage_statistics();
    stores.add_hyphenation_exception(ExceptionSpec {
        word: "ab".to_owned(),
        positions: vec![1],
    });
    stores.add_hyphenation_exception(ExceptionSpec {
        word: "cde".to_owned(),
        positions: vec![2],
    });
    let after = stores.engine_usage_statistics();

    assert_eq!(after.strings - before.strings, 2);
    assert_eq!(after.string_characters - before.string_characters, 8);
}

#[test]
fn exact_environment_identity_ignores_intern_allocation_order() {
    fn build(filler_first: bool) -> Stores {
        let mut stores = Stores::new();
        if filler_first {
            let _filler = stores.intern("filler");
            let _filler_tokens = stores.intern_token_list(&[Token::param(1)]);
            let _filler_glue = stores.intern_glue(GlueSpec {
                width: Scaled::from_raw(99),
                ..GlueSpec::ZERO
            });
        }

        let target = stores.intern("target");
        let alpha = stores.intern("alpha");
        let tokens = stores.intern_token_list(&[
            Token::Cs(alpha.symbol()),
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            },
        ]);
        let glue = stores.intern_glue(GlueSpec {
            width: Scaled::from_raw(7),
            ..GlueSpec::ZERO
        });
        let definition =
            stores.intern_macro(MacroMeaning::new(MeaningFlags::PROTECTED, tokens, tokens));
        stores.set_meaning(
            target,
            Meaning::Macro {
                flags: MeaningFlags::PROTECTED,
                definition: definition.id(),
            },
        );
        stores.set_toks(0, tokens);
        stores.set_skip(0, glue);
        stores.initialize_exact_env_identity();
        stores
    }

    assert_eq!(
        build(false).exact_env_identity(),
        build(true).exact_env_identity()
    );
}

#[test]
fn env_token_roots_follow_save_stack_rollback_and_generation_fork() {
    let mut stores = Stores::new();
    let outer = stores.intern_token_list(&[Token::Char {
        ch: 'o',
        cat: Catcode::Other,
    }]);
    let local = stores.intern_token_list(&[Token::Char {
        ch: 'l',
        cat: Catcode::Other,
    }]);
    let global = stores.intern_token_list(&[Token::Char {
        ch: 'g',
        cat: Catcode::Other,
    }]);
    let outer_root = stores.tokens.owner(outer).expect("outer root");
    let local_root = stores.tokens.owner(local).expect("local root");
    let global_root = stores.tokens.owner(global).expect("global root");

    stores.set_toks_global(0, outer);
    let outer_current = outer_root.strong_count();
    stores.enter_group();
    stores.set_toks(0, local);
    assert_eq!(stores.toks(0), local);
    assert_eq!(
        outer_root.strong_count(),
        outer_current,
        "open-group undo replaces the displaced current owner exactly"
    );
    let local_current_and_redo = local_root.strong_count();

    // An equal local assignment still creates a save-stack edge at the new
    // epoch. Its old/new owners must be real even though the word is equal.
    stores.enter_group();
    stores.set_toks(0, local);
    assert_eq!(local_root.strong_count(), local_current_and_redo + 2);
    let _ = stores.leave_group();
    assert_eq!(local_root.strong_count(), local_current_and_redo);

    // A later global supersession refiles the first outer owner and keeps the
    // surviving global owner when the group journal is compacted.
    stores.set_toks_global(0, global);
    let global_live = global_root.strong_count();
    let _ = stores.leave_group();
    assert_eq!(stores.toks(0), global);
    assert_eq!(global_root.strong_count(), global_live);
    assert_eq!(
        local_root.strong_count(),
        local_current_and_redo - 2,
        "superseded current and redo owners are released at group exit"
    );

    let snapshot = stores.checkpoint();
    let outer_before_rollback_write = outer_root.strong_count();
    stores.set_toks(0, outer);
    assert_eq!(stores.toks(0), outer);
    assert_eq!(
        outer_root.strong_count(),
        outer_before_rollback_write + 2,
        "current cell and undo redo edge both own the post-checkpoint value"
    );
    stores.rollback(&snapshot);
    assert_eq!(stores.toks(0), global);
    assert_eq!(
        outer_root.strong_count(),
        outer_before_rollback_write,
        "journal truncation releases the rolled-back current and redo roots"
    );

    let fork = stores.clone();
    assert_eq!(fork.toks(0), global);
    assert!(
        fork.env
            .token_root(CellId::new(BankTag::Toks, 0))
            .expect("forked Env token root")
            .ptr_eq(&global_root),
        "generation forks share exact immutable payloads"
    );
}

#[test]
fn macro_definitions_own_parameter_and_replacement_token_children() {
    let mut stores = Stores::new();
    let parameter = stores.intern_token_list(&[Token::param(1)]);
    let replacement = stores.intern_token_list(&[Token::Char {
        ch: 'r',
        cat: Catcode::Other,
    }]);
    let parameter_root = stores.tokens.owner(parameter).expect("parameter root");
    let replacement_root = stores.tokens.owner(replacement).expect("replacement root");
    let parameter_before = parameter_root.strong_count();
    let replacement_before = replacement_root.strong_count();

    let definition = stores.intern_macro(MacroMeaning::new(
        MeaningFlags::from_bits(0),
        parameter,
        replacement,
    ));
    let (stored_parameter, stored_replacement) = stores.macros.testing_token_roots(definition.id());
    assert!(stored_parameter.ptr_eq(&parameter_root));
    assert!(stored_replacement.ptr_eq(&replacement_root));
    drop((stored_parameter, stored_replacement));
    assert_eq!(parameter_root.strong_count(), parameter_before + 2);
    assert_eq!(replacement_root.strong_count(), replacement_before + 2);

    let fork = stores.clone();
    let fork_definition = fork
        .macros
        .resolve_stored(MacroDefinitionId::new(definition.raw()))
        .expect("forked macro definition");
    let (fork_parameter, fork_replacement) = fork.macros.testing_token_roots(fork_definition);
    assert!(fork_parameter.ptr_eq(&parameter_root));
    assert!(fork_replacement.ptr_eq(&replacement_root));
}

#[test]
fn semantic_hash_only_walks_hyphenation_after_root_changes() {
    let mut stores = Stores::new();
    let initial_cursor = stores.state_hash_cursor();
    let mut initial = stores.checkpoint();
    let _ = stores.state_hash_slice(&initial_cursor, &mut initial);
    assert_eq!(
        stores.semantic_hash_cache.testing_hyphenation_hash_calls(),
        1,
        "the first framed projection computes its discardable fingerprint"
    );

    stores
        .add_hyphenation_pattern(PatternSpec {
            letters: "alpha".chars().collect(),
            values: vec![0, 1, 0, 0, 0, 0],
        })
        .expect("pattern fits the default trie capacity");
    let mut with_pattern = stores.checkpoint();
    let _ = stores.state_hash_slice(&initial_cursor, &mut with_pattern);
    assert_eq!(
        stores.semantic_hash_cache.testing_hyphenation_hash_calls(),
        2
    );

    let pattern_cursor = stores.state_hash_cursor_from_snapshot(&with_pattern);
    stores.set_count(0, 1);
    let mut unrelated_change = stores.checkpoint();
    let _ = stores.state_hash_slice(&pattern_cursor, &mut unrelated_change);
    assert_eq!(
        stores.semantic_hash_cache.testing_hyphenation_hash_calls(),
        2,
        "an unrelated state change must not rehash the retained hyphenation root"
    );

    stores.add_hyphenation_exception(ExceptionSpec {
        word: "hyphenation".to_owned(),
        positions: vec![2, 6],
    });
    let mut with_exception = stores.checkpoint();
    let _ = stores.state_hash_slice(
        &stores.state_hash_cursor_from_snapshot(&unrelated_change),
        &mut with_exception,
    );
    assert_eq!(
        stores.semantic_hash_cache.testing_hyphenation_hash_calls(),
        3
    );

    stores.rollback(&with_pattern);
    stores.set_count(0, 2);
    let mut after_rollback = stores.checkpoint();
    let _ = stores.state_hash_slice(&pattern_cursor, &mut after_rollback);
    assert_eq!(
        stores.semantic_hash_cache.testing_hyphenation_hash_calls(),
        2,
        "rollback restores the retained projection for the snapshot's hyphenation root"
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
    assert_eq!(
        cumulative.source_token_origin(SourceId::new(1), 2, 3),
        OriginId::UNKNOWN,
        "an invalid span degrades to unknown instead of aliasing or aborting"
    );
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
fn checked_save_stack_projection_samples_before_each_owner_push() {
    // TeX82 §§273/275 check save_ptr before pushing boundaries and restore
    // records. Section 276's aftergroup token is likewise checked before its
    // one-word push. The projection must identify the newest physical record
    // across the Env journal, CodeTables, and aftergroup payloads.
    let mut stores = Stores::new();
    stores.enter_group();
    assert_eq!(stores.checked_save_stack_words(false), 0);

    stores.set_count(0, 1);
    assert_eq!(stores.checked_save_stack_words(false), 1);

    stores.set_catcode('@', Catcode::Letter);
    assert_eq!(stores.checked_save_stack_words(false), 3);

    stores.push_aftergroup(Token::Char {
        ch: 'x',
        cat: Catcode::Other,
    });
    assert_eq!(stores.checked_save_stack_words(false), 5);

    stores.set_count_global(0, 2);
    assert_eq!(
        stores.checked_save_stack_words(false),
        5,
        "§275's global definition does not push a save record"
    );

    stores.enter_group();
    assert_eq!(stores.checked_save_stack_words(false), 6);
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
    let stale = stores.intern_token_list_ref_in_domain(&[crate::token::Token::param(1)], None);

    stores.rollback(&snapshot);
    let reused = stores.intern_token_list(&[crate::token::Token::param(2)]);

    assert_eq!(reused.raw(), stale.id().raw());
    assert_ne!(reused, stale.id());
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
fn provenance_records_round_trip_through_stores_boundary() {
    let mut stores = Stores::new();
    let symbol = stores.intern("m");
    let params = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[Token::Cs(symbol.symbol())]);
    let definition = stores.intern_macro(MacroMeaning::new(MeaningFlags::EMPTY, params, body));
    let source = stores.source_origin(SourceId::new(3), 40, 5, 2);
    let macro_origin = stores.macro_invocation_origin(
        definition.id(),
        source,
        OriginId::UNKNOWN,
        OriginId::UNKNOWN,
    );
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

    assert_eq!(stores.bootstrap_origin(), OriginId::UNKNOWN);
    assert_eq!(
        stores.origin(source),
        OriginRecord::Source(SourceOrigin::new(SourceId::new(3), 40, 5, 2))
    );
    assert_eq!(
        stores.origin(macro_origin),
        OriginRecord::MacroInvocation(MacroInvocationOrigin::from_nonowning_operand(
            stores.macro_definition_observation_operand(definition.id()) as u64,
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
}

#[test]
fn rollback_restores_provenance_as_part_of_snapshot_tuple() {
    let mut stores = Stores::new();
    let _kept = stores.synthetic_origin(SyntheticOriginKind::Engine);
    let snapshot = stores.checkpoint();
    let stale = stores.synthetic_origin(SyntheticOriginKind::Primitive);

    stores.rollback(&snapshot);
    let reused = stores.synthetic_origin(SyntheticOriginKind::Format);

    assert_ne!(reused.raw(), stale.raw());
    assert_eq!(stores.origin_if_live(stale), None);
    assert_eq!(
        stores.origin(reused),
        OriginRecord::Synthetic(SyntheticOrigin::new(SyntheticOriginKind::Format))
    );
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
fn macro_definition_precomputes_parameter_delimiter_ranges() {
    let mut stores = Stores::new();
    let params = stores.intern_token_list(&[
        Token::Char {
            ch: 'a',
            cat: Catcode::Letter,
        },
        Token::param(1),
        Token::Char {
            ch: ',',
            cat: Catcode::Other,
        },
        Token::param(2),
        Token::Char {
            ch: ';',
            cat: Catcode::Other,
        },
    ]);
    let body = stores.intern_token_list(&[]);
    let definition = stores.intern_macro(MacroMeaning::new(MeaningFlags::EMPTY, params, body));

    let pattern = stores.macro_definition_parameter_pattern(definition.id());
    assert_eq!(pattern.parameter_count(), 2);
    assert_eq!(pattern.leading_end(5), 1);
    assert_eq!(pattern.delimiter_bounds(0, 5), (2, 3));
    assert_eq!(pattern.delimiter_bounds(1, 5), (4, 5));
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
            .macro_definition(first.id())
            .semantic_eq(stores.macro_definition(second.id()))
    );
    assert_eq!(
        stores.macro_definition_observation_operand(first.id()),
        249_985
    );
    assert_eq!(
        stores.macro_definition_observation_operand(second.id()),
        249_983
    );
}

#[test]
fn identical_macro_definitions_keep_distinct_provenance() {
    let mut stores = Stores::new();
    let symbol = stores.intern("same");
    let params = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[Token::Cs(symbol.symbol())]);
    let macro_meaning = MacroMeaning::new(MeaningFlags::PROTECTED, params, body);
    let first_origin = stores.synthetic_origin_ref(SyntheticOriginKind::Engine);
    let second_origin = stores.synthetic_origin_ref(SyntheticOriginKind::Format);
    let first_body_origins = stores.allocate_origin_list_ref(std::slice::from_ref(&first_origin));
    let second_body_origins = stores.allocate_origin_list_ref(std::slice::from_ref(&second_origin));

    let first = stores.intern_macro_with_provenance(
        macro_meaning,
        Some(MacroDefinitionProvenance::new(
            first_origin.clone(),
            crate::provenance::OriginListRef::empty(),
            first_body_origins.clone(),
        )),
    );
    let second = stores.intern_macro_with_provenance(
        macro_meaning,
        Some(MacroDefinitionProvenance::new(
            second_origin.clone(),
            crate::provenance::OriginListRef::empty(),
            second_body_origins.clone(),
        )),
    );

    assert_ne!(first, second);
    assert!(
        stores
            .macro_definition(first.id())
            .semantic_eq(stores.macro_definition(second.id()))
    );
    assert_eq!(
        stores
            .macro_definition_provenance(first.id())
            .definition_origin(),
        first_origin.id()
    );
    assert_eq!(
        stores
            .macro_definition_provenance(second.id())
            .replacement_origins(),
        second_body_origins.id()
    );
}

#[test]
fn missing_macro_definition_provenance_degrades_to_unknown() {
    let mut stores = Stores::new();
    let params = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[]);
    let definition = stores.intern_macro(MacroMeaning::new(MeaningFlags::EMPTY, params, body));

    assert_eq!(
        stores.macro_definition_provenance(definition.id()),
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
    let stale_id = stale.id();
    drop(stale);

    stores.rollback(&snapshot);
    let reused_body = stores.intern_token_list(&[Token::Cs(symbol.symbol())]);
    let reused = stores.intern_macro(MacroMeaning::new(
        MeaningFlags::PROTECTED,
        params,
        reused_body,
    ));

    assert_eq!(
        stores.macro_definition(kept.id()).replacement_text(),
        kept_body
    );
    assert_eq!(reused.raw(), stale_id.raw());
    assert_ne!(reused.id(), stale_id);
    assert!(!stores.macros.contains(stale_id));
    assert_eq!(
        stores.macro_definition(reused.id()).replacement_text(),
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
fn initial_tex82_code_tables_cover_all_ascii_exceptions_and_formulas() {
    let stores = Stores::new();

    // tex.web §232 initializes the complete 8-bit code-table region.
    for code in 0_u8..=u8::MAX {
        let ch = char::from(code);
        let expected_catcode = match code {
            0 => Catcode::Ignored,
            13 => Catcode::EndLine,
            b' ' => Catcode::Space,
            b'%' => Catcode::Comment,
            b'\\' => Catcode::Escape,
            127 => Catcode::Invalid,
            b'A'..=b'Z' | b'a'..=b'z' => Catcode::Letter,
            _ => Catcode::Other,
        };
        let expected_mathcode = match code {
            b'0'..=b'9' => 0x7000 + u32::from(code),
            b'A'..=b'Z' | b'a'..=b'z' => 0x7100 + u32::from(code),
            _ => u32::from(code),
        };
        let (expected_lccode, expected_uccode) = match code {
            b'A'..=b'Z' => (u32::from(code + (b'a' - b'A')), u32::from(code)),
            b'a'..=b'z' => (u32::from(code), u32::from(code - (b'a' - b'A'))),
            _ => (0, 0),
        };

        assert_eq!(stores.catcode(ch), expected_catcode, "catcode {code}");
        assert_eq!(stores.mathcode(ch), expected_mathcode, "mathcode {code}");
        assert_eq!(stores.lccode(ch), expected_lccode, "lccode {code}");
        assert_eq!(stores.uccode(ch), expected_uccode, "uccode {code}");
        assert_eq!(
            stores.sfcode(ch),
            if code.is_ascii_uppercase() { 999 } else { 1000 },
            "sfcode {code}"
        );
        assert_eq!(
            stores.delcode(ch),
            if code == b'.' { 0 } else { -1 },
            "delcode {code}"
        );
    }

    // tex.web §240 zeroes the whole integer region, including count
    // registers, then assigns exactly these six nonzero parameter defaults.
    // The full slot range includes e-TeX/pdfTeX profile controls, which must
    // remain disabled until their profile initialization runs.
    for raw in 0..crate::env::banks::PARAMETER_COUNT as u16 {
        let param = IntParam::new(raw);
        let expected = match param {
            IntParam::TOLERANCE => 10_000,
            IntParam::MAG => 1000,
            IntParam::HANG_AFTER => 1,
            IntParam::MAX_DEAD_CYCLES => 25,
            IntParam::ESCAPE_CHAR => i32::from(b'\\'),
            IntParam::END_LINE_CHAR => 13,
            _ => 0,
        };
        assert_eq!(stores.int_param(param), expected, "integer parameter {raw}");
    }
    for register in 0..=u8::MAX {
        assert_eq!(
            stores.count(register.into()),
            0,
            "count register {register}"
        );
    }
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
        id.nodes(),
        &[
            Node::MathOn(Scaled::from_raw(0)),
            Node::MathOff(Scaled::from_raw(0))
        ]
    );

    builder.push(Node::Char {
        font: NULL_FONT,
        ch: 'x',
        origin: crate::provenance::OriginRef::unknown(),
    });
    let reused = stores.finish_node_list(&mut builder);
    assert_eq!(
        reused.nodes(),
        &[Node::Char {
            font: NULL_FONT,
            ch: 'x',
            origin: crate::provenance::OriginRef::unknown(),
        }]
    );
}

#[test]
fn compact_builder_freezes_with_owned_semantics_and_promotes_for_mixed_material() {
    let mut stores = Stores::new();
    let mut compact = stores.node_list_builder();
    compact.push_unknown_character(NULL_FONT, 'A');
    compact.push_kern(Scaled::from_raw(-17), KernKind::Explicit);
    assert_eq!(
        compact.compact_width(Scaled::from_raw(101)),
        Some(Scaled::from_raw(84))
    );
    let frozen = stores.freeze_node_list_ref(compact);

    let expected = [
        Node::Char {
            font: NULL_FONT,
            ch: 'A',
            origin: crate::provenance::OriginRef::unknown(),
        },
        Node::Kern {
            amount: Scaled::from_raw(-17),
            kind: KernKind::Explicit,
        },
    ];
    assert_eq!(frozen.nodes(), &expected);
    let owned = stores.freeze_node_list(&expected);
    assert_eq!(frozen.semantic_fingerprint(), owned.semantic_fingerprint());
    assert!(frozen.shares_payload(&owned));

    let mut mixed = stores.node_list_builder();
    mixed.push_unknown_character(NULL_FONT, 'B');
    mixed.push(Node::Penalty(9));
    assert_eq!(mixed.as_slice().len(), 2);
    assert_eq!(
        stores.freeze_node_list_ref(mixed).nodes(),
        &[
            Node::Char {
                font: NULL_FONT,
                ch: 'B',
                origin: crate::provenance::OriginRef::unknown(),
            },
            Node::Penalty(9),
        ]
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
        spec: crate::glue::GlueSpecRef::testing_new(stale),
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
        spec: crate::glue::GlueSpecRef::testing_new(foreign_glue),
        kind: crate::node::GlueKind::Normal,
        leader: None,
    });

    let _ = stores.finish_node_list(&mut builder);
}

#[test]
#[should_panic(expected = "stored token-list slot is not live")]
fn freeze_node_list_rejects_noncanonical_owner_retained_across_rollback() {
    let mut stores = Stores::new();
    let snapshot = stores.checkpoint();
    let stale = stores.intern_token_list_ref_in_domain(&[crate::token::Token::param(1)], None);

    stores.rollback(&snapshot);
    let _ = stores.freeze_node_list(&[Node::Mark {
        class: 0,
        tokens: stale.clone(),
    }]);
}

#[test]
#[should_panic(expected = "token list is not live in this Universe timeline")]
fn finish_node_list_rejects_foreign_whatsit_token_list() {
    let mut stores = Stores::new();
    let mut foreign = stores.clone();
    let foreign_tokens =
        foreign.intern_token_list_ref_in_domain(&[crate::token::Token::param(1)], None);
    let mut builder = stores.node_list_builder();
    builder.push(Node::Whatsit(crate::node::Whatsit::DeferredWrite {
        sink: crate::world::PrintSink::TerminalAndLog,
        tokens: foreign_tokens,
    }));

    let _ = stores.finish_node_list(&mut builder);
}

#[test]
fn direct_child_owner_survives_aggregate_rollback() {
    let mut stores = Stores::new();
    let snapshot = stores.checkpoint();
    let stale = one_char(&mut stores, 'x');

    stores.rollback(&snapshot);
    stores.freeze_node_list(&[Node::Penalty(1), Node::Penalty(2)]);
    let root = stores.freeze_node_list(&[Node::Adjust(crate::node::AdjustNode::ordinary(stale))]);
    assert!(matches!(
        root.nodes().first(),
        Some(crate::node_arena::NodeRef::Adjust(_))
    ));
}

#[test]
fn direct_child_owner_can_cross_universe_boundaries() {
    let mut stores = Stores::new();
    let mut foreign = Stores::new();
    let foreign_child = one_char(&mut foreign, 'x');
    let mut builder = stores.node_list_builder();
    builder.push(Node::HList(BoxNode::new(BoxNodeFields {
        width: scaled(10),
        height: scaled(7),
        depth: scaled(3),
        shift: scaled(0),
        box_lr: crate::node::BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: foreign_child,
    })));

    let root = stores.freeze_node_list_ref(builder);
    assert!(matches!(root.get(0), Some(Node::HList(_))));
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
fn rollback_allows_snapshot_before_still_open_inner_groups() {
    let mut stores = Stores::new();
    let symbol = stores.intern("kept");
    stores.enter_group();
    let snapshot = stores.checkpoint();

    stores.enter_group();
    stores.set_meaning(symbol, Meaning::CharGiven('x'));
    stores.enter_group();
    stores.set_meaning(symbol, Meaning::CharGiven('y'));

    stores.rollback(&snapshot);
    assert_eq!(stores.env_group_depth(), 1);
    assert_eq!(stores.meaning(symbol), Meaning::Undefined);
}

#[test]
#[should_panic(expected = "Stores snapshots are invalidated by exiting a group that encloses them")]
fn rollback_rejects_exited_group_replaced_at_same_depth() {
    let mut stores = Stores::new();
    stores.enter_group();
    let snapshot = stores.checkpoint();

    assert_eq!(stores.leave_group(), Vec::<Token>::new());
    stores.enter_group();

    stores.rollback(&snapshot);
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
    let stale_id = stale.id();
    drop(stale);

    stores.rollback(&snapshot);
    stores.set_meaning(
        symbol,
        Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition: stale_id,
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
fn same_epoch_list_stored_twice_reuses_one_direct_payload() {
    let mut stores = Stores::new();
    let list = one_char(&mut stores, 'a');

    stores.install_box(0, list.clone());
    stores.install_box(1, list);

    let first = stores.box_reg_ref(0).expect("box 0 should be non-void");
    let second = stores.box_reg_ref(1).expect("box 1 should be non-void");
    assert_eq!(first.id(), second.id());
    assert!(first.shares_payload(&second));
}

#[test]
fn box_cells_and_undo_retain_direct_owners() {
    let mut stores = Stores::new();
    let baseline = one_char(&mut stores, 'b');
    stores.install_box(0, baseline);
    let baseline = stores.box_reg_ref(0).expect("baseline box owner");
    drop(baseline);
    assert_eq!(
        stores.box_reg_ref(0).expect("Env owns baseline").to_vec(),
        [one_char_node('b')]
    );

    stores.enter_group();
    let replacement = one_char(&mut stores, 'r');
    stores.install_box(0, replacement);
    let replacement = stores.box_reg_ref(0).expect("replacement box owner");
    drop(replacement);
    assert_eq!(
        stores
            .box_reg_ref(0)
            .expect("Env owns replacement")
            .to_vec(),
        [one_char_node('r')]
    );

    assert_eq!(stores.leave_group(), Vec::<Token>::new());
    assert_eq!(
        stores
            .box_reg_ref(0)
            .expect("undo restored baseline owner")
            .to_vec(),
        [one_char_node('b')]
    );
}

#[test]
fn direct_box_fork_keeps_inherited_roots_and_separates_new_roots() {
    let mut parent = Stores::new();
    let inherited = one_char(&mut parent, 'i');
    let mut child = parent.clone();

    assert_eq!(inherited.to_vec(), [one_char_node('i')]);

    let parent_only = one_char(&mut parent, 'p');
    let child_only = one_char(&mut child, 'c');

    assert_ne!(parent_only.id().arena(), child_only.id().arena());
    assert_eq!(parent_only.to_vec(), [one_char_node('p')]);
    assert_eq!(child_only.to_vec(), [one_char_node('c')]);
}

#[test]
fn released_direct_box_key_cannot_upgrade_after_final_drop() {
    let mut stores = Stores::new();
    let stale_owner = one_char(&mut stores, 'o');
    let stale = stale_owner.id();
    let observer = stale_owner.downgrade();
    drop(stale_owner);
    assert!(observer.upgrade().is_none());

    let replacement = one_char(&mut stores, 'n');

    assert_ne!(stale.arena(), replacement.id().arena());
    assert_eq!(replacement.to_vec(), [one_char_node('n')]);
}

#[test]
fn repeated_direct_box_preparation_bounds_weak_metadata() {
    const REPLACEMENTS: usize = 20_000;

    let mut stores = Stores::new();
    let mut live = one_char(&mut stores, 'a');
    let stale = live.id();
    let stale_observer = live.downgrade();

    for index in 1..REPLACEMENTS {
        let replacement = one_char(&mut stores, if index % 2 == 0 { 'a' } else { 'b' });
        drop(live);
        live = replacement;
    }

    assert!(stale_observer.upgrade().is_none());
    assert_ne!(stale.arena(), live.id().arena());
    let (weak_len, weak_capacity) = stores.node_ref_index.shape();
    assert!(weak_len <= 64);
    assert!(weak_capacity <= 64);
}

#[test]
fn direct_box_replacement_carries_word_and_box_rule_sidecars_together() {
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
            box_lr: crate::node::BoxLr::Normal,
            glue_set: GlueSetRatio::ZERO,
            glue_sign: Sign::Normal,
            glue_order: Order::Normal,
            children: child,
        }))]);
        let promoted = root;
        if let Some(previous) = live.replace(promoted) {
            stale.get_or_insert_with(|| previous.downgrade());
            drop(previous);
        }
    }

    let live = live.expect("one survivor remains");
    let Some(crate::node_arena::NodeRef::HList(box_node)) = live.nodes().first() else {
        panic!("survivor root should retain its box sidecar")
    };
    assert_eq!(box_node.width, Scaled::from_raw(31));
    assert_eq!(
        live.resolve(box_node.children)
            .expect("box child belongs to direct owner")
            .nodes(),
        &[Node::Rule {
            width: Some(Scaled::from_raw(31)),
            height: None,
            depth: Some(Scaled::from_raw(-31)),
        }]
    );
    assert!(stale.expect("a stale root exists").upgrade().is_none());
}

#[test]
fn coalesced_box_replacements_roll_back_to_the_checkpoint_owner() {
    const REPLACEMENTS: usize = 20_000;

    let mut stores = Stores::new();
    let baseline = one_char(&mut stores, 'o');
    stores.install_box(0, baseline);
    let baseline = stores.box_owner(0).expect("baseline box should be stored");
    let snapshot = stores.checkpoint();
    let mut stale = None;

    for index in 0..REPLACEMENTS {
        let replacement = one_char(&mut stores, if index % 2 == 0 { 'a' } else { 'b' });
        stores.install_box(0, replacement);
        stale.get_or_insert_with(|| stores.box_owner(0).expect("replacement should be stored"));
    }

    let _stale = stale.expect("at least one replacement should be stored");

    stores.rollback(&snapshot);
    assert_eq!(stores.box_owner(0).as_ref(), Some(&baseline));
    assert_eq!(
        stores
            .box_reg_ref(0)
            .expect("restored direct owner")
            .to_vec(),
        [one_char_node('o')]
    );
}

#[test]
fn storing_direct_owner_in_second_register_shares_payload_until_release() {
    let mut stores = Stores::new();
    let list = one_char(&mut stores, 'a');

    stores.install_box(0, list);
    let owner = stores.box_reg_ref(0).expect("box should be non-void");
    let observer = owner.downgrade();
    stores.write_box_reg_ref(1, Some(owner.clone()), false);

    assert!(
        stores
            .box_reg_ref(1)
            .expect("second owner")
            .shares_payload(&owner)
    );

    assert_eq!(stores.take_box_owner(0).as_ref(), Some(&owner));
    drop(owner);
    assert!(observer.upgrade().is_some());

    let replacement = one_char(&mut stores, 'b');
    stores.install_box(1, replacement);
}

#[test]
fn group_exit_and_rollback_restore_box_refs_once() {
    let mut stores = Stores::new();
    let outer = one_char(&mut stores, 'o');
    stores.install_box(0, outer);
    let baseline = stores.box_owner(0).expect("outer box should be stored");
    let snapshot = stores.checkpoint();

    stores.enter_group();
    let inner = one_char(&mut stores, 'i');
    stores.install_box(0, inner);

    assert_eq!(stores.leave_group(), Vec::<Token>::new());
    assert_eq!(stores.box_owner(0).as_ref(), Some(&baseline));
    assert_eq!(
        stores.box_reg_ref(0).expect("restored owner").to_vec(),
        [one_char_node('o')]
    );

    stores.rollback(&snapshot);
    assert_eq!(stores.box_owner(0).as_ref(), Some(&baseline));
}

#[test]
fn same_level_box_journal_keeps_value_live_across_nested_group_exit() {
    let mut stores = Stores::new();
    let outer = one_char(&mut stores, 'o');
    stores.install_box(0, outer);
    let baseline = stores.box_owner(0).expect("outer box should be stored");

    stores.enter_group();
    let inner = one_char(&mut stores, 'i');
    stores.install_box(0, inner);
    let local = stores.box_reg_ref(0).expect("local box should be stored");
    let observer = local.downgrade();
    stores.enter_group();
    assert_eq!(stores.take_box_owner_same_level(0).as_ref(), Some(&local));
    drop(local);
    assert!(observer.upgrade().is_some());

    assert_eq!(stores.leave_group(), Vec::<Token>::new());
    assert_eq!(stores.box_owner(0), None);
    assert!(observer.upgrade().is_some());

    assert_eq!(stores.leave_group(), Vec::<Token>::new());
    assert!(observer.upgrade().is_none());
    assert_eq!(stores.box_owner(0).as_ref(), Some(&baseline));
    assert_eq!(
        stores.box_reg_ref(0).expect("baseline restored").id(),
        baseline.id()
    );
}

#[test]
fn global_box_assignment_survives_group_and_journal_owner_survives_rollback() {
    let mut stores = Stores::new();
    let outer = one_char(&mut stores, 'o');
    stores.install_box(0, outer);
    let baseline = stores.box_owner(0).expect("outer box should be stored");
    let snapshot = stores.checkpoint();

    stores.enter_group();
    let global = one_char(&mut stores, 'g');
    stores.install_box_global(0, global);
    let global = stores.box_owner(0).expect("global box should be stored");

    assert_eq!(stores.leave_group(), Vec::<Token>::new());
    assert_eq!(stores.box_owner(0).as_ref(), Some(&global));

    stores.rollback(&snapshot);
    assert_eq!(stores.box_owner(0).as_ref(), Some(&baseline));
}

#[test]
fn same_value_global_box_adds_only_journal_owner() {
    let mut stores = Stores::new();
    let list = one_char(&mut stores, 'a');
    stores.install_box(0, list);
    let survivor = stores.box_owner(0).expect("box should be stored");
    let snapshot = stores.checkpoint();

    stores.enter_group();
    stores.install_box_global(0, survivor.clone());
    assert_eq!(stores.leave_group(), Vec::<Token>::new());
    assert_eq!(stores.box_owner(0).as_ref(), Some(&survivor));

    stores.rollback(&snapshot);
    assert_eq!(stores.box_owner(0).as_ref(), Some(&survivor));
}

#[test]
fn same_value_local_box_assignment_preserves_live_register_owner() {
    let mut stores = Stores::new();
    let list = one_char(&mut stores, 'a');
    stores.install_box(0, list);
    let survivor = stores.box_owner(0).expect("box should be stored");

    stores.install_box(0, survivor.clone());

    assert_eq!(stores.box_owner(0).as_ref(), Some(&survivor));
    assert_eq!(
        stores.box_reg_ref(0).expect("direct owner").to_vec(),
        [one_char_node('a')]
    );
}

#[test]
fn local_box_after_global_drops_local_survivor_on_group_exit() {
    let mut stores = Stores::new();
    let outer = one_char(&mut stores, 'o');
    stores.install_box(0, outer);
    let baseline = stores.box_owner(0).expect("outer box should be stored");
    let snapshot = stores.checkpoint();

    stores.enter_group();
    let global = one_char(&mut stores, 'g');
    stores.install_box_global(0, global);
    let global = stores.box_owner(0).expect("global box should be stored");
    let local = one_char(&mut stores, 'l');
    stores.install_box(0, local);

    assert_eq!(stores.leave_group(), Vec::<Token>::new());
    assert_eq!(stores.box_owner(0).as_ref(), Some(&global));

    stores.rollback(&snapshot);
    assert_eq!(stores.box_owner(0).as_ref(), Some(&baseline));
}

#[test]
fn promoted_nested_box_retains_local_child_payloads() {
    let mut stores = Stores::new();
    let inner = one_char(&mut stores, 'x');
    let middle = stores.freeze_node_list(&[Node::HList(BoxNode::new(BoxNodeFields {
        width: scaled(10),
        height: scaled(7),
        depth: scaled(3),
        shift: scaled(0),
        box_lr: crate::node::BoxLr::Normal,
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
        box_lr: crate::node::BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: middle,
    }))]);

    stores.install_box(0, outer);
    let promoted_outer = stores.box_reg_ref(0).expect("box should be promoted");
    let Some(crate::node_arena::NodeRef::VList(outer_box)) = promoted_outer.nodes().first() else {
        panic!("outer survivor list should contain one vlist");
    };
    assert_different_roots(promoted_outer.id(), outer_box.children);
    let middle = promoted_outer
        .resolve(outer_box.children)
        .expect("owned middle list");
    let Some(crate::node_arena::NodeRef::HList(middle_box)) = middle.nodes().first() else {
        panic!("middle survivor list should contain one hlist");
    };
    assert_different_roots(middle.id(), middle_box.children);
    assert!(
        promoted_outer.resolve(middle_box.children).is_none(),
        "the outer payload must not scan through its direct child owner"
    );
    assert_eq!(
        middle
            .resolve(middle_box.children)
            .expect("owned inner list")
            .nodes(),
        &[Node::Char {
            font: NULL_FONT,
            ch: 'x',
            origin: crate::provenance::OriginRef::unknown(),
        }]
    );
}

#[test]
fn promotion_retains_one_shared_direct_child_owner() {
    let mut stores = Stores::new();
    let child = one_char(&mut stores, 'x');
    stores.install_box(0, child);
    let child = stores.box_reg_ref(0).expect("child box should be promoted");
    let fields = BoxNodeFields {
        width: scaled(10),
        height: scaled(7),
        depth: scaled(3),
        shift: scaled(0),
        box_lr: crate::node::BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: child.clone(),
    };
    let outer = stores.freeze_node_list(&[
        Node::HList(BoxNode::new(fields.clone())),
        Node::VList(BoxNode::new(fields)),
    ]);

    stores.install_box(255, outer);
    let promoted = stores
        .box_reg_ref(255)
        .expect("outer box should be promoted");
    let nodes = promoted.nodes();
    let (
        Some(crate::node_arena::NodeRef::HList(first)),
        Some(crate::node_arena::NodeRef::VList(second)),
    ) = (nodes.get(0), nodes.get(1))
    else {
        panic!("promoted root should preserve both wrapper boxes");
    };

    assert_different_roots(promoted.id(), first.children);
    assert_eq!(
        first.children, second.children,
        "shared child has one direct structural owner"
    );
    assert_eq!(
        promoted
            .resolve(first.children)
            .expect("owned shared child")
            .nodes(),
        &[Node::Char {
            font: NULL_FONT,
            ch: 'x',
            origin: crate::provenance::OriginRef::unknown(),
        }]
    );
}

#[test]
fn promotion_resolves_every_direct_child_bearing_compact_row() {
    let mut stores = Stores::new();
    let child = one_char(&mut stores, 'c');
    let box_node = BoxNode::new(BoxNodeFields {
        width: scaled(1),
        height: scaled(2),
        depth: scaled(3),
        shift: scaled(4),
        box_lr: crate::node::BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: child.clone(),
    });
    let noad = MathNoad {
        kind: NoadKind::Normal(NoadClass::Ord),
        nucleus: MathField::SubBox(child.clone()),
        subscript: MathField::SubMlist(child.clone()),
        superscript: MathField::SubBox(child.clone()),
    };
    let root = stores.freeze_node_list(&[
        Node::HList(box_node.clone()),
        Node::VList(box_node.clone()),
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
            children: child.clone(),
        })),
        Node::Glue {
            spec: crate::glue::testing_zero_glue_ref(),
            kind: GlueKind::Leaders,
            leader: Some(LeaderPayload::HList(box_node)),
        },
        Node::Disc {
            kind: DiscKind::Discretionary,
            pre: child.clone(),
            post: child.clone(),
            replace: child.clone(),
            physical_replace_count: 3,
        },
        Node::Ins {
            class: 1,
            size: scaled(10),
            split_top_skip: crate::glue::testing_zero_glue_ref(),
            split_max_depth: scaled(11),
            floating_penalty: 12,
            content: child.clone(),
        },
        Node::MathNoad(noad),
        Node::FractionNoad(MathFraction {
            numerator: child.clone(),
            denominator: child.clone(),
            thickness: FractionThickness::Default,
            left_delimiter: None,
            right_delimiter: None,
        }),
        Node::MathChoice(MathChoice {
            display: child.clone(),
            text: child.clone(),
            script: child.clone(),
            script_script: child.clone(),
        }),
        Node::MathList(MathListNode {
            display: false,
            content: child.clone(),
        }),
        Node::Adjust(crate::node::AdjustNode::ordinary(child)),
    ]);

    stores.install_box(17, root);
    let promoted = stores.box_reg_ref(17).expect("root should be promoted");
    assert!(promoted.nodes().into_iter().any(|node| matches!(
        node,
        crate::node_arena::NodeRef::Disc {
            physical_replace_count: 3,
            ..
        }
    )));
    let mut child_count = 0;
    for node in promoted.nodes() {
        for child in node.children() {
            assert_different_roots(promoted.id(), child);
            assert_eq!(
                promoted
                    .resolve(child)
                    .expect("owned promoted child")
                    .nodes(),
                &[Node::Char {
                    font: NULL_FONT,
                    ch: 'c',
                    origin: crate::provenance::OriginRef::unknown(),
                }]
            );
            child_count += 1;
        }
    }
    assert_eq!(child_count, 19);
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
        let current_ref = current;
        current = stores.freeze_node_list(&[Node::HList(BoxNode::new(BoxNodeFields {
            width: scaled(1),
            height: scaled(1),
            depth: scaled(0),
            shift: scaled(0),
            box_lr: crate::node::BoxLr::Normal,
            glue_set: GlueSetRatio::ZERO,
            glue_sign: Sign::Normal,
            glue_order: Order::Normal,
            children: current_ref,
        }))]);
    }

    stores.install_box(0, current);
    let mut promoted = stores.box_reg_ref(0).expect("box should be promoted");
    for _ in 0..4096 {
        let Some(crate::node_arena::NodeRef::HList(box_node)) = promoted.nodes().first() else {
            panic!("deep promoted chain should remain hlist nodes");
        };
        assert_different_roots(promoted.id(), box_node.children);
        promoted = promoted
            .resolve(box_node.children)
            .expect("owned nested box");
    }
    assert_eq!(
        promoted.nodes(),
        &[Node::Char {
            font: NULL_FONT,
            ch: 'x',
            origin: crate::provenance::OriginRef::unknown(),
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

fn one_char(stores: &mut Stores, ch: char) -> NodeListRef {
    stores.freeze_node_list(&[one_char_node(ch)])
}

fn one_char_node(ch: char) -> Node {
    Node::Char {
        font: NULL_FONT,
        ch,
        origin: crate::provenance::OriginRef::unknown(),
    }
}

fn assert_different_roots(a: NodeListId, b: NodeListId) {
    let (ArenaRef::Owned(a), ArenaRef::Owned(b)) = (a.arena(), b.arena()) else {
        panic!("expected survivor ids");
    };
    assert_ne!(a, b);
}

fn scaled(raw: i32) -> Scaled {
    Scaled::from_raw(raw)
}
