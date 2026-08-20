use crate::hyphenation::{ExceptionSpec, PatternSpec};
use crate::ids::TokenListId;
use crate::macro_store::MacroMeaning;
use crate::meaning::{Meaning, MeaningFlags};
use crate::page::PageMark;
use crate::scaled::Scaled;
use crate::token::{Catcode, Token};
use crate::{ParagraphShapeLine, PenaltyArrayKind, Universe, World};

mod live_boundary;
#[cfg(feature = "testing")]
mod replay;
#[cfg(feature = "testing")]
mod replay_common;

#[test]
fn etex_math_boundary_stack_counts_missing_and_extra_by_identity() {
    // Merged e-TeX WEB §53a counts unmatched ends as extra and open begins
    // remaining at list end as missing, while properly nested M/L/R pairs cancel.
    use crate::node::{LrAnomalies, MathBoundary as B, MathBoundaryStack};

    let mut matched = MathBoundaryStack::default();
    for boundary in [B::BeginL, B::BeginR, B::EndR, B::EndL] {
        matched.observe(boundary);
    }
    assert_eq!(matched.finish(), LrAnomalies::default());

    let mut anomalous = MathBoundaryStack::default();
    for boundary in [B::BeginM, B::BeginR, B::EndL, B::EndM] {
        anomalous.observe(boundary);
    }
    assert_eq!(
        anomalous.finish(),
        LrAnomalies {
            missing: 2,
            extra: 2
        }
    );
}

#[test]
fn smoke() {
    assert!(!env!("CARGO_PKG_NAME").is_empty());
}

#[test]
fn margin_kern_glyph_provenance_survives_snapshot_and_format_round_trips() {
    use crate::font::NULL_FONT;
    use crate::node::{MarginKernSide, Node};

    let expected = Node::MarginKern {
        amount: Scaled::from_raw(-12_345),
        side: MarginKernSide::Right,
        font: NULL_FONT,
        ch: b'.',
    };
    let mut universe = Universe::new();
    let list = universe.publish_page_nodes(std::slice::from_ref(&expected));
    universe.assign_page_box_global(17, list);
    let snapshot = universe.snapshot();
    let root = universe
        .copy_box_to_page(17)
        .expect("box survives snapshot");
    assert_eq!(
        universe
            .page_node_list(root)
            .expect("copied box belongs to the page arena")
            .nodes(),
        std::slice::from_ref(&expected)
    );
    universe.rollback(&snapshot);

    let bytes = universe.dump_format().expect("margin-kern format dumps");
    let mut loaded =
        Universe::from_format(World::default(), &bytes).expect("margin-kern format loads");
    let root = loaded.copy_box_to_page(17).expect("box survives format");
    assert_eq!(
        loaded
            .page_node_list(root)
            .expect("restored box belongs to the page arena")
            .nodes(),
        std::slice::from_ref(&expected)
    );
}

#[test]
fn format_roundtrip_counts_structural_end_match_in_macro_observation_width() {
    // TeX82 §§289/294/341/473 place `end_match` between a definition's
    // parameter and replacement text. The format loader compacts away an
    // unreachable frozen definition, then must still count that separator
    // when deriving the next retained macro's observed `def_ref` head.
    let mut universe = Universe::new();
    let empty = universe.intern_token_list(&[]);
    let frozen = universe.intern_macro(MacroMeaning::new(MeaningFlags::OUTER, empty, empty));
    universe.register_primitive_meaning(
        "frozen-only",
        Meaning::Macro {
            flags: MeaningFlags::OUTER,
            definition: frozen.id(),
        },
    );
    let body = universe.intern_token_list(&[Token::Char {
        ch: '2',
        cat: Catcode::Other,
    }]);
    let live = universe.intern_macro(MacroMeaning::new(MeaningFlags::EMPTY, empty, body));
    let symbol = universe.intern("version");
    universe.set_meaning(
        symbol,
        Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition: live.id(),
        },
    );
    let second_body = universe.intern_token_list(&[Token::Char {
        ch: '6',
        cat: Catcode::Other,
    }]);
    let second = universe.intern_macro(MacroMeaning::new(MeaningFlags::EMPTY, empty, second_body));
    let second_symbol = universe.intern("revision");
    universe.set_meaning(
        second_symbol,
        Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition: second.id(),
        },
    );

    let format = universe.dump_format().expect("macro format");
    let loaded = Universe::from_format(World::memory(), &format).expect("load macro format");
    let Meaning::Macro { definition, .. } =
        loaded.meaning(loaded.symbol("version").expect("loaded version symbol"))
    else {
        panic!("loaded version meaning is a macro");
    };
    assert_eq!(
        loaded.macro_definition_observation_operand(definition),
        249_985
    );
    let Meaning::Macro { definition, .. } =
        loaded.meaning(loaded.symbol("revision").expect("loaded revision symbol"))
    else {
        panic!("loaded revision meaning is a macro");
    };
    assert_eq!(
        loaded.macro_definition_observation_operand(definition),
        249_982
    );
}

