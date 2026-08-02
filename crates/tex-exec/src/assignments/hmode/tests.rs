use super::*;
use crate::mode::PendingHChar;
use crate::{CanonicalMainControl, MainControlStep};
use std::sync::Arc;
use tex_command::{
    CommandObservation, CommandObserver, FontResource, RegisteredSourceKind, SourceRegistration,
};
use tex_lex::MemoryInput;
use tex_state::hyphenation::ExceptionSpec;
use tex_state::node::Node;
use tex_state::provenance::SyntheticOriginKind;
use tex_state::token::TracedTokenWord;
use tex_state::{EffectRecord, PrintSink};

fn canonical_control_with_cmr10(stores: &mut Universe, source: &str) -> CanonicalMainControl {
    const CMR10: &[u8] = include_bytes!("../../../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
    let mut control = CanonicalMainControl::tex82_initex(stores);
    stores
        .world_mut()
        .set_memory_file("cmr10.tfm", CMR10.to_vec())
        .expect("seed cmr10");
    let metrics = tex_state::InputReadState::read_input_file(
        &mut stores.input_open_context(),
        std::path::Path::new("cmr10.tfm"),
    )
    .expect("read cmr10 fixture");
    control.capabilities_mut().register_font(
        "cmr10.tfm",
        FontResource::Tfm {
            metrics,
            opentype: None,
        },
    );
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(source.as_bytes()),
        ))
        .expect("register canonical source");
    control
}

