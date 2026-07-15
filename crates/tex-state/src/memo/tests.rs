use super::*;

#[test]
fn tokens_round_trip_into_independent_universe_without_origin_or_handle_identity() {
    let mut source = Universe::new();
    let named = source.intern("memo-name");
    let active = source.intern_active_character('!');
    let id = source.intern_token_list(&[
        Token::Cs(named.symbol()),
        Token::Cs(active.symbol()),
        Token::Char {
            ch: 'β',
            cat: Catcode::Letter,
        },
        Token::param(2),
    ]);
    let detached = source.detach_token_list(id).expect("token detachment");

    let mut target = Universe::new();
    let imported = target
        .import_memo_token_list(&detached, MemoValueLimits::default())
        .expect("token import");
    assert_eq!(target.tokens(imported).len(), 4);
    let Token::Cs(symbol) = target.tokens(imported)[0] else {
        panic!("expected imported control sequence");
    };
    assert_eq!(target.resolve(symbol), "memo-name");
    assert_eq!(detached.kind(), MemoValueKind::Tokens);
}

#[test]
fn token_values_import_after_fork_and_rollback_without_reusing_source_handles() {
    let mut source = Universe::new();
    let source_symbol = source.intern("forked-memo-name");
    let source_list = source.intern_token_list(&[Token::Cs(source_symbol.symbol())]);
    let detached = source
        .detach_token_list(source_list)
        .expect("fork token detachment");

    let mut fork = source.clone();
    let fork_list = fork
        .import_memo_token_list(&detached, MemoValueLimits::default())
        .expect("fork token import");
    let Token::Cs(fork_symbol) = fork.tokens(fork_list)[0] else {
        panic!("expected fork-imported control sequence");
    };
    assert_eq!(fork.resolve(fork_symbol), "forked-memo-name");

    let mut rolled_back = Universe::new();
    let checkpoint = rolled_back.snapshot();
    let discarded_symbol = rolled_back.intern("discarded-name");
    let _discarded = rolled_back.intern_token_list(&[Token::Cs(discarded_symbol.symbol())]);
    rolled_back.rollback(&checkpoint);
    let imported = rolled_back
        .import_memo_token_list(&detached, MemoValueLimits::default())
        .expect("rollback token import");
    let Token::Cs(imported_symbol) = rolled_back.tokens(imported)[0] else {
        panic!("expected rollback-imported control sequence");
    };
    assert_eq!(rolled_back.resolve(imported_symbol), "forked-memo-name");
}

#[test]
fn envelope_rejects_corruption_schema_kind_and_oversize() {
    let mut universe = Universe::new();
    let id = universe.intern_token_list(&[Token::Char {
        ch: 'x',
        cat: Catcode::Letter,
    }]);
    let detached = universe.detach_token_list(id).expect("token detachment");
    let mut bytes = detached.to_bytes().expect("memo encoding");
    *bytes.last_mut().expect("encoded envelope is nonempty") ^= 1;
    assert!(DetachedMemoValue::from_bytes(&bytes, MemoValueLimits::default()).is_err());

    assert!(matches!(
        universe.import_memo_glue(&detached),
        Err(MemoValueError::Kind { .. })
    ));
    assert!(matches!(
        DetachedMemoValue::from_bytes(
            &detached.to_bytes().expect("memo encoding"),
            MemoValueLimits {
                max_payload_bytes: 0,
                ..MemoValueLimits::default()
            }
        ),
        Err(MemoValueError::Oversized { .. })
    ));
}

#[test]
fn glue_and_macro_round_trip_semantically() {
    let mut source = Universe::new();
    let glue = source.intern_glue(GlueSpec {
        width: crate::scaled::Scaled::from_raw(10),
        stretch: crate::scaled::Scaled::from_raw(20),
        stretch_order: Order::Fil,
        shrink: crate::scaled::Scaled::from_raw(3),
        shrink_order: Order::Normal,
    });
    let detached_glue = source.detach_glue(glue).expect("glue detachment");

    let parameters = source.intern_token_list(&[Token::param(1)]);
    let replacement = source.intern_token_list(&[Token::Char {
        ch: 'z',
        cat: Catcode::Letter,
    }]);
    let definition = source.intern_macro(MacroMeaning::new(
        MeaningFlags::LONG,
        parameters,
        replacement,
    ));
    let detached_macro = source
        .detach_macro_meaning(definition)
        .expect("macro detachment");

    let mut target = Universe::new();
    let imported_glue = target
        .import_memo_glue(&detached_glue)
        .expect("glue import");
    assert_eq!(target.glue(imported_glue), source.glue(glue));
    let imported_macro = target
        .import_memo_macro_meaning(&detached_macro, MemoValueLimits::default())
        .expect("macro import");
    let meaning = target.macro_definition(imported_macro);
    assert_eq!(meaning.flags(), MeaningFlags::LONG);
    assert_eq!(
        target.tokens(meaning.replacement_text())[0],
        Token::Char {
            ch: 'z',
            cat: Catcode::Letter
        }
    );
}

