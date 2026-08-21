//! Regression coverage for `DumpConfig` and the `\showbox`/`\showlists`
//! node-list renderer.

use super::*;
use tex_state::env::banks::IntParam;
use tex_state::glue::Order;
use tex_state::math::{LimitType, MathChar, MathChoice, MathField, MathNoad, NoadClass, NoadKind};
use tex_state::node::{
    AdjustNode, BoxNodeFields, DiscKind, GlueKind, KernKind, LeaderPayload, MarginKernSide, Node,
    Sign, UnsetKind, UnsetNode, UnsetNodeFields, Whatsit,
};
use tex_state::scaled::{GlueSetRatio, Scaled};
use tex_state::token::{Catcode, Token};

fn with_context<R>(
    test: impl for<'id> FnOnce(&mut tex_state::CommandContext<'_, tex_state::GenerationBrand<'id>>) -> R,
) -> R {
    crate::test_harness::with_tex82_universe(|universe| {
        crate::test_harness::with_admitted(universe, test)
    })
}

fn with_plain_context<R>(
    test: impl for<'id> FnOnce(&mut tex_state::CommandContext<'_, tex_state::GenerationBrand<'id>>) -> R,
) -> R {
    crate::test_harness::with_plain_universe(|universe| {
        crate::test_harness::with_admitted(universe, test)
    })
}

fn assign_int<G>(context: &mut tex_state::CommandContext<'_, G>, parameter: IntParam, value: i32) {
    context
        .assign_int_param(parameter, value, tex_state::AssignmentScope::Global)
        .expect("node-dump fixture integer assignment");
}

fn node_tokens(tokens: impl IntoIterator<Item = Token>) -> tex_state::node::NodeTokenList {
    tex_state::node::NodeTokenList::new(
        tokens
            .into_iter()
            .map(tex_state::token::TokenWord::pack)
            .collect::<Vec<_>>(),
    )
}

fn page_vec<G>(context: &tex_state::CommandContext<'_, G>, root: PageListId) -> Vec<Node> {
    context
        .page_node_list(root)
        .expect("test list belongs to the page arena")
        .nodes()
        .to_vec()
}

/// TeX82 §§696--697 print and test the four packed delimiter quarters.
/// The scanner's upper math-class bits are outside that field, so they neither
/// make a null delimiter visible nor appear in a non-null diagnostic.
#[test]
fn fraction_dump_renders_the_packed_delimiter_field() {
    with_context(|context| {
        for (left, right, expected) in [
            (None, None, "\\fraction, thickness = default\n"),
            (Some(0), Some(0), "\\fraction, thickness = default\n"),
            (
                Some(0x0400_0000),
                Some(0x0700_0000),
                "\\fraction, thickness = default\n",
            ),
            (
                Some(0x0416_2362),
                Some(0),
                "\\fraction, thickness = default, left-delimiter \"162362\n",
            ),
            (
                Some(0),
                Some(0x0716_2362),
                "\\fraction, thickness = default, right-delimiter \"162362\n",
            ),
            (
                Some(0x04ab_cdef),
                Some(0x0712_3456),
                "\\fraction, thickness = default, left-delimiter \"ABCDEF, right-delimiter \"123456\n",
            ),
        ] {
            let mut out = String::new();
            dump_fraction_header(&context, FractionThickness::Default, left, right, &mut out);
            assert_eq!(out, expected);
        }
    });
}

/// TeX82 §696 uses the same four-quarter rendering for radical and
/// left/right noads as §697 uses for fraction delimiters. The upper scanner
/// class bits are excluded, but a null noad is still printed.
#[test]
fn noad_dump_renders_the_packed_delimiter_field() {
    with_context(|context| {
        let nodes = [
            Node::MathNoad(MathNoad::new(
                NoadKind::Radical {
                    delimiter: 0x0728_2382,
                },
                MathField::Empty,
            )),
            Node::MathNoad(MathNoad::new(
                NoadKind::Radical {
                    delimiter: 0x0700_0000,
                },
                MathField::Empty,
            )),
            Node::MathNoad(MathNoad::new(
                NoadKind::LeftDelimiter {
                    delimiter: 0x0416_2362,
                },
                MathField::Empty,
            )),
            Node::MathNoad(MathNoad::new(
                NoadKind::LeftDelimiter {
                    delimiter: 0x0400_0000,
                },
                MathField::Empty,
            )),
            Node::MathNoad(MathNoad::new(
                NoadKind::RightDelimiter {
                    delimiter: 0x0712_3456,
                },
                MathField::Empty,
            )),
            Node::MathNoad(MathNoad::new(
                NoadKind::RightDelimiter {
                    delimiter: 0x0700_0000,
                },
                MathField::Empty,
            )),
        ];
        let list = context.publish_page_nodes(nodes.to_vec());

        assert_eq!(
            dump_page_list(
                &context,
                list.clone(),
                DumpConfig {
                    breadth: 10,
                    depth: 10,
                    profile: tex_command::CommandProfile::TEX82,
                },
            ),
            concat!(
                "\\radical\"282382\n",
                "\\radical\"0\n",
                "\\left\"162362\n",
                "\\left\"0\n",
                "\\right\"123456\n",
                "\\right\"0\n",
            ),
        );
    });
}

#[test]
fn deferred_write_dump_uses_show_token_list_control_word_separator() {
    with_context(|context| {
        let help = context.intern_relaxed_control_sequence("help");
        let tokens = node_tokens([
            Token::Cs(help),
            Token::Char {
                ch: '!',
                cat: Catcode::Other,
            },
        ]);
        let write = Node::Whatsit(Whatsit::DeferredWrite {
            sink: tex_state::PrintSink::TerminalAndLog,
            tokens,
        });

        assert_eq!(
            dump_node_slice(
                &context,
                &[write],
                DumpConfig {
                    breadth: 10,
                    depth: 10,
                    profile: tex_command::CommandProfile::TEX82,
                },
            ),
            "\\write*{\\help !}\n",
        );
    });
}

#[test]
fn whatsit_dump_uses_live_escape_character() {
    with_context(|context| {
        assign_int(context, IntParam::ESCAPE_CHAR, i32::from(b'|'));
        let tokens = node_tokens([]);
        let write = Node::Whatsit(Whatsit::DeferredWrite {
            sink: tex_state::PrintSink::Log,
            tokens,
        });

        assert_eq!(
            dump_node_slice(
                &context,
                &[write],
                DumpConfig {
                    breadth: 10,
                    depth: 10,
                    profile: tex_command::CommandProfile::TEX82,
                },
            ),
            "|write-{}\n",
        );
    });
}

#[test]
fn special_dump_prints_eight_bit_payload_as_tex_character_strings() {
    with_context(|context| {
        let special = Node::Whatsit(Whatsit::Special {
            class: "dvi".to_owned(),
            payload: b"A\x00\x1f\x7f\x80\xff".to_vec(),
        });

        assert_eq!(
            dump_node_slice(
                &context,
                &[special],
                DumpConfig {
                    breadth: 10,
                    depth: 10,
                    profile: tex_command::CommandProfile::TEX82,
                },
            ),
            "\\special{A^^@^^_^^?^^80^^ff}\n",
        );
    });
}

#[test]
fn ligature_dump_includes_original_character_list() {
    with_context(|context| {
        let identifier = context.intern_relaxed_control_sequence("f");
        context.set_font_identifier_symbol(tex_state::font::NULL_FONT, identifier);
        let ligature = Node::Lig {
            font: tex_state::font::NULL_FONT,
            ch: 'X',
            orig: vec!['a', 'b'],
            origins: vec![tex_state::token::OriginId::UNKNOWN; 2],
            left_hit: false,
            right_hit: false,
        };

        assert_eq!(
            dump_node_slice(
                &context,
                &[ligature],
                DumpConfig {
                    breadth: 10,
                    depth: 10,
                    profile: tex_command::CommandProfile::TEX82,
                },
            ),
            "\\f X (ligature ab)\n",
        );
    });
}