fn run_canonical_to_input_end(control: &mut CanonicalMainControl, stores: &mut Universe) {
    loop {
        match control.step(stores).expect("canonical program executes") {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }
}

#[test]
fn legacy_tex82_section_581_warns_only_for_positive_tracing_lost_chars() {
    let warning = "Missing character: There is no Z in font nullfont!\n";
    for tracing_lost_chars in [-1, 0, 1] {
        for tracing_online in [-1, 0, 1] {
            let mut stores = Universe::with_world(tex_state::World::memory()).with_plain_catcodes();
            stores.set_int_param(IntParam::TRACING_LOST_CHARS, tracing_lost_chars);
            stores.set_int_param(IntParam::TRACING_ONLINE, tracing_online);

            let nullfont = stores.current_font();
            crate::diagnostics::report_missing_character_warning(&mut stores, nullfont, 'Z', false);

            let terminal: String = stores
                .world()
                .effect_records()
                .iter()
                .filter_map(|effect| match effect {
                    EffectRecord::StreamWrite {
                        sink: PrintSink::Terminal | PrintSink::TerminalAndLog,
                        text,
                    } => Some(text.as_str()),
                    _ => None,
                })
                .collect();
            let transcript: String = stores
                .world()
                .effect_records()
                .iter()
                .filter_map(|effect| match effect {
                    EffectRecord::StreamWrite {
                        sink: PrintSink::Log | PrintSink::TerminalAndLog,
                        text,
                    } => Some(text.as_str()),
                    _ => None,
                })
                .collect();
            // tex.web §581 says `if tracing_lost_chars>0 then`.
            let warns = tracing_lost_chars > 0;
            assert_eq!(
                terminal.matches(warning).count(),
                usize::from(warns && tracing_online > 0),
                "\\tracinglostchars={tracing_lost_chars}, \\tracingonline={tracing_online}"
            );
            assert_eq!(
                transcript.matches(warning).count(),
                usize::from(warns),
                "\\tracinglostchars={tracing_lost_chars}, \\tracingonline={tracing_online}"
            );
            assert_eq!(
                terminal.matches("nullfont!\n\n").count()
                    + transcript.matches("nullfont!\n\n").count(),
                0,
                "§581 ends the warning with exactly one newline"
            );
        }
    }
}

#[test]
fn non_character_accent_lookahead_replays_the_original_traced_token() {
    let mut stores = Universe::new_with_plain_catcodes();
    crate::install_unexpandable_primitives(&mut stores);
    let origin = stores.synthetic_origin(SyntheticOriginKind::Test);
    let closing_group = TracedTokenWord::pack(
        Token::Char {
            ch: '}',
            cat: Catcode::EndGroup,
        },
        origin,
    );
    let mut input = InputStack::new(MemoryInput::new(""));
    push_traced_tokens(&mut input, &mut stores, [closing_group]);

    let base = scan_accent_base(
        &mut ModeNest::new(),
        &mut input,
        &mut stores,
        &mut crate::ExecutionContext::new("texput"),
        TracedTokenWord::pack(
            Token::Char {
                ch: '^',
                cat: Catcode::Other,
            },
            OriginId::UNKNOWN,
        ),
    )
    .expect("accent lookahead should recover");

    assert_eq!(base, None);
    let summary = input.summary();
    let mut resumed = InputStack::from_summary(&summary, |_, _, _| {
        Ok::<_, core::convert::Infallible>(MemoryInput::new(""))
    })
    .expect("pushed-back token should be checkpoint-resumable");
    let replayed = resumed
        .next_traced_token(&mut stores)
        .expect("read replayed token")
        .expect("closing group should be backed up");
    assert_eq!(replayed, closing_group);
}

#[test]
fn accent_lookahead_runs_assignments_and_accepts_char_num() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = crate::CanonicalMainControl::tex82_initex(&mut stores);
    let source = control
        .command_mut()
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(br"\accent19 \count0=7 \char65\end".as_slice()),
        ))
        .expect("accent source registers");
    control
        .command_mut()
        .open_registered_source(source)
        .expect("accent source opens");
    let mut recorder = AccentObservationRecorder::default();
    loop {
        match control
            .step_with_observer(&mut stores, &mut recorder)
            .expect("accent source executes")
        {
            crate::MainControlStep::End | crate::MainControlStep::EndOfInput => break,
            crate::MainControlStep::Continue => {}
        }
    }

    assert_eq!(stores.count(0), 7);
    let meanings = recorder
        .0
        .iter()
        .filter_map(|observation| match observation {
            CommandObservation::Command(command)
                if matches!(command.command.as_str(), "accent" | "register" | "char_num") =>
            {
                Some(command.command.as_str())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        meanings,
        [
            "accent", "accent", "accent", "accent", "register", "register", "char_num", "char_num",
        ],
        "raw and expanded meaning observations retain their canonical count and order"
    );
    assert!(
        recorder.0.iter().any(|observation| matches!(
            observation,
            CommandObservation::Scanner(scanner)
                if scanner.kind == "integer" && scanner.value == "65"
        )),
        "the accent base character scan is observed"
    );
    assert!(
        recorder.0.iter().any(|observation| matches!(
        observation,
        CommandObservation::Mutation(mutation)
            if mutation.target == "register" && mutation.value == "count:0=7"
        )),
        "the lookahead assignment is observed"
    );
}

#[derive(Default)]
struct AccentObservationRecorder(Vec<CommandObservation>);

impl CommandObserver for AccentObservationRecorder {
    fn committed(&mut self, observation: CommandObservation) {
        self.0.push(observation);
    }
}

#[test]
fn sentence_space_factor_does_not_jump_after_an_uppercase_letter() {
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_sfcode('.', 3000);
    let mut nest = ModeNest::new();

    update_space_factor(&mut nest.current_list_mutation(), &stores, 'A');
    assert_eq!(nest.current_list().space_factor(), 999);

    update_space_factor(&mut nest.current_list_mutation(), &stores, '.');
    assert_eq!(nest.current_list().space_factor(), 1000);

    update_space_factor(&mut nest.current_list_mutation(), &stores, 'a');
    update_space_factor(&mut nest.current_list_mutation(), &stores, '.');
    assert_eq!(nest.current_list().space_factor(), 3000);
}

#[test]
fn opentype_cmap_accepts_a_non_byte_horizontal_character() {
    use tex_fonts::{
        AcceptedFontContainers, FontFeaturePolicy, FontLimits, FontMetrics, FontPurposes,
        FontRequest, FontRequestKey, OpenTypeFont, OpenTypeProgramSelection, ResolvedFont,
        VariationSelection, WritingDirection,
    };

    let key = FontRequestKey::new(
        "cmu-serif-roman",
        0,
        VariationSelection::default(),
        FontFeaturePolicy::default(),
    )
    .expect("font key");
    let request = FontRequest {
        key: key.clone(),
        accepted_containers: AcceptedFontContainers::WASM,
        purposes: FontPurposes::LAYOUT_AND_HTML,
    };
    let font = OpenTypeFont::parse(
        &request,
        ResolvedFont {
            request: key,
            container: tex_fonts::FontContainer::Woff2,
            declared_object_sha256: None,
            declared_program_identity: None,
            provenance: None,
            legacy_mapping: None,
            bytes: include_bytes!("../../../../umber-wasm/assets/cmu-serif-500-roman.woff2")
                .to_vec(),
        },
        FontLimits::default(),
    )
    .expect("fixture font");
    let ch = font
        .cmap
        .mappings()
        .keys()
        .copied()
        .find(|scalar| *scalar > u32::from(u8::MAX))
        .and_then(char::from_u32)
        .expect("fixture has a non-byte mapping");
    let size = Scaled::from_raw(10 * Scaled::UNITY);
    let loaded = tex_fonts::LoadedFont::new(
        "cmu-serif",
        "cmu-serif.tfm",
        [0; 32],
        0,
        size,
        size,
        vec![Scaled::from_raw(0); 7],
        FontMetrics::new(Vec::new(), Vec::new(), None, None, Vec::new()),
    )
    .with_opentype(OpenTypeProgramSelection {
        font,
        variation: VariationSelection::default(),
        features: FontFeaturePolicy::default(),
        direction: WritingDirection::LeftToRight,
    });
    let mut stores = Universe::new_with_plain_catcodes();
    let font = stores.intern_font(loaded);
    stores.set_current_font(font);
    let mut nest = ModeNest::new();

    append_hchar(&mut nest, &mut stores, ch, OriginId::UNKNOWN).expect("character appends");
    flush_pending_hchars(
        &mut nest,
        &mut stores,
        tex_command::CommandFuelLedger::default().fuel_mut(),
    )
    .expect("OpenType character flushes");

    assert!(matches!(
        nest.current_list().nodes(),
        [Node::Char { font: actual_font, ch: actual_ch, .. }]
            if *actual_font == font && *actual_ch == ch
    ));
}

#[test]
fn list_commit_flushes_pending_characters_and_raw_pop_rejects_them() {
    let mut stores = Universe::new_with_plain_catcodes();
    let font = opentype_test_font(&mut stores, 10);
    stores.set_current_font(font);
    let mut nest = ModeNest::new();
    nest.push(Mode::RestrictedHorizontal)
        .expect("test mode push");
    append_hchar(&mut nest, &mut stores, 'A', OriginId::UNKNOWN).expect("character appends");

    assert!(matches!(
        nest.pop(),
        Err(ExecError::UncommittedPendingHchars)
    ));
    let level = commit_current_list(
        &mut nest,
        &mut stores,
        tex_command::CommandFuelLedger::default().fuel_mut(),
    )
    .expect("commit flushes before pop");
    assert!(matches!(
        level.list().nodes(),
        [Node::Char { ch: 'A', .. }, ..]
    ));
}

fn opentype_test_font(stores: &mut Universe, points: i32) -> tex_state::ids::FontId {
    use tex_fonts::{
        AcceptedFontContainers, FontFeaturePolicy, FontLimits, FontPurposes, FontRequest,
        FontRequestKey, OpenTypeFont, OpenTypeProgramSelection, ResolvedFont, VariationSelection,
        WritingDirection,
    };

    let features = FontFeaturePolicy::default();
    let key = FontRequestKey::new(
        format!("cmu-serif-shaping-{points}"),
        0,
        VariationSelection::default(),
        features.clone(),
    )
    .expect("font key");
    let font = OpenTypeFont::parse(
        &FontRequest {
            key: key.clone(),
            accepted_containers: AcceptedFontContainers::WASM,
            purposes: FontPurposes::LAYOUT_AND_HTML,
        },
        ResolvedFont {
            request: key,
            container: tex_fonts::FontContainer::Woff2,
            declared_object_sha256: None,
            declared_program_identity: None,
            provenance: None,
            legacy_mapping: None,
            bytes: include_bytes!("../../../../umber-wasm/assets/cmu-serif-500-roman.woff2")
                .to_vec(),
        },
        FontLimits::default(),
    )
    .expect("fixture font");
    let size = Scaled::from_raw(points * Scaled::UNITY);
    stores.intern_font(tex_fonts::LoadedFont::new_opentype(
        "cmu-serif-shaping",
        "cmu-serif-shaping.woff2",
        size,
        size,
        OpenTypeProgramSelection {
            font,
            variation: VariationSelection::default(),
            features,
            direction: WritingDirection::LeftToRight,
        },
    ))
}

#[test]
fn opentype_run_is_batched_and_uses_shaped_cluster_advance() {
    let mut stores = Universe::new_with_plain_catcodes();
    let font = opentype_test_font(&mut stores, 10);
    stores.set_current_font(font);
    let mut nest = ModeNest::new();

    for ch in "ffi".chars() {
        append_hchar(&mut nest, &mut stores, ch, OriginId::UNKNOWN).expect("character appends");
    }
    flush_pending_hchars(
        &mut nest,
        &mut stores,
        tex_command::CommandFuelLedger::default().fuel_mut(),
    )
    .expect("run flushes");

    let nodes = nest.current_list().nodes();
    assert_eq!(
        nodes
            .iter()
            .filter(|node| matches!(node, Node::Char { .. }))
            .count(),
        3
    );
    assert!(nodes.iter().any(|node| matches!(
        node,
        Node::Kern {
            kind: KernKind::Font,
            ..
        }
    )));
    let shaped = tex_shape::shape_run(
        stores.font(font).shaping_font().expect("fixture shapes"),
        "ffi",
        stores
            .font(font)
            .shaping_features()
            .expect("feature policy"),
        tex_shape::Direction::LeftToRight,
    );
    let expected: i32 = shaped
        .glyphs
        .iter()
        .map(|glyph| glyph.x_advance.raw())
        .sum();
    let actual: i32 = nodes
        .iter()
        .map(|node| match node {
            Node::Char { ch, .. } => stores
                .font_character_metrics(font, *ch)
                .expect("mapped character")
                .width
                .raw(),
            Node::Kern { amount, .. } => amount.raw(),
            _ => 0,
        })
        .sum();
    assert_eq!(actual, expected);
}

#[test]
fn long_opentype_run_preserves_every_source_character() {
    let mut stores = Universe::new_with_plain_catcodes();
    let font = opentype_test_font(&mut stores, 10);
    stores.set_current_font(font);
    let mut nest = ModeNest::new();

    for _ in 0..4096 {
        append_hchar(&mut nest, &mut stores, 'a', OriginId::UNKNOWN).expect("character appends");
    }
    flush_pending_hchars(
        &mut nest,
        &mut stores,
        tex_command::CommandFuelLedger::default().fuel_mut(),
    )
    .expect("long run flushes");

    assert_eq!(
        nest.current_list()
            .nodes()
            .iter()
            .filter(|node| matches!(node, Node::Char { ch: 'a', .. }))
            .count(),
        4096,
    );
}

#[test]
fn reshaping_respects_font_kern_glue_and_discretionary_boundaries() {
    let mut stores = Universe::new_with_plain_catcodes();
    let first = opentype_test_font(&mut stores, 10);
    let second = opentype_test_font(&mut stores, 12);
    let empty = stores.freeze_node_list(&[]);
    let glue = stores.glue_param(GlueParam::SPACE_SKIP);
    let boundary_nodes = [
        Node::Kern {
            amount: Scaled::from_raw(17),
            kind: KernKind::Explicit,
        },
        Node::Glue {
            spec: glue,
            kind: GlueKind::Normal,
            leader: None,
        },
        Node::Disc {
            kind: DiscKind::Discretionary,
            pre: empty,
            post: empty,
            replace: empty,
            physical_replace_count: 0,
        },
    ];

    for boundary in boundary_nodes {
        let mut nodes = vec![
            Node::Char {
                font: first,
                ch: 'f',
                origin: OriginId::UNKNOWN,
            },
            boundary.clone(),
            Node::Char {
                font: first,
                ch: 'i',
                origin: OriginId::UNKNOWN,
            },
            Node::Char {
                font: second,
                ch: 'f',
                origin: OriginId::UNKNOWN,
            },
        ];
        reshape_open_type_runs(&stores, &mut nodes);
        let boundary_index = nodes
            .iter()
            .position(|node| node == &boundary)
            .expect("boundary retained");
        assert!(matches!(
            nodes[boundary_index - 1],
            Node::Char { ch: 'f', .. }
        ));
        assert!(
            nodes[boundary_index + 1..]
                .iter()
                .any(|node| matches!(node, Node::Char { ch: 'i', .. }))
        );
    }
}

#[test]
fn flushing_a_character_run_appends_its_right_boundary_kern() {
    use tex_fonts::metrics::CharTag;
    use tex_fonts::{CharMetrics, FontMetrics, LigKernInstruction, LoadedFont};

    let mut characters = vec![None; 256];
    characters[usize::from(b'A')] = Some(CharMetrics {
        width: Scaled::from_raw(Scaled::UNITY),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
        italic_correction: Scaled::from_raw(0),
        tag: CharTag::LigKern {
            program_index: 0,
            start_index: 0,
        },
    });
    let boundary_kern = Scaled::from_raw(12_345);
    let metrics = FontMetrics::new(
        characters,
        vec![LigKernInstruction {
            skip_byte: 128,
            next_char: 255,
            command: Some(LigKernCommand::Kern(boundary_kern)),
        }],
        Some(255),
        None,
        Vec::new(),
    );
    metrics
        .validate()
        .expect("right-boundary test metrics should be valid");
    let mut stores = Universe::new_with_plain_catcodes();
    let font = stores.intern_font(LoadedFont::new(
        "right-boundary-kern",
        "right-boundary-kern.tfm",
        [0; 32],
        0,
        Scaled::from_raw(10 * Scaled::UNITY),
        Scaled::from_raw(10 * Scaled::UNITY),
        vec![Scaled::from_raw(0); 7],
        metrics,
    ));
    let mut nest = ModeNest::new();
    nest.current_list_mutation()
        .begin_pending_hchars(font, 'A', OriginId::UNKNOWN);

    flush_pending_hchars(
        &mut nest,
        &mut stores,
        tex_command::CommandFuelLedger::default().fuel_mut(),
    )
    .expect("character run flushes");

    assert!(matches!(
        nest.current_list().nodes(),
        [
            Node::Char { font: actual_font, ch: 'A', .. },
            Node::Kern { amount, kind: KernKind::Font },
        ] if *actual_font == font && *amount == boundary_kern
    ));
}

#[test]
fn italic_correction_flushes_a_pending_ligature_before_reading_its_metric() {
    use tex_fonts::metrics::CharTag;
    use tex_fonts::{CharMetrics, FontMetrics, LigKernInstruction, LigatureCommand, LoadedFont};

    let italic = Scaled::from_raw(23_456);
    let mut characters = vec![None; 256];
    for (ch, correction, tag) in [
        (
            b'A',
            Scaled::from_raw(0),
            CharTag::LigKern {
                program_index: 0,
                start_index: 0,
            },
        ),
        (b'B', italic, CharTag::None),
    ] {
        characters[usize::from(ch)] = Some(CharMetrics {
            width: Scaled::from_raw(Scaled::UNITY),
            height: Scaled::from_raw(0),
            depth: Scaled::from_raw(0),
            italic_correction: correction,
            tag,
        });
    }
    let metrics = FontMetrics::new(
        characters,
        vec![LigKernInstruction {
            skip_byte: 128,
            next_char: b'A',
            command: Some(LigKernCommand::Ligature(LigatureCommand {
                replacement: b'B',
                delete_current: true,
                delete_next: true,
                pass_over: 0,
            })),
        }],
        None,
        None,
        Vec::new(),
    );
    metrics.validate().expect("synthetic ligature TFM is valid");
    let mut stores = Universe::new_with_plain_catcodes();
    let font = stores.intern_font(LoadedFont::new(
        "italic-ligature",
        "italic-ligature.tfm",
        [0; 32],
        0,
        Scaled::from_raw(10 * Scaled::UNITY),
        Scaled::from_raw(10 * Scaled::UNITY),
        vec![Scaled::from_raw(0); 7],
        metrics,
    ));
    let first_origin = stores.synthetic_origin(SyntheticOriginKind::Test);
    let second_origin = stores.synthetic_origin(SyntheticOriginKind::Test);
    let mut nest = ModeNest::new();
    append_pending_hchar(
        &mut nest.current_list_mutation(),
        &mut stores,
        Mode::RestrictedHorizontal,
        font,
        false,
        'A',
        first_origin,
    );
    append_pending_hchar(
        &mut nest.current_list_mutation(),
        &mut stores,
        Mode::RestrictedHorizontal,
        font,
        false,
        'A',
        second_origin,
    );

    append_italic_correction(
        &mut nest,
        &mut stores,
        tex_command::CommandFuelLedger::default().fuel_mut(),
    )
    .expect("italic correction appends");

    assert!(matches!(
        nest.current_list().nodes(),
        [
            Node::Lig {
                font: actual_font,
                ch: 'B',
                orig,
                origins,
                ..
            },
            Node::Kern {
                amount,
                kind: KernKind::Explicit,
            },
        ] if *actual_font == font
            && orig.as_ref() == ['A', 'A']
            && origins.as_ref() == [first_origin, second_origin]
            && *amount == italic
    ));
}

#[test]
fn right_boundary_kern_prevents_a_following_italic_correction() {
    use tex_fonts::metrics::CharTag;
    use tex_fonts::{CharMetrics, FontMetrics, LigKernInstruction, LoadedFont};

    let boundary_kern = Scaled::from_raw(12_345);
    let italic = Scaled::from_raw(54_321);
    let mut characters = vec![None; 256];
    characters[usize::from(b'A')] = Some(CharMetrics {
        width: Scaled::from_raw(Scaled::UNITY),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
        italic_correction: italic,
        tag: CharTag::LigKern {
            program_index: 0,
            start_index: 0,
        },
    });
    let metrics = FontMetrics::new(
        characters,
        vec![LigKernInstruction {
            skip_byte: 128,
            next_char: 255,
            command: Some(LigKernCommand::Kern(boundary_kern)),
        }],
        Some(255),
        None,
        Vec::new(),
    );
    metrics.validate().expect("synthetic boundary TFM is valid");
    let mut stores = Universe::new_with_plain_catcodes();
    let font = stores.intern_font(LoadedFont::new(
        "italic-boundary",
        "italic-boundary.tfm",
        [0; 32],
        0,
        Scaled::from_raw(10 * Scaled::UNITY),
        Scaled::from_raw(10 * Scaled::UNITY),
        vec![Scaled::from_raw(0); 7],
        metrics,
    ));
    let mut nest = ModeNest::new();
    nest.current_list_mutation()
        .begin_pending_hchars(font, 'A', OriginId::UNKNOWN);

    append_italic_correction(
        &mut nest,
        &mut stores,
        tex_command::CommandFuelLedger::default().fuel_mut(),
    )
    .expect("italic correction appends");

    assert!(
        matches!(
            nest.current_list().nodes(),
            [
                Node::Char { ch: 'A', .. },
                Node::Kern {
                    amount: boundary,
                    kind: KernKind::Font,
                },
            ] if *boundary == boundary_kern
        ),
        "{:?}",
        nest.current_list().nodes()
    );
}

#[test]
fn batched_tfm_run_records_an_absolute_insertion_index() {
    use tex_fonts::{CharMetrics, FontMetrics, LoadedFont};

    let mut characters = vec![None; 256];
    characters[usize::from(b'A')] = Some(CharMetrics {
        width: Scaled::from_raw(Scaled::UNITY),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
        italic_correction: Scaled::from_raw(0),
        tag: tex_fonts::metrics::CharTag::None,
    });
    let mut stores = Universe::new_with_plain_catcodes();
    let font = stores.intern_font(LoadedFont::new(
        "batched-tfm",
        "batched-tfm.tfm",
        [0; 32],
        0,
        Scaled::from_raw(10 * Scaled::UNITY),
        Scaled::from_raw(10 * Scaled::UNITY),
        vec![Scaled::from_raw(0); 7],
        FontMetrics::new(characters, Vec::new(), None, None, Vec::new()),
    ));
    let emitted = [
        Node::Kern {
            amount: Scaled::from_raw(1),
            kind: KernKind::Explicit,
        },
        Node::Kern {
            amount: Scaled::from_raw(2),
            kind: KernKind::Explicit,
        },
    ];
    let mut pending = None;

    assert!(append_tfm_hchar(
        &mut pending,
        &mut stores,
        font,
        'A',
        OriginId::UNKNOWN,
        emitted.len(),
    ));

    assert_eq!(pending.expect("pending TFM run").insertion_index, 2);
}

#[test]
fn accent_delta_rounds_half_scaled_points_like_tex82() {
    assert_eq!(
        tex_state::scaled::text_accent_delta(
            Scaled::from_raw(10),
            Scaled::from_raw(1),
            Scaled::from_raw(0),
            Scaled::from_raw(0),
            Scaled::from_raw(0),
            Scaled::from_raw(0),
        ),
        Scaled::from_raw(5)
    );
}

#[test]
fn paragraph_leading_accent_is_replayed_after_entering_horizontal_mode() {
    let mut stores = Universe::with_world(tex_state::World::memory()).with_plain_catcodes();
    let mut control =
        canonical_control_with_cmr10(&mut stores, "\\font\\f=cmr10 \\relax \\f \\accent19 E");
    run_canonical_to_input_end(&mut control, &mut stores);

    assert_eq!(control.current_mode(), crate::Mode::Horizontal);
    let nodes = control.current_list().nodes();
    assert!(
        matches!(
            nodes,
            [
                Node::HList(_),
                Node::Kern {
                    kind: KernKind::Accent,
                    ..
                },
                Node::HList(_),
                Node::Kern {
                    kind: KernKind::Accent,
                    ..
                },
                Node::Char { ch: 'E', .. },
                ..
            ]
        ),
        "unexpected paragraph-leading accent nodes: {nodes:?}"
    );
    let Node::HList(accent_box) = &nodes[2] else {
        unreachable!("matched shifted accent box")
    };
    assert!(matches!(
        stores.nodes(accent_box.children).testing_decoded(),
        [Node::Char { ch, .. }] if *ch == char::from(19)
    ));
}

#[test]
fn unrestricted_reconstitution_inserts_null_disc_after_font_hyphen() {
    let mut stores = Universe::with_world(tex_state::World::memory()).with_plain_catcodes();
    let mut control = canonical_control_with_cmr10(&mut stores, "\\font\\f=cmr10 \\relax \\f");
    run_canonical_to_input_end(&mut control, &mut stores);
    let font = stores.current_font();
    stores.set_font_hyphen_char(font, i32::from(b'-'));
    let pending: Vec<_> = "in-line"
        .chars()
        .map(|ch| PendingHChar {
            font,
            ch,
            origin: tex_state::token::OriginId::UNKNOWN,
        })
        .collect();

    let unrestricted = reconstitute(&mut stores, &pending, false, true);
    let restricted = reconstitute(&mut stores, &pending, false, false);

    assert!(matches!(
        unrestricted.as_slice(),
        [
            Node::Char { ch: 'i', .. },
            Node::Char { ch: 'n', .. },
            Node::Char { ch: '-', .. },
            Node::Disc {
                kind: DiscKind::ExplicitHyphen,
                ..
            },
            Node::Char { ch: 'l', .. },
            Node::Char { ch: 'i', .. },
            Node::Char { ch: 'n', .. },
            Node::Char { ch: 'e', .. },
        ]
    ));
    assert!(
        !restricted
            .iter()
            .any(|node| matches!(node, Node::Disc { .. }))
    );
}

#[test]
fn literal_hyphen_omits_discretionary_in_restricted_horizontal_mode() {
    // TeX82 §1035 inserts the null discretionary after a font hyphen only
    // when `mode>0`; restricted horizontal mode has the negative mode value.
    let mut stores = Universe::with_world(tex_state::World::memory()).with_plain_catcodes();
    let mut control = canonical_control_with_cmr10(&mut stores, "\\font\\f=cmr10 \\relax \\f");
    run_canonical_to_input_end(&mut control, &mut stores);
    let font = stores.current_font();
    stores.set_font_hyphen_char(font, i32::from(b'-'));
    let mut nest = ModeNest::new();
    nest.push(Mode::RestrictedHorizontal)
        .expect("restricted hmode");

    append_canonical_character(&mut nest, &mut stores, '-', OriginId::UNKNOWN)
        .expect("literal hyphen append");
    flush_pending_hchars(
        &mut nest,
        &mut stores,
        tex_command::CommandFuelLedger::default().fuel_mut(),
    )
    .expect("restricted hlist flush");

    assert!(matches!(
        nest.current_list().nodes(),
        [Node::Char { ch: '-', .. }]
    ));
}

#[test]
fn hyphenation_inside_ff_ligature_preserves_the_unbroken_ligature() {
    let mut stores = Universe::with_world(tex_state::World::memory()).with_plain_catcodes();
    let mut control = canonical_control_with_cmr10(&mut stores, "\\font\\f=cmr10 \\relax \\f");
    run_canonical_to_input_end(&mut control, &mut stores);
    stores.add_hyphenation_exception(ExceptionSpec {
        word: "difference".to_owned(),
        positions: vec![3],
    });
    let font = stores.current_font();
    stores.set_font_hyphen_char(font, i32::from(b'-'));
    let nodes: Vec<_> = "difference"
        .chars()
        .map(|ch| Node::Char {
            font,
            ch,
            origin: tex_state::token::OriginId::UNKNOWN,
        })
        .collect();

    let hyphenated = super::super::hyphenation::test_hyphenated_word(&mut stores, &nodes);
    let disc = hyphenated
        .iter()
        .find_map(|node| match node {
            Node::Disc {
                pre, post, replace, ..
            } => Some((*pre, *post, *replace)),
            _ => None,
        })
        .expect("the exception should create a discretionary");

    assert!(matches!(
        stores.nodes(disc.2).testing_decoded(),
        [Node::Lig {
            ch: '\u{b}',
            orig,
            ..
        }] if orig == &['f', 'f']
    ));
    assert!(
        matches!(
            stores.nodes(disc.0).testing_decoded(),
            [Node::Char { ch: 'f', .. }, Node::Char { ch: '-', .. }]
        ),
        "unexpected pre-break nodes: {:?}",
        stores.nodes(disc.0).testing_decoded()
    );
    assert!(matches!(
        stores.nodes(disc.1).testing_decoded(),
        [Node::Char { ch: 'f', .. }]
    ));
}

#[test]
fn composite_rechar_keeps_ligature_provenance_when_emitted() {
    let current = PendingHRunChar {
        font: tex_state::ids::FontId::testing_new(7),
        ch: 'A',
        orig: vec!['B'].into(),
        origins: vec![tex_state::token::OriginId::UNKNOWN].into(),
        ligature_present: true,
        left_hit: false,
        right_hit: false,
    };

    assert!(matches!(
        rechar_node(current.clone()),
        Node::Lig {
            font,
            ch: 'A',
            orig,
            ..
        } if font == current.font && orig == ['B']
    ));
}

#[test]
fn arbitrary_chained_ligature_keeps_complete_source_provenance() {
    use tex_fonts::metrics::CharTag;
    use tex_fonts::{CharMetrics, FontMetrics, LigKernInstruction, LigatureCommand, LoadedFont};

    let mut characters = vec![None; 256];
    characters[usize::from(b'A')] = Some(CharMetrics {
        width: Scaled::from_raw(Scaled::UNITY),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
        italic_correction: Scaled::from_raw(0),
        tag: CharTag::LigKern {
            program_index: 0,
            start_index: 0,
        },
    });
    let metrics = FontMetrics::new(
        characters,
        vec![LigKernInstruction {
            skip_byte: 128,
            next_char: b'A',
            command: Some(LigKernCommand::Ligature(LigatureCommand {
                replacement: b'A',
                delete_current: true,
                delete_next: true,
                pass_over: 0,
            })),
        }],
        None,
        None,
        Vec::new(),
    );
    metrics
        .validate()
        .expect("test font metrics should be valid");
    let mut stores = Universe::new_with_plain_catcodes();
    let font = stores.intern_font(LoadedFont::new(
        "same-glyph-ligature",
        "same-glyph-ligature.tfm",
        [0; 32],
        0,
        Scaled::from_raw(10 * Scaled::UNITY),
        Scaled::from_raw(10 * Scaled::UNITY),
        vec![Scaled::from_raw(0); 7],
        metrics,
    ));
    stores.set_current_font(font);
    let mut nest = ModeNest::new();
    nest.push(Mode::RestrictedHorizontal).expect("test hmode");
    let origins = [
        stores.synthetic_origin(SyntheticOriginKind::Test),
        stores.synthetic_origin(SyntheticOriginKind::Test),
        stores.synthetic_origin(SyntheticOriginKind::Test),
    ];
    for origin in origins {
        append_canonical_character(&mut nest, &mut stores, 'A', origin)
            .expect("public pending-character append");
    }
    flush_pending_hchars(
        &mut nest,
        &mut stores,
        tex_command::CommandFuelLedger::default().fuel_mut(),
    )
    .expect("public pending-character flush");

    assert!(matches!(
        nest.current_list().nodes(),
        [Node::Lig {
            ch: 'A',
            orig,
            origins: actual_origins,
            ..
        }] if orig == &['A', 'A', 'A'] && actual_origins == &origins
    ));
}

#[test]
fn retained_left_boundary_ligature_reenters_the_lig_kern_program() {
    use tex_fonts::metrics::CharTag;
    use tex_fonts::{
        CharMetrics, FontMetrics, LigKernCommand, LigKernInstruction, LigatureCommand, LoadedFont,
    };

    let mut characters = vec![None; 256];
    for (code, tag) in [
        (b'1', CharTag::None),
        (
            b'5',
            CharTag::LigKern {
                program_index: 1,
                start_index: 1,
            },
        ),
    ] {
        characters[usize::from(code)] = Some(CharMetrics {
            width: Scaled::from_raw(0),
            height: Scaled::from_raw(0),
            depth: Scaled::from_raw(0),
            italic_correction: Scaled::from_raw(0),
            tag,
        });
    }
    let boundary_kern = Scaled::from_raw(-131_073);
    let metrics = FontMetrics::new(
        characters,
        vec![
            LigKernInstruction {
                skip_byte: 128,
                next_char: b'1',
                command: Some(LigKernCommand::Ligature(LigatureCommand {
                    replacement: b'5',
                    delete_current: false,
                    delete_next: true,
                    pass_over: 0,
                })),
            },
            LigKernInstruction {
                skip_byte: 128,
                next_char: b'1',
                command: Some(LigKernCommand::Kern(boundary_kern)),
            },
        ],
        None,
        Some(0),
        Vec::new(),
    );
    metrics.validate().expect("synthetic retained-ligature TFM");
    let mut stores = Universe::new_with_plain_catcodes();
    let font = stores.intern_font(LoadedFont::new(
        "retained-boundary",
        "retained-boundary.tfm",
        [0; 32],
        0,
        Scaled::from_raw(10 * Scaled::UNITY),
        Scaled::from_raw(10 * Scaled::UNITY),
        vec![Scaled::from_raw(0); 7],
        metrics,
    ));
    stores.set_current_font(font);
    let mut nest = ModeNest::new();
    nest.push(Mode::RestrictedHorizontal).expect("test hmode");
    append_canonical_character(&mut nest, &mut stores, '1', OriginId::UNKNOWN)
        .expect("public pending-character append");
    append_canonical_character(&mut nest, &mut stores, '1', OriginId::UNKNOWN)
        .expect("public pending-character append");
    let mut fuel = tex_command::CommandFuelLedger::default();
    flush_pending_hchars_without_right_boundary(&mut nest, &mut stores, fuel.fuel_mut())
        .expect("public flush completes");
    let nodes = nest.current_list().nodes();

    assert!(matches!(
        nodes,
        [
            Node::Lig { ch: '5', orig, .. },
            Node::Kern { amount, kind: KernKind::Font },
            Node::Char { ch: '1', .. },
        ] if orig == &['1'] && *amount == boundary_kern
    ));
}

#[test]
fn missing_glyph_terminates_the_live_ligature_run() {
    // TeX82 §1034 leaves main_loop when the current font lacks a character;
    // the next surviving character therefore begins a new ligature run.
    use tex_fonts::metrics::CharTag;
    use tex_fonts::{FontMetrics, LigKernCommand, LigKernInstruction, LigatureCommand, LoadedFont};

    let mut characters = vec![None; 256];
    for (code, tag) in [
        (
            b'A',
            CharTag::LigKern {
                program_index: 0,
                start_index: 0,
            },
        ),
        (b'B', CharTag::None),
        (b'C', CharTag::None),
    ] {
        characters[usize::from(code)] = Some(tex_fonts::CharMetrics {
            width: Scaled::from_raw(0),
            height: Scaled::from_raw(0),
            depth: Scaled::from_raw(0),
            italic_correction: Scaled::from_raw(0),
            tag,
        });
    }
    let metrics = FontMetrics::new(
        characters,
        vec![LigKernInstruction {
            skip_byte: 128,
            next_char: b'B',
            command: Some(LigKernCommand::Ligature(LigatureCommand {
                replacement: b'C',
                delete_current: true,
                delete_next: true,
                pass_over: 0,
            })),
        }],
        None,
        None,
        Vec::new(),
    );
    metrics.validate().expect("synthetic missing-glyph TFM");
    let mut stores = Universe::new_with_plain_catcodes();
    let font = stores.intern_font(LoadedFont::new(
        "missing-boundary",
        "missing-boundary.tfm",
        [0; 32],
        0,
        Scaled::from_raw(10 * Scaled::UNITY),
        Scaled::from_raw(10 * Scaled::UNITY),
        vec![Scaled::from_raw(0); 7],
        metrics,
    ));
    stores.set_current_font(font);
    let mut nest = ModeNest::new();
    nest.push(Mode::RestrictedHorizontal).expect("test hmode");
    for ch in ['A', 'X', 'B'] {
        append_canonical_character(&mut nest, &mut stores, ch, OriginId::UNKNOWN)
            .expect("append test character");
    }
    let mut fuel = tex_command::CommandFuelLedger::default();
    flush_pending_hchars_without_right_boundary(&mut nest, &mut stores, fuel.fuel_mut())
        .expect("flush test run");

    assert!(matches!(
        nest.current_list().nodes(),
        [Node::Char { ch: 'A', .. }, Node::Char { ch: 'B', .. }]
    ));
    assert!(matches!(
        nest.current_list().physical_nodes(),
        [Node::Char { ch: 'A', .. }, Node::Char { ch: 'B', .. }]
    ));
}

fn ligature_test_font(
    programs: &[(u8, u8, tex_fonts::LigatureCommand)],
    left_boundary: Option<(u8, tex_fonts::LigatureCommand)>,
    right_boundary: Option<(u8, tex_fonts::LigatureCommand)>,
) -> (Universe, FontId) {
    use tex_fonts::metrics::CharTag;
    use tex_fonts::{CharMetrics, FontMetrics, LigKernCommand, LigKernInstruction, LoadedFont};

    let mut characters = vec![None; 256];
    for code in b'A'..=b'Z' {
        characters[usize::from(code)] = Some(CharMetrics {
            width: Scaled::from_raw(0),
            height: Scaled::from_raw(0),
            depth: Scaled::from_raw(0),
            italic_correction: Scaled::from_raw(0),
            tag: CharTag::None,
        });
    }
    let mut instructions = Vec::new();
    let boundary_start = left_boundary.map(|(right, command)| {
        let index = instructions.len() as u16;
        instructions.push(LigKernInstruction {
            skip_byte: 128,
            next_char: right,
            command: Some(LigKernCommand::Ligature(command)),
        });
        index
    });
    for &(left, right, command) in programs {
        let index = instructions.len() as u16;
        characters[usize::from(left)]
            .as_mut()
            .expect("program character")
            .tag = CharTag::LigKern {
            program_index: u8::try_from(index).expect("short test program"),
            start_index: index,
        };
        instructions.push(LigKernInstruction {
            skip_byte: 128,
            next_char: right,
            command: Some(LigKernCommand::Ligature(command)),
        });
    }
    let boundary_char = right_boundary.map(|(left, command)| {
        let index = instructions.len() as u16;
        characters[usize::from(left)]
            .as_mut()
            .expect("right-boundary character")
            .tag = CharTag::LigKern {
            program_index: u8::try_from(index).expect("short test program"),
            start_index: index,
        };
        instructions.push(LigKernInstruction {
            skip_byte: 128,
            next_char: 255,
            command: Some(LigKernCommand::Ligature(command)),
        });
        255
    });
    let metrics = FontMetrics::new(
        characters,
        instructions,
        boundary_char,
        boundary_start,
        Vec::new(),
    );
    metrics.validate().expect("synthetic ligature program");
    let mut stores = Universe::new_with_plain_catcodes();
    let font = stores.intern_font(LoadedFont::new(
        "ligature-machine",
        "ligature-machine.tfm",
        [0; 32],
        0,
        Scaled::from_raw(10 * Scaled::UNITY),
        Scaled::from_raw(10 * Scaled::UNITY),
        vec![Scaled::from_raw(0); 7],
        metrics,
    ));
    (stores, font)
}

fn ligature_command(op_byte: u8, replacement: u8) -> tex_fonts::LigatureCommand {
    tex_fonts::LigatureCommand {
        replacement,
        delete_current: op_byte & 2 == 0,
        delete_next: op_byte & 1 == 0,
        pass_over: op_byte >> 2,
    }
}

fn false_boundary_test_font() -> (Universe, FontId) {
    use tex_fonts::metrics::CharTag;
    use tex_fonts::{CharMetrics, FontMetrics, LigKernCommand, LigKernInstruction, LoadedFont};

    let zero = Scaled::from_raw(0);
    let mut characters = vec![None; 256];
    characters[usize::from(b'A')] = Some(CharMetrics {
        width: zero,
        height: zero,
        depth: zero,
        italic_correction: zero,
        tag: CharTag::LigKern {
            program_index: 0,
            start_index: 0,
        },
    });
    characters[usize::from(b'C')] = Some(CharMetrics {
        width: zero,
        height: zero,
        depth: zero,
        italic_correction: zero,
        tag: CharTag::None,
    });
    let metrics = FontMetrics::new(
        characters,
        vec![LigKernInstruction {
            skip_byte: 128,
            next_char: b'B',
            command: Some(LigKernCommand::Kern(Scaled::from_raw(123))),
        }],
        Some(b'B'),
        None,
        Vec::new(),
    );
    metrics.validate().expect("false-boundary metric program");
    let mut stores = Universe::new_with_plain_catcodes();
    let font = stores.intern_font(LoadedFont::new(
        "false-boundary",
        "false-boundary.tfm",
        [0; 32],
        0,
        Scaled::from_raw(10 * Scaled::UNITY),
        Scaled::from_raw(10 * Scaled::UNITY),
        vec![Scaled::from_raw(0); 7],
        metrics,
    ));
    stores.set_current_font(font);
    (stores, font)
}

#[test]
fn public_flush_suppresses_false_boundary_pair_without_consuming_lookahead() {
    let (mut stores, font) = false_boundary_test_font();
    stores.set_int_param(IntParam::PDF_APPEND_KERN, 1);
    stores.set_int_param(IntParam::PDF_PREPEND_KERN, 1);
    stores.set_pdf_font_code(tex_state::PdfFontCode::Knac, font, b'B', 100);
    stores.set_pdf_font_code(tex_state::PdfFontCode::Knbc, font, b'B', 100);
    let mut nest = ModeNest::new();
    nest.push(Mode::RestrictedHorizontal).expect("test hmode");
    let left = stores.synthetic_origin(SyntheticOriginKind::Test);
    let right = stores.synthetic_origin(SyntheticOriginKind::Test);
    append_canonical_character(&mut nest, &mut stores, 'A', left).expect("left character");
    append_canonical_character(&mut nest, &mut stores, 'B', right).expect("lookahead character");
    let mut fuel = tex_command::CommandFuelLedger::new(16).expect("finite test fuel");

    flush_pending_hchars_with_fuel(&mut nest, &mut stores, fuel.fuel_mut())
        .expect("public flush is fueled");

    assert_eq!(stores.current_font(), font);
    assert!(matches!(
        nest.current_list().nodes(),
        [Node::Char { ch: 'A', origin, .. }] if *origin == left
    ));
}

#[test]
fn false_bchar_terminates_before_its_right_boundary_kern() {
    let (mut stores, _) = false_boundary_test_font();
    let mut nest = ModeNest::new();
    nest.push(Mode::RestrictedHorizontal).expect("test hmode");
    append_canonical_character(&mut nest, &mut stores, 'A', OriginId::UNKNOWN)
        .expect("real character");
    append_canonical_character(&mut nest, &mut stores, 'B', OriginId::UNKNOWN)
        .expect("false bchar lookahead");
    let mut fuel = tex_command::CommandFuelLedger::new(16).expect("finite test fuel");

    flush_pending_hchars_with_fuel(&mut nest, &mut stores, fuel.fuel_mut())
        .expect("false bchar flush");

    assert!(matches!(
        nest.current_list().nodes(),
        [Node::Char { ch: 'A', .. }]
    ));
    assert!(
        !nest.current_list().nodes().iter().any(|node| matches!(
            node,
            Node::Kern {
                kind: KernKind::Font,
                ..
            }
        )),
        "the nonexistent false_bchar must terminate before right-boundary kern lookup"
    );
}

#[test]
fn public_flush_fuel_exhaustion_rolls_back_pending_run() {
    let (mut stores, font) =
        ligature_test_font(&[(b'A', b'B', ligature_command(0, b'C'))], None, None);
    stores.set_current_font(font);
    let mut nest = ModeNest::new();
    nest.push(Mode::RestrictedHorizontal).expect("test hmode");
    append_canonical_character(&mut nest, &mut stores, 'A', OriginId::UNKNOWN)
        .expect("left character");
    append_canonical_character(&mut nest, &mut stores, 'B', OriginId::UNKNOWN)
        .expect("right character");
    let before = nest.current_list().pending_hchars().cloned();
    let mut exhausted = tex_command::CommandFuelLedger::new(1).expect("one transition");

    assert!(matches!(
        flush_pending_hchars_with_fuel(&mut nest, &mut stores, exhausted.fuel_mut()),
        Err(ExecError::Command(
            tex_command::CommandError::FuelExhausted {
                limit: 1,
                burned: 1
            }
        ))
    ));
    assert_eq!(nest.current_list().nodes(), &[]);
    assert_eq!(nest.current_list().pending_hchars(), before.as_ref());

    let mut retry = tex_command::CommandFuelLedger::new(16).expect("retry fuel");
    flush_pending_hchars_with_fuel(&mut nest, &mut stores, retry.fuel_mut())
        .expect("retry commits");
    assert_eq!(node_characters(nest.current_list().nodes()), ['C']);
}

#[test]
fn no_right_boundary_flush_shares_fuel_and_rolls_back_on_exhaustion() {
    let (mut stores, font) =
        ligature_test_font(&[(b'A', b'B', ligature_command(0, b'C'))], None, None);
    stores.set_current_font(font);
    let mut nest = ModeNest::new();
    nest.push(Mode::RestrictedHorizontal).expect("test hmode");
    append_canonical_character(&mut nest, &mut stores, 'A', OriginId::UNKNOWN)
        .expect("left character");
    append_canonical_character(&mut nest, &mut stores, 'B', OriginId::UNKNOWN)
        .expect("right character");
    let before = nest.current_list().pending_hchars().cloned();
    let mut fuel = tex_command::CommandFuelLedger::new(1).expect("one transition");

    assert!(matches!(
        flush_pending_hchars_without_right_boundary(&mut nest, &mut stores, fuel.fuel_mut()),
        Err(ExecError::Command(
            tex_command::CommandError::FuelExhausted { .. }
        ))
    ));
    assert_eq!(fuel.burned(), 1);
    assert!(nest.current_list().nodes().is_empty());
    assert_eq!(nest.current_list().pending_hchars(), before.as_ref());
}

#[test]
fn reconstruction_uses_one_monotonic_caller_ledger() {
    let (mut stores, font) =
        ligature_test_font(&[(b'A', b'B', ligature_command(0, b'C'))], None, None);
    let pending = [
        crate::mode::PendingHChar {
            font,
            ch: 'A',
            origin: OriginId::UNKNOWN,
        },
        crate::mode::PendingHChar {
            font,
            ch: 'B',
            origin: OriginId::UNKNOWN,
        },
    ];
    let mut fuel = tex_command::CommandFuelLedger::new(16).expect("finite ledger");

    let first = reconstitute_with_fuel(&mut stores, &pending, false, false, fuel.fuel_mut())
        .expect("first reconstruction");
    let after_first = fuel.burned();
    let second = reconstitute_with_fuel(&mut stores, &pending, false, false, fuel.fuel_mut())
        .expect("second reconstruction");

    assert_eq!(node_characters(&first), ['C']);
    assert_eq!(second, first);
    assert!(after_first > 0);
    assert_eq!(fuel.burned(), after_first * 2);
}

#[test]
fn detached_session_operations_share_fuel_without_refund_or_partial_commit() {
    let (mut stores, font) =
        ligature_test_font(&[(b'A', b'B', ligature_command(0, b'C'))], None, None);
    stores.set_current_font(font);
    let mut nest = ModeNest::new();
    nest.push(Mode::RestrictedHorizontal).expect("test hmode");
    let mut execution =
        crate::ExecutionContext::new("cumulative-command-fuel").with_command_fuel_limit(3);

    append_canonical_character(&mut nest, &mut stores, 'A', OriginId::UNKNOWN)
        .expect("first left character");
    append_canonical_character(&mut nest, &mut stores, 'B', OriginId::UNKNOWN)
        .expect("first right character");
    flush_pending_hchars_with_fuel(&mut nest, &mut stores, execution.command_fuel())
        .expect("one ligature operation fits the session limit");
    assert_eq!(node_characters(nest.current_list().nodes()), ['C']);

    let (state, input_resolver, font_resolver, image_resolver, recorder) =
        execution.into_owned_parts();
    execution = crate::ExecutionContext::from_owned_state(
        "cumulative-command-fuel",
        state,
        input_resolver,
        font_resolver,
        image_resolver,
        recorder,
    );

    append_canonical_character(&mut nest, &mut stores, 'A', OriginId::UNKNOWN)
        .expect("second left character");
    append_canonical_character(&mut nest, &mut stores, 'B', OriginId::UNKNOWN)
        .expect("second right character");
    let nodes_before = nest.current_list().nodes().to_vec();
    let pending_before = nest.current_list().pending_hchars().cloned();
    let stores_before = stores.snapshot().state_hash();
    let observation_keys_before = execution
        .paragraph_dependency_cache
        .keys()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();

    for operation in ["second", "third"] {
        assert!(
            matches!(
                flush_pending_hchars_with_fuel(&mut nest, &mut stores, execution.command_fuel()),
                Err(ExecError::Command(
                    tex_command::CommandError::FuelExhausted {
                        limit: 3,
                        burned: 3
                    }
                ))
            ),
            "{operation} cumulative operation must exhaust the retained ledger"
        );
        assert_eq!(nest.current_list().nodes(), nodes_before);
        assert_eq!(
            nest.current_list().pending_hchars(),
            pending_before.as_ref(),
            "{operation} failure must retain the pending run"
        );
        assert_eq!(
            stores.snapshot().state_hash(),
            stores_before,
            "{operation} failure must not mutate engine state"
        );
        assert_eq!(
            execution
                .paragraph_dependency_cache
                .keys()
                .copied()
                .collect::<std::collections::BTreeSet<_>>(),
            observation_keys_before,
            "{operation} failure must not publish observations"
        );
    }
}

#[test]
fn public_empty_flush_preserves_list_and_fuel_cardinality() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut nest = ModeNest::new();
    nest.push(Mode::RestrictedHorizontal).expect("test hmode");
    let mut fuel = tex_command::CommandFuelLedger::new(1).expect("finite test fuel");

    flush_pending_hchars_with_fuel(&mut nest, &mut stores, fuel.fuel_mut())
        .expect("empty public flush");

    assert_eq!(fuel.burned(), 0);
    assert!(nest.current_list().nodes().is_empty());
    assert!(nest.current_list().pending_hchars().is_none());
}