#[test]
fn hyphenation_pattern_lifecycle_rolls_back_and_formats_closed() {
    // TeX82 §§919/960 and §1335: trie initialization is live rollback state,
    // while every dumped-and-restored trie is already initialized.
    let mut universe = Universe::new();
    assert!(universe.hyphenation_patterns_open());
    let snapshot = universe.snapshot();
    universe.close_hyphenation_patterns();
    assert!(!universe.hyphenation_patterns_open());
    universe.rollback(&snapshot);
    assert!(universe.hyphenation_patterns_open());

    let format = universe.dump_format().expect("hyphenation format");
    let loaded = Universe::from_format(World::default(), &format).expect("load hyphenation format");
    assert!(!loaded.hyphenation_patterns_open());
}

#[test]
fn packed_hyphenation_data_survives_two_format_loads_identically() {
    // TeX82 §§1335--1341 persist the initialized trie and its operation
    // table. Exercise overlapping operations, language separation, and an
    // exception without duplicating e-TeX saved-code coverage from e51h.83.
    let mut universe = Universe::new();
    for (language, letters, values) in [
        (0, "ab", vec![0, 1, 0]),
        (0, "abc", vec![0, 0, 3, 0]),
        (7, "ab", vec![0, 0, 5]),
    ] {
        universe
            .add_hyphenation_pattern_for_language(
                language,
                PatternSpec {
                    letters: letters.chars().collect(),
                    values,
                },
            )
            .expect("pattern fits the pdfTeX trie");
    }
    universe.add_hyphenation_exception_for_language(
        7,
        ExceptionSpec {
            word: "abcd".into(),
            positions: vec![2],
        },
    );

    let image = universe
        .dump_format()
        .expect("dump packed hyphenation data");
    let once = Universe::from_format(World::memory(), &image).expect("first format load");
    assert_eq!(once.hyphen_positions_for_language(0, "abcd", 0, 0), [1, 2]);
    assert_eq!(once.hyphen_positions_for_language(7, "ab", 0, 0), [2]);
    assert_eq!(once.hyphen_positions_for_language(7, "abcd", 0, 0), [2]);
    assert_eq!(
        once.hyphen_positions_for_language(8, "abcd", 0, 0),
        Vec::<usize>::new()
    );

    let once_image = once.dump_format().expect("redump first loaded format");
    assert_eq!(once_image, image);
    let twice = Universe::from_format(World::memory(), &once_image).expect("second format load");
    assert_eq!(twice.hyphen_positions_for_language(0, "abcd", 0, 0), [1, 2]);
    assert_eq!(twice.hyphen_positions_for_language(7, "ab", 0, 0), [2]);
    assert_eq!(twice.hyphen_positions_for_language(7, "abcd", 0, 0), [2]);
    assert_eq!(
        twice.dump_format().expect("redump second loaded format"),
        image
    );
}

