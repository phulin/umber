use super::*;

#[test]
fn short_diagnostic_pairs_discretionaries_across_ligature_expansion() {
    // The physical projection expands the leading semantic ligature into two
    // characters. Positional zipping would therefore miss the matching disc
    // and retain its deliberately wrong `B` branch instead of the semantic
    // hyphen. Ordered discretionary pairing preserves both side-list identity
    // and the physical replacement topology.
    let mut stores = Universe::new();
    let font = tex_state::font::NULL_FONT;
    let empty = stores.freeze_node_list(&[]);
    let semantic_hyphen = stores.freeze_node_list(&[Node::Char {
        font,
        ch: '-',
        origin: tex_state::token::OriginId::UNKNOWN,
    }]);
    let physical_b = stores.freeze_node_list(&[Node::Char {
        font,
        ch: 'B',
        origin: tex_state::token::OriginId::UNKNOWN,
    }]);
    let semantic = [
        Node::Lig {
            font,
            ch: 'B',
            orig: vec!['B', 'B'],
            origins: vec![tex_state::token::OriginId::UNKNOWN; 2],
            left_hit: false,
            right_hit: false,
        },
        Node::Disc {
            kind: tex_state::node::DiscKind::AutomaticHyphen,
            pre: semantic_hyphen,
            post: empty,
            replace: empty,
            physical_replace_count: 0,
        },
    ];
    let physical = [
        Node::Char {
            font,
            ch: 'B',
            origin: tex_state::token::OriginId::UNKNOWN,
        },
        Node::Char {
            font,
            ch: 'B',
            origin: tex_state::token::OriginId::UNKNOWN,
        },
        Node::Disc {
            kind: tex_state::node::DiscKind::AutomaticHyphen,
            pre: physical_b,
            post: empty,
            replace: physical_b,
            physical_replace_count: 1,
        },
    ];

    let projected = super::packaging::project_short_diagnostic_discs(&physical, &semantic);
    let Node::Disc {
        pre,
        replace,
        physical_replace_count,
        ..
    } = projected[2]
    else {
        panic!("third physical node remains a discretionary")
    };
    assert_eq!(pre, semantic_hyphen);
    assert_eq!(replace, physical_b);
    assert_eq!(physical_replace_count, 1);
}

#[test]
fn unboxing_removes_margin_kerns_without_mutating_source_or_reordering_material() {
    let mut stores = Universe::new_with_plain_catcodes();
    let font = tex_state::font::NULL_FONT;
    let source_nodes = [
        Node::Penalty(1),
        Node::MarginKern {
            amount: Scaled::from_raw(-Scaled::UNITY),
            side: tex_state::node::MarginKernSide::Left,
            font,
            ch: b'A',
        },
        Node::Penalty(2),
        Node::MarginKern {
            amount: Scaled::from_raw(-2 * Scaled::UNITY),
            side: tex_state::node::MarginKernSide::Right,
            font,
            ch: b'.',
        },
        Node::Penalty(3),
    ];
    let children = stores.freeze_node_list(&source_nodes);
    let mut nest = ModeNest::new();
    nest.push(Mode::RestrictedHorizontal)
        .expect("test mode push");
    let mut fuel = tex_command::CommandFuelLedger::default();

    append_unboxed(
        &mut nest,
        &mut stores,
        Some(UnboxSource::PinnedSurvivor(children)),
        fuel.fuel_mut(),
    )
    .expect("unbox succeeds");

    assert_eq!(
        nest.current_list().nodes(),
        &[Node::Penalty(1), Node::Penalty(2), Node::Penalty(3)],
        "non-margin nodes retain their original order",
    );
    assert_eq!(
        stores.nodes(children),
        source_nodes.as_slice(),
        "copy-style unboxing does not mutate the frozen source box",
    );
}