#[test]
fn node_dump_honors_live_newline_character_in_ligature_components() {
    // TeX82 §§59/173--174: `show_node_list` displays a ligature's character
    // and original list through `print_ASCII`/`short_display`, both aliases
    // of the one-character-string `print` path. It tests `\newlinechar`
    // before expanding an unprintable byte to caret notation.
    with_context(|context| {
        assign_int(context, IntParam::NEWLINE_CHAR, 10);
        let identifier = context.intern_relaxed_control_sequence("f");
        context.set_font_identifier_symbol(tex_state::font::NULL_FONT, identifier);
        let ligature = Node::Lig {
            font: tex_state::font::NULL_FONT,
            ch: '-',
            orig: vec!['[', '\n', ']'],
            origins: vec![tex_state::token::OriginId::UNKNOWN; 3],
            left_hit: false,
            right_hit: false,
        };
        let config = || DumpConfig {
            breadth: 10,
            depth: 10,
            profile: tex_command::CommandProfile::TEX82,
        };

        assert_eq!(
            dump_node_slice(&context, std::slice::from_ref(&ligature), config()),
            "\\f - (ligature [\n])\n",
        );

        assign_int(context, IntParam::NEWLINE_CHAR, -1);
        assert_eq!(
            dump_node_slice(&context, &[ligature], config()),
            "\\f - (ligature [^^J])\n",
        );
    });
}

#[test]
fn physical_character_nodes_use_tex_eight_bit_print_ascii_spelling() {
    with_context(|context| {
        let identifier = context.intern_relaxed_control_sequence("f");
        context.set_font_identifier_symbol(tex_state::font::NULL_FONT, identifier);
        let character = Node::Char {
            font: tex_state::font::NULL_FONT,
            ch: '\u{82}',
            origin: tex_state::token::OriginId::UNKNOWN,
        };
        let ligature = Node::Lig {
            font: tex_state::font::NULL_FONT,
            ch: '\u{82}',
            orig: vec!['C', 'A'],
            origins: vec![tex_state::token::OriginId::UNKNOWN; 2],
            left_hit: false,
            right_hit: false,
        };

        assert_eq!(
            dump_node_slice(
                &context,
                &[character, ligature],
                DumpConfig {
                    breadth: 10,
                    depth: 10,
                    profile: tex_command::CommandProfile::TEX82,
                },
            ),
            "\\f ^^82\n\\f ^^82 (ligature CA)\n",
        );
    });
}

#[test]
fn ligature_semantic_equality_ignores_original_character_provenance() {
    let with_unknown_origins: Node = Node::Lig {
        font: tex_state::font::NULL_FONT,
        ch: 'X',
        orig: vec!['a', 'b'],
        origins: vec![tex_state::token::OriginId::UNKNOWN; 2],
        left_hit: false,
        right_hit: false,
    };
    let without_origins: Node = Node::Lig {
        font: tex_state::font::NULL_FONT,
        ch: 'X',
        orig: vec!['a', 'b'],
        origins: Vec::new(),
        left_hit: false,
        right_hit: false,
    };

    assert_eq!(with_unknown_origins, without_origins);
}

#[test]
fn ligature_dump_marks_left_and_right_boundaries() {
    with_context(|context| {
        let identifier = context.intern_relaxed_control_sequence("f");
        context.set_font_identifier_symbol(tex_state::font::NULL_FONT, identifier);
        let ligature = Node::Lig {
            font: tex_state::font::NULL_FONT,
            ch: 'X',
            orig: vec!['a', 'b'],
            origins: vec![tex_state::token::OriginId::UNKNOWN; 2],
            left_hit: true,
            right_hit: true,
        };

        assert_eq!(
            dump_node_slice(
                &context,
                &[ligature],
                DumpConfig {
                    breadth: 10,
                    depth: 10,
                    profile: tex_command::CommandProfile::TEX82,
                },
            ),
            "\\f X (ligature |ab|)\n",
        );
    });
}

/// Builds the `\hbox{\kern1pt}` box register from bd `umber2-alfh.6`'s
/// reproduction (`tests/corpus/command-semantic/main-control/show-box/show-box.tex`):
/// `\setbox0=\hbox{\kern1pt}`.
fn hbox_with_one_point_kern<G>(context: &mut tex_state::CommandContext<'_, G>) -> PageListId {
    let kern = Node::Kern {
        amount: Scaled::from_raw(Scaled::UNITY),
        kind: KernKind::Explicit,
    };
    let children = context.publish_page_nodes(vec![kern]);
    let hbox = Node::HList(BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(Scaled::UNITY),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
        shift: Scaled::from_raw(0),
        box_lr: tex_state::node::BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children,
    }));
    context.publish_page_nodes(vec![hbox])
}

#[test]
fn node_dump_covers_leader_kern_math_penalty_and_adjustment_rows() {
    with_context(|context| {
        let leader_glue = GlueSpec {
            width: Scaled::from_raw(2 * Scaled::UNITY),
            ..GlueSpec::ZERO
        };
        let empty = PageListId::empty();
        let leader = LeaderPayload::HList(zero_sized_hbox(empty.clone()));
        let adjustment = context.publish_page_nodes(vec![
            Node::Kern {
                amount: Scaled::from_raw(Scaled::UNITY),
                kind: KernKind::Explicit,
            },
            Node::Penalty(10000),
        ]);
        let nodes = [
            Node::Glue {
                spec: leader_glue,
                kind: GlueKind::Cleaders,
                leader: Some(leader.clone()),
            },
            Node::Glue {
                spec: leader_glue,
                kind: GlueKind::Xleaders,
                leader: Some(leader),
            },
            Node::Kern {
                amount: Scaled::from_raw(Scaled::UNITY),
                kind: KernKind::Explicit,
            },
            Node::Kern {
                amount: Scaled::from_raw(2 * Scaled::UNITY),
                kind: KernKind::Font,
            },
            Node::Kern {
                amount: Scaled::from_raw(3 * Scaled::UNITY),
                kind: KernKind::Mu,
            },
            Node::Kern {
                amount: Scaled::from_raw(4 * Scaled::UNITY),
                kind: KernKind::Accent,
            },
            Node::MathOn(Scaled::from_raw(0)),
            Node::MathOff(Scaled::from_raw(0)),
            Node::MathOn(Scaled::from_raw(3 * Scaled::UNITY)),
            Node::MathOff(Scaled::from_raw(-3 * Scaled::UNITY)),
            Node::Penalty(-10000),
            Node::Adjust(AdjustNode::ordinary(adjustment.clone())),
        ];

        assert_eq!(
            dump_node_slice(
                &context,
                &nodes,
                DumpConfig {
                    breadth: 100,
                    depth: 100,
                    profile: tex_command::CommandProfile::TEX82,
                },
            ),
            concat!(
                "\\cleaders 2.0\n",
                ".\\hbox(0.0+0.0)x0.0\n",
                "\\xleaders 2.0\n",
                ".\\hbox(0.0+0.0)x0.0\n",
                "\\kern 1.0\n",
                "\\kern2.0\n",
                "\\mkern3.0mu\n",
                "\\kern 4.0 (for accent)\n",
                "\\mathon\n",
                "\\mathoff\n",
                "\\mathon, surrounded 3.0\n",
                "\\mathoff, surrounded -3.0\n",
                "\\penalty -10000\n",
                "\\vadjust\n",
                ".\\kern 1.0\n",
                ".\\penalty 10000\n",
            ),
        );

        assert_eq!(leader_glue.width, Scaled::from_raw(2 * Scaled::UNITY));
        assert!(empty.is_empty());
        assert!(matches!(
            page_vec(&context, adjustment).as_slice(),
            [Node::Kern { .. }, Node::Penalty(10000)]
        ));
    });
}