#[test]
fn public_flush_places_pdf_auto_kern_before_explicit_hyphen_disc() {
    let mut stores = Universe::with_world(tex_state::World::memory()).with_plain_catcodes();
    let mut control = canonical_control_with_cmr10(&mut stores, "\\font\\f=cmr10 \\relax \\f");
    run_canonical_to_input_end(&mut control, &mut stores);
    let font = stores.current_font();
    stores.set_font_hyphen_char(font, i32::from(b'-'));
    stores.set_int_param(IntParam::PDF_APPEND_KERN, 1);
    stores.set_pdf_font_code(tex_state::PdfFontCode::Knac, font, b'-', 100);
    let mut nest = ModeNest::new();
    nest.push(Mode::Horizontal).expect("test hmode");
    let origin = stores.synthetic_origin(SyntheticOriginKind::Test);
    append_canonical_character(&mut nest, &mut stores, '-', origin).expect("hyphen append");

    flush_pending_hchars(
        &mut nest,
        &mut stores,
        tex_command::CommandFuelLedger::default().fuel_mut(),
    )
    .expect("public pending-character flush");

    assert!(matches!(
        nest.current_list().nodes(),
        [
            Node::Char { ch: '-', origin: actual, .. },
            Node::Kern { kind: KernKind::Auto, .. },
            Node::Disc { kind: DiscKind::ExplicitHyphen, .. },
        ] if *actual == origin
    ));
}