#[test]
fn saved_hyphen_code_tables_survive_two_format_loads_exactly() {
    let mut universe = Universe::new();
    universe.save_hyphenation_codes(7, [('A', 'a'), ('B', 'b')]);
    universe.save_hyphenation_codes(8, std::iter::empty());

    assert_eq!(universe.saved_hyphenation_code(7, 'A'), Some(Some('a')));
    assert_eq!(universe.saved_hyphenation_code(7, 'Z'), Some(None));
    assert_eq!(universe.saved_hyphenation_code(8, 'A'), Some(None));
    assert_eq!(universe.saved_hyphenation_code(9, 'A'), None);

    let image = universe.dump_format().expect("dump saved hyphen codes");
    let once = Universe::from_format(World::memory(), &image).expect("first format load");
    assert_eq!(once.saved_hyphenation_code(7, 'A'), Some(Some('a')));
    assert_eq!(once.saved_hyphenation_code(7, 'Z'), Some(None));
    assert_eq!(once.saved_hyphenation_code(8, 'A'), Some(None));
    assert_eq!(once.saved_hyphenation_code(9, 'A'), None);

    let once_image = once.dump_format().expect("redump saved hyphen codes");
    assert_eq!(once_image, image);
    let twice = Universe::from_format(World::memory(), &once_image).expect("second format load");
    assert_eq!(twice.saved_hyphenation_code(7, 'A'), Some(Some('a')));
    assert_eq!(twice.saved_hyphenation_code(7, 'Z'), Some(None));
    assert_eq!(twice.saved_hyphenation_code(8, 'A'), Some(None));
    assert_eq!(twice.saved_hyphenation_code(9, 'A'), None);
    assert_eq!(twice.dump_format().expect("redump second load"), image);
}

#[test]
fn paragraph_shape_is_grouped_checkpointed_and_format_stable() {
    let outer = [ParagraphShapeLine {
        indent: Scaled::from_raw(3),
        width: Scaled::from_raw(40),
    }];
    let inner = [ParagraphShapeLine {
        indent: Scaled::from_raw(-7),
        width: Scaled::from_raw(90),
    }];
    let mut universe = Universe::new();
    assert_eq!(universe.paragraph_shape_len(), 0);
    universe.set_paragraph_shape(&outer, false);
    assert_eq!(universe.paragraph_shape_len(), outer.len());
    let snapshot = universe.snapshot();

    universe.enter_group();
    universe.set_paragraph_shape(&inner, false);
    assert_eq!(universe.paragraph_shape_len(), inner.len());
    assert_eq!(universe.paragraph_shape(), inner);
    let _ = universe.leave_group();
    assert_eq!(universe.paragraph_shape_len(), outer.len());
    assert_eq!(universe.paragraph_shape(), outer);

    universe.set_paragraph_shape(&inner, false);
    universe.rollback(&snapshot);
    assert_eq!(universe.paragraph_shape_len(), outer.len());
    assert_eq!(universe.paragraph_shape(), outer);

    let format = universe.dump_format().expect("paragraph shape format");
    let loaded = Universe::from_format(World::default(), &format).expect("load paragraph shape");
    assert_eq!(loaded.paragraph_shape_len(), outer.len());
    assert_eq!(loaded.paragraph_shape(), outer);
}

#[test]
fn penalty_arrays_are_grouped_checkpointed_and_repeat_their_last_value() {
    let mut universe = Universe::new();
    universe.set_penalty_array(PenaltyArrayKind::Club, &[200, 100], false);
    let snapshot = universe.snapshot();

    assert_eq!(universe.penalty_array_value(PenaltyArrayKind::Club, -1), 0);
    assert_eq!(universe.penalty_array_value(PenaltyArrayKind::Club, 0), 2);
    assert_eq!(universe.penalty_array_value(PenaltyArrayKind::Club, 1), 200);
    assert_eq!(universe.penalty_array_value(PenaltyArrayKind::Club, 5), 100);

    universe.enter_group();
    universe.set_penalty_array(PenaltyArrayKind::Club, &[7], false);
    assert_eq!(universe.penalty_array(PenaltyArrayKind::Club), vec![7]);
    let _ = universe.leave_group();
    assert_eq!(
        universe.penalty_array(PenaltyArrayKind::Club),
        vec![200, 100]
    );

    universe.set_penalty_array(PenaltyArrayKind::Club, &[], false);
    universe.rollback(&snapshot);
    assert_eq!(
        universe.penalty_array(PenaltyArrayKind::Club),
        vec![200, 100]
    );

    let format = universe.dump_format().expect("penalty array format");
    let loaded = Universe::from_format(World::default(), &format).expect("load penalty array");
    assert_eq!(loaded.penalty_array(PenaltyArrayKind::Club), vec![200, 100]);
}