#[test]
fn kern_subtype_dump_matrix_preserves_canonical_spacing_and_annotations() {
    // TeX82 §184 inserts a space after `\kern` for every non-normal
    // subtype. pdfTeX's added automatic and margin forms retain their own
    // annotations; a normal font/italic kern remains the sole bare form.
    with_context(|context| {
        let font = context.current_font();
        let nodes = [
            Node::Kern {
                amount: Scaled::from_raw(Scaled::UNITY),
                kind: KernKind::Font,
            },
            Node::Kern {
                amount: Scaled::from_raw(2 * Scaled::UNITY),
                kind: KernKind::Explicit,
            },
            Node::Kern {
                amount: Scaled::from_raw(3 * Scaled::UNITY),
                kind: KernKind::Accent,
            },
            Node::Kern {
                amount: Scaled::from_raw(4 * Scaled::UNITY),
                kind: KernKind::Mu,
            },
            Node::Kern {
                amount: Scaled::from_raw(5 * Scaled::UNITY),
                kind: KernKind::Auto,
            },
            Node::Kern {
                amount: Scaled::from_raw(6 * Scaled::UNITY),
                kind: KernKind::LeftMargin,
            },
            Node::Kern {
                amount: Scaled::from_raw(7 * Scaled::UNITY),
                kind: KernKind::RightMargin,
            },
            Node::MarginKern {
                amount: Scaled::from_raw(8 * Scaled::UNITY),
                side: MarginKernSide::Left,
                font,
                ch: b'A',
            },
            Node::MarginKern {
                amount: Scaled::from_raw(9 * Scaled::UNITY),
                side: MarginKernSide::Right,
                font,
                ch: b'A',
            },
        ];

        assert_eq!(
            dump_node_slice(
                &context,
                &nodes,
                DumpConfig {
                    breadth: 100,
                    depth: 100,
                    profile: tex_command::CommandProfile::PDFTEX14029,
                },
            ),
            concat!(
                "\\kern1.0\n",
                "\\kern 2.0\n",
                "\\kern 3.0 (for accent)\n",
                "\\mkern4.0mu\n",
                "\\kern 5.0 (for \\pdfprependkern/\\pdfappendkern)\n",
                "\\kern6.0 (left margin)\n",
                "\\kern7.0 (right margin)\n",
                "\\kern8.0 (left margin)\n",
                "\\kern9.0 (right margin)\n",
            ),
        );
    });
}

#[test]
fn glue_subtype_dump_matrix_preserves_canonical_subtype_units() {
    with_context(|context| {
        let spec = GlueSpec {
            width: Scaled::from_raw(Scaled::UNITY),
            ..GlueSpec::ZERO
        };
        let empty = PageListId::empty();
        let leader = LeaderPayload::HList(zero_sized_hbox(empty.clone()));
        let cases = [
            (GlueKind::Normal, None, "\\glue 1.0\n"),
            (GlueKind::SpaceSkip, None, "\\glue(\\spaceskip) 1.0\n"),
            (GlueKind::XSpaceSkip, None, "\\glue(\\xspaceskip) 1.0\n"),
            (GlueKind::TabSkip, None, "\\glue(\\tabskip) 1.0\n"),
            (GlueKind::BaselineSkip, None, "\\glue(\\baselineskip) 1.0\n"),
            (GlueKind::LineSkip, None, "\\glue(\\lineskip) 1.0\n"),
            (GlueKind::TopSkip, None, "\\glue(\\topskip) 1.0\n"),
            (GlueKind::SplitTopSkip, None, "\\glue(\\splittopskip) 1.0\n"),
            (GlueKind::LeftSkip, None, "\\glue(\\leftskip) 1.0\n"),
            (GlueKind::RightSkip, None, "\\glue(\\rightskip) 1.0\n"),
            (GlueKind::ParSkip, None, "\\glue(\\parskip) 1.0\n"),
            (GlueKind::ParFillSkip, None, "\\glue(\\parfillskip) 1.0\n"),
            (
                GlueKind::AboveDisplaySkip,
                None,
                "\\glue(\\abovedisplayskip) 1.0\n",
            ),
            (
                GlueKind::BelowDisplaySkip,
                None,
                "\\glue(\\belowdisplayskip) 1.0\n",
            ),
            (
                GlueKind::AboveDisplayShortSkip,
                None,
                "\\glue(\\abovedisplayshortskip) 1.0\n",
            ),
            (
                GlueKind::BelowDisplayShortSkip,
                None,
                "\\glue(\\belowdisplayshortskip) 1.0\n",
            ),
            (
                GlueKind::Leaders,
                Some(leader.clone()),
                "\\leaders 1.0\n.\\hbox(0.0+0.0)x0.0\n",
            ),
            (
                GlueKind::Cleaders,
                Some(leader.clone()),
                "\\cleaders 1.0\n.\\hbox(0.0+0.0)x0.0\n",
            ),
            (
                GlueKind::Xleaders,
                Some(leader),
                "\\xleaders 1.0\n.\\hbox(0.0+0.0)x0.0\n",
            ),
            (GlueKind::MuSkip, None, "\\glue(\\mskip) 1.0mu\n"),
            (GlueKind::ThinMuSkip, None, "\\glue(\\thinmuskip) 1.0\n"),
            (GlueKind::MedMuSkip, None, "\\glue(\\medmuskip) 1.0\n"),
            (GlueKind::ThickMuSkip, None, "\\glue(\\thickmuskip) 1.0\n"),
            (GlueKind::NonScript, None, "\\glue(\\nonscript)\n"),
        ];

        for (kind, payload, expected) in cases {
            let node = Node::Glue {
                spec,
                kind,
                leader: payload,
            };
            assert_eq!(
                dump_node_slice(
                    &context,
                    &[node],
                    DumpConfig {
                        breadth: 10,
                        depth: 10,
                        profile: tex_command::CommandProfile::TEX82,
                    }
                ),
                expected,
                "wrong dump for {kind:?}",
            );
            assert_eq!(spec.width, Scaled::from_raw(Scaled::UNITY));
            assert!(empty.is_empty());
        }
    });
}

#[test]
fn zero_glue_dump_distinguishes_nonscript_sentinel_from_printed_specs() {
    with_context(|context| {
        let zero = GlueSpec::ZERO;
        let empty = PageListId::empty();
        let leader = LeaderPayload::HList(zero_sized_hbox(empty));
        let cases = [
            (GlueKind::NonScript, None, "\\glue(\\nonscript)\n"),
            (GlueKind::Normal, None, "\\glue 0.0\n"),
            (GlueKind::MuSkip, None, "\\glue(\\mskip) 0.0mu\n"),
            (
                GlueKind::Leaders,
                Some(leader),
                "\\leaders 0.0\n.\\hbox(0.0+0.0)x0.0\n",
            ),
        ];

        for (kind, payload, expected) in cases {
            assert_eq!(
                dump_node_slice(
                    &context,
                    &[Node::Glue {
                        spec: zero,
                        kind,
                        leader: payload,
                    }],
                    DumpConfig {
                        breadth: 10,
                        depth: 10,
                        profile: tex_command::CommandProfile::TEX82,
                    },
                ),
                expected,
                "wrong zero-glue dump for {kind:?}",
            );
        }

        assert_eq!(
            dump_node_slice(
                &context,
                &[
                    Node::Glue {
                        spec: zero,
                        kind: GlueKind::NonScript,
                        leader: None,
                    },
                    Node::Kern {
                        amount: Scaled::from_raw(Scaled::UNITY),
                        kind: KernKind::Explicit,
                    },
                ],
                DumpConfig {
                    breadth: 10,
                    depth: 10,
                    profile: tex_command::CommandProfile::TEX82,
                },
            ),
            "\\glue(\\nonscript)\n\\kern 1.0\n",
        );
    });
}