fn node_characters(nodes: &[Node]) -> Vec<char> {
    nodes
        .iter()
        .filter_map(|node| match node {
            Node::Char { ch, .. } | Node::Lig { ch, .. } => Some(*ch),
            _ => None,
        })
        .collect()
}

#[test]
fn complete_ligature_machine_covers_retain_delete_and_pass_over_operations() {
    // TeX82 §§1034-1036 name these eight useful op-byte shapes explicitly.
    for (op_byte, expected) in [
        (0, "C"),
        (1, "CB"),
        (2, "AC"),
        (3, "ACB"),
        (5, "CB"),
        (6, "AC"),
        (7, "ACB"),
        (11, "ACB"),
    ] {
        let (mut stores, font) =
            ligature_test_font(&[(b'A', b'B', ligature_command(op_byte, b'C'))], None, None);
        stores.set_current_font(font);
        let mut nest = ModeNest::new();
        nest.push(Mode::RestrictedHorizontal).expect("test hmode");
        for ch in ['A', 'B'] {
            append_canonical_character(&mut nest, &mut stores, ch, OriginId::UNKNOWN)
                .expect("public pending-character append");
        }
        flush_pending_hchars(
            &mut nest,
            &mut stores,
            tex_command::CommandFuelLedger::default().fuel_mut(),
        )
        .expect("public pending-character flush");
        let nodes = nest.current_list().nodes();
        assert_eq!(
            node_characters(nodes).iter().collect::<String>(),
            expected,
            "op byte {op_byte}"
        );
    }
}

