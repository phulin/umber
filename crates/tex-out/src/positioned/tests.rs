use tex_arith::Scaled;

use crate::{
    BoxNode, ContentHash, FontResource, GlueKind, GlueOrder, GlueSetRatio, GlueSign, GlueSpec,
    JobInfo, KernKind, PageEffect, PageNode, PdfAccessibilityEffect, UnvalidatedPageArtifact,
};

use super::{PositionedEvent, TextUnit, lower_page};
use crate::dvi::coordinates::{CoordinateError, compare_page};

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
            TextUnit::Code(b'A'),
            TextUnit::Code(b'f'),
            TextUnit::Code(b'f'),
            TextUnit::Code(b'i'),
            TextUnit::Space,
            TextUnit::Code(b'B'),
        ]
    );
    assert_eq!(runs[1].x, sp(89));
    assert_eq!(runs[1].baseline, sp(70));
    assert_eq!(runs[1].units, vec![TextUnit::Code(b'C')]);
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
    assert_eq!(runs[0].units, vec![TextUnit::Code(b'A'), TextUnit::Space]);
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
        vec![TextUnit::Space, TextUnit::Code(b'B')]
    );
    assert_eq!(nested_run.positions, vec![sp(20), sp(27)]);
    compare_page(&page, &positioned).expect("leading browser space preserves DVI glyph anchor");
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
    compare_page(&page, &positioned).expect("PDF-only effects do not alter DVI coordinates");
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
fn dvi_oracle_rejects_one_sp_anchor_drift_with_event_context() {
    let page = page(PageNode::HList(box_node(
        100,
        40,
        10,
        vec![PageNode::Char {
            font_id: 1,
            ch: b'A' as u32,
            width: sp(20),
        }],
    )));
    let mut positioned = lower_page(&page, 7).expect("lower page");
    compare_page(&page, &positioned).expect("exact DVI coordinates");
    let run = positioned
        .events
        .iter_mut()
        .find_map(|event| match event {
            PositionedEvent::TextRun(run) => Some(run),
            _ => None,
        })
        .expect("text run");
    run.baseline = run.baseline.checked_add(sp(1)).expect("one sp");
    let error = compare_page(&page, &positioned).expect_err("baseline drift must fail");
    assert!(matches!(error, CoordinateError::Mismatch { page: 7, .. }));
    assert!(error.to_string().contains("text anchor differs"));
}

#[test]
fn dvi_oracle_ignores_within_run_glyph_advances_and_width() {
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
            PageNode::Char {
                font_id: 1,
                ch: b'V' as u32,
                width: sp(20),
            },
        ],
    )));
    let positioned = lower_page(&page, 8).expect("lower page");
    let mut changed_advances = page.clone();
    let PageNode::HList(root) = &mut changed_advances.testing_mut().root else {
        unreachable!()
    };
    let PageNode::Char { width, .. } = &mut root.children[0] else {
        unreachable!()
    };
    *width = sp(73);
    compare_page(&changed_advances, &positioned)
        .expect("interior browser-owned glyph positions are excluded");
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
    compare_page(&page, &positioned).expect("positioned anchors match flattened DVI");
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
                tfm_content_hash: ContentHash::from_bytes(&[font_id]),
                tfm_checksum: 0,
                design_size: sp(655_360),
                at_size: sp(655_360),
                layout_policy: tex_fonts::FontLayoutPolicy::ClassicTfmExact,
                mapping_fallback: None,
                opentype: None,
                semantic_identity: tex_fonts::FontSourceIdentity::from_bytes([font_id; 32]),
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