#[test]
fn glue_unit_order_and_sign_matrix_is_exact_and_immutable() {
    with_context(|context| {
        let cases = [
            (
                Order::Normal,
                Order::Filll,
                "2.0 plus -3.0 minus 4.0filll",
                "2.0mu plus -3.0mu minus 4.0filll",
            ),
            (
                Order::Fil,
                Order::Fill,
                "2.0 plus -3.0fil minus 4.0fill",
                "2.0mu plus -3.0fil minus 4.0fill",
            ),
            (
                Order::Fill,
                Order::Fil,
                "2.0 plus -3.0fill minus 4.0fil",
                "2.0mu plus -3.0fill minus 4.0fil",
            ),
            (
                Order::Filll,
                Order::Normal,
                "2.0 plus -3.0filll minus 4.0",
                "2.0mu plus -3.0filll minus 4.0mu",
            ),
        ];
        for (stretch_order, shrink_order, ordinary, math) in cases {
            let value = GlueSpec {
                width: Scaled::from_raw(2 * Scaled::UNITY),
                stretch: Scaled::from_raw(-3 * Scaled::UNITY),
                stretch_order,
                shrink: Scaled::from_raw(4 * Scaled::UNITY),
                shrink_order,
            };
            let spec = value;
            for (kind, expected) in [(GlueKind::Normal, ordinary), (GlueKind::MuSkip, math)] {
                let node = Node::Glue {
                    spec,
                    kind,
                    leader: None,
                };
                let prefix = if kind == GlueKind::MuSkip {
                    "\\glue(\\mskip) "
                } else {
                    "\\glue "
                };
                assert_eq!(
                    dump_node_slice(
                        &context,
                        &[node],
                        DumpConfig {
                            breadth: 10,
                            depth: 10,
                            profile: tex_command::CommandProfile::TEX82,
                        }
                    ),
                    format!("{prefix}{expected}\n"),
                );
                assert_eq!(spec, value, "dumping must not rewrite glue");
            }
        }

        let signs = [
            (
                2,
                3,
                4,
                "2.0 plus 3.0 minus 4.0",
                "2.0mu plus 3.0mu minus 4.0mu",
            ),
            (
                2,
                3,
                -4,
                "2.0 plus 3.0 minus -4.0",
                "2.0mu plus 3.0mu minus -4.0mu",
            ),
            (
                2,
                -3,
                4,
                "2.0 plus -3.0 minus 4.0",
                "2.0mu plus -3.0mu minus 4.0mu",
            ),
            (
                2,
                -3,
                -4,
                "2.0 plus -3.0 minus -4.0",
                "2.0mu plus -3.0mu minus -4.0mu",
            ),
            (
                -2,
                3,
                4,
                "-2.0 plus 3.0 minus 4.0",
                "-2.0mu plus 3.0mu minus 4.0mu",
            ),
            (
                -2,
                3,
                -4,
                "-2.0 plus 3.0 minus -4.0",
                "-2.0mu plus 3.0mu minus -4.0mu",
            ),
            (
                -2,
                -3,
                4,
                "-2.0 plus -3.0 minus 4.0",
                "-2.0mu plus -3.0mu minus 4.0mu",
            ),
            (
                -2,
                -3,
                -4,
                "-2.0 plus -3.0 minus -4.0",
                "-2.0mu plus -3.0mu minus -4.0mu",
            ),
        ];
        for (width, stretch, shrink, ordinary, math) in signs {
            let value = GlueSpec {
                width: Scaled::from_raw(width * Scaled::UNITY),
                stretch: Scaled::from_raw(stretch * Scaled::UNITY),
                stretch_order: Order::Normal,
                shrink: Scaled::from_raw(shrink * Scaled::UNITY),
                shrink_order: Order::Normal,
            };
            assert_eq!(format_glue(value, ""), ordinary);
            assert_eq!(format_glue(value, "mu"), math);
        }
    });
}

#[test]
fn box_lr_projects_profile_specific_canonical_node_dump_evidence() {
    with_context(|context| {
        let empty = PageListId::empty();
        for (box_lr, suffix) in [
            (tex_state::node::BoxLr::Normal, ""),
            (tex_state::node::BoxLr::Reversed, ", reversed"),
            // TeX82 §184 has no display-list box subtype.
            (tex_state::node::BoxLr::DList, ""),
        ] {
            let list = context.publish_page_nodes(vec![Node::HList(BoxNode::new(BoxNodeFields {
                width: Scaled::from_raw(0),
                height: Scaled::from_raw(0),
                depth: Scaled::from_raw(0),
                shift: Scaled::from_raw(0),
                box_lr,
                glue_set: GlueSetRatio::ZERO,
                glue_sign: Sign::Normal,
                glue_order: Order::Normal,
                children: empty.clone(),
            }))]);
            let config = DumpConfig::read(&context);
            assert_eq!(
                dump_page_list(&context, list, config),
                format!("\\hbox(0.0+0.0)x0.0{suffix}\n"),
            );
        }

        let display = context.publish_page_nodes(vec![Node::HList(BoxNode::new(BoxNodeFields {
            width: Scaled::from_raw(0),
            height: Scaled::from_raw(0),
            depth: Scaled::from_raw(0),
            shift: Scaled::from_raw(0),
            box_lr: tex_state::node::BoxLr::DList,
            glue_set: GlueSetRatio::ZERO,
            glue_sign: Sign::Normal,
            glue_order: Order::Normal,
            children: empty,
        }))]);
        let config = DumpConfig::read(&context).for_profile(tex_command::CommandProfile::ETEX26);
        assert_eq!(
            dump_page_list(&context, display, config),
            "\\hbox(0.0+0.0)x0.0, display\n",
        );
    });
}

#[test]
fn shifted_display_box_and_parametric_glue_project_independently() {
    // TeX82 §184 prints the shift but no internal display-list marker;
    // §189 still names neighboring glue from its parameter subtype.
    with_context(|context| {
        let empty = PageListId::empty();
        let baseline = GlueSpec {
            width: Scaled::from_raw(10 * Scaled::UNITY),
            stretch: Scaled::from_raw(41 * Scaled::UNITY),
            ..GlueSpec::ZERO
        };
        let list = context.publish_page_nodes(vec![
            Node::HList(BoxNode::new(BoxNodeFields {
                width: Scaled::from_raw(0),
                height: Scaled::from_raw(0),
                depth: Scaled::from_raw(0),
                shift: Scaled::from_raw(50 * Scaled::UNITY),
                box_lr: tex_state::node::BoxLr::DList,
                glue_set: GlueSetRatio::ZERO,
                glue_sign: Sign::Normal,
                glue_order: Order::Normal,
                children: empty,
            })),
            Node::Glue {
                spec: baseline,
                kind: GlueKind::BaselineSkip,
                leader: None,
            },
        ]);

        let config = DumpConfig::read(&context);
        assert_eq!(
            dump_page_list(&context, list, config),
            "\\hbox(0.0+0.0)x0.0, shifted 50.0\n\\glue(\\baselineskip) 10.0 plus 41.0\n",
        );
    });
}

#[test]
fn stretching_box_dump_preserves_negative_glue_set_ratio() {
    // tex.web §186 prints the signed `glue_set` value independently of
    // `glue_sign`; negative stretch totals can produce this combination.
    with_context(|context| {
        let empty = PageListId::empty();
        let list = context.publish_page_nodes(vec![Node::HList(BoxNode::new(BoxNodeFields {
            width: Scaled::from_raw(0),
            height: Scaled::from_raw(0),
            depth: Scaled::from_raw(0),
            shift: Scaled::from_raw(0),
            box_lr: tex_state::node::BoxLr::Normal,
            glue_set: GlueSetRatio::from_ratio_parts(-2, 1),
            glue_sign: Sign::Stretching,
            glue_order: Order::Normal,
            children: empty,
        }))]);

        let config = DumpConfig::read(&context);
        assert_eq!(
            dump_page_list(&context, list, config),
            "\\hbox(0.0+0.0)x0.0, glue set -2.0\n"
        );
    });
}

/// bd `umber2-alfh.6`: with `\showboxbreadth`/`\showboxdepth` left at
/// INITEX's default of 0 (tex.web §240's `eqtb` zeroing loop), `\showbox`
/// printed `etc.` for a one-item box instead of the box line, because
/// `dump_nodes` used `show_box_breadth` directly as the per-level item limit.
/// TeX82 §198's `show_box` clamps a non-positive `breadth_max` to 5 before
/// `show_node_list` (§182) ever sees it; a limit of 0 is not "show nothing",
/// it is the sentinel for "the parameter was never set."
#[test]
fn default_show_box_breadth_renders_top_level_box_instead_of_etc() {
    with_context(|context| {
        assert_eq!(context.int_param(IntParam::SHOW_BOX_BREADTH), 0);
        assert_eq!(context.int_param(IntParam::SHOW_BOX_DEPTH), 0);

        let list = hbox_with_one_point_kern(context);
        let config = DumpConfig::read(&context);
        let text = dump_page_list(&context, list, config);

        // Real pdftex 1.40.29 writes exactly this line for `\showbox0` after
        // `\setbox0=\hbox{\kern1pt}` (confirmed against the pinned oracle).
        assert_eq!(text, "\\hbox(0.0+0.0)x1.0 []\n");
    });
}

/// A user-set non-positive `\showboxbreadth` gets the same §198 fallback to
/// 5 as the untouched default, not just the value `0` INITEX happens to
/// leave behind.
#[test]
fn negative_show_box_breadth_also_falls_back_to_five() {
    with_context(|context| {
        assign_int(context, IntParam::SHOW_BOX_BREADTH, -3);
        let list = hbox_with_one_point_kern(context);
        let config = DumpConfig::read(&context);
        assert_eq!(config.breadth, 5);
        let text = dump_page_list(&context, list, config);
        assert_eq!(text, "\\hbox(0.0+0.0)x1.0 []\n");
    });
}