#[test]
fn generated_ligature_pair_reenters_the_program() {
    let (mut stores, font) = ligature_test_font(
        &[
            (b'A', b'B', ligature_command(0, b'C')),
            (b'C', b'D', ligature_command(0, b'E')),
        ],
        None,
        None,
    );
    stores.set_current_font(font);
    let mut nest = ModeNest::new();
    nest.push(Mode::RestrictedHorizontal).expect("test hmode");
    for ch in ['A', 'B', 'D'] {
        append_canonical_character(&mut nest, &mut stores, ch, OriginId::UNKNOWN)
            .expect("public pending-character append");
    }
    flush_pending_hchars(
        &mut nest,
        &mut stores,
        tex_command::CommandFuelLedger::default().fuel_mut(),
    )
    .expect("public pending-character flush");
    let nodes = nest.current_list().nodes();

    assert_eq!(node_characters(nodes), ['E']);
    assert!(matches!(&nodes[0], Node::Lig { orig, .. } if orig == &['A', 'B', 'D']));
}

#[test]
fn complete_ligature_machine_processes_both_boundaries() {
    for (left_boundary, right_boundary) in [
        (Some((b'A', ligature_command(0, b'L'))), None),
        (None, Some((b'A', ligature_command(0, b'R')))),
    ] {
        let (mut stores, font) = ligature_test_font(&[], left_boundary, right_boundary);
        stores.set_current_font(font);
        let mut nest = ModeNest::new();
        nest.push(Mode::RestrictedHorizontal).expect("test hmode");
        append_canonical_character(&mut nest, &mut stores, 'A', OriginId::UNKNOWN)
            .expect("public pending-character append");
        flush_pending_hchars(
            &mut nest,
            &mut stores,
            tex_command::CommandFuelLedger::default().fuel_mut(),
        )
        .expect("public pending-character flush");
        let nodes = nest.current_list().nodes();
        assert_eq!(
            node_characters(nodes),
            [if left_boundary.is_some() { 'L' } else { 'R' }]
        );
        assert!(matches!(
            nodes,
            [Node::Lig {
                left_hit,
                right_hit,
                ..
            }] if *left_hit == left_boundary.is_some()
                && *right_hit == right_boundary.is_some()
        ));
    }
}