#[test]
fn etex_vertical_discards_rollback_but_are_omitted_from_formats() {
    let mut universe = Universe::new();
    universe.push_page_discard(crate::node::Node::Penalty(17));
    universe.set_split_discards(vec![crate::node::Node::Penalty(23)]);
    let snapshot = universe.snapshot();

    assert_eq!(
        universe.take_page_discards(),
        vec![crate::node::Node::Penalty(17)]
    );
    universe.clear_split_discards();
    universe.rollback(&snapshot);
    assert_eq!(universe.page_discards(), &[crate::node::Node::Penalty(17)]);
    assert_eq!(universe.split_discards(), &[crate::node::Node::Penalty(23)]);

    let format = universe
        .dump_format()
        .expect("discard lists are not dumped");
    let loaded = Universe::from_format(World::default(), &format).expect("load discard format");
    assert!(loaded.page_discards().is_empty());
    assert!(loaded.split_discards().is_empty());
}

#[test]
fn hyphenation_state_rolls_back_with_snapshots() {
    let mut universe = Universe::new();
    universe.add_hyphenation_exception(ExceptionSpec {
        word: "before".to_owned(),
        positions: vec![2],
    });
    let snapshot = universe.snapshot();
    universe.add_hyphenation_exception(ExceptionSpec {
        word: "after".to_owned(),
        positions: vec![3],
    });
    universe
        .add_hyphenation_pattern(PatternSpec {
            letters: "after".chars().collect(),
            values: vec![0, 0, 1, 0, 0, 0],
        })
        .expect("pattern fits the default trie capacity");

    assert_eq!(universe.hyphen_positions("after", 1, 1), vec![3]);
    universe.rollback(&snapshot);
    assert_eq!(universe.hyphen_positions("before", 1, 1), vec![2]);
    assert!(universe.hyphen_positions("after", 1, 1).is_empty());
}

#[test]
fn page_mark_slots_roll_back_with_snapshots() {
    let mut universe = Universe::new();
    let before = universe.intern_token_list(&[Token::Char {
        ch: 'a',
        cat: Catcode::Letter,
    }]);
    universe.set_page_mark(PageMark::Bot, before);
    universe.set_page_mark_class(PageMark::Bot, 27, before);
    let snapshot = universe.snapshot();

    let after = universe.intern_token_list(&[Token::Char {
        ch: 'b',
        cat: Catcode::Letter,
    }]);
    universe.set_page_mark(PageMark::Top, after);
    universe.set_page_mark(PageMark::First, after);
    universe.set_page_mark(PageMark::Bot, after);
    universe.set_page_mark(PageMark::SplitFirst, after);
    universe.set_page_mark(PageMark::SplitBot, after);
    universe.set_page_mark_class(PageMark::Top, 27, after);
    universe.set_page_mark_class(PageMark::First, 27, after);
    universe.set_page_mark_class(PageMark::Bot, 27, after);

    universe.rollback(&snapshot);

    assert_eq!(universe.page_mark(PageMark::Top), TokenListId::EMPTY);
    assert_eq!(universe.page_mark(PageMark::First), TokenListId::EMPTY);
    assert_eq!(universe.page_mark(PageMark::Bot), before);
    assert_eq!(universe.page_mark(PageMark::SplitFirst), TokenListId::EMPTY);
    assert_eq!(universe.page_mark(PageMark::SplitBot), TokenListId::EMPTY);
    assert_eq!(
        universe.page_mark_class(PageMark::Top, 27),
        TokenListId::EMPTY
    );
    assert_eq!(universe.page_mark_class(PageMark::Bot, 27), before);
}

/// e-TeX `etex.web` merged change blocks 21, 49, and 77 retain TeX82 class
/// zero in the dense `cur_mark` slots and store nonzero classes separately.
/// Exercise the byte boundary explicitly so 255 and 256 cannot alias each
/// other or class zero through a truncated sparse key.
#[test]
fn page_mark_dense_and_sparse_boundary_classes_have_exact_identity() {
    let mut universe = Universe::new();
    let zero = universe.intern_token_list(&[Token::Char {
        ch: '0',
        cat: Catcode::Other,
    }]);
    let class_255 = universe.intern_token_list(&[Token::Char {
        ch: 'a',
        cat: Catcode::Letter,
    }]);
    let class_256 = universe.intern_token_list(&[Token::Char {
        ch: 'b',
        cat: Catcode::Letter,
    }]);

    for mark in [
        PageMark::Top,
        PageMark::First,
        PageMark::Bot,
        PageMark::SplitFirst,
        PageMark::SplitBot,
    ] {
        universe.set_page_mark_class(mark, 0, zero);
        universe.set_page_mark_class(mark, 255, class_255);
        universe.set_page_mark_class(mark, 256, class_256);
        assert_eq!(universe.page_mark(mark), zero);
        assert_eq!(universe.page_mark_class(mark, 0), zero);
        assert_eq!(universe.page_mark_class(mark, 255), class_255);
        assert_eq!(universe.page_mark_class(mark, 256), class_256);
    }
    assert_eq!(universe.page_mark_classes().collect::<Vec<_>>(), [255, 256]);
}

