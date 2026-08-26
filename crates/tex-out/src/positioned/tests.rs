use tex_arith::Scaled;

use crate::{
    BoxNode, FontResource, GlueKind, GlueOrder, GlueSetRatio, GlueSign, GlueSpec, JobInfo,
    KernKind, LeaderPayload, PageEffect, PageNode, PdfAccessibilityEffect, UnvalidatedPageArtifact,
};

use super::{PositionedEvent, TextUnit, lower_page};

fn sp(raw: i32) -> Scaled {
    Scaled::from_raw(raw)
}

#[test]
fn text_runs_keep_exact_unit_anchors_and_baseline() {
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
                        source: vec![b'f' as u32, b'f' as u32, b'i' as u32],
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
            TextUnit::Code(u32::from(b'A')),
            TextUnit::Code(u32::from(b'f')),
            TextUnit::Code(u32::from(b'f')),
            TextUnit::Code(u32::from(b'i')),
            TextUnit::Space,
            TextUnit::Code(u32::from(b'B')),
        ]
    );
    assert_eq!(runs[1].x, sp(89));
    assert_eq!(runs[1].baseline, sp(70));
    assert_eq!(runs[1].units, vec![TextUnit::Code(u32::from(b'C'))]);
    assert_eq!(
        runs[0].positions,
        vec![sp(0), sp(22), sp(22), sp(22), sp(52), sp(62)]
    );
    assert_eq!(runs[1].positions, vec![sp(89)]);
    assert_eq!(
        runs[0].physical_codes,
        vec![Some(b'A'), Some(11), None, None, None, Some(b'B')]
    );
    assert_eq!(runs[1].physical_codes, vec![Some(b'C')]);
}

#[test]
fn interword_glue_survives_a_font_change_with_its_original_font_and_anchor() {
    let page = page(PageNode::HList(box_node(
        100,
        40,
        10,
        vec![
            PageNode::Char {
                font_id: 1,
                ch: b'A' as u32,
                width: sp(20),
            },
            PageNode::Glue {
                spec: GlueSpec {
                    width: sp(12),
                    stretch: sp(0),
                    stretch_order: GlueOrder::Normal,
                    shrink: sp(0),
                    shrink_order: GlueOrder::Normal,
                },
                kind: GlueKind::Normal,
                leader: None,
            },
            PageNode::Char {
                font_id: 2,
                ch: b'B' as u32,
                width: sp(20),
            },
        ],
    )));
    let positioned = lower_page(&page, 9).expect("lower page");
    let runs = positioned
        .events
        .iter()
        .filter_map(|event| match event {
            PositionedEvent::TextRun(run) => Some(run),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].font_id, 1);
    assert_eq!(
        runs[0].units,
        vec![TextUnit::Code(u32::from(b'A')), TextUnit::Space]
    );
    assert_eq!(runs[0].positions, vec![sp(0), sp(20)]);
    assert_eq!(runs[1].font_id, 2);
    assert_eq!(runs[1].positions, vec![sp(32)]);
}