#[test]
fn char_primitive_continues_the_pending_ligature_run() {
    let mut stores = Universe::with_world(tex_state::World::memory()).with_plain_catcodes();
    let mut control = canonical_control_with_cmr10(
        &mut stores,
        "\\font\\f=cmr10 \\relax \\f \\setbox0=\\hbox{f\\char102}",
    );
    run_canonical_to_input_end(&mut control, &mut stores);

    let root = stores.box_reg(0).expect("box0");
    let Some(tex_state::node_arena::NodeRef::HList(hbox)) = stores.nodes(root).first() else {
        panic!("box0 should contain an hbox");
    };
    assert!(matches!(
        stores.nodes(hbox.children).testing_decoded(),
        [Node::Lig {
            orig,
            ..
        }] if orig == &['f', 'f']
    ));
}

#[test]
fn chained_ligature_retains_every_source_character() {
    let mut stores = Universe::with_world(tex_state::World::memory()).with_plain_catcodes();
    let mut control = canonical_control_with_cmr10(
        &mut stores,
        "\\font\\f=cmr10 \\relax \\f \\setbox0=\\hbox{ffi}",
    );
    run_canonical_to_input_end(&mut control, &mut stores);
    let root = stores.box_reg(0).expect("box0");
    let Some(tex_state::node_arena::NodeRef::HList(hbox)) = stores.nodes(root).first() else {
        panic!("box0 should contain an hbox");
    };
    assert!(matches!(
        stores.nodes(hbox.children).testing_decoded(),
        [Node::Lig { orig, .. }] if orig == &['f', 'f', 'i']
    ));
}