#[test]
fn frozen_alignment_token_kinds_have_distinct_semantic_hashes() {
    let mut universe = Universe::new();
    let checkpoint = universe.snapshot();
    let end_template = universe.intern_token_list(&[Token::frozen_end_template()]);
    universe.set_toks(0, end_template);
    let end_template_hash = universe.snapshot().state_hash();

    universe.rollback(&checkpoint);
    let endv = universe.intern_token_list(&[Token::frozen_endv()]);
    universe.set_toks(0, endv);
    let endv_hash = universe.snapshot().state_hash();

    assert_ne!(end_template_hash, endv_hash);
}

#[test]
fn frozen_primitive_tokens_have_distinct_semantic_hashes_and_round_trip() {
    let mut universe = Universe::new();
    let checkpoint = universe.snapshot();
    let first = universe.intern_token_list(&[Token::frozen_primitive(7)]);
    universe.set_toks(0, first);
    let first_hash = universe.snapshot().state_hash();

    universe.rollback(&checkpoint);
    let second = universe.intern_token_list(&[Token::frozen_primitive(8)]);
    universe.set_toks(0, second);
    let second_hash = universe.snapshot().state_hash();

    assert_ne!(first_hash, second_hash);

    let bytes = universe.dump_format().expect("frozen primitive format");
    let restored =
        Universe::from_format(World::memory(), &bytes).expect("restore frozen primitive");
    assert_eq!(
        restored.tokens(restored.toks(0)),
        &[Token::frozen_primitive(8)]
    );
    assert_eq!(restored.dump_format().expect("format redumps"), bytes);
}

#[test]
fn permanent_frozen_control_sequences_retain_their_eqtb_names() {
    let universe = Universe::new();
    assert_eq!(
        universe.frozen_primitive_name(Token::frozen_end_template()),
        Some("endtemplate")
    );
    assert_eq!(
        universe.frozen_primitive_name(Token::frozen_endv()),
        Some("endtemplate")
    );
    assert_eq!(
        universe.frozen_primitive_name(Token::frozen_relax()),
        Some("relax")
    );
}

#[test]
fn frozen_primitive_rendering_uses_its_registered_eqtb_name() {
    let mut universe = Universe::new();
    universe.register_primitive_meaning("relax", Meaning::Relax);
    let primitive = universe
        .primitive_token("relax")
        .expect("registered primitive has a frozen token");

    let mut shown = String::new();
    crate::token_show::append_token_show_text(&universe, primitive, &mut shown);
    assert_eq!(shown, "\\relax ");
    assert_eq!(
        crate::token_show::token_text(&universe, primitive),
        "\\relax"
    );

    let mut end_template = String::new();
    crate::token_show::append_token_show_text(
        &universe,
        Token::frozen_end_template(),
        &mut end_template,
    );
    assert_eq!(end_template, "\\endtemplate ");
}

#[test]
fn frozen_relax_has_distinct_semantic_identity_and_format_round_trips() {
    let mut universe = Universe::new();
    let checkpoint = universe.snapshot();
    let primitive = universe.intern_token_list(&[Token::frozen_primitive(7)]);
    universe.set_toks(0, primitive);
    let primitive_hash = universe.snapshot().state_hash();

    universe.rollback(&checkpoint);
    let relax = universe.intern_token_list(&[Token::frozen_relax()]);
    universe.set_toks(0, relax);
    let relax_hash = universe.snapshot().state_hash();
    assert_ne!(relax_hash, primitive_hash);

    let bytes = universe.dump_format().expect("frozen relax format");
    let restored = Universe::from_format(World::memory(), &bytes).expect("restore frozen relax");
    assert_eq!(restored.tokens(restored.toks(0)), &[Token::frozen_relax()]);
    assert_eq!(restored.dump_format().expect("format redumps"), bytes);
}
