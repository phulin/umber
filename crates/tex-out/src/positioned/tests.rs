use tex_arith::Scaled;

use crate::{
    BoxNode, ContentHash, FontResource, GlueKind, GlueOrder, GlueSetRatio, GlueSign, GlueSpec,
    JobInfo, KernKind, PageNode, UnvalidatedPageArtifact,
};

use super::{PositionedEvent, TextUnit, lower_page};

fn sp(raw: i32) -> Scaled {
    Scaled::from_raw(raw)
}

#[test]
fn text_runs_keep_exact_anchor_and_baseline_but_not_glyph_positions() {
    let page = page(PageNode::VList(box_node(
        500,
        100,
        20,
        vec![
            PageNode::Kern {
                amount: sp(30),
                kind: KernKind::Explicit,
            },
            PageNode::HList(box_node(
                400,
                40,
                5,
                vec![
                    PageNode::Char {
                        font_id: 1,
                        ch: b'A' as u32,
                        width: sp(25),
                    },
                    PageNode::Kern {
                        amount: sp(-3),
                        kind: KernKind::Font,
                    },
                    PageNode::Lig {
                        font_id: 1,
                        ch: 11,
                        left: b'f' as u32,
                        right: b'i' as u32,
                        width: sp(30),
                    },
                    PageNode::Glue {
                        spec: GlueSpec {
                            width: sp(10),
                            stretch: sp(0),
                            stretch_order: GlueOrder::Normal,
                            shrink: sp(0),
                            shrink_order: GlueOrder::Normal,
                        },
                        kind: GlueKind::Normal,
                        leader: None,
                    },
                    PageNode::Char {
                        font_id: 1,
                        ch: b'B' as u32,
                        width: sp(20),
                    },
                    PageNode::Kern {
                        amount: sp(7),
                        kind: KernKind::Explicit,
                    },
                    PageNode::Char {
                        font_id: 1,
                        ch: b'C' as u32,
                        width: sp(20),
                    },
                ],
            )),
        ],
    )));
    let positioned = lower_page(&page, 1).expect("lower page");
    let runs = positioned
        .events
        .iter()
        .filter_map(|event| match event {
            PositionedEvent::TextRun(run) => Some(run),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].x, sp(0));
    assert_eq!(runs[0].baseline, sp(70));
    assert_eq!(
        runs[0].units,
        vec![
            TextUnit::Code(b'A'),
            TextUnit::Code(b'f'),
            TextUnit::Code(b'i'),
            TextUnit::Space,
            TextUnit::Code(b'B'),
        ]
    );
    assert_eq!(runs[1].x, sp(89));
    assert_eq!(runs[1].baseline, sp(70));
    assert_eq!(runs[1].units, vec![TextUnit::Code(b'C')]);
}

#[test]
fn rules_and_shifted_boxes_use_dvi_coordinates() {
    let mut shifted = box_node(
        50,
        20,
        4,
        vec![PageNode::Rule {
            width: Some(sp(30)),
            height: Some(sp(6)),
            depth: Some(sp(2)),
        }],
    );
    shifted.shift = sp(7);
    let page = page(PageNode::HList(box_node(
        100,
        40,
        10,
        vec![PageNode::HList(shifted)],
    )));
    let positioned = lower_page(&page, 4).expect("lower page");
    let rule = positioned
        .events
        .iter()
        .find_map(|event| match event {
            PositionedEvent::Rule(rule) => Some(rule),
            _ => None,
        })
        .expect("rule event");

    assert_eq!(
        (rule.x, rule.y, rule.width, rule.height),
        (sp(0), sp(41), sp(30), sp(8))
    );
}

fn page(root: PageNode) -> crate::PageArtifact {
    UnvalidatedPageArtifact {
        job: JobInfo {
            mag: 1000,
            banner: "test".to_owned(),
            h_offset: sp(0),
            v_offset: sp(0),
        },
        fonts: vec![FontResource {
            font_id: 1,
            name: "cmr10".to_owned(),
            tfm_content_hash: ContentHash::from_bytes(b"cmr10"),
            tfm_checksum: 0,
            design_size: sp(655_360),
            at_size: sp(655_360),
        }],
        counts: [0; 10],
        root,
        effects: Vec::new(),
    }
    .validate()
    .expect("valid page")
}

fn box_node(width: i32, height: i32, depth: i32, children: Vec<PageNode>) -> BoxNode {
    BoxNode {
        width: sp(width),
        height: sp(height),
        depth: sp(depth),
        shift: sp(0),
        glue_set: GlueSetRatio::ZERO,
        glue_sign: GlueSign::Normal,
        glue_order: GlueOrder::Normal,
        children,
    }
}
