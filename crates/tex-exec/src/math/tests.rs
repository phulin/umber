use super::*;
use tex_lex::MemoryInput;
use tex_state::env::banks::{DimenParam, GlueParam, IntParam};
use tex_state::glue::{GlueSpec, Order};
use tex_state::node::{BoxNode, BoxNodeFields, Sign};
use tex_state::scaled::GlueSetRatio;
use tex_state::{EffectRecord, PrintSink};

fn sp(raw: i32) -> Scaled {
    Scaled::from_raw(raw * Scaled::UNITY)
}

fn terminal_text(stores: &Universe) -> String {
    stores
        .world()
        .effect_records()
        .iter()
        .filter_map(|record| match record {
            EffectRecord::StreamWrite {
                sink: PrintSink::Terminal | PrintSink::TerminalAndLog | PrintSink::Log,
                text,
            } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn display_alignment_finish_assignments_delimiters_and_spacing() {
    let mut stores = Universe::new_with_plain_catcodes();
    crate::install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\count0=7 $$x"));
    let mut execution = crate::ExecutionContext::new("texput");

    finish_display_alignment_assignments(&mut input, &mut stores, &mut execution)
        .expect("post-alignment assignments execute");
    assert_eq!(stores.count(0), 7);
    let fallback = stores.synthetic_origin(tex_state::provenance::SyntheticOriginKind::Test);
    assert_ne!(
        consume_display_alignment_closer(&mut input, &mut stores, fallback)
            .expect("double math shift closes"),
        fallback
    );
    assert_eq!(
        tex_expand::semantic_token(
            input
                .next_traced_token(&mut stores)
                .expect("following token reads")
                .expect("following token remains")
        ),
        Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        }
    );

    let mut missing = InputStack::new(MemoryInput::new("q"));
    assert_eq!(
        consume_display_alignment_closer(&mut missing, &mut stores, fallback)
            .expect("missing closer recovers"),
        fallback
    );
    assert!(terminal_text(&stores).contains("Missing $$ inserted"));
    assert_eq!(
        tex_expand::semantic_token(
            missing
                .next_traced_token(&mut stores)
                .expect("backed-up token reads")
                .expect("non-math command is backed up")
        ),
        Token::Char {
            ch: 'q',
            cat: Catcode::Letter,
        }
    );

    stores.set_int_param(IntParam::PRE_DISPLAY_PENALTY, 11);
    stores.set_int_param(IntParam::POST_DISPLAY_PENALTY, 22);
    stores.set_dimen_param(DimenParam::DISPLAY_INDENT, sp(5));
    let above = stores.intern_glue(GlueSpec {
        width: sp(3),
        stretch: Scaled::from_raw(0),
        stretch_order: Order::Normal,
        shrink: Scaled::from_raw(0),
        shrink_order: Order::Normal,
    });
    let below = stores.intern_glue(GlueSpec {
        width: sp(4),
        stretch: Scaled::from_raw(0),
        stretch_order: Order::Normal,
        shrink: Scaled::from_raw(0),
        shrink_order: Order::Normal,
    });
    stores.set_glue_param(GlueParam::ABOVE_DISPLAY_SKIP, above);
    stores.set_glue_param(GlueParam::BELOW_DISPLAY_SKIP, below);
    let empty = stores.freeze_node_list(&[]);
    let alignment = Node::HList(BoxNode::new(BoxNodeFields {
        width: sp(9),
        height: sp(2),
        depth: sp(1),
        shift: Scaled::from_raw(0),
        display: false,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: empty,
    }));
    let mut nest = ModeNest::new();
    nest.push(Mode::InternalVertical);

    finish_display_alignment(
        &mut nest,
        &mut stores,
        crate::align::FinishedAlignment {
            nodes: vec![alignment],
            aux_prev_depth: Some(sp(7)),
        },
    )
    .expect("display list material inserts");

    let nodes = nest.current_list().nodes();
    assert!(matches!(
        nodes,
        [
            Node::Penalty(11),
            Node::Glue { spec: above_spec, kind: GlueKind::AboveDisplaySkip, .. },
            Node::HList(row),
            Node::Penalty(22),
            Node::Glue { spec: below_spec, kind: GlueKind::BelowDisplaySkip, .. },
        ] if *above_spec == above
            && *below_spec == below
            && row.display
            && row.shift == sp(5)
    ));
    assert_eq!(nest.current_list().prev_depth(), Some(sp(7)));

    let mut resume_input = InputStack::new(MemoryInput::new("z"));
    resume_after_display_alignment(&mut nest, &mut resume_input, &mut stores, Vec::new())
        .expect("post-display scanning resumes");
    assert_eq!(nest.current_mode(), Mode::Horizontal);
    assert_eq!(
        tex_expand::semantic_token(
            resume_input
                .next_traced_token(&mut stores)
                .expect("resumed token reads")
                .expect("resumed token remains")
        ),
        Token::Char {
            ch: 'z',
            cat: Catcode::Letter,
        }
    );
}
