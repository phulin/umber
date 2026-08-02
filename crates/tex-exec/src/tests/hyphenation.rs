use super::support::*;
use super::*;
use tex_command::CommandProfile;

fn pretolerance_memo_config() -> tex_state::PureMemoConfig {
    tex_state::PureMemoConfig {
        recording: tex_state::PureMemoRecordingPolicy {
            pretolerance: true,
            paragraphs: false,
            pages: false,
            shipouts: false,
        },
        ..tex_state::PureMemoConfig::default()
    }
}
use tex_state::node::{GlueKind, KernKind, Node};
use tex_state::scaled::Scaled;

#[test]
fn patterns_and_exceptions_drive_word_hyphenation() {
    let stores = super::core::run_canonical_tex82(
        "\\patterns{a1ba t2e1st}\\hyphenation{tes-ting}\\lefthyphenmin=1 \\righthyphenmin=1 \\end",
    );

    assert_eq!(
        crate::assignments::test_hyphenated_word_text(&stores, "aba"),
        "a-ba"
    );
    assert_eq!(
        crate::assignments::test_hyphenated_word_text(&stores, "testing"),
        "tes-ting"
    );
    assert_eq!(
        crate::assignments::test_hyphenated_word_text(&stores, "test"),
        "te-st"
    );
}

#[test]
fn patterns_report_deterministic_pattern_memory_exhaustion() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    stores.set_hyphenation_trie_capacity(3);
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    control
        .register_root_source(tex_command::SourceRegistration::new(
            tex_command::RegisteredSourceKind::Generated,
            b"\\patterns{a1b a1c}\\end".to_vec(),
        ))
        .expect("register pattern-memory source");
    for _ in 0..16 {
        if control.step(&mut stores).expect("canonical pattern step") == MainControlStep::End {
            break;
        }
    }

    assert_eq!(
        control.fatal_error(),
        Some(tex_command::FatalError::overflow("pattern memory", 3))
    );
    assert_eq!(
        crate::assignments::test_hyphenated_word_text(&stores, "ab"),
        "a-b",
        "the pattern which exactly filled capacity remains installed"
    );
    assert_eq!(
        crate::assignments::test_hyphenated_word_text(&stores, "ac"),
        "ac",
        "the overflowing insertion does not partially mutate the trie"
    );
}

#[test]
fn etex_saved_hyphen_codes_are_language_specific_and_survive_lccode_changes() {
    let mut stores = super::core::run_canonical_etex(
        "\\savinghyphcodes=1 \\language=1 \\lccode`A=`a \\patterns{a1ba} \
         \\lccode`A=`z \\lefthyphenmin=1 \\righthyphenmin=1 \
         \\language=2 \\lccode`A=`x \\patterns{x1ba} \\lccode`A=`z \
         \\hyphenation{Ab-a} \
         \\language=1 \\end",
    );

    // `\lccode`A` ends the run as `z`, so every mapping below comes from the
    // per-language codes e-TeX saved when each `\patterns` list was read.
    assert_eq!(
        crate::assignments::test_hyphenated_word_text(&stores, "Aba"),
        "a-ba"
    );
    stores.set_int_param(IntParam::LANGUAGE, 2);
    assert_eq!(
        crate::assignments::test_hyphenated_word_text(&stores, "Aba"),
        "xb-a"
    );
    stores.set_int_param(IntParam::LANGUAGE, 1);
    assert_eq!(
        crate::assignments::test_hyphenated_word_text(&stores, "Aba"),
        "a-ba"
    );
}