#[test]
fn hyphenation_does_not_partially_consume_a_boundary_ligature() {
    let mut stores = Universe::new_with_plain_catcodes();
    let font = stores.current_font();
    stores.set_lccode('C', 'c' as u32);
    stores.set_lccode('/', 0);
    let nodes = [
        Node::Char {
            font,
            ch: 'C',
            origin: tex_state::token::OriginId::UNKNOWN,
        },
        Node::Lig {
            font,
            ch: 'B',
            orig: vec!['C', '/'],
            origins: vec![tex_state::token::OriginId::UNKNOWN; 2],
            left_hit: false,
            right_hit: false,
        },
    ];

    let hyphenated = super::super::hyphenation::test_hyphenated_word(&mut stores, &nodes);
    assert!(matches!(
        hyphenated.as_slice(),
        [Node::Char { ch: 'C', .. }, Node::Lig { ch: 'B', .. }]
    ));
}

#[test]
fn hyphenation_keeps_scanning_across_font_kerns() {
    let mut stores = Universe::with_world(tex_state::World::memory()).with_plain_catcodes();
    let mut control = canonical_control_with_cmr10(&mut stores, "\\font\\f=cmr10 \\relax \\f");
    run_canonical_to_input_end(&mut control, &mut stores);
    stores.add_hyphenation_exception(ExceptionSpec {
        word: "availability".to_owned(),
        positions: vec![5, 9],
    });
    let font = stores.current_font();
    stores.set_font_hyphen_char(font, i32::from(b'-'));
    let pending: Vec<_> = "availability"
        .chars()
        .map(|ch| PendingHChar {
            font,
            ch,
            origin: tex_state::token::OriginId::UNKNOWN,
        })
        .collect();
    let nodes = reconstitute(&mut stores, &pending, false, false);
    assert!(
        nodes.iter().any(|node| matches!(
            node,
            Node::Kern {
                kind: KernKind::Font,
                ..
            }
        )),
        "the fixture must exercise an internal font kern: {nodes:?}"
    );

    let hyphenated = super::super::hyphenation::test_hyphenated_word(&mut stores, &nodes);
    assert_eq!(
        hyphenated
            .iter()
            .filter(|node| matches!(node, Node::Disc { .. }))
            .count(),
        2,
        "both exception points must survive font-kern reconstitution: {hyphenated:?}"
    );
}

