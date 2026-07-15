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