#[test]
fn current_output_font_flows_into_leading_glue_in_a_nested_box() {
    let nested = PageNode::HList(box_node(
        40,
        20,
        5,
        vec![
            PageNode::Glue {
                spec: GlueSpec {
                    width: sp(7),
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
        ],
    ));
    let page = page(PageNode::HList(box_node(
        100,
        40,
        10,
        vec![
            PageNode::Char {
                font_id: 1,
                ch: b'A' as u32,
                width: sp(20),
            },
            nested,
        ],
    )));
    let positioned = lower_page(&page, 10).expect("lower page");
    let nested_run = positioned
        .events
        .iter()
        .filter_map(|event| match event {
            PositionedEvent::TextRun(run) => Some(run),
            _ => None,
        })
        .nth(1)
        .expect("nested text run");
    assert_eq!(
        nested_run.units,
        vec![TextUnit::Space, TextUnit::Code(u32::from(b'B'))]
    );
    assert_eq!(nested_run.positions, vec![sp(20), sp(27)]);
}

#[test]
fn pdf_accessibility_effects_keep_order_and_exact_anchor() {
    let root = PageNode::HList(box_node(
        100,
        40,
        10,
        vec![
            PageNode::Kern {
                amount: sp(17),
                kind: KernKind::Explicit,
            },
            PageNode::WhatsitAnchor { effect_index: 0 },
            PageNode::WhatsitAnchor { effect_index: 1 },
            PageNode::WhatsitAnchor { effect_index: 2 },
        ],
    ));
    let mut page = page(PageNode::HList(box_node(100, 40, 10, Vec::new())));
    page.testing_mut().root = root;
    page.testing_mut().effects = vec![
        PageEffect::PdfAccessibility(PdfAccessibilityEffect::InterwordSpaceOn),
        PageEffect::PdfAccessibility(PdfAccessibilityEffect::FakeSpace),
        PageEffect::PdfAccessibility(PdfAccessibilityEffect::InterwordSpaceOff),
    ];
    let positioned = lower_page(&page, 2).expect("lower page");
    let controls = positioned
        .events
        .iter()
        .filter_map(|event| match event {
            PositionedEvent::PdfAccessibility(control) => Some(control),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(controls.len(), 3);
    assert_eq!(controls[0].x, sp(17));
    assert_eq!(controls[0].y, sp(40));
    assert_eq!(
        controls
            .iter()
            .map(|control| control.control)
            .collect::<Vec<_>>(),
        vec![
            PdfAccessibilityEffect::InterwordSpaceOn,
            PdfAccessibilityEffect::FakeSpace,
            PdfAccessibilityEffect::InterwordSpaceOff,
        ]
    );
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

#[test]
fn form_references_advance_at_pdftex_hlist_and_vlist_baselines() {
    let effects = vec![
        PageEffect::PdfRefXForm {
            object: 1,
            width: sp(10),
            height: sp(7),
            depth: sp(3),
        },
        PageEffect::PdfRefXForm {
            object: 2,
            width: sp(20),
            height: sp(11),
            depth: sp(4),
        },
    ];
    let horizontal_root = PageNode::HList(box_node(
        30,
        20,
        5,
        vec![
            PageNode::WhatsitAnchor { effect_index: 0 },
            PageNode::WhatsitAnchor { effect_index: 1 },
        ],
    ));
    let mut horizontal = page(PageNode::HList(box_node(0, 0, 0, Vec::new())));
    horizontal.testing_mut().root = horizontal_root;
    horizontal.testing_mut().effects = effects.clone();
    let positioned = lower_page(&horizontal, 0).expect("lower horizontal forms");
    let positions = positioned
        .events
        .iter()
        .filter_map(|event| match event {
            PositionedEvent::PdfGraphics(graphics) => Some((graphics.x, graphics.y)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(positions, vec![(sp(0), sp(20)), (sp(10), sp(20))]);

    let vertical_root = PageNode::VList(box_node(
        20,
        25,
        0,
        vec![
            PageNode::WhatsitAnchor { effect_index: 0 },
            PageNode::WhatsitAnchor { effect_index: 1 },
        ],
    ));
    let mut vertical = page(PageNode::VList(box_node(0, 0, 0, Vec::new())));
    vertical.testing_mut().root = vertical_root;
    vertical.testing_mut().effects = effects;
    let positioned = lower_page(&vertical, 0).expect("lower vertical forms");
    let positions = positioned
        .events
        .iter()
        .filter_map(|event| match event {
            PositionedEvent::PdfGraphics(graphics) => Some((graphics.x, graphics.y)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(positions, vec![(sp(0), sp(7)), (sp(0), sp(21))]);
}

#[test]
fn explicit_letterspace_movements_anchor_each_physical_glyph() {
    let page = page(PageNode::HList(box_node(
        100,
        40,
        10,
        vec![
            PageNode::Kern {
                amount: sp(4),
                kind: KernKind::Explicit,
            },
            PageNode::Char {
                font_id: 1,
                ch: b'A' as u32,
                width: sp(20),
            },
            PageNode::Kern {
                amount: sp(5),
                kind: KernKind::Explicit,
            },
            PageNode::Kern {
                amount: sp(4),
                kind: KernKind::Explicit,
            },
            PageNode::Char {
                font_id: 1,
                ch: b'B' as u32,
                width: sp(20),
            },
            PageNode::Kern {
                amount: sp(5),
                kind: KernKind::Explicit,
            },
        ],
    )));
    let positioned = lower_page(&page, 9).expect("lower flattened letterspace page");
    let runs = positioned
        .events
        .iter()
        .filter_map(|event| match event {
            PositionedEvent::TextRun(run) => Some(run),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(runs.len(), 2);
    assert_eq!((runs[0].x, runs[0].font_id), (sp(4), 1));
    assert_eq!((runs[1].x, runs[1].font_id), (sp(33), 1));
}

#[test]
fn deep_box_geometry_uses_bounded_explicit_frames() {
    const DEPTH: usize = 2_048;
    let mut root = PageNode::Rule {
        width: Some(sp(1)),
        height: Some(sp(1)),
        depth: Some(sp(0)),
    };
    for _ in 0..DEPTH {
        root = PageNode::HList(box_node(1, 1, 0, vec![root]));
    }
    let page = page(root);

    let positioned = lower_page(&page, 11).expect("lower deeply nested page");
    assert_eq!(
        positioned
            .events
            .iter()
            .filter(|event| matches!(event, PositionedEvent::Box(_)))
            .count(),
        DEPTH
    );
    crate::dvi::DviPagePlan::compile(&page).expect("compile deeply nested DVI geometry");
}

#[test]
fn maximum_depth_canonical_artifact_compiles_with_bounded_frames() {
    const DEPTH: usize = 4_095;
    let mut root = PageNode::Rule {
        width: Some(sp(1)),
        height: Some(sp(1)),
        depth: Some(sp(0)),
    };
    for _ in 0..DEPTH {
        root = PageNode::HList(box_node(1, 1, 0, vec![root]));
    }
    let page = page(root);
    let bytes = page.to_bytes().expect("encode maximum-depth artifact");

    crate::dvi::DviPagePlan::compile_v10(&bytes)
        .expect("compile maximum-depth canonical artifact bytes");
}

#[test]
fn maximum_depth_nested_leader_dvi_uses_bounded_frames() {
    const NESTED_LEADERS: usize = 4_093;
    let page = nested_hleader_page(NESTED_LEADERS);
    let bytes = page
        .to_bytes()
        .expect("encode maximum-depth nested-leader artifact");
    // This fixture exists to exercise byte replay. Keep its recursively owned
    // construction tree out of the production replay's RSS/stack assertion.
    std::mem::forget(page);

    crate::dvi::DviPagePlan::compile_v10(&bytes)
        .expect("compile maximum-depth nested-leader bytes");
}

#[test]
fn maximum_depth_nested_leader_positioned_geometry_uses_bounded_frames() {
    const NESTED_LEADERS: usize = 4_093;
    let page = nested_hleader_page(NESTED_LEADERS);

    let positioned = lower_page(&page, 12).expect("lower maximum-depth nested leaders");
    assert_eq!(
        positioned
            .events
            .iter()
            .filter(|event| matches!(event, PositionedEvent::Box(_)))
            .count(),
        NESTED_LEADERS + 1
    );
    assert_eq!(
        positioned
            .events
            .iter()
            .filter(|event| matches!(event, PositionedEvent::Rule(_)))
            .count(),
        1
    );
    std::mem::forget(page);
}

#[test]
fn nested_leader_geometry_preserves_depth_first_order() {
    let page = nested_hleader_page(2);
    let positioned = lower_page(&page, 13).expect("lower nested leader order fixture");
    let order = positioned
        .events
        .iter()
        .map(|event| match event {
            PositionedEvent::Box(node) => format!("box:{}:{}", node.id, node.depth),
            PositionedEvent::BoxEnd(node) => format!("end:{}:{}", node.id, node.depth),
            PositionedEvent::Rule(_) => "rule".to_owned(),
            _ => "other".to_owned(),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        order,
        [
            "box:0:1", "box:1:2", "box:2:3", "rule", "end:2:3", "end:1:2", "end:0:1"
        ]
    );

    let bytes = page.to_bytes().expect("encode nested leader order fixture");
    assert_eq!(
        crate::dvi::DviPagePlan::compile(&page).expect("compile owned nested leaders"),
        crate::dvi::DviPagePlan::compile_v10(&bytes).expect("replay nested leader bytes")
    );
}

#[test]
fn malformed_nested_leader_font_ids_fail_closed() {
    let first = PageNode::Char {
        font_id: 1,
        ch: b'A' as u32,
        width: sp(41),
    };
    let nested = PageNode::Glue {
        spec: leader_glue_spec(),
        kind: GlueKind::Leaders,
        leader: Some(LeaderPayload::HList(box_node(11, 1, 0, vec![first]))),
    };
    let later = PageNode::Char {
        font_id: 1,
        ch: b'B' as u32,
        width: sp(42),
    };
    let page = page(PageNode::HList(box_node(1, 1, 0, vec![nested, later])));
    let mut bytes = page.to_bytes().expect("encode malformed-order fixture");
    for (ch, width, invalid_font) in [(b'A', 41_i32, 99_u32), (b'B', 42, 100)] {
        let needle = [
            &[0][..],
            &1_u32.to_le_bytes(),
            &u32::from(ch).to_le_bytes(),
            &width.to_le_bytes(),
        ]
        .concat();
        let offset = bytes
            .windows(needle.len())
            .position(|window| window == needle)
            .expect("character scalar encoding");
        bytes[offset + 1..offset + 5].copy_from_slice(&invalid_font.to_le_bytes());
    }
    assert_eq!(
        crate::dvi::DviPagePlan::compile_v10(&bytes),
        Err(crate::dvi::DviError::Artifact {
            message: "truncated page artifact".to_owned(),
        })
    );
}

fn nested_hleader_page(levels: usize) -> crate::PageArtifact {
    let mut child = PageNode::Rule {
        width: Some(sp(1)),
        height: Some(sp(1)),
        depth: Some(sp(0)),
    };
    for _ in 0..levels {
        child = PageNode::Glue {
            spec: leader_glue_spec(),
            kind: GlueKind::Leaders,
            leader: Some(LeaderPayload::HList(box_node(11, 1, 0, vec![child]))),
        };
    }
    page(PageNode::HList(box_node(1, 1, 0, vec![child])))
}

fn leader_glue_spec() -> GlueSpec {
    GlueSpec {
        width: sp(1),
        stretch: sp(0),
        stretch_order: GlueOrder::Normal,
        shrink: sp(0),
        shrink_order: GlueOrder::Normal,
    }
}

fn page(root: PageNode) -> crate::PageArtifact {
    UnvalidatedPageArtifact {
        job: JobInfo {
            mag: 1000,
            banner: "test".to_owned(),
            h_offset: sp(0),
            v_offset: sp(0),
            page_origin_x: sp(0),
            page_origin_y: sp(0),
            page_width: sp(0),
            page_height: sp(0),
        },
        fonts: (1_u8..=2)
            .map(|font_id| FontResource {
                font_id: u32::from(font_id),
                name: format!("cmr{font_id}0"),
                tfm_content_hash: tex_fonts::font_content_hash(&[font_id]),
                tfm_checksum: 0,
                design_size: sp(655_360),
                at_size: sp(655_360),
                layout_policy: tex_fonts::FontLayoutPolicy::ClassicTfmExact,
                mapping_fallback: None,
                opentype: None,
                semantic_identity: tex_fonts::FontSourceIdentity::from_bytes([font_id; 8]),
                construction: crate::FontResourceConstruction::Loaded,
            })
            .collect(),
        counts: [0; 10],
        root,
        effects: Vec::new(),
        math_events: Vec::new(),
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