#[test]
fn hyphenation_preserves_the_font_kern_after_a_reconstituted_word() {
    let mut stores = Universe::new_with_plain_catcodes();
    let font = stores.current_font();
    for ch in "abcd".chars() {
        stores.set_lccode(ch, ch as u32);
    }
    stores.add_hyphenation_exception(ExceptionSpec {
        word: "abcd".to_owned(),
        positions: vec![2],
    });
    stores.set_int_param(IntParam::LEFT_HYPHEN_MIN, 1);
    stores.set_int_param(IntParam::RIGHT_HYPHEN_MIN, 1);
    stores.set_font_hyphen_char(font, i32::from(b'-'));
    let trailing = Scaled::from_raw(-54_614);
    let nodes = [
        Node::Char {
            font,
            ch: 'a',
            origin: tex_state::token::OriginId::UNKNOWN,
        },
        Node::Char {
            font,
            ch: 'b',
            origin: tex_state::token::OriginId::UNKNOWN,
        },
        Node::Char {
            font,
            ch: 'c',
            origin: tex_state::token::OriginId::UNKNOWN,
        },
        Node::Char {
            font,
            ch: 'd',
            origin: tex_state::token::OriginId::UNKNOWN,
        },
        Node::Kern {
            amount: trailing,
            kind: KernKind::Font,
        },
    ];

    let hyphenated = super::super::hyphenation::test_hyphenated_word(&mut stores, &nodes);

    assert!(
        matches!(hyphenated.last(), Some(Node::Kern { amount, kind: KernKind::Font }) if *amount == trailing)
    );
}

#[test]
fn hyphenation_does_not_repeat_a_left_boundary_kern() {
    let mut stores = Universe::new_with_plain_catcodes();
    let font = stores.current_font();
    stores.set_lccode('A', 'a' as u32);
    let nodes = [
        Node::Kern {
            amount: Scaled::from_raw(-65537),
            kind: KernKind::Font,
        },
        Node::Char {
            font,
            ch: 'A',
            origin: tex_state::token::OriginId::UNKNOWN,
        },
    ];

    let hyphenated = super::super::hyphenation::test_hyphenated_word(&mut stores, &nodes);

    assert!(matches!(
        hyphenated.as_slice(),
        [
            Node::Kern {
                kind: KernKind::Font,
                ..
            },
            Node::Char { ch: 'A', .. }
        ]
    ));
}

#[test]
fn discretionary_absorbs_font_kern_across_hyphenated_line_boundary() {
    let mut stores = Universe::with_world(tex_state::World::memory()).with_plain_catcodes();
    let mut control = canonical_control_with_cmr10(&mut stores, "\\font\\f=cmr10 \\relax \\f");
    run_canonical_to_input_end(&mut control, &mut stores);
    stores.add_hyphenation_exception(ExceptionSpec {
        word: "sentence".to_owned(),
        positions: vec![3],
    });
    let font = stores.current_font();
    stores.set_font_hyphen_char(font, i32::from(b'-'));
    let pending: Vec<_> = "sentence"
        .chars()
        .map(|ch| PendingHChar {
            font,
            ch,
            origin: tex_state::token::OriginId::UNKNOWN,
        })
        .collect();
    let nodes = reconstitute(&mut stores, &pending, false, false);

    let hyphenated = super::super::hyphenation::test_hyphenated_word(&mut stores, &nodes);
    let disc_index = hyphenated
        .iter()
        .position(|node| matches!(node, Node::Disc { .. }))
        .expect("sentence exception should insert a discretionary");
    let Node::Disc { replace, .. } = &hyphenated[disc_index] else {
        unreachable!()
    };

    assert!(matches!(
        stores.nodes(*replace).testing_decoded(),
        [Node::Kern {
            kind: KernKind::Font,
            ..
        }]
    ));
    assert!(!matches!(
        hyphenated.get(disc_index + 1),
        Some(Node::Kern {
            kind: KernKind::Font,
            ..
        })
    ));
}

#[test]
fn ffi_reconstitution_suppresses_an_unsynchronized_second_hyphenation_point() {
    // TeX82 §§904 and 913-916 ignore a second point before the two branches
    // have synchronized beyond the ligature that contains the first point.
    let mut stores = Universe::with_world(tex_state::World::memory()).with_plain_catcodes();
    let mut control = canonical_control_with_cmr10(&mut stores, "\\font\\f=cmr10 \\relax \\f");
    run_canonical_to_input_end(&mut control, &mut stores);
    stores.add_hyphenation_exception(ExceptionSpec {
        word: "office".to_owned(),
        positions: vec![2, 3],
    });
    stores.set_int_param(IntParam::LEFT_HYPHEN_MIN, 1);
    stores.set_int_param(IntParam::RIGHT_HYPHEN_MIN, 1);
    let font = stores.current_font();
    stores.set_font_hyphen_char(font, i32::from(b'-'));
    let pending: Vec<_> = "office"
        .chars()
        .map(|ch| PendingHChar {
            font,
            ch,
            origin: tex_state::token::OriginId::UNKNOWN,
        })
        .collect();
    let nodes = reconstitute(&mut stores, &pending, false, false);
    assert!(
        nodes.iter().any(
            |node| matches!(node, Node::Lig { orig, .. } if orig.as_slice() == ['f', 'f', 'i'])
        )
    );

    let hyphenated = super::super::hyphenation::test_hyphenated_word(&mut stores, &nodes);

    assert_eq!(
        hyphenated
            .iter()
            .filter(|node| matches!(node, Node::Disc { .. }))
            .count(),
        1,
        "the point inside the same ffi synchronization span must be suppressed: {hyphenated:?}"
    );
}