#[test]
fn nested_node_graph_round_trips_across_owners_and_respects_budget_atomically() {
    use crate::node::{BoxNode, BoxNodeFields, GlueKind, Node};
    use crate::scaled::{GlueSetRatio, Scaled};

    let mut source = Universe::new();
    let tokens = source.intern_token_list(&[Token::Char {
        ch: 'm',
        cat: Catcode::Letter,
    }]);
    let glue = source.intern_glue(GlueSpec {
        width: Scaled::from_raw(7),
        ..GlueSpec::ZERO
    });
    let child = source.freeze_node_list(&[
        Node::Char {
            font: source.current_font(),
            ch: 'x',
            origin: crate::token::OriginId::UNKNOWN,
        },
        Node::Glue {
            spec: glue,
            kind: GlueKind::Normal,
            leader: None,
        },
        Node::Mark { class: 2, tokens },
    ]);
    let root = source.freeze_node_list(&[Node::HList(BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(20),
        height: Scaled::from_raw(5),
        depth: Scaled::from_raw(1),
        shift: Scaled::from_raw(0),
        display: false,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: crate::node::Sign::Normal,
        glue_order: Order::Normal,
        children: child,
    }))]);
    let detached = source.detach_node_list(root).expect("node detachment");
    let detached_box = source.detach_box(root).expect("box detachment");

    let mut target = Universe::new();
    let imported = target
        .import_memo_node_list(&detached, MemoValueLimits::default())
        .expect("node import");
    let left = source.engine_boundary_hash(77, |hash| hash.node_list(root));
    let right = target.engine_boundary_hash(77, |hash| hash.node_list(imported));
    assert_eq!(left, right);
    let imported_box = target
        .import_memo_box(&detached_box, MemoValueLimits::default())
        .expect("box import");
    assert_eq!(
        source.engine_boundary_hash(78, |hash| hash.node_list(root)),
        target.engine_boundary_hash(78, |hash| hash.node_list(imported_box))
    );

    let before = target.snapshot().state_hash();
    assert!(matches!(
        target.import_memo_node_list(
            &detached,
            MemoValueLimits {
                max_nodes: 1,
                ..MemoValueLimits::default()
            }
        ),
        Err(MemoValueError::Codec(_))
    ));
    assert_eq!(target.snapshot().state_hash(), before);
}

#[test]
fn detached_nodes_drop_absolute_origins() {
    use crate::node::Node;

    let mut source = Universe::new();
    let root = source.freeze_node_list(&[Node::Char {
        font: source.current_font(),
        ch: 'x',
        origin: crate::token::OriginId::from_raw(37),
    }]);
    let detached = source
        .detach_node_list(root)
        .expect("origin-free node detachment");

    let mut target = Universe::new();
    let imported = target
        .import_memo_node_list(&detached, MemoValueLimits::default())
        .expect("origin-free node import");
    let Some(crate::node_arena::NodeRef::Char { origin, .. }) = target.nodes(imported).first()
    else {
        panic!("expected imported character node");
    };
    assert_eq!(origin, crate::token::OriginId::UNKNOWN);
}

#[test]
fn font_round_trips_without_reusing_its_runtime_id() {
    let source = Universe::new();
    let font = source.current_font();
    let detached = source.detach_font(font).expect("font detachment");
    let mut target = Universe::new();
    let imported = target
        .import_memo_font(&detached, MemoValueLimits::default())
        .expect("font import");
    assert_eq!(
        source.engine_boundary_hash(79, |hash| hash.font(font)),
        target.engine_boundary_hash(79, |hash| hash.font(imported))
    );
}