/// An explicit, still-positive breadth smaller than the item count must
/// keep truncating with `etc.`, per §182's `incr(n); if n>breadth_max then
/// ... print("etc.")`: the §198 fallback only replaces a non-positive
/// value, it does not disable truncation altogether.
#[test]
fn explicit_positive_breadth_still_truncates_with_etc() {
    with_context(|context| {
        let kern = |amount| Node::Kern {
            amount: Scaled::from_raw(amount),
            kind: KernKind::Explicit,
        };
        let list = context.publish_page_nodes(vec![
            kern(Scaled::UNITY),
            kern(2 * Scaled::UNITY),
            kern(3 * Scaled::UNITY),
        ]);
        let config = DumpConfig {
            breadth: 2,
            depth: 0,
            profile: tex_command::CommandProfile::TEX82,
        };
        let text = dump_page_list(&context, list, config);
        assert_eq!(text, "\\kern 1.0\n\\kern 2.0\netc.\n");
    });
}

/// TeX82 §186 (`tex.web:3740-3752`) decodes an unset node's quarterword
/// span count as one less than its displayed column count. A zero field has no
/// suffix; nonzero fields use the exact ` (N columns)` spelling before the
/// independently ordered stretch and shrink components.
#[test]
fn unset_box_prints_encoded_column_count_and_glue_fields() {
    with_context(|context| {
        let children = context.publish_page_nodes(vec![Node::Kern {
            amount: Scaled::from_raw(Scaled::UNITY),
            kind: KernKind::Explicit,
        }]);
        let unset = |span_count| {
            Node::Unset(UnsetNode::new(UnsetNodeFields {
                kind: UnsetKind::HBox,
                width: Scaled::from_raw(4 * Scaled::UNITY),
                height: Scaled::from_raw(5 * Scaled::UNITY),
                depth: Scaled::from_raw(6 * Scaled::UNITY),
                span_count,
                stretch: Scaled::from_raw(2 * Scaled::UNITY),
                stretch_order: Order::Fil,
                shrink: Scaled::from_raw(3 * Scaled::UNITY),
                shrink_order: Order::Normal,
                children: children.clone(),
            }))
        };
        let source = [unset(0), unset(1), unset(2)];
        let list = context.publish_page_nodes(source.to_vec());
        let before_source = source.clone();
        let before_children = page_vec(&context, children);

        assert_eq!(
            dump_page_list(
                &context,
                list.clone(),
                DumpConfig {
                    breadth: 100,
                    depth: 100,
                    profile: tex_command::CommandProfile::TEX82,
                },
            ),
            concat!(
                "\\unsetbox(5.0+6.0)x4.0, stretch 2.0fil, shrink 3.0\n",
                ".\\kern 1.0\n",
                "\\unsetbox(5.0+6.0)x4.0 (2 columns), stretch 2.0fil, shrink 3.0\n",
                ".\\kern 1.0\n",
                "\\unsetbox(5.0+6.0)x4.0 (3 columns), stretch 2.0fil, shrink 3.0\n",
                ".\\kern 1.0\n",
            ),
        );
        assert_eq!(page_vec(&context, list), before_source);
        assert_eq!(page_vec(&context, children), before_children);

        assert_eq!(
            dump_node_slice(
                &context,
                &source[1..2],
                DumpConfig {
                    breadth: 100,
                    depth: 0,
                    profile: tex_command::CommandProfile::TEX82,
                },
            ),
            "\\unsetbox(5.0+6.0)x4.0 (2 columns), stretch 2.0fil, shrink 3.0 []\n",
        );
        assert_eq!(page_vec(&context, list), before_source);
        assert_eq!(page_vec(&context, children), before_children);
    });
}

fn zero_sized_hbox(children: PageListId) -> BoxNode {
    BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(0),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
        shift: Scaled::from_raw(0),
        box_lr: tex_state::node::BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children,
    })
}

/// TeX82 §198 initializes `show_node_list` at depth -1. Thus depth zero
/// still prints the selected box itself, depth one admits exactly its child
/// box, and depth two admits that box's child. A negative threshold suppresses
/// the entire traversal. The ` []` marker records a nonempty hidden child list.
#[test]
fn showbox_depth_limits_nested_boxes_at_exact_thresholds() {
    with_context(|context| {
        let kern = Node::Kern {
            amount: Scaled::from_raw(Scaled::UNITY),
            kind: KernKind::Explicit,
        };
        let inner_children = context.publish_page_nodes(vec![kern.clone()]);
        let inner = Node::HList(zero_sized_hbox(inner_children.clone()));
        let outer_children = context.publish_page_nodes(vec![inner.clone()]);
        let outer = Node::HList(zero_sized_hbox(outer_children.clone()));
        let root = context.publish_page_nodes(vec![outer.clone()]);

        let before_root = vec![outer];
        let before_outer = vec![inner];
        let before_inner = vec![kern];
        let render = |depth| {
            dump_page_list(
                &context,
                root.clone(),
                DumpConfig {
                    breadth: 5,
                    depth,
                    profile: tex_command::CommandProfile::TEX82,
                },
            )
        };

        assert_eq!(render(-1), "");
        assert_eq!(render(0), "\\hbox(0.0+0.0)x0.0 []\n");
        assert_eq!(render(1), "\\hbox(0.0+0.0)x0.0\n.\\hbox(0.0+0.0)x0.0 []\n");
        assert_eq!(
            render(2),
            concat!(
                "\\hbox(0.0+0.0)x0.0\n",
                ".\\hbox(0.0+0.0)x0.0\n",
                "..\\kern 1.0\n",
            )
        );
        assert_eq!(page_vec(&context, root), before_root);
        assert_eq!(page_vec(&context, outer_children), before_outer);
        assert_eq!(page_vec(&context, inner_children), before_inner);
    });
}

/// Subsidiary lists use the same exact depth and breadth accounting as box
/// children. This covers the non-box edges most prone to being accidentally
/// traversed with an independent limit: adjustments, leader payload boxes,
/// and discretionary pre/post/replacement lists.
#[test]
fn showbox_limits_side_lists_leaders_and_discretionaries_without_mutation() {
    with_context(|context| {
        let kern = |points| Node::Kern {
            amount: Scaled::from_raw(points * Scaled::UNITY),
            kind: KernKind::Explicit,
        };
        let two_kerns = context.publish_page_nodes(vec![kern(1), kern(2)]);
        let empty = PageListId::empty();
        let glue = tex_state::glue::GlueSpec::ZERO;

        let adjust = Node::Adjust(AdjustNode::ordinary(two_kerns.clone()));
        let leader = Node::Glue {
            spec: glue,
            kind: GlueKind::Leaders,
            leader: Some(LeaderPayload::HList(zero_sized_hbox(two_kerns.clone()))),
        };
        let disc = Node::Disc {
            kind: DiscKind::Discretionary,
            pre: two_kerns.clone(),
            post: two_kerns.clone(),
            replace: two_kerns.clone(),
            physical_replace_count: 2,
        };
        let source = [adjust, leader, disc];
        let before = page_vec(&context, two_kerns);
        let render = |node: &Node| {
            dump_node_slice(
                &context,
                std::slice::from_ref(node),
                DumpConfig {
                    breadth: 1,
                    depth: 10,
                    profile: tex_command::CommandProfile::TEX82,
                },
            )
        };

        assert_eq!(render(&source[0]), "\\vadjust\n.\\kern 1.0\n.etc.\n");
        assert_eq!(
            render(&source[1]),
            concat!(
                "\\leaders 0.0\n",
                ".\\hbox(0.0+0.0)x0.0\n",
                "..\\kern 1.0\n",
                "..etc.\n",
            )
        );
        assert_eq!(
            render(&source[2]),
            concat!(
                "\\discretionary replacing 2\n",
                ".\\kern 1.0\n",
                ".etc.\n",
                "|\\kern 1.0\n",
                "|etc.\n",
            )
        );
        assert_eq!(
            page_vec(&context, two_kerns),
            before,
            "dumping must be read-only"
        );
        assert!(empty.is_empty());
    });
}