#[test]
fn etex_saved_hyphen_codes_distinguish_zero_from_absent_and_capture_grouped_values() {
    let mut stores = super::core::run_canonical_etex(
        r"\savinghyphcodes=0 \language=1 \lccode`A=`a \patterns{a1b}
          \savinghyphcodes=1
          \language=2 {\lccode`A=`x \patterns{x1b}}
          \language=3 \global\lccode`A=`y {\lccode`A=0 \patterns{y1b}}
          \language=4 \lccode`A=0 \patterns{z1b}
          \end",
    );

    assert_eq!(stores.saved_hyphenation_code(1, 'A'), None);
    assert_eq!(stores.saved_hyphenation_code(2, 'A'), Some(Some('x')));
    assert_eq!(stores.saved_hyphenation_code(3, 'A'), Some(None));
    assert_eq!(stores.saved_hyphenation_code(4, 'A'), Some(None));
    assert_eq!(
        stores.saved_hyphenation_code(5, 'A'),
        None,
        "a language without a saved table keeps live-lccode fallback"
    );
    stores.set_int_param(IntParam::LEFT_HYPHEN_MIN, 1);
    stores.set_int_param(IntParam::RIGHT_HYPHEN_MIN, 1);
    stores.set_int_param(IntParam::LANGUAGE, 1);
    assert_eq!(
        crate::assignments::test_hyphenated_word_text(&stores, "Ab"),
        "Ab"
    );
    stores.set_int_param(IntParam::LANGUAGE, 2);
    assert_eq!(
        crate::assignments::test_hyphenated_word_text(&stores, "Ab"),
        "x-b"
    );
    stores.set_int_param(IntParam::LANGUAGE, 3);
    assert_eq!(
        crate::assignments::test_hyphenated_word_text(&stores, "Ab"),
        "Ab"
    );
}

#[test]
fn saved_zero_codes_do_not_consume_the_63_letter_exception_limit() {
    let source = format!(
        "\\savinghyphcodes=1 \\lccode`X=0 \\patterns{{a1b}} \\hyphenation{{{}X-a}}\\end",
        "a".repeat(62)
    );
    let stores = super::core::run_canonical_etex(&source);

    assert_eq!(
        stores.hyphenation_exception(&"a".repeat(63)),
        Some(&[62][..]),
        "the ignored zero-code character does not displace the eligible 63rd letter"
    );
}

#[test]
fn word_hyphenation_honors_hyphen_minima() {
    let mut stores = super::core::run_canonical_tex82(
        "\\patterns{a1ba}\\righthyphenmin=1 \\lefthyphenmin=3 \\end",
    );

    assert_eq!(
        crate::assignments::test_hyphenated_word_text(&stores, "aba"),
        "aba"
    );
    stores.set_int_param(IntParam::LEFT_HYPHEN_MIN, 1);
    assert_eq!(
        crate::assignments::test_hyphenated_word_text(&stores, "aba"),
        "a-ba"
    );
}

#[test]
fn paragraph_hyphenation_honors_uchyph_for_uppercase_start() {
    let mut stores = super::core::run_canonical_tex82_with_fonts(
        "\\font\\tenrm=cmr10 \\relax \\tenrm \\patterns{a1ba}\\lefthyphenmin=1 \\righthyphenmin=1 \\end",
    );
    let font = stores.current_font();
    let word: Vec<_> = "Aba"
        .chars()
        .map(|ch| tex_state::node::Node::Char {
            font,
            ch,
            origin: tex_state::token::OriginId::UNKNOWN,
        })
        .collect();

    stores.set_int_param(IntParam::UC_HYPH, 0);
    let lowercase_only = crate::assignments::test_hyphenated_hlist(&mut stores, &word);
    stores.set_int_param(IntParam::UC_HYPH, 1);
    let uppercase_enabled = crate::assignments::test_hyphenated_hlist(&mut stores, &word);

    assert!(
        !lowercase_only
            .iter()
            .any(|node| matches!(node, tex_state::node::Node::Disc { .. }))
    );
    assert!(
        uppercase_enabled
            .iter()
            .any(|node| matches!(node, tex_state::node::Node::Disc { .. }))
    );
}

#[test]
fn paragraph_hyphenation_requires_an_in_range_hyphen_and_omits_a_missing_glyph() {
    let mut stores = super::core::run_canonical_tex82_with_fonts(
        "\\font\\tenrm=cmr10 \\relax \\tenrm \\patterns{a1ba}\\lefthyphenmin=1 \\righthyphenmin=1 \\end",
    );
    let font = stores.current_font();
    let word: Vec<_> = "aba"
        .chars()
        .map(|ch| tex_state::node::Node::Char {
            font,
            ch,
            origin: tex_state::token::OriginId::UNKNOWN,
        })
        .collect();

    stores.set_font_hyphen_char(font, -1);
    let disabled = crate::assignments::test_hyphenated_hlist(&mut stores, &word);
    let missing_code = (0u8..=u8::MAX)
        .find(|&code| !stores.font_char_exists(font, code))
        .expect("test font has an in-range missing character");
    stores.set_font_hyphen_char(font, i32::from(missing_code));
    let missing_glyph = crate::assignments::test_hyphenated_hlist(&mut stores, &word);
    stores.set_font_hyphen_char(font, i32::from(b'-'));
    let enabled = crate::assignments::test_hyphenated_hlist(&mut stores, &word);

    assert!(
        !disabled
            .iter()
            .any(|node| matches!(node, tex_state::node::Node::Disc { .. }))
    );
    assert!(
        missing_glyph.iter().any(|node| {
            matches!(node, Node::Disc { pre, .. } if stores.nodes(*pre).is_empty())
        }),
        "TeX retains the discretionary but new_character returns null"
    );
    assert!(
        enabled
            .iter()
            .any(|node| matches!(node, tex_state::node::Node::Disc { .. }))
    );
}

#[test]
fn paragraph_hyphenation_preserves_existing_chars_when_no_break_is_found() {
    let mut stores =
        super::core::run_canonical_tex82_with_fonts("\\font\\tenrm=cmr10 \\relax \\tenrm \\end");
    let font = stores.current_font();
    let word = vec![
        tex_state::node::Node::Char {
            font,
            ch: 'f',
            origin: tex_state::token::OriginId::UNKNOWN,
        },
        tex_state::node::Node::Char {
            font,
            ch: 'f',
            origin: tex_state::token::OriginId::UNKNOWN,
        },
    ];

    let unchanged = crate::assignments::test_hyphenated_hlist(&mut stores, &word);
    assert_eq!(
        unchanged, word,
        "no-break hyphenation must not create an ff ligature"
    );
}

#[test]
fn unchanged_hyphenation_reuses_the_owned_paragraph_buffer() {
    let mut stores = stores_with_fonts();
    let font = stores.current_font();
    let glue = stores.glue_param(tex_state::env::banks::GlueParam::PAR_SKIP);
    let boundary = tex_state::node::Node::Glue {
        spec: glue,
        kind: tex_state::node::GlueKind::Normal,
        leader: None,
    };
    let mut paragraph = Vec::with_capacity(16);
    paragraph.push(boundary.clone());
    paragraph.extend("unmatched".chars().map(|ch| tex_state::node::Node::Char {
        font,
        ch,
        origin: tex_state::token::OriginId::UNKNOWN,
    }));
    paragraph.push(boundary);
    let allocation = paragraph.as_ptr();

    let unchanged = crate::assignments::test_hyphenated_hlist_owned(&mut stores, paragraph);

    assert_eq!(unchanged.as_ptr(), allocation);
    assert!(
        !unchanged
            .iter()
            .any(|node| matches!(node, tex_state::node::Node::Disc { .. }))
    );
}

#[test]
fn paragraph_hyphenation_stops_at_a_font_change() {
    let mut stores = super::core::run_canonical_tex82_with_fonts(
        "\\font\\a=cmr10 \\font\\b=cmmi10 \\relax \\hyphenation{ab-cdefgh} \\end",
    );
    let first = font_meaning(&stores, "a");
    let second = font_meaning(&stores, "b");
    stores.set_font_hyphen_char(first, i32::from(b'-'));
    stores.set_font_hyphen_char(second, i32::from(b'-'));
    let glue = stores.glue_param(GlueParam::PAR_SKIP);
    let mut nodes = vec![Node::Glue {
        spec: glue,
        kind: GlueKind::Normal,
        leader: None,
    }];
    nodes.extend("abcd".chars().map(|ch| Node::Char {
        font: first,
        ch,
        origin: tex_state::token::OriginId::UNKNOWN,
    }));
    nodes.extend("efgh".chars().map(|ch| Node::Char {
        font: second,
        ch,
        origin: tex_state::token::OriginId::UNKNOWN,
    }));
    nodes.push(Node::Glue {
        spec: glue,
        kind: GlueKind::Normal,
        leader: None,
    });

    let hyphenated = crate::assignments::test_hyphenated_hlist(&mut stores, &nodes);

    assert!(
        !hyphenated
            .iter()
            .any(|node| matches!(node, Node::Disc { .. })),
        "TeX82 sections 897 and 899 stop the word at a font change"
    );
}

#[test]
fn paragraph_hyphenation_distinguishes_font_and_normal_kerns() {
    // pdfTeX §§26030--27481 preserve TeX's word boundary: a font kern may
    // occur inside a word, while an explicit normal kern terminates it.
    let mut stores = super::core::run_canonical_tex82_with_fonts(
        "\\font\\tenrm=cmr10 \\relax \\tenrm \\hyphenation{ab-cd} \\lefthyphenmin=1 \\righthyphenmin=1 \\end",
    );
    let font = stores.current_font();
    let word = |kind| {
        let mut nodes: Vec<_> = "ab"
            .chars()
            .map(|ch| Node::Char {
                font,
                ch,
                origin: tex_state::token::OriginId::UNKNOWN,
            })
            .collect();
        nodes.push(Node::Kern {
            amount: Scaled::from_raw(0),
            kind,
        });
        nodes.extend("cd".chars().map(|ch| Node::Char {
            font,
            ch,
            origin: tex_state::token::OriginId::UNKNOWN,
        }));
        nodes
    };

    let font_kern = crate::assignments::test_hyphenated_hlist(&mut stores, &word(KernKind::Font));
    let normal_kern =
        crate::assignments::test_hyphenated_hlist(&mut stores, &word(KernKind::Explicit));

    assert!(
        font_kern
            .iter()
            .any(|node| matches!(node, Node::Disc { .. }))
    );
    assert!(
        !normal_kern
            .iter()
            .any(|node| matches!(node, Node::Disc { .. }))
    );
}

#[test]
fn directional_regions_preserve_text_hyphenation_eligibility() {
    // e-TeX 2.6 [17.3822--3880] adds L/R boundaries to horizontal lists;
    // TeX82 §§896--899 still select and collect the ordinary text between
    // them. BeginM/EndM are math-origin artifacts and remain barriers.
    let mut stores = super::core::run_canonical_tex82_with_fonts(
        r"\font\tenrm=cmr10 \relax \tenrm
          \hyphenation{di-rec-tion} \lefthyphenmin=1 \righthyphenmin=1 \end",
    );
    let font = stores.current_font();
    stores.set_font_hyphen_char(font, i32::from(b'-'));
    let chars = || {
        "direction"
            .chars()
            .map(|ch| Node::Char {
                font,
                ch,
                origin: tex_state::token::OriginId::UNKNOWN,
            })
            .collect::<Vec<_>>()
    };
    let break_count = |stores: &mut Universe, nodes: Vec<Node>| {
        crate::assignments::test_hyphenated_hlist(stores, &nodes)
            .iter()
            .filter(|node| matches!(node, Node::Disc { .. }))
            .count()
    };
    use tex_state::node::MathBoundary::{BeginL, BeginM, BeginR, EndL, EndM, EndR};

    assert_eq!(break_count(&mut stores, chars()), 2, "plain control");
    for boundaries in [
        vec![BeginL, EndL],
        vec![BeginR, EndR],
        vec![BeginL, BeginR, EndR, EndL],
        vec![BeginR, BeginL, EndL, EndR],
    ] {
        let split = boundaries.len() / 2;
        let mut nodes: Vec<_> = boundaries[..split]
            .iter()
            .copied()
            .map(Node::Direction)
            .collect();
        nodes.extend(chars());
        nodes.extend(boundaries[split..].iter().copied().map(Node::Direction));
        assert_eq!(break_count(&mut stores, nodes), 2, "{boundaries:?}");
    }

    for boundaries in [[BeginM, EndM], [BeginL, BeginM]] {
        let mut nodes = vec![Node::Direction(boundaries[0])];
        nodes.extend(chars());
        nodes.push(Node::Direction(boundaries[1]));
        assert_eq!(break_count(&mut stores, nodes), 0, "{boundaries:?}");
    }
}

#[test]
fn directional_candidates_keep_language_whatsits_and_nontext_barriers_distinct() {
    // TeX82 §§896--899 save language/minima while seeking the candidate.
    // L/R markers and language whatsits are transparent there; normal kerns
    // and boxes (including e-TeX box_lr variants) remain nonletter barriers.
    let mut stores = super::core::run_canonical_etex(
        r"\savinghyphcodes=1 \language=7 \patterns{d1irection}
          \language=0 \lefthyphenmin=1 \righthyphenmin=1 \end",
    );
    let font = stores.current_font();
    stores.set_font_hyphen_char(font, i32::from(b'-'));
    let word = || {
        "direction"
            .chars()
            .map(|ch| Node::Char {
                font,
                ch,
                origin: tex_state::token::OriginId::UNKNOWN,
            })
            .collect::<Vec<_>>()
    };
    let language = Node::Whatsit(tex_state::node::Whatsit::Language {
        language: 7,
        left_hyphen_min: 1,
        right_hyphen_min: 1,
    });
    let breaks = |stores: &mut Universe, nodes: Vec<Node>| {
        crate::assignments::test_hyphenated_hlist(stores, &nodes)
            .iter()
            .filter(|node| matches!(node, Node::Disc { .. }))
            .count()
    };
    let mut eligible = vec![
        Node::Direction(tex_state::node::Direction::BeginR),
        language,
    ];
    eligible.extend(word());
    eligible.push(Node::Direction(tex_state::node::Direction::EndR));
    assert_eq!(breaks(&mut stores, eligible), 1);

    for barrier in [
        Node::Kern {
            amount: Scaled::from_raw(0),
            kind: KernKind::Explicit,
        },
        Node::HList(tex_state::node::BoxNode::new(
            tex_state::node::BoxNodeFields {
                width: Scaled::from_raw(0),
                height: Scaled::from_raw(0),
                depth: Scaled::from_raw(0),
                shift: Scaled::from_raw(0),
                box_lr: tex_state::node::BoxLr::Reversed,
                glue_set: tex_state::scaled::GlueSetRatio::ZERO,
                glue_sign: tex_state::node::Sign::Normal,
                glue_order: tex_state::glue::Order::Normal,
                children: stores.freeze_node_list(&[]),
            },
        )),
    ] {
        let mut nodes = vec![barrier];
        nodes.extend(word());
        assert_eq!(breaks(&mut stores, nodes), 0);
    }
}

#[test]
fn exception_markers_after_the_63_letter_boundary_are_discarded() {
    // pdfTeX §§26030--27481 bound exception words and their marker vector
    // together. Markers after discarded letters must not alias position 63.
    let source = format!("\\hyphenation{{{}-aa-}}\\end", "a".repeat(62));
    let stores = super::core::run_canonical_tex82(&source);

    assert_eq!(
        stores.hyphenation_exception(&"a".repeat(63)),
        Some(&[62][..]),
        "the marker after the discarded 64th letter is not folded onto 63"
    );
}

#[test]
fn punctuation_and_whatsits_preserve_the_next_hyphenation_candidate() {
    // TeX82 §896 skips nonletters and whatsits while seeking the first letter.
    let mut stores = super::core::run_canonical_tex82_with_fonts(
        "\\font\\tenrm=cmr10 \\relax \\tenrm \\hyphenation{tes-ting} \\lefthyphenmin=1 \\righthyphenmin=1 \\end",
    );
    let font = stores.current_font();
    let mut nodes = vec![
        Node::Char {
            font,
            ch: ',',
            origin: tex_state::token::OriginId::UNKNOWN,
        },
        Node::Whatsit(tex_state::node::Whatsit::Special {
            class: "dvi".to_owned(),
            payload: Vec::new(),
        }),
    ];
    nodes.extend("testing".chars().map(|ch| Node::Char {
        font,
        ch,
        origin: tex_state::token::OriginId::UNKNOWN,
    }));

    let hyphenated = crate::assignments::test_hyphenated_hlist(&mut stores, &nodes);

    assert!(matches!(
        hyphenated.first(),
        Some(Node::Char { ch: ',', .. })
    ));
    assert!(matches!(hyphenated.get(1), Some(Node::Whatsit(_))));
    assert_eq!(
        hyphenated
            .iter()
            .filter(|node| matches!(node, Node::Disc { .. }))
            .count(),
        1
    );
}

#[test]
fn paragraph_candidates_keep_patterns_and_exceptions_language_qualified() {
    // pdfTeX §§26030--27481 retain TeX82 §896's candidate language:
    // `\language<0` and `\language>255` select language zero, while 255 is a
    // distinct pattern/exception namespace. This intentionally uses live
    // `\lccode` fallback; saved-code switching belongs to umber2-e51h.83.
    let mut stores = super::core::run_canonical_tex82_with_fonts(
        r"\language=-1 \hyphenation{de-fault}
          \language=255 \patterns{b1ound} \hyphenation{ex-ception}
          \language=256 \patterns{a1fter}
          \lefthyphenmin=1 \righthyphenmin=1 \end",
    );
    let font = stores.current_font();
    stores.set_font_hyphen_char(font, i32::from(b'-'));
    let chars = |word: &str| {
        word.chars()
            .map(|ch| Node::Char {
                font,
                ch,
                origin: tex_state::token::OriginId::UNKNOWN,
            })
            .collect::<Vec<_>>()
    };
    let language = |language| {
        Node::Whatsit(tex_state::node::Whatsit::Language {
            language,
            left_hyphen_min: 1,
            right_hyphen_min: 1,
        })
    };
    let breaks = |stores: &mut Universe, nodes: Vec<Node>| {
        crate::assignments::test_hyphenated_hlist(stores, &nodes)
            .iter()
            .filter(|node| matches!(node, Node::Disc { .. }))
            .count()
    };

    assert_eq!(breaks(&mut stores, chars("default")), 1);
    assert_eq!(breaks(&mut stores, chars("after")), 1);

    let mut language_255_exception = vec![language(255)];
    language_255_exception.extend(chars("exception"));
    assert_eq!(breaks(&mut stores, language_255_exception), 1);
    let mut language_255_pattern = vec![language(255)];
    language_255_pattern.extend(chars("bound"));
    assert_eq!(breaks(&mut stores, language_255_pattern), 1);

    let mut wrong_language = vec![language(255)];
    wrong_language.extend(chars("default"));
    assert_eq!(breaks(&mut stores, wrong_language), 0);
}

#[test]
fn language_whatsit_after_candidate_start_does_not_requalify_that_word() {
    // TeX82 §896 updates `cur_lang` while seeking a candidate, then §897
    // holds that language fixed while collecting the word. A language whatsit
    // that terminates the candidate affects the next post-glue search only.
    let mut stores = super::core::run_canonical_tex82_with_fonts(
        r"\language=0 \hyphenation{be-fore}
          \language=255 \hyphenation{af-ter}
          \lefthyphenmin=1 \righthyphenmin=1 \end",
    );
    let font = stores.current_font();
    stores.set_font_hyphen_char(font, i32::from(b'-'));
    let chars = |word: &str| {
        word.chars()
            .map(|ch| Node::Char {
                font,
                ch,
                origin: tex_state::token::OriginId::UNKNOWN,
            })
            .collect::<Vec<_>>()
    };
    let language_255 = Node::Whatsit(tex_state::node::Whatsit::Language {
        language: 255,
        left_hyphen_min: 1,
        right_hyphen_min: 1,
    });
    let glue = Node::Glue {
        spec: stores.glue_param(GlueParam::PAR_SKIP),
        kind: GlueKind::Normal,
        leader: None,
    };
    let mut paragraph = vec![glue.clone()];
    paragraph.extend(chars("before"));
    paragraph.push(language_255);
    paragraph.push(glue);
    paragraph.extend(chars("after"));

    let hyphenated = crate::assignments::test_hyphenated_hlist_owned(&mut stores, paragraph);
    assert_eq!(
        hyphenated
            .iter()
            .filter(|node| matches!(node, Node::Disc { .. }))
            .count(),
        2,
        "the in-progress word uses language zero; the post-glue word uses 255"
    );
}

#[test]
fn automatic_discretionary_rejects_replacement_counts_above_127() {
    // TeX82 §918 discards the discretionary when r_count exceeds 127.
    let mut stores = super::core::run_canonical_tex82("\\end");
    let replacement = vec![Node::Penalty(0); 128];

    assert!(
        crate::assignments::test_automatic_discretionary(&mut stores, &replacement[..127])
            .is_some()
    );
    assert!(crate::assignments::test_automatic_discretionary(&mut stores, &replacement).is_none());
}

#[test]
fn automatic_discretionaries_retain_exact_physical_replacement_counts() {
    // TeX82 §§904/914/918 counts the reconstitution's physical linked nodes,
    // not Umber's structured replacement-list entries.
    let mut stores = super::core::run_canonical_tex82("\\end");
    let font = stores.current_font();
    let boundary_kern = Node::Kern {
        amount: Scaled::from_raw(1),
        kind: KernKind::Font,
    };
    let ligature = || Node::Lig {
        font,
        ch: 'A',
        orig: vec!['A', 'A'],
        origins: vec![tex_state::token::OriginId::UNKNOWN; 2],
        left_hit: false,
        right_hit: false,
    };
    let replacements = [
        vec![boundary_kern],
        vec![ligature()],
        Vec::new(),
        vec![ligature()],
    ];
    let counts = replacements.map(|replace| {
        let Node::Disc {
            physical_replace_count,
            ..
        } = crate::assignments::test_automatic_discretionary(&mut stores, &replace)
            .expect("bounded replacement creates a discretionary")
        else {
            unreachable!()
        };
        physical_replace_count
    });

    assert_eq!(counts, [2, 1, 0, 1]);
}

#[test]
fn boundary_discretionary_physical_pre_branch_reconstitutes_preceding_span() {
    let mut stores =
        super::core::run_canonical_tex82_with_fonts("\\font\\tenrm=cmr10 \\relax \\tenrm \\end");
    let font = stores.current_font();
    stores.set_font_hyphen_char(font, i32::from(b'-'));
    let empty = stores.freeze_node_list(&[]);
    let semantic_pre = stores.freeze_node_list(&[Node::Char {
        font,
        ch: '-',
        origin: tex_state::token::OriginId::UNKNOWN,
    }]);
    let replacement = stores.freeze_node_list(&[Node::Kern {
        amount: Scaled::from_raw(3 * Scaled::UNITY),
        kind: KernKind::Font,
    }]);
    let nodes = vec![
        Node::Lig {
            font,
            ch: 'A',
            orig: vec!['A', 'A'],
            origins: vec![tex_state::token::OriginId::UNKNOWN; 2],
            left_hit: false,
            right_hit: false,
        },
        Node::Disc {
            kind: tex_state::node::DiscKind::AutomaticHyphen,
            pre: semantic_pre,
            post: empty,
            replace: replacement,
            physical_replace_count: 2,
        },
    ];

    let physical = crate::assignments::test_physical_pre_break_projection(&mut stores, &nodes);
    let Node::Disc { pre, .. } = physical[1] else {
        unreachable!()
    };
    let projected_chars = stores
        .nodes(pre)
        .iter()
        .flat_map(|node| match node {
            tex_state::node_arena::NodeRef::Char { ch, .. } => vec![ch],
            tex_state::node_arena::NodeRef::Lig { orig, .. } => orig.to_vec(),
            _ => Vec::new(),
        })
        .collect::<Vec<_>>();
    assert_eq!(projected_chars, ['A', 'A', '-']);
    assert!(matches!(
        stores.nodes(semantic_pre).first(),
        Some(tex_state::node_arena::NodeRef::Char { ch: '-', .. })
    ));
}

#[test]
fn through_ligature_physical_post_branch_owns_span_to_synchronization() {
    let stores = super::core::run_canonical_tex82("\\end");
    let font = stores.current_font();
    let replacement = Node::Lig {
        font,
        ch: 'A',
        orig: vec!['B', 'B'],
        origins: vec![tex_state::token::OriginId::UNKNOWN; 2],
        left_hit: false,
        right_hit: false,
    };
    let following = [
        Node::Kern {
            amount: Scaled::from_raw(2 * Scaled::UNITY),
            kind: KernKind::Font,
        },
        Node::Lig {
            font,
            ch: 'A',
            orig: vec!['B', 'B'],
            origins: vec![tex_state::token::OriginId::UNKNOWN; 2],
            left_hit: false,
            right_hit: false,
        },
    ];

    let minor = [
        replacement.clone(),
        Node::Kern {
            amount: Scaled::from_raw(2 * Scaled::UNITY),
            kind: KernKind::Font,
        },
        Node::Char {
            font,
            ch: 'B',
            origin: tex_state::token::OriginId::UNKNOWN,
        },
        Node::Kern {
            amount: Scaled::from_raw(4 * Scaled::UNITY),
            kind: KernKind::Font,
        },
    ];
    let (count, projected) = crate::assignments::test_physical_post_break_span(
        6,
        (2, 3, 4),
        &replacement,
        &following,
        &minor,
    );
    assert_eq!(count, 3);
    assert!(
        matches!(projected.as_slice(), [Node::Lig { orig, .. }, Node::Kern { kind: KernKind::Font, .. }, Node::Char { ch: 'B', .. }, Node::Kern { kind: KernKind::Font, .. }] if orig == &['B', 'B'])
    );
}

#[test]
fn through_ligature_synchronization_counts_nodes_not_source_characters() {
    // TeX82 §§914--918 counts major-branch linked nodes until the post-break
    // reconstitution reaches the same character boundary. A structured `CA`
    // ligature is one replacement node, while the post branch is the single
    // `A` character at that boundary.
    let stores = super::core::run_canonical_tex82("\\end");
    let font = stores.current_font();
    let replacement = Node::Lig {
        font,
        ch: '\u{82}',
        orig: vec!['C', 'A'],
        origins: vec![tex_state::token::OriginId::UNKNOWN; 2],
        left_hit: false,
        right_hit: false,
    };
    let following = [replacement.clone()];
    let minor = [
        Node::Char {
            font,
            ch: 'A',
            origin: tex_state::token::OriginId::UNKNOWN,
        },
        following[0].clone(),
    ];

    let (count, projected) = crate::assignments::test_physical_post_break_span(
        10,
        (6, 7, 8),
        &replacement,
        &following,
        &minor,
    );

    assert_eq!(count, 1);
    assert!(matches!(projected.as_slice(), [Node::Char { ch: 'A', .. }]));
}

#[test]
fn successful_pretolerance_does_not_allocate_hyphenation_nodes() {
    let mut stores = super::core::run_canonical_tex82_with_fonts(
        "\\font\\tenrm=cmr10 \\relax \\tenrm \\hyphenation{ab-cdefgh} \\end",
    );
    let font = stores.current_font();
    stores.set_font_hyphen_char(font, i32::from(b'-'));
    let par_fill = stores.glue_param(GlueParam::PAR_FILL_SKIP);
    let mut nodes = vec![
        Node::Char {
            font,
            ch: 'x',
            origin: tex_state::token::OriginId::UNKNOWN,
        },
        Node::Glue {
            spec: stores.glue_param(GlueParam::SPACE_SKIP),
            kind: GlueKind::Normal,
            leader: None,
        },
    ];
    nodes.extend("abcdefgh".chars().map(|ch| Node::Char {
        font,
        ch,
        origin: tex_state::token::OriginId::UNKNOWN,
    }));
    nodes.push(Node::Penalty(10_000));
    nodes.push(Node::Glue {
        spec: par_fill,
        kind: GlueKind::ParFillSkip,
        leader: None,
    });
    let params = tex_typeset::linebreak::LineBreakParams {
        pretolerance: 10_000,
        tolerance: 10_000,
        line_penalty: 0,
        hyphen_penalty: 50,
        ex_hyphen_penalty: 50,
        adj_demerits: 0,
        double_hyphen_demerits: 0,
        final_hyphen_demerits: 0,
        emergency_stretch: Scaled::from_raw(0),
        looseness: 0,
        last_line_fit: 0,
        pdf_adjust_spacing: 0,
        expansion_steps: None,
        pdf_protrude_chars: 0,
        left_skip: stores.glue(stores.glue_param(GlueParam::LEFT_SKIP)),
        right_skip: stores.glue(stores.glue_param(GlueParam::RIGHT_SKIP)),
        par_fill_skip: stores.glue(par_fill),
        shape: tex_typeset::linebreak::LineShape::natural(Scaled::from_raw(400 * Scaled::UNITY)),
    };
    let nodes_before = stores.testing_epoch_node_count();

    let _ = crate::assignments::test_break_hlist(&mut stores, nodes, params);

    assert_eq!(stores.testing_epoch_node_count(), nodes_before);
}

#[test]
fn pretolerance_memo_hits_and_every_explicit_parameter_changes_its_strong_key() {
    use tex_state::glue::GlueSpec;
    use tex_typeset::linebreak::{LineShape, LineShapeEntry, ParagraphShape};

    let mut stores = super::core::run_canonical_tex82("\\end");
    stores.enable_pure_memo(pretolerance_memo_config());
    let nodes = vec![
        Node::Rule {
            width: Some(Scaled::from_raw(10)),
            height: Some(Scaled::from_raw(5)),
            depth: Some(Scaled::from_raw(0)),
        },
        Node::Penalty(-10_000),
    ];
    let base = tex_typeset::linebreak::LineBreakParams {
        pdf_adjust_spacing: 0,
        expansion_steps: None,
        pdf_protrude_chars: 0,
        pretolerance: 10_000,
        tolerance: 9_999,
        line_penalty: 10,
        hyphen_penalty: 50,
        ex_hyphen_penalty: 51,
        adj_demerits: 52,
        double_hyphen_demerits: 53,
        final_hyphen_demerits: 54,
        emergency_stretch: Scaled::from_raw(55),
        looseness: 0,
        last_line_fit: 56,
        left_skip: GlueSpec::ZERO,
        right_skip: GlueSpec::ZERO,
        par_fill_skip: GlueSpec::ZERO,
        shape: LineShape::natural(Scaled::from_raw(1_000)),
    };

    let first = crate::assignments::test_break_hlist(&mut stores, nodes.clone(), base.clone());
    let second = crate::assignments::test_break_hlist(&mut stores, nodes.clone(), base.clone());
    assert_eq!(first.breaks, second.breaks);
    assert_eq!(stores.pure_memo_stats().hits, 1);

    let base_key = crate::assignments::test_pretolerance_memo_key(&stores, &nodes, &base);
    let mut variants = Vec::new();
    macro_rules! changed {
        ($field:ident, $value:expr) => {{
            let mut params = base.clone();
            params.$field = $value;
            variants.push(params);
        }};
    }
    changed!(pretolerance, 9_998);
    changed!(tolerance, 9_997);
    changed!(line_penalty, 11);
    changed!(hyphen_penalty, 60);
    changed!(ex_hyphen_penalty, 61);
    changed!(adj_demerits, 62);
    changed!(double_hyphen_demerits, 63);
    changed!(final_hyphen_demerits, 64);
    changed!(emergency_stretch, Scaled::from_raw(65));
    changed!(looseness, 1);
    changed!(last_line_fit, 66);
    changed!(pdf_adjust_spacing, 2);
    changed!(pdf_protrude_chars, 2);
    changed!(expansion_steps, Some((10, 5)));
    changed!(
        left_skip,
        GlueSpec {
            width: Scaled::from_raw(1),
            ..GlueSpec::ZERO
        }
    );
    changed!(
        right_skip,
        GlueSpec {
            stretch: Scaled::from_raw(1),
            ..GlueSpec::ZERO
        }
    );
    changed!(
        par_fill_skip,
        GlueSpec {
            shrink: Scaled::from_raw(1),
            ..GlueSpec::ZERO
        }
    );
    let mut shape = base.shape.clone();
    shape.hang_indent = Scaled::from_raw(1);
    variants.push(tex_typeset::linebreak::LineBreakParams {
        shape,
        ..base.clone()
    });
    let mut shape = base.shape.clone();
    shape.hang_after = 2;
    variants.push(tex_typeset::linebreak::LineBreakParams {
        shape,
        ..base.clone()
    });
    let mut shape = base.shape.clone();
    shape.line_offset = 3;
    variants.push(tex_typeset::linebreak::LineBreakParams {
        shape,
        ..base.clone()
    });
    let mut shape = base.shape.clone();
    shape.parshape = Some(ParagraphShape {
        lines: vec![LineShapeEntry {
            indent: Scaled::from_raw(4),
            width: Scaled::from_raw(900),
        }],
    });
    variants.push(tex_typeset::linebreak::LineBreakParams {
        shape,
        ..base.clone()
    });

    for variant in variants {
        assert_ne!(
            crate::assignments::test_pretolerance_memo_key(&stores, &nodes, &variant),
            base_key
        );
    }
}

#[test]
fn malformed_pretolerance_entry_is_rejected_and_recomputed() {
    use tex_state::{DetachedMemoValue, DetachedPureKernelPlan, PureMemoStats};

    let mut stores = super::core::run_canonical_tex82("\\end");
    stores.enable_pure_memo(pretolerance_memo_config());
    let nodes = vec![Node::Penalty(-10_000)];
    let params = tex_typeset::linebreak::LineBreakParams {
        pdf_adjust_spacing: 0,
        expansion_steps: None,
        pdf_protrude_chars: 0,
        pretolerance: 10_000,
        tolerance: 10_000,
        line_penalty: 0,
        hyphen_penalty: 0,
        ex_hyphen_penalty: 0,
        adj_demerits: 0,
        double_hyphen_demerits: 0,
        final_hyphen_demerits: 0,
        emergency_stretch: Scaled::from_raw(0),
        looseness: 0,
        last_line_fit: 0,
        left_skip: tex_state::glue::GlueSpec::ZERO,
        right_skip: tex_state::glue::GlueSpec::ZERO,
        par_fill_skip: tex_state::glue::GlueSpec::ZERO,
        shape: tex_typeset::linebreak::LineShape::natural(Scaled::from_raw(1_000)),
    };
    let key = crate::assignments::test_pretolerance_memo_key(&stores, &nodes, &params);
    let malformed = DetachedMemoValue::from_pure_kernel_plan(&DetachedPureKernelPlan {
        kernel: "line-break-pretolerance".to_owned(),
        plan_schema: 1,
        payload: vec![1, 2, 3],
    })
    .expect("malformed plan envelope");
    stores.insert_pure_memo(key, malformed);

    let result = crate::assignments::test_break_hlist(&mut stores, nodes, params);
    assert!(!result.breaks.is_empty());
    let PureMemoStats { malformed, .. } = stores.pure_memo_stats();
    assert_eq!(malformed, 1);
}

fn run_canonical_paragraph_program(
    source: &str,
    configure: impl FnOnce(&mut Universe),
) -> (Universe, Vec<u8>, usize) {
    run_canonical_paragraph_program_with_profile(source, CommandProfile::TEX82, configure)
}

fn run_canonical_paragraph_program_with_profile(
    source: &str,
    profile: CommandProfile,
    configure: impl FnOnce(&mut Universe),
) -> (Universe, Vec<u8>, usize) {
    let mut stores = stores_with_fonts();
    configure(&mut stores);
    let mut control = match profile {
        CommandProfile::TEX82 => CanonicalMainControl::tex82_initex(&mut stores),
        CommandProfile::ETEX26 => {
            tex_command::install_tex82_expandable_primitives(&mut stores);
            tex_command::install_etex_expandable_primitives(&mut stores);
            install_unexpandable_primitives(&mut stores);
            install_etex_unexpandable_primitives(&mut stores);
            CanonicalMainControl::prepared_initex(profile)
        }
        _ => panic!("paragraph helper supports only TeX82 and e-TeX"),
    };
    let metrics = tex_state::InputReadState::read_input_file(
        &mut tex_state::InputOpenState::input_open_context(&mut stores),
        std::path::Path::new("cmr10.tfm"),
    )
    .expect("seeded cmr10 fixture reads");
    control.capabilities_mut().register_font(
        "cmr10.tfm",
        tex_command::FontResource::Tfm {
            metrics,
            opentype: None,
        },
    );
    control
        .register_root_source(tex_command::SourceRegistration::new(
            tex_command::RegisteredSourceKind::Generated,
            source.as_bytes().to_vec(),
        ))
        .expect("register paragraph source");

    let mut steps = 0;
    loop {
        steps += 1;
        if control.step(&mut stores).expect("canonical paragraph step") == MainControlStep::End {
            break;
        }
        assert!(
            steps < 65_536,
            "canonical paragraph source did not terminate"
        );
    }

    let mut dvi = tex_out::dvi::DviStreamWriter::new(Vec::new());
    for page in control.take_prepared_dvi_pages() {
        dvi.write_page_plan(&page.into_plan()).expect("DVI page");
    }
    (stores, dvi.finish().expect("DVI finish"), steps)
}

#[test]
fn enabled_pretolerance_memo_preserves_end_to_end_state_effects_and_dvi() {
    fn run(
        enabled: bool,
    ) -> (
        usize,
        Vec<u8>,
        u64,
        Vec<EffectRecord>,
        tex_state::PureMemoStats,
    ) {
        let source = r"\hsize=20pt \pretolerance=10000
            identical paragraph text\par
            \prevgraf=0 \interlinepenalty=111 \clubpenalty=222 \widowpenalty=333
            \hbadness=0 \hfuzz=1pt \mag=1200
            identical paragraph text\par
            \prevgraf=0 \language=7 \lefthyphenmin=1 \righthyphenmin=1
            identical paragraph text\par
            \vfill\eject\end";
        let (mut stores, dvi, steps) = run_canonical_paragraph_program(source, |stores| {
            if enabled {
                stores.enable_pure_memo(pretolerance_memo_config());
            }
        });
        let hash = stores.snapshot().state_hash();
        let effects = stores.world().effect_records().to_vec();
        let memo = stores.pure_memo_stats();
        (steps, dvi, hash, effects, memo)
    }

    let (cold_steps, cold_dvi, cold_hash, cold_effects, _) = run(false);
    let (memo_steps, memo_dvi, memo_hash, memo_effects, memo) = run(true);
    assert_eq!(memo_steps, cold_steps);
    assert_eq!(memo_dvi, cold_dvi);
    assert_eq!(memo_hash, cold_hash);
    assert_eq!(memo_effects, cold_effects);
    assert!(memo.hits >= 1, "expected the repeated paragraph to hit");
    assert!(
        memo.misses >= 2,
        "the initial and language-mutated paragraphs must miss"
    );
}

#[test]
fn direct_batch_paragraphs_do_not_build_incremental_history() {
    fn run(enabled: bool) -> (Vec<u8>, u64, tex_state::PureMemoStats) {
        let source = "\\font\\tenrm=cmr10 \\tenrm repeated literal paragraph text\\par\nrepeated literal paragraph text\\par\nrepeated literal paragraph text\\par\n\\vfill\\eject\\end";
        let (mut stores, bytes, _) = run_canonical_paragraph_program(source, |stores| {
            if enabled {
                stores.enable_pure_memo(tex_state::PureMemoConfig::default());
                stores.enable_paragraph_memo();
            }
        });
        let hash = stores.snapshot().state_hash();
        (bytes, hash, stores.pure_memo_stats())
    }

    let (cold_dvi, cold_hash, _) = run(false);
    let (memo_dvi, memo_hash, stats) = run(true);
    assert_eq!(memo_dvi, cold_dvi);
    assert_eq!(memo_hash, cold_hash);
    assert_eq!(stats.paragraph_hits, 0, "{stats:?}");
    assert_eq!(stats.paragraph_inserts, 0, "{stats:?}");
    assert_eq!(stats.paragraph_commands_skipped, 0);
    assert_eq!(stats.paragraph_eligible_regions, 0, "{stats:?}");
    assert_eq!(
        stats.paragraph_opportunities.published.regions, 0,
        "{stats:?}"
    );
}

#[test]
fn paragraph_front_end_replays_validated_count_mutations() {
    fn run(enabled: bool) -> (i32, i32, Vec<u8>, tex_state::PureMemoStats) {
        let paragraph =
            "\\count5=41 \\global\\count6=9 \\language=7 stateful paragraph text\\par\n";
        let source = format!("{paragraph}{paragraph}{paragraph}{paragraph}\\vfill\\eject\\end");
        let (stores, dvi, _) = run_canonical_paragraph_program(&source, |stores| {
            if enabled {
                stores.enable_pure_memo(tex_state::PureMemoConfig::default());
                stores.enable_paragraph_memo();
            }
        });
        (
            stores.count(5),
            stores.count(6),
            dvi,
            stores.pure_memo_stats(),
        )
    }

    let (cold_local, cold_global, cold_dvi, _) = run(false);
    let (memo_local, memo_global, memo_dvi, stats) = run(true);
    assert_eq!((memo_local, memo_global), (cold_local, cold_global));
    assert_eq!(memo_dvi, cold_dvi);
    assert_eq!(stats.paragraph_hits, 0, "{stats:?}");
    assert_eq!(stats.paragraph_mutations_replayed, 0, "{stats:?}");
    assert_eq!(stats.paragraph_eligible_regions, 0, "{stats:?}");
}

#[test]
fn grouped_paragraph_redo_preserves_local_and_global_assignment_scope() {
    let local = "{\\count5=41 grouped local text\\par}\n";
    let global = "{\\global\\count6=9 grouped global text\\par}\n";
    let source = format!("{local}{local}{local}{global}{global}{global}\\vfill\\eject\\end");
    let (stores, _, _) = run_canonical_paragraph_program(&source, |stores| {
        stores.enable_pure_memo(tex_state::PureMemoConfig::default());
        stores.enable_paragraph_memo();
    });
    assert_eq!(
        stores.count(5),
        0,
        "local replay must unwind with its group"
    );
    assert_eq!(stores.count(6), 9, "global replay must survive group exit");
    let stats = stores.pure_memo_stats();
    assert_eq!(stats.paragraph_hits, 0, "{stats:?}");
    assert_eq!(stats.paragraph_mutations_replayed, 0, "{stats:?}");
    assert_eq!(stats.paragraph_eligible_regions, 0, "{stats:?}");
}

#[test]
fn effectful_paragraph_commands_remain_replay_barriers() {
    let paragraph = "\\message{visible}\\advance\\count7 by1 effectful paragraph text\\par\n";
    let source = format!("{paragraph}{paragraph}{paragraph}\\vfill\\eject\\end");
    let (stores, _, _) = run_canonical_paragraph_program(&source, |stores| {
        stores.enable_pure_memo(tex_state::PureMemoConfig::default());
        stores.enable_paragraph_memo();
    });
    assert_eq!(stores.count(7), 3);
    let stats = stores.pure_memo_stats();
    assert_eq!(stats.paragraph_hits, 0);
    assert_eq!(stats.paragraph_eligible_regions, 0, "{stats:?}");
}

#[test]
fn deterministic_message_effects_replay_in_original_order() {
    let paragraph = "\\message{visible}message paragraph text\\par\n";
    let source = format!("{paragraph}{paragraph}{paragraph}\\end");
    let (stores, _, _) = run_canonical_paragraph_program(&source, |stores| {
        stores.enable_pure_memo(tex_state::PureMemoConfig::default());
        stores.enable_paragraph_memo();
    });
    let terminal = stores
        .world()
        .memory_terminal_output()
        .expect("canonical messages reach the terminal in order");
    assert_eq!(
        String::from_utf8_lossy(terminal).matches("visible").count(),
        3
    );
    let stats = stores.pure_memo_stats();
    assert_eq!(stats.paragraph_hits, 0, "{stats:?}");
    assert_eq!(stats.paragraph_eligible_regions, 0, "{stats:?}");
}

#[test]
fn direct_batch_executor_does_not_publish_paragraph_regions() {
    let source = r"\font\tenrm=cmr10 \tenrm
        \def\body{office \accent18 a\discretionary{-}{}{x}}
        \everypar{\message{EP}}
        {\csname body\endcsname \mark{m}\insert0{\hbox{x}}\vadjust{\kern1pt}\par}
        \end";
    let (stores, _, _) = run_canonical_paragraph_program(source, |stores| {
        stores.enable_pure_memo(tex_state::PureMemoConfig::default());
        stores.enable_paragraph_memo();
    });

    let regions = stores.recorded_paragraphs();
    assert!(regions.is_empty(), "{regions:#?}");
}

#[test]
fn direct_batch_executor_does_not_arm_incremental_barrier_tracking() {
    let run = |body: &str| {
        let source = format!("\\font\\tenrm=cmr10 \\tenrm {body}\\end");
        let (stores, _, _) = run_canonical_paragraph_program_with_profile(
            &source,
            CommandProfile::ETEX26,
            |stores| {
                stores.enable_pure_memo(tex_state::PureMemoConfig::default());
                stores.enable_paragraph_memo();
            },
        );
        stores.pure_memo_stats()
    };
    let display = run("display text$$x$$after\\par");
    assert_eq!(display.paragraph_display_math_barriers, 0, "{display:?}");
    let scantokens = run("scanned \\scantokens{more} text\\par");
    assert_eq!(
        scantokens.paragraph_scantokens_barriers, 0,
        "{scantokens:?}"
    );
}

#[test]
fn randomized_pretolerance_cache_differential_matches_disabled_kernel() {
    let mut seed = 0x9e37_79b9_u32;
    let mut source = String::from("\\font\\tenrm=cmr10 \\tenrm ");

    for _case in 0..128 {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let mut paragraph = String::new();
        for index in 0..(8 + seed as usize % 40) {
            if index % 2 == 0 {
                let width = 1 + ((seed >> (index % 16)) as i32 & 31);
                paragraph.push_str(&format!("\\vrule width{width}sp height1sp depth0sp"));
            } else {
                paragraph.push_str("\\hskip4sp plus2sp ");
            }
        }
        let hsize = 30 + seed % 300;
        let tolerance = 1_000 + seed % 9_000;
        let line_penalty = seed % 100;
        let adj_demerits = seed % 1_000;
        let emergency_stretch = seed % 20;
        source.push_str(&format!(
            "\\hsize={hsize}sp \\pretolerance=10000 \\tolerance={tolerance} \
             \\linepenalty={line_penalty} \\hyphenpenalty=50 \\exhyphenpenalty=50 \
             \\adjdemerits={adj_demerits} \\doublehyphendemerits=1000 \
             \\finalhyphendemerits=500 \\emergencystretch={emergency_stretch}sp \
             {paragraph}\\par {paragraph}\\par "
        ));
    }
    source.push_str("\\vfill\\eject\\end");

    let run = |enabled: bool| {
        let (mut stores, dvi, steps) = run_canonical_paragraph_program(&source, |stores| {
            if enabled {
                stores.enable_pure_memo(pretolerance_memo_config());
            }
        });
        let hash = stores.snapshot().state_hash();
        let effects = stores.world().effect_records().to_vec();
        (steps, dvi, hash, effects, stores.pure_memo_stats())
    };

    let (disabled_steps, disabled_dvi, disabled_hash, disabled_effects, _) = run(false);
    let (enabled_steps, enabled_dvi, enabled_hash, enabled_effects, stats) = run(true);
    assert_eq!(enabled_steps, disabled_steps);
    assert_eq!(enabled_dvi, disabled_dvi);
    assert_eq!(enabled_hash, disabled_hash);
    assert_eq!(enabled_effects, disabled_effects);
    assert!(
        stats.hits >= 128,
        "expected one cache hit per repeated case: {stats:?}"
    );
}