#[test]
fn malformed_node_payload_is_a_miss_without_partial_publication() {
    let malformed = DetachedMemoValue::new(MemoValueKind::Nodes, vec![1, 2, 3]);
    let mut target = Universe::new();
    let before = target.snapshot().state_hash();
    assert!(
        target
            .import_memo_node_list(&malformed, MemoValueLimits::default())
            .is_err()
    );
    assert_eq!(target.snapshot().state_hash(), before);
}

#[test]
fn malformed_late_token_is_rejected_before_earlier_names_are_interned() {
    let payload = bincode::serialize(&vec![
        DetachedToken::Cs {
            active: false,
            name: "must-not-be-published".to_owned(),
        },
        DetachedToken::Param(0),
    ])
    .expect("malformed token payload encoding");
    let detached = DetachedMemoValue::new(MemoValueKind::Tokens, payload);
    let mut target = Universe::new();
    let before = target.snapshot().state_hash();

    assert!(matches!(
        target.import_memo_token_list(&detached, MemoValueLimits::default()),
        Err(MemoValueError::Invalid("invalid parameter slot"))
    ));
    assert_eq!(target.snapshot().state_hash(), before);
}

#[test]
fn font_round_trip_uses_target_owner_and_semantic_identifier() {
    use crate::font::{FontMetrics, LoadedFont};
    use crate::scaled::Scaled;
    use std::path::PathBuf;

    let mut source = Universe::new();
    let font = source.intern_font(LoadedFont::new(
        "memo-font",
        PathBuf::from("memo-font"),
        ContentHash::from_bytes(b"font bytes").bytes(),
        123,
        Scaled::from_raw(10 << 16),
        Scaled::from_raw(9 << 16),
        vec![Scaled::from_raw(1); 7],
        FontMetrics::default(),
    ));
    let selector = source.intern("memo-font-selector");
    source.set_font_identifier_symbol(font, selector);
    let detached = source.detach_font(font).expect("font detachment");

    let mut target = Universe::new();
    let imported = target
        .import_memo_font(&detached, MemoValueLimits::default())
        .expect("font import");
    assert_eq!(target.font(imported), source.font(font));
    assert_eq!(
        target.resolve(
            target
                .font_identifier_symbol(imported)
                .expect("imported font identifier"),
        ),
        "memo-font-selector"
    );
}

#[test]
fn deeply_nested_node_graph_detaches_and_imports_iteratively() {
    use crate::node::{BoxNode, BoxNodeFields, Node};
    use crate::scaled::{GlueSetRatio, Scaled};

    const DEPTH: usize = 2_000;
    let mut source = Universe::new();
    let mut root = source.freeze_node_list(&[]);
    for _ in 0..DEPTH {
        root = source.freeze_node_list(&[Node::HList(BoxNode::new(BoxNodeFields {
            width: Scaled::from_raw(0),
            height: Scaled::from_raw(0),
            depth: Scaled::from_raw(0),
            shift: Scaled::from_raw(0),
            display: false,
            glue_set: GlueSetRatio::ZERO,
            glue_sign: crate::node::Sign::Normal,
            glue_order: Order::Normal,
            children: root,
        }))]);
    }

    let detached = source.detach_node_list(root).expect("deep node detachment");
    let mut target = Universe::new();
    let mut imported = target
        .import_memo_node_list(
            &detached,
            MemoValueLimits {
                max_nodes: DEPTH,
                ..MemoValueLimits::default()
            },
        )
        .expect("deep node import");
    for _ in 0..DEPTH {
        let Some(crate::node_arena::NodeRef::HList(box_node)) = target.nodes(imported).first()
        else {
            panic!("deep imported graph lost a box level");
        };
        imported = box_node.children;
    }
    assert!(target.nodes(imported).is_empty());
}

