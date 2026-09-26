use tex_state::font::{CharMetrics, CharTag, FontMetrics, LoadedFont, NULL_FONT};
use tex_state::glue::GlueSpec;
use tex_state::ids::FontId;
use tex_state::node::{GlueKind, GlueSpecOrigin, KernKind, Node};
use tex_state::node_view::NodeView;
use tex_state::scaled::Scaled;
use tex_state::token::OriginId;
use tex_state::{AssignmentScope, CommandContext};

use super::preceding_glyph;
use crate::paragraph_end::hyphenation::hyphenated_hlist_with_fuel;
use tex_state::env::banks::IntParam;
use tex_state::hyphenation::ExceptionSpec;

#[test]
fn punctuation_prefix_preserves_ligature_source_for_reconstitution() {
    // TeX82 §914's `init_list` is the two source quotes, not the single
    // displayed quote. The source lets reconstitution find the quote–A kern.
    let quote = Node::Lig {
        font: NULL_FONT,
        ch: '\u{10}',
        orig: vec!['`', '`'],
        origins: vec![OriginId::UNKNOWN; 2],
        left_hit: false,
        right_hit: false,
    };
    let prefix = preceding_glyph(NodeView::from(&quote), NULL_FONT).expect("same-font quote");
    assert_eq!(
        prefix
            .chars
            .iter()
            .map(|entry| entry.ch)
            .collect::<String>(),
        "``"
    );
    assert!(prefix.no_left_boundary);

    let parenthesis = Node::Char {
        font: NULL_FONT,
        ch: '(',
        origin: OriginId::UNKNOWN,
    };
    let prefix = preceding_glyph(NodeView::from(&parenthesis), NULL_FONT).expect("punctuation");
    assert_eq!(
        prefix
            .chars
            .iter()
            .map(|entry| entry.ch)
            .collect::<String>(),
        "("
    );
    assert!(prefix.no_left_boundary);
}

#[test]
fn non_glyph_or_different_font_cannot_seed_reconstitution() {
    let other_font = FontId::testing_new(1);
    let different = Node::Char {
        font: other_font,
        ch: '(',
        origin: OriginId::UNKNOWN,
    };
    assert!(preceding_glyph(NodeView::from(&different), NULL_FONT).is_none());

    let kern = Node::Kern {
        amount: Scaled::from_raw(123),
        kind: KernKind::Font,
    };
    assert!(preceding_glyph(NodeView::from(&kern), NULL_FONT).is_none());
}

fn test_font<G>(
    stores: &mut CommandContext<'_, G>,
    name: &str,
    characters: &[u8],
    tagged: &[(u8, usize)],
    program: Vec<tex_fonts::LigKernInstruction>,
    left_boundary_program: Option<u16>,
) -> FontId {
    let mut metrics = vec![None; 256];
    for &ch in characters {
        metrics[usize::from(ch)] = Some(CharMetrics {
            width: Scaled::from_raw(Scaled::UNITY / 2),
            height: Scaled::from_raw(0),
            depth: Scaled::from_raw(0),
            italic_correction: Scaled::from_raw(0),
            tag: CharTag::None,
        });
    }
    for &(ch, start) in tagged {
        metrics[usize::from(ch)]
            .as_mut()
            .expect("tagged glyph exists")
            .tag = CharTag::LigKern {
            program_index: u8::try_from(start).expect("short program"),
            start_index: u16::try_from(start).expect("short program"),
        };
    }
    let size = Scaled::from_raw(10 * Scaled::UNITY);
    let font = stores.intern_font(LoadedFont::new(
        name,
        format!("{name}.tfm"),
        tex_fonts::font_content_hash(name.as_bytes()),
        0,
        size,
        size,
        vec![Scaled::from_raw(0); 7],
        FontMetrics::new(metrics, program, None, left_boundary_program, Vec::new()),
    ));
    stores.set_font_hyphen_char(font, i32::from(b'-'));
    font
}