#[test]
fn discretionary_dump_suppresses_replacement_and_marks_post_break() {
    with_context(|context| {
        let kern = |points| Node::Kern {
            amount: Scaled::from_raw(points * Scaled::UNITY),
            kind: KernKind::Explicit,
        };
        let pre = context.publish_page_nodes(vec![kern(1)]);
        let post = context.publish_page_nodes(vec![kern(2), kern(4)]);
        let replace = context.publish_page_nodes(vec![kern(3)]);
        let disc = Node::Disc {
            kind: DiscKind::Discretionary,
            pre: pre.clone(),
            post: post.clone(),
            replace: replace.clone(),
            physical_replace_count: 1,
        };

        assert_eq!(
            dump_node_slice(
                &context,
                std::slice::from_ref(&disc),
                DumpConfig {
                    breadth: 10,
                    depth: 10,
                    profile: tex_command::CommandProfile::TEX82,
                },
            ),
            "\\discretionary replacing 1\n.\\kern 1.0\n|\\kern 2.0\n|\\kern 4.0\n",
        );
        assert_eq!(page_vec(&context, pre), [kern(1)]);
        assert_eq!(page_vec(&context, post), [kern(2), kern(4)]);
        assert_eq!(page_vec(&context, replace), [kern(3)]);
        assert!(matches!(
             disc,
             Node::Disc {
                 pre: actual_pre,
                post: actual_post,
                replace: actual_replace,
                ..
            } if actual_pre == pre && actual_post == post && actual_replace == replace
        ));
    });
}

#[test]
fn discretionary_dump_uses_live_escape_character() {
    with_context(|context| {
        assign_int(context, IntParam::ESCAPE_CHAR, i32::from(b'|'));
        let empty = PageListId::empty();
        let disc = Node::Disc {
            kind: DiscKind::Discretionary,
            pre: empty.clone(),
            post: empty.clone(),
            replace: empty,
            physical_replace_count: 0,
        };

        assert_eq!(
            dump_node_slice(
                &context,
                &[disc],
                DumpConfig {
                    breadth: 10,
                    depth: 10,
                    profile: tex_command::CommandProfile::TEX82,
                },
            ),
            "|discretionary\n",
        );
    });
}

#[test]
fn discretionary_dump_retains_the_physical_replacement_span() {
    with_context(|context| {
        let empty = PageListId::empty();
        let structured_replace = context.publish_page_nodes(vec![Node::Penalty(9)]);
        let nodes = [
            Node::Disc {
                kind: DiscKind::AutomaticHyphen,
                pre: empty.clone(),
                post: empty.clone(),
                replace: structured_replace,
                physical_replace_count: 2,
            },
            Node::Penalty(1),
            Node::Penalty(2),
            Node::Penalty(3),
        ];

        let diagnostic_children = context.publish_page_nodes(nodes.to_vec());
        let mut box_node = zero_sized_hbox(empty);
        box_node.diagnostic_children = Some(diagnostic_children);
        assert_eq!(
            dump_node_slice(
                &context,
                &[Node::HList(box_node)],
                DumpConfig {
                    breadth: 10,
                    depth: 10,
                    profile: tex_command::CommandProfile::TEX82,
                },
            ),
            concat!(
                "\\hbox(0.0+0.0)x0.0\n",
                ".\\discretionary replacing 2\n",
                ".\\penalty 1\n",
                ".\\penalty 2\n",
                ".\\penalty 3\n",
            ),
        );
    });
}

#[test]
fn diagnostic_box_reorders_boundary_disc_and_retains_multi_disc_spans() {
    with_context(|context| {
        let empty = PageListId::empty();
        let pre = context.publish_page_nodes(vec![Node::Penalty(7)]);
        let boundary_kern = Node::Kern {
            amount: Scaled::from_raw(1),
            kind: KernKind::Font,
        };
        let boundary_replace = context.publish_page_nodes(vec![boundary_kern.clone()]);
        let font = context.current_font();
        let ligature = || Node::Lig {
            font,
            ch: 'A',
            orig: vec!['A', 'A'],
            origins: vec![tex_state::token::OriginId::UNKNOWN; 2],
            left_hit: false,
            right_hit: false,
        };
        let ligature_replace = context.publish_page_nodes(vec![ligature()]);
        let physical = context.publish_page_nodes(vec![
            ligature(),
            Node::Disc {
                kind: DiscKind::AutomaticHyphen,
                pre,
                post: empty.clone(),
                replace: boundary_replace,
                physical_replace_count: 2,
            },
            boundary_kern,
            Node::Disc {
                kind: DiscKind::AutomaticHyphen,
                pre: empty.clone(),
                post: empty.clone(),
                replace: ligature_replace,
                physical_replace_count: 3,
            },
            ligature(),
            Node::Penalty(9),
        ]);
        let mut box_node = zero_sized_hbox(empty);
        box_node.diagnostic_children = Some(physical);

        assert_eq!(
            dump_node_slice(
                &context,
                &[Node::HList(box_node)],
                DumpConfig {
                    breadth: 10,
                    depth: 10,
                    profile: tex_command::CommandProfile::TEX82,
                },
            ),
            concat!(
                "\\hbox(0.0+0.0)x0.0\n",
                ".\\discretionary replacing 2\n",
                "..\\penalty 7\n",
                ".\\nullfont A (ligature AA)\n",
                ".\\kern0.00002\n",
                ".\\discretionary replacing 3\n",
                ".\\nullfont A (ligature AA)\n",
                ".\\penalty 9\n",
            ),
        );
    });
}

#[test]
fn etex_mlr_boundaries_dump_with_exact_identity() {
    // Merged e-TeX WEB §12 keeps all six M/L/R math-node subtypes distinct.
    with_context(|context| {
        let list = context.publish_page_nodes(vec![
            Node::Direction(tex_state::node::Direction::BeginM),
            Node::Direction(tex_state::node::Direction::EndM),
            Node::Direction(tex_state::node::Direction::BeginL),
            Node::Direction(tex_state::node::Direction::EndL),
            Node::Direction(tex_state::node::Direction::BeginR),
            Node::Direction(tex_state::node::Direction::EndR),
        ]);
        assert_eq!(
            dump_page_list(
                &context,
                list,
                DumpConfig {
                    breadth: 10,
                    depth: 10,
                    profile: tex_command::CommandProfile::TEX82,
                }
            ),
            "\\beginM\n\\endM\n\\beginL\n\\endL\n\\beginR\n\\endR\n"
        );
    });
}

/// pdfTeX §§190/193 retain insertion and numbered-mark identity in list
/// diagnostics, in node order, without consuming or rewriting either frozen
/// source list. This is the exact body printed for the equivalent
/// `\vbox{\insert3{\hrule height5pt}\marks12{hello}}`.
#[test]
fn pdftex_insertion_and_numbered_mark_dump_exact_identity_in_source_order() {
    with_plain_context(|context| {
        let zero = tex_state::glue::GlueSpec::ZERO;
        let insertion_content = context.publish_page_nodes(vec![Node::Rule {
            width: None,
            height: Some(Scaled::from_raw(5 * Scaled::UNITY)),
            depth: Some(Scaled::from_raw(0)),
        }]);
        let mark_tokens = node_tokens([
            tex_state::token::Token::Char {
                ch: 'h',
                cat: tex_state::token::Catcode::Letter,
            },
            tex_state::token::Token::Char {
                ch: 'i',
                cat: tex_state::token::Catcode::Letter,
            },
        ]);
        let nodes = [
            Node::Ins {
                class: 3,
                size: Scaled::from_raw(5 * Scaled::UNITY),
                split_top_skip: zero,
                split_max_depth: Scaled::MAX_DIMEN,
                floating_penalty: 17,
                content: insertion_content,
            },
            Node::Mark {
                class: 12,
                tokens: mark_tokens,
            },
        ];
        let source = context.publish_page_nodes(nodes.to_vec());

        let expected = concat!(
            "\\insert3, natural size 5.0; split(0.0,16383.99998); float cost 17\n",
            ".\\rule(5.0+0.0)x*\n",
            "\\marks12{hi}\n",
        );
        let config = DumpConfig {
            breadth: 100,
            depth: 100,
            profile: tex_command::CommandProfile::TEX82,
        };
        assert_eq!(dump_page_list(&context, source.clone(), config), expected);
        assert_eq!(page_vec(&context, source), nodes, "dumping is immutable");
    });
}

