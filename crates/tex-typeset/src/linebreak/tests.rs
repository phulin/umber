use super::*;
use tex_state::Universe;
use tex_state::glue::{GlueSpec, Order};
use tex_state::node::{DiscKind, GlueKind, KernKind, Node};
use tex_state::scaled::Scaled;

fn sp(raw: i32) -> Scaled {
    Scaled::from_raw(raw)
}

fn params(width: i32) -> LineBreakParams {
    LineBreakParams {
        pretolerance: 100,
        tolerance: 1000,
        line_penalty: 10,
        hyphen_penalty: 50,
        ex_hyphen_penalty: 50,
        adj_demerits: 10_000,
        double_hyphen_demerits: 10_000,
        final_hyphen_demerits: 5_000,
        emergency_stretch: sp(0),
        looseness: 0,
        shape: LineShape::natural(sp(width)),
    }
}

fn kern(width: i32) -> Node {
    Node::Kern {
        amount: sp(width),
        kind: KernKind::Explicit,
    }
}

fn rule(width: i32) -> Node {
    Node::Rule {
        width: Some(sp(width)),
        height: None,
        depth: None,
    }
}

#[test]
fn breaks_at_legal_glue() {
    let mut universe = Universe::new();
    let glue = universe.intern_glue(GlueSpec {
        width: sp(10),
        stretch: sp(10),
        stretch_order: Order::Normal,
        shrink: sp(5),
        shrink_order: Order::Normal,
    });
    let nodes = vec![
        Node::Kern {
            amount: sp(20),
            kind: KernKind::Explicit,
        },
        Node::Glue {
            spec: glue,
            kind: GlueKind::Normal,
        },
        Node::Kern {
            amount: sp(20),
            kind: KernKind::Explicit,
        },
        Node::Glue {
            spec: glue,
            kind: GlueKind::Normal,
        },
        Node::Kern {
            amount: sp(20),
            kind: KernKind::Explicit,
        },
    ];
    let mut hook = NoHyphenation;
    let result = line_break(&universe, &nodes, params(30), &mut hook);
    assert_eq!(
        result.breaks.last().map(|br| br.position),
        Some(nodes.len())
    );
}

#[test]
fn parshape_repeats_last_line_and_overrides_hanging() {
    let shape = LineShape {
        hsize: sp(100),
        parshape: Some(ParagraphShape {
            lines: vec![
                LineShapeEntry {
                    indent: sp(3),
                    width: sp(40),
                },
                LineShapeEntry {
                    indent: sp(5),
                    width: sp(30),
                },
            ],
        }),
        hang_indent: sp(20),
        hang_after: 0,
    };

    assert_eq!(
        shape.dimensions(1),
        LineDimensions {
            indent: sp(3),
            width: sp(40),
        }
    );
    assert_eq!(
        shape.dimensions(3),
        LineDimensions {
            indent: sp(5),
            width: sp(30),
        }
    );
}

#[test]
fn hangindent_selects_affected_lines() {
    let mut shape = LineShape {
        hsize: sp(100),
        parshape: None,
        hang_indent: sp(25),
        hang_after: 1,
    };
    assert_eq!(
        shape.dimensions(1),
        LineDimensions {
            indent: sp(0),
            width: sp(100),
        }
    );
    assert_eq!(
        shape.dimensions(2),
        LineDimensions {
            indent: sp(25),
            width: sp(75),
        }
    );

    shape.hang_indent = sp(-25);
    shape.hang_after = -2;
    assert_eq!(
        shape.dimensions(1),
        LineDimensions {
            indent: sp(0),
            width: sp(75),
        }
    );
    assert_eq!(
        shape.dimensions(3),
        LineDimensions {
            indent: sp(0),
            width: sp(100),
        }
    );
}

#[test]
fn break_glue_does_not_contribute_to_preceding_line_width() {
    let mut universe = Universe::new();
    let glue = universe.intern_glue(GlueSpec {
        width: sp(1000),
        stretch: sp(0),
        stretch_order: Order::Normal,
        shrink: sp(0),
        shrink_order: Order::Normal,
    });
    let nodes = vec![
        rule(20),
        Node::Glue {
            spec: glue,
            kind: GlueKind::Normal,
        },
        rule(20),
    ];
    let mut hook = NoHyphenation;
    let result = line_break(&universe, &nodes, params(20), &mut hook);
    assert_eq!(result.breaks.first().map(|br| br.position), Some(2));
}

#[test]
fn discretionary_penalty_comes_from_source_kind() {
    let mut universe = Universe::new();
    let pre = universe.freeze_node_list(&[kern(0)]);
    let empty = universe.freeze_node_list(&[]);
    let mut params = params(20);
    params.pretolerance = -1;
    params.hyphen_penalty = 321;
    params.ex_hyphen_penalty = 654;
    let nodes = vec![
        kern(20),
        Node::Disc {
            kind: DiscKind::AutomaticHyphen,
            pre,
            post: empty,
            replace: empty,
        },
        kern(20),
    ];
    let mut hook = NoHyphenation;
    let result = line_break(&universe, &nodes, params.clone(), &mut hook);
    assert_eq!(result.breaks.first().map(|br| br.penalty), Some(321));

    let nodes = vec![
        kern(20),
        Node::Disc {
            kind: DiscKind::ExplicitHyphen,
            pre,
            post: empty,
            replace: empty,
        },
        kern(20),
    ];
    let result = line_break(&universe, &nodes, params, &mut hook);
    assert_eq!(result.breaks.first().map(|br| br.penalty), Some(654));
}

#[test]
fn final_hyphen_demerits_apply_to_penultimate_hyphenated_line() {
    let mut universe = Universe::new();
    let empty = universe.freeze_node_list(&[]);
    let nodes = vec![
        kern(20),
        Node::Disc {
            kind: DiscKind::AutomaticHyphen,
            pre: empty,
            post: empty,
            replace: empty,
        },
        kern(20),
    ];
    let mut base = params(20);
    base.pretolerance = -1;
    base.hyphen_penalty = 0;
    base.final_hyphen_demerits = 0;
    let mut hook = NoHyphenation;
    let without = line_break(&universe, &nodes, base.clone(), &mut hook).demerits;
    base.final_hyphen_demerits = 1234;
    let with = line_break(&universe, &nodes, base, &mut hook).demerits;
    assert_eq!(with - without, 1234);
}