fn hyphenated_nodes<G>(
    stores: &mut CommandContext<'_, G>,
    font: FontId,
    first: Node,
    word: &str,
) -> Vec<Node> {
    let mut nodes = vec![
        Node::Glue {
            origin: GlueSpecOrigin::Owned,
            spec: GlueSpec::ZERO,
            kind: GlueKind::Normal,
            leader: None,
        },
        first,
    ];
    nodes.extend(word.chars().map(|ch| Node::Char {
        font,
        ch,
        origin: OriginId::UNKNOWN,
    }));
    nodes.push(Node::Penalty(0));
    let source = stores.publish_page_nodes(nodes);
    let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
    let mut scratch = crate::mode::HorizontalModeScratch::default();
    let mut ledger = tex_command::CommandFuelLedger::new(10_000).expect("bounded fuel");
    let result = hyphenated_hlist_with_fuel(
        stores,
        &mut effects,
        source,
        &mut scratch,
        ledger.fuel_mut(),
    )
    .expect("hyphenation reconstitutes the word");
    stores
        .page_nodes(result.semantic)
        .expect("semantic list")
        .iter()
        .map(|node| node.to_owned())
        .collect()
}

#[test]
fn hyphenation_reconstitutes_quote_with_following_letter() {
    // TeX82 §914 starts at the preceding quote ligature and restores the
    // quote–A kern while rebuilding an otherwise hyphenatable word.
    crate::test_harness::with_nonstop_plain_universe(|universe| {
        let mut stores = universe.command_context().expect("admitted test state");
        let font = test_font(
            &mut stores,
            "quoted-word",
            b"-`\x10Automated",
            &[(b'`', 0), (0x10, 1)],
            vec![
                tex_fonts::LigKernInstruction {
                    skip_byte: 128,
                    next_char: b'`',
                    command: Some(tex_fonts::LigKernCommand::Ligature(
                        tex_fonts::LigatureCommand {
                            replacement: 0x10,
                            delete_current: true,
                            delete_next: true,
                            pass_over: 0,
                        },
                    )),
                },
                tex_fonts::LigKernInstruction {
                    skip_byte: 128,
                    next_char: b'A',
                    command: Some(tex_fonts::LigKernCommand::Kern(Scaled::from_raw(-5_000))),
                },
            ],
            None,
        );
        stores.add_hyphenation_exception_for_language(
            0,
            ExceptionSpec {
                word: "automated".into(),
                positions: vec![2],
            },
        );
        stores
            .assign_int_param(IntParam::UC_HYPH, 1, AssignmentScope::Global)
            .expect("allow uppercase hyphenation");
        let quote = Node::Lig {
            font,
            ch: '\u{10}',
            orig: vec!['`', '`'],
            origins: vec![OriginId::UNKNOWN; 2],
            left_hit: false,
            right_hit: false,
        };
        let nodes = hyphenated_nodes(&mut stores, font, quote, "Automated");
        assert!(matches!(nodes.get(1), Some(Node::Lig { ch: '\u{10}', .. })));
        assert!(
            matches!(nodes.get(2), Some(Node::Kern { amount, kind: KernKind::Font }) if amount.raw() == -5_000)
        );
    });
}

#[test]
fn preceding_parenthesis_suppresses_spurious_left_boundary_kern() {
    // TeX82 §914 reconstitutes from `ha` (`(`), so the font's left-boundary
    // program must not insert its J kern at the start of the rebuilt word.
    crate::test_harness::with_nonstop_plain_universe(|universe| {
        let mut stores = universe.command_context().expect("admitted test state");
        let font = test_font(
            &mut stores,
            "parenthesized-word",
            b"-(july",
            &[],
            vec![tex_fonts::LigKernInstruction {
                skip_byte: 128,
                next_char: b'j',
                command: Some(tex_fonts::LigKernCommand::Kern(Scaled::from_raw(5_000))),
            }],
            Some(0),
        );
        stores.add_hyphenation_exception_for_language(
            0,
            ExceptionSpec {
                word: "july".into(),
                positions: vec![2],
            },
        );
        let parenthesis = Node::Char {
            font,
            ch: '(',
            origin: OriginId::UNKNOWN,
        };
        let nodes = hyphenated_nodes(&mut stores, font, parenthesis, "july");
        assert!(matches!(nodes.get(1), Some(Node::Char { ch: '(', .. })));
        assert!(matches!(nodes.get(2), Some(Node::Char { ch: 'j', .. })));
        assert!(!nodes.iter().any(|node| matches!(node, Node::Kern { amount, kind: KernKind::Font } if amount.raw() == 5_000)));
    });
}