/// TeX82 §200 prints a class-zero mark as `\\mark{<tokens>}`. Rendering is
/// observational: it neither consumes the frozen token list nor exposes the
/// e-TeX class field or any arena bookkeeping.
#[test]
fn mark_dump_prints_token_list_once() {
    with_plain_context(|context| {
        let foo = context.intern_relaxed_control_sequence("foo");
        let literal = node_tokens([tex_state::token::Token::Char {
            ch: 'A',
            cat: tex_state::token::Catcode::Letter,
        }]);
        let control_sequence = node_tokens([tex_state::token::Token::Cs(foo)]);
        let empty = node_tokens([]);
        let literal_before = literal.words().to_vec();
        let control_sequence_before = control_sequence.words().to_vec();
        let nodes = [
            Node::Mark {
                class: 0,
                tokens: literal.clone(),
            },
            Node::Mark {
                class: 0,
                tokens: control_sequence.clone(),
            },
            Node::Mark {
                class: 0,
                tokens: empty.clone(),
            },
        ];
        let source = context.publish_page_nodes(nodes.to_vec());

        assert_eq!(
            dump_page_list(
                &context,
                source.clone(),
                DumpConfig {
                    breadth: 100,
                    depth: 100,
                    profile: tex_command::CommandProfile::TEX82,
                },
            ),
            "\\mark{A}\n\\mark{\\foo}\n\\mark{}\n"
        );
        assert_eq!(page_vec(&context, source), nodes);
        let [
            Node::Mark {
                tokens: literal, ..
            },
            Node::Mark {
                tokens: control_sequence,
                ..
            },
            Node::Mark { tokens: empty, .. },
        ] = &nodes
        else {
            panic!("mark fixture changed shape")
        };
        assert_eq!(literal.words(), literal_before);
        assert_eq!(control_sequence.words(), control_sequence_before);
        assert!(empty.is_empty());
    });
}

/// TeX82 §193's insertion-node arm prints every symbolic field before
/// recursively displaying the insertion list. Formatting is observational:
/// neither the insertion payload nor its child order may be consumed or
/// rewritten while `show_node_list` walks the frozen lists.
#[test]
fn insertion_node_dump_prints_all_web_fields() {
    with_context(|context| {
        let split_top_skip = tex_state::glue::GlueSpec::ZERO;
        let children = [
            Node::Rule {
                width: Some(Scaled::from_raw(0)),
                height: Some(Scaled::from_raw(5 * Scaled::UNITY)),
                depth: Some(Scaled::from_raw(0)),
            },
            Node::Penalty(23),
        ];
        let content = context.publish_page_nodes(children.to_vec());
        let insertion = Node::Ins {
            class: 7,
            size: Scaled::from_raw(5 * Scaled::UNITY),
            split_top_skip,
            split_max_depth: Scaled::from_raw(4 * Scaled::UNITY),
            floating_penalty: 100,
            content: content.clone(),
        };
        let source = context.publish_page_nodes(vec![insertion.clone()]);

        assert_eq!(
            dump_page_list(
                &context,
                source.clone(),
                DumpConfig {
                    breadth: 100,
                    depth: 100,
                    profile: tex_command::CommandProfile::TEX82,
                },
            ),
            concat!(
                "\\insert7, natural size 5.0; split(0.0,4.0); float cost 100\n",
                ".\\rule(5.0+0.0)x0.0\n",
                ".\\penalty 23\n",
            ),
        );
        let source_after = page_vec(&context, source);
        assert_eq!(source_after, std::slice::from_ref(&insertion));
        assert_eq!(page_vec(&context, content), children);
        let [
            Node::Ins {
                content: attached_content,
                ..
            },
        ] = source_after.as_slice()
        else {
            panic!("insertion payload was detached");
        };
        assert_eq!(attached_content, &content);
    });
}

/// e-TeX `etex.web` change blocks 21 and 49 extend TeX82's mark node with a
/// class, while merged block 77 prints class zero in the legacy `\mark` form
/// and every sparse class as `\marks<n>`. The 255/256 boundary must therefore
/// preserve the integer itself, not an arena slot or a generic placeholder.
#[test]
fn etex_numbered_mark_dump_renders_dense_and_sparse_boundary_classes_exactly() {
    with_plain_context(|context| {
        let tokens = node_tokens([tex_state::token::Token::Char {
            ch: 'x',
            cat: tex_state::token::Catcode::Letter,
        }]);
        let list = context.publish_page_nodes(vec![
            Node::Mark {
                class: 0,
                tokens: tokens.clone(),
            },
            Node::Mark {
                class: 255,
                tokens: tokens.clone(),
            },
            Node::Mark { class: 256, tokens },
        ]);

        assert_eq!(
            dump_page_list(
                &context,
                list,
                DumpConfig {
                    breadth: 100,
                    depth: 100,
                    profile: tex_command::CommandProfile::TEX82,
                },
            ),
            "\\mark{x}\n\\marks255{x}\n\\marks256{x}\n"
        );
    });
}

fn math_char(family: u8, character: char) -> MathChar {
    MathChar {
        family,
        character,
        origin: Default::default(),
    }
}

/// TeX82 §§691--697 (`tex.web:13623-13751`) assign a distinct exact
/// display to every noad kind and to the five math-field forms. Empty fields
/// must remain silent; the final noad exercises math-char, math-text-char,
/// sub-box, and sub-mlist fields in their nucleus/script positions.
#[test]
fn showlists_renders_all_math_noad_variants_and_empty_fields() {
    with_context(|context| {
        let sub_box = context.publish_page_nodes(vec![Node::Kern {
            amount: Scaled::from_raw(Scaled::UNITY),
            kind: KernKind::Explicit,
        }]);
        let sub_mlist = context.publish_page_nodes(vec![Node::MathNoad(MathNoad::new(
            NoadKind::Normal(NoadClass::Ord),
            MathField::MathChar(math_char(3, 'm')),
        ))]);
        let mut nodes = Vec::new();
        for class in [
            NoadClass::Ord,
            NoadClass::Op,
            NoadClass::Bin,
            NoadClass::Rel,
            NoadClass::Open,
            NoadClass::Close,
            NoadClass::Punct,
            NoadClass::Inner,
        ] {
            nodes.push(Node::MathNoad(MathNoad::new(
                NoadKind::Normal(class),
                MathField::Empty,
            )));
        }
        for limits in [
            LimitType::DisplayLimits,
            LimitType::Limits,
            LimitType::NoLimits,
        ] {
            nodes.push(Node::MathNoad(MathNoad::new(
                NoadKind::Operator(limits),
                MathField::Empty,
            )));
        }
        for kind in [
            NoadKind::Radical { delimiter: 0x12345 },
            NoadKind::Accent {
                accent: math_char(2, '^'),
            },
            NoadKind::LeftDelimiter { delimiter: 0x28300 },
            NoadKind::RightDelimiter { delimiter: 0x29301 },
            NoadKind::MiddleDelimiter { delimiter: 0x26A00 },
            NoadKind::Underline,
            NoadKind::Overline,
            NoadKind::VCenter,
        ] {
            nodes.push(Node::MathNoad(MathNoad::new(kind, MathField::Empty)));
        }
        nodes.push(Node::MathNoad(MathNoad {
            kind: NoadKind::Normal(NoadClass::Ord),
            nucleus: MathField::MathTextChar(math_char(1, 'x')),
            superscript: MathField::SubBox(sub_box),
            subscript: MathField::SubMlist(sub_mlist),
        }));
        let list = context.publish_page_nodes(nodes.to_vec());

        assert_eq!(
            dump_page_list(
                &context,
                list,
                DumpConfig {
                    breadth: 100,
                    depth: 100,
                    profile: tex_command::CommandProfile::TEX82,
                },
            ),
            concat!(
                "\\mathord\n",
                "\\mathop\n",
                "\\mathbin\n",
                "\\mathrel\n",
                "\\mathopen\n",
                "\\mathclose\n",
                "\\mathpunct\n",
                "\\mathinner\n",
                "\\mathop\n",
                "\\mathop\\limits\n",
                "\\mathop\\nolimits\n",
                "\\radical\"12345\n",
                "\\accent\\fam2 ^\n",
                "\\left\"28300\n",
                "\\right\"29301\n",
                "\\middle\"26A00\n",
                "\\underline\n",
                "\\overline\n",
                "\\vcenter\n",
                "\\mathord\n",
                ".\\fam1 x\n",
                "^\\kern 1.0\n",
                "_\\mathord\n",
                "_.\\fam3 m\n",
            ),
        );
    });
}