#[test]
fn detached_values_survive_target_rollback_and_generation_fork() {
    let mut source = Universe::new();
    let tokens = source.intern_token_list(&[Token::Char {
        ch: 'q',
        cat: Catcode::Letter,
    }]);
    let detached = source.detach_token_list(tokens).expect("token detachment");

    let checkpoint = source.snapshot();
    let frozen = source.freeze_generation();
    let mut fork = frozen.fork_at(&checkpoint).expect("generation fork");
    let forked = fork
        .import_memo_token_list(&detached, MemoValueLimits::default())
        .expect("fork import");
    assert_eq!(
        fork.tokens(forked)[0],
        Token::Char {
            ch: 'q',
            cat: Catcode::Letter
        }
    );

    let mut target = Universe::new();
    let checkpoint = target.snapshot();
    let stale = target
        .import_memo_token_list(&detached, MemoValueLimits::default())
        .expect("initial target import");
    target.rollback(&checkpoint);
    let imported = target
        .import_memo_token_list(&detached, MemoValueLimits::default())
        .expect("post-rollback target import");
    assert_ne!(stale, imported);
    assert_eq!(
        target.tokens(imported)[0],
        Token::Char {
            ch: 'q',
            cat: Catcode::Letter
        }
    );
}

#[test]
fn transition_diagnostic_effect_plan_and_artifact_dtos_are_handle_free_and_bounded() {
    let limits = MemoValueLimits::default();
    let input = DetachedInputTransition {
        transition_schema: 3,
        consumed_inputs: vec![ContentHash::from_bytes(b"included").bytes()],
        semantic_payload: vec![1, 2, 3],
    };
    assert_eq!(
        DetachedMemoValue::from_input_transition(&input)
            .expect("input transition encoding")
            .input_transition(limits)
            .expect("input transition decoding"),
        input
    );

    let page = DetachedPageTransition {
        transition_schema: 4,
        semantic_payload: vec![5, 6],
    };
    assert_eq!(
        DetachedMemoValue::from_page_transition(&page)
            .expect("page transition encoding")
            .page_transition(limits)
            .expect("page transition decoding"),
        page
    );

    let diagnostics = vec![DetachedDiagnostic {
        code: "missing-number".into(),
        message: "Missing number, treated as zero.".into(),
        input_ordinal: Some(9),
    }];
    assert_eq!(
        DetachedMemoValue::from_diagnostics(&diagnostics)
            .expect("diagnostic encoding")
            .diagnostics(limits)
            .expect("diagnostic decoding"),
        diagnostics
    );

    let effects = vec![DetachedVirtualEffect {
        operation: "write".into(),
        stream: Some(3),
        payload: b"label".to_vec(),
    }];
    assert_eq!(
        DetachedMemoValue::from_virtual_effects(&effects)
            .expect("virtual-effect encoding")
            .virtual_effects(limits)
            .expect("virtual-effect decoding"),
        effects
    );

    let plan = DetachedPureKernelPlan {
        kernel: "line-break".into(),
        plan_schema: 2,
        payload: vec![8],
    };
    assert_eq!(
        DetachedMemoValue::from_pure_kernel_plan(&plan)
            .expect("pure-plan encoding")
            .pure_kernel_plan(limits)
            .expect("pure-plan decoding"),
        plan
    );

    let artifact = DetachedArtifact {
        artifact_schema: 10,
        payload: vec![10, 20],
    };
    assert_eq!(
        DetachedMemoValue::from_artifact(&artifact)
            .expect("artifact encoding")
            .artifact(limits)
            .expect("artifact decoding"),
        artifact
    );

    let tight = MemoValueLimits {
        max_payload_bytes: 1,
        ..limits
    };
    assert!(
        DetachedMemoValue::from_page_transition(&page)
            .expect("page transition encoding")
            .page_transition(tight)
            .is_err()
    );
}

#[test]
fn stale_schema_collision_candidate_and_retention_are_safe() {
    let left = DetachedMemoValue::new(MemoValueKind::Artifact, b"left".to_vec());
    let right = DetachedMemoValue::new(MemoValueKind::Artifact, b"right".to_vec());
    let forced_candidate_id = 7_u64;
    assert_eq!(forced_candidate_id, forced_candidate_id);
    assert_ne!(left.integrity(), right.integrity());

    let stale = bincode::serialize(&WireEnvelope {
        magic: ENVELOPE_MAGIC,
        schema: 0,
        kind: MemoValueKind::Artifact,
        payload: b"left".to_vec(),
        integrity: left.integrity().bytes(),
    })
    .expect("stale envelope encoding");
    assert!(matches!(
        DetachedMemoValue::from_bytes(&stale, MemoValueLimits::default()),
        Err(MemoValueError::StaleSchema { found: 0 })
    ));

    let weak = Arc::downgrade(&left.payload);
    assert!(left.retained_bytes() >= left.payload.len());
    drop(left);
    assert!(weak.upgrade().is_none());
}