/// TeX82 §692 (`tex.web:13639-13668`) prints ` []` for each nonempty
/// subsidiary field hidden by `\showboxdepth`, while an empty field prints no
/// marker. This is the exact `$a_b\showlists` shape at depth zero.
#[test]
fn showlists_depth_cutoff_prints_nonempty_math_field_marker() {
    with_context(|context| {
        let mut noad = MathNoad::new(
            NoadKind::Normal(NoadClass::Ord),
            MathField::MathChar(math_char(1, 'a')),
        );
        noad.subscript = MathField::MathChar(math_char(1, 'b'));
        let list = context.publish_page_nodes(vec![Node::MathNoad(noad)]);

        assert_eq!(
            dump_page_list(
                &context,
                list,
                DumpConfig {
                    breadth: 100,
                    depth: 0,
                    profile: tex_command::CommandProfile::TEX82,
                },
            ),
            "\\mathord [] []\n",
        );
    });
}

/// TeX82 §681 (`tex.web:13366-13426`) gives an empty sub-mlist a nonempty
/// math-field type with a null list pointer. Section 692 prints its subsidiary
/// marker and `{}`, unlike the wholly absent `empty` field.
#[test]
fn math_dump_distinguishes_empty_submlist() {
    with_context(|context| {
        let empty = PageListId::empty();
        let list = context.publish_page_nodes(vec![
            Node::MathNoad(MathNoad::new(
                NoadKind::Normal(NoadClass::Ord),
                MathField::Empty,
            )),
            Node::MathNoad(MathNoad::new(
                NoadKind::Normal(NoadClass::Ord),
                MathField::SubMlist(empty),
            )),
        ]);

        assert_eq!(
            dump_page_list(
                &context,
                list,
                DumpConfig {
                    breadth: 100,
                    depth: 100,
                    profile: tex_command::CommandProfile::TEX82,
                },
            ),
            "\\mathord\n\\mathord\n.{}\n",
        );
    });
}

/// TeX82 §§692/697 preserve the math-field tag independently of its
/// payload pointer: `empty` is silent, math characters print directly, a null
/// `sub_box` recursively prints nothing, and a null `sub_mlist` prints `{}`.
/// Fraction numerator and denominator fields are always the latter kind.
#[test]
fn subsidiary_math_field_matrix_preserves_empty_tags_and_indentation() {
    with_context(|context| {
        let empty = PageListId::empty();
        let child = context.publish_page_nodes(vec![Node::MathNoad(MathNoad::new(
            NoadKind::Normal(NoadClass::Ord),
            MathField::MathChar(math_char(2, 'x')),
        ))]);
        let config = DumpConfig {
            breadth: 100,
            depth: 100,
            profile: tex_command::CommandProfile::TEX82,
        };

        for (field, expected) in [
            (MathField::Empty, ""),
            (MathField::MathChar(math_char(2, 'x')), "...\\fam2 x\n"),
            (MathField::MathTextChar(math_char(2, 'x')), "...\\fam2 x\n"),
            (MathField::SubBox(empty.clone()), ""),
            (
                MathField::SubBox(child.clone()),
                "...\\mathord\n....\\fam2 x\n",
            ),
            (MathField::SubMlist(empty.clone()), "...{}\n"),
            (
                MathField::SubMlist(child.clone()),
                "...\\mathord\n....\\fam2 x\n",
            ),
        ] {
            let mut out = String::new();
            dump_math_field(&context, &field, &config, 2, '.', &mut out);
            assert_eq!(out, expected, "field={field:?}");
        }

        for (numerator, denominator, expected) in [
            (
                empty.clone(),
                child.clone(),
                "\\fraction, thickness = default\n\\{}\n/\\mathord\n/.\\fam2 x\n",
            ),
            (
                child,
                empty,
                "\\fraction, thickness = default\n\\\\mathord\n\\.\\fam2 x\n/{}\n",
            ),
        ] {
            let fraction = MathFraction {
                numerator,
                denominator,
                thickness: FractionThickness::Default,
                left_delimiter: None,
                right_delimiter: None,
            };
            let mut out = String::new();
            dump_fraction(&context, &fraction, &config, -1, &mut out);
            assert_eq!(out, expected);
        }
    });
}

/// TeX82 §§182/692 retain the subsidiary field marker at its own prefix
/// level throughout recursive box/mlist traversal. This is the structural
/// shape exercised by TRIP's hairy display: a superscript box has a nested
/// child, while adjacent noads in the subscript mlist each retain `_`.
#[test]
fn subsidiary_markers_propagate_through_nested_boxes_and_adjacent_noads() {
    with_context(|context| {
        let box_children = context.publish_page_nodes(vec![Node::Kern {
            amount: Scaled::from_raw(Scaled::UNITY),
            kind: KernKind::Explicit,
        }]);
        let sub_box = context.publish_page_nodes(vec![Node::HList(zero_sized_hbox(box_children))]);
        let sub_mlist = context.publish_page_nodes(vec![
            Node::MathNoad(MathNoad::new(
                NoadKind::Normal(NoadClass::Ord),
                MathField::MathChar(math_char(1, 'B')),
            )),
            Node::MathNoad(MathNoad::new(
                NoadKind::Normal(NoadClass::Ord),
                MathField::MathChar(math_char(0, '-')),
            )),
        ]);
        let list = context.publish_page_nodes(vec![Node::MathNoad(MathNoad {
            kind: NoadKind::Normal(NoadClass::Ord),
            nucleus: MathField::Empty,
            superscript: MathField::SubBox(sub_box),
            subscript: MathField::SubMlist(sub_mlist.clone()),
        })]);

        assert_eq!(
            dump_page_list(
                &context,
                list,
                DumpConfig {
                    breadth: 100,
                    depth: 100,
                    profile: tex_command::CommandProfile::TEX82,
                },
            ),
            concat!(
                "\\mathord\n",
                "^\\hbox(0.0+0.0)x0.0\n",
                "^.\\kern 1.0\n",
                "_\\mathord\n",
                "_.\\fam1 B\n",
                "_\\mathord\n",
                "_.\\fam0 -\n",
            ),
        );

        let mut nested = String::new();
        dump_math_field(
            &context,
            &MathField::SubMlist(sub_mlist),
            &DumpConfig {
                breadth: 100,
                depth: 100,
                profile: tex_command::CommandProfile::TEX82,
            },
            2,
            '^',
            &mut nested,
        );
        assert_eq!(
            nested,
            "..^\\mathord\n..^.\\fam1 B\n..^\\mathord\n..^.\\fam0 -\n"
        );
    });
}

/// TeX82 §§689--690 (`tex.web:13581-13622`) visit all four choice arms;
/// §692 applies the same depth cutoff independently within each arm.
#[test]
fn math_dump_depth_and_choice_arms() {
    with_context(|context| {
        let arm = |context: &mut tex_state::CommandContext<'_, _>, ch| {
            context.publish_page_nodes(vec![Node::MathNoad(MathNoad::new(
                NoadKind::Normal(NoadClass::Ord),
                MathField::MathChar(math_char(0, ch)),
            ))])
        };
        let display = arm(&mut *context, 'D');
        let text = arm(&mut *context, 'T');
        let script = arm(&mut *context, 'S');
        let script_script = arm(&mut *context, 's');
        let list = context.publish_page_nodes(vec![Node::MathChoice(MathChoice {
            display,
            text,
            script,
            script_script,
        })]);

        assert_eq!(
            dump_page_list(
                &context,
                list,
                DumpConfig {
                    breadth: 100,
                    depth: 0,
                    profile: tex_command::CommandProfile::TEX82,
                },
            ),
            concat!(
                "\\mathchoice\n",
                "D\\mathord []\n",
                "T\\mathord []\n",
                "S\\mathord []\n",
                "s\\mathord []\n",
            ),
        );
    });
}
