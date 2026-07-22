use super::{
    FormatError, GenerationForkError, TakeUnboxResult, UnboxKind, Universe, utf8_scalar_len_at,
};
use crate::env::banks::IntParam;
use crate::font::{MAX_FONT_DIMEN, NULL_FONT};
use crate::glue::{GlueSpec, Order};
use crate::hyphenation::{ExceptionSpec, PatternSpec};
use crate::ids::{ArenaRef, FontId, NodeListId};
use crate::input::{
    ConditionFrameSummary, ConditionFrameToken, InputFrameSummary, InputSummary, LexerState,
    MacroArgumentRange, MacroArguments, SourceFrameSummary, SourceId, TokenListReplayKind,
    TracedTokenList,
};
use crate::macro_store::MacroMeaning;
use crate::meaning::{Meaning, MeaningFlags, RawMeaning};
use crate::node::{BoxNode, BoxNodeFields, GlueKind, KernKind, LeaderPayload, Node, Sign};
use crate::page::{PageDimension, PageInteger};
use crate::provenance::{
    InsertedOriginKind, OriginRecord, SourceOrigin, SynthesizedOriginKind, SyntheticOriginKind,
};
use crate::scaled::{GlueSetRatio, Scaled};
use crate::source_fragments::{EditorLayout, FragmentStore, LayoutGeneration, Piece};
use crate::source_map::{SourceDescriptor, SourceMapError};
use crate::token::{Catcode, OriginId, Token, TracedTokenWord};
use crate::world::{
    ContentDomain, ContentHash, EffectRecord, JobClock, PrintSink, StreamSlot, World,
};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

#[test]
fn page_group_selector_consumes_live_signed_warning_control() {
    for (control, warning) in [(0, true), (23, false), (-23, false)] {
        let mut universe = Universe::new();
        universe.set_int_param(IntParam::PDF_SUPPRESS_WARNING_PAGE_GROUP, control);
        let mut selector = universe.pdf_page_group_selector();
        assert_eq!(
            selector.include(true),
            crate::PdfPageGroupInclusion::SelectForOutputPage
        );
        let crate::PdfPageGroupInclusion::KeepOnIncludedForm {
            warning: actual_warning,
        } = selector.include(true)
        else {
            panic!("second page group must remain on its included form");
        };
        assert_eq!(actual_warning.is_some(), warning, "control {control}");
    }
}

#[test]
fn pdf_match_captures_are_checkpointed_and_hashed() {
    let mut universe = Universe::new();
    universe.set_pdf_match_state(b"first".to_vec(), vec![Some((0, 5))], 1, true);
    let first = universe.snapshot();
    assert_eq!(
        universe.pdf_match_capture(0),
        Some((0, b"first".as_slice()))
    );

    universe.set_pdf_match_state(b"second".to_vec(), vec![Some((1, 4))], 1, true);
    assert_eq!(universe.pdf_match_capture(0), Some((1, b"eco".as_slice())));
    assert_ne!(universe.snapshot().state_hash(), first.state_hash());

    universe.rollback(&first);
    assert_eq!(
        universe.pdf_match_capture(0),
        Some((0, b"first".as_slice()))
    );
    assert_eq!(universe.snapshot().state_hash(), first.state_hash());
}

#[test]
fn pdftex_utility_mutations_replay_with_identical_hashes() {
    let world = World::memory_with_pdftex_inputs(
        crate::JobClock::DEFAULT,
        1,
        1_000_000,
        crate::ShellEscapePolicy::Disabled,
    );
    let mut universe = Universe::with_world(world);
    let first = universe.snapshot();
    let random = universe.pdf_uniform_deviate(10);
    universe.world_mut().set_pdf_time_micros(2_000_000);
    let changed = universe.snapshot().state_hash();
    assert_ne!(changed, first.state_hash());

    universe.rollback(&first);
    assert_eq!(universe.pdf_uniform_deviate(10), random);
    universe.world_mut().set_pdf_time_micros(2_000_000);
    assert_eq!(universe.snapshot().state_hash(), changed);
}

#[test]
fn bounded_scalar_decode_does_not_validate_the_remaining_source_suffix() {
    assert_eq!(utf8_scalar_len_at(&[b'x', 0xff], 0), Some(1));
    assert_eq!(utf8_scalar_len_at(&[0xc3, 0xa9, 0xff], 0), Some(2));
    assert_eq!(utf8_scalar_len_at(&[0xc3, 0xff], 0), None);
}

#[test]
fn inserted_origin_classification_skips_direct_source_resolution() {
    let mut universe = Universe::new();
    universe
        .register_source(
            SourceId::new(0),
            SourceDescriptor::generated(Arc::from(&b"x"[..])),
        )
        .expect("source registration");
    let direct = universe.source_token_origin(SourceId::new(0), 0, 1);
    let noexpand = universe.inserted_origin(
        InsertedOriginKind::NoExpand,
        Token::Char {
            ch: 'x',
            cat: Catcode::Other,
        },
        direct,
    );

    assert!(!universe.origin_is_inserted_kind(direct, InsertedOriginKind::NoExpand));
    assert!(universe.origin_is_inserted_kind(noexpand, InsertedOriginKind::NoExpand));
}

#[test]
fn editor_fragment_origin_remains_live_across_universe_rollback() {
    let mut fragments = FragmentStore::new();
    let (fragment, registration) = fragments
        .testing_append_at(Arc::from(&b"editor"[..]), 1, 100)
        .expect("fragment append");
    let layout = EditorLayout::new(
        "root.tex",
        LayoutGeneration::new(1),
        vec![Piece::new(fragment, 0, 6)],
        &fragments,
    )
    .expect("editor layout");
    let mut universe = Universe::new();
    universe
        .install_editor_fragments(&fragments, &layout)
        .expect("fragment installation");
    let origin = registration
        .direct_origin(1, 2)
        .expect("direct fragment origin");
    let expected = registration.span(1, 2).expect("fragment span");
    let snapshot = universe.snapshot();

    let _discarded = universe.synthetic_origin(SyntheticOriginKind::Test);
    universe.rollback(&snapshot);

    assert_eq!(
        universe.origin_if_live(origin),
        Some(OriginRecord::SourceSpan(expected))
    );
    assert_eq!(universe.origin(origin), OriginRecord::SourceSpan(expected));
}

#[test]
#[should_panic(expected = "origin id is not live in this Universe timeline")]
fn inserted_origin_classification_rejects_rolled_back_arena_origin() {
    let mut universe = Universe::new();
    let snapshot = universe.snapshot();
    let noexpand = universe.inserted_origin(
        InsertedOriginKind::NoExpand,
        Token::Char {
            ch: 'x',
            cat: Catcode::Other,
        },
        OriginId::UNKNOWN,
    );
    universe.rollback(&snapshot);

    let _ = universe.origin_is_inserted_kind(noexpand, InsertedOriginKind::NoExpand);
}

#[test]
fn unknown_meaning_flags_participate_in_semantic_hashes() {
    let mut universe = Universe::new();
    let symbol = universe.intern("future-extension");
    let baseline = universe.snapshot();

    universe.set_meaning(
        symbol,
        Meaning::Unknown(RawMeaning::testing_new_with_flags(
            200,
            MeaningFlags::from_bits(0x40),
            7,
        )),
    );
    let first = universe.snapshot().state_hash();

    universe.rollback(&baseline);
    universe.set_meaning(
        symbol,
        Meaning::Unknown(RawMeaning::testing_new_with_flags(
            200,
            MeaningFlags::from_bits(0x80),
            7,
        )),
    );

    assert_ne!(universe.snapshot().state_hash(), first);
}

#[test]
fn maximum_fontdimen_is_distinct_grouped_rollback_safe_and_format_stable() {
    let mut universe = Universe::new();
    let identifier = universe.intern("boundaryfont");
    let font =
        universe.intern_font_with_identifier(test_font("boundaryfont", b"boundary"), identifier);
    universe.set_meaning(identifier, Meaning::Font(font));
    universe
        .set_font_dimen(font, 1, Scaled::from_raw(11))
        .expect("first fontdimen is writable");
    let baseline = universe.snapshot();
    let baseline_snapshot_hash = baseline.state_hash();
    let baseline_hash = universe.testing_state_hash();

    universe.enter_group();
    universe
        .set_font_dimen(font, MAX_FONT_DIMEN, Scaled::from_raw(22))
        .expect("maximum fontdimen is writable");
    assert_eq!(
        universe.font_dimen(font, MAX_FONT_DIMEN),
        Scaled::from_raw(22)
    );
    assert_ne!(universe.testing_state_hash(), baseline_hash);
    assert!(universe.leave_group().is_empty());
    assert_eq!(
        universe.font_dimen(font, MAX_FONT_DIMEN),
        Scaled::from_raw(22)
    );
    let grouped_write_hash = universe.testing_state_hash();
    assert_ne!(grouped_write_hash, baseline_hash);

    let invalid = universe
        .set_font_dimen(font, MAX_FONT_DIMEN + 1, Scaled::from_raw(99))
        .expect_err("fontdimen above the slot domain is rejected");
    assert!(matches!(
        invalid,
        super::FontParameterError::NumberOutOfRange { .. }
    ));
    assert_eq!(
        universe.font_dimen(font, MAX_FONT_DIMEN + 1),
        Scaled::from_raw(0)
    );
    assert_eq!(universe.font_dimen(font, 1), Scaled::from_raw(11));
    assert_eq!(universe.testing_state_hash(), grouped_write_hash);

    universe.rollback(&baseline);
    assert_eq!(
        universe.font_dimen(font, MAX_FONT_DIMEN),
        Scaled::from_raw(0)
    );
    assert_eq!(universe.testing_state_hash(), baseline_hash);

    universe.enter_group();
    universe
        .set_font_dimen(font, MAX_FONT_DIMEN, Scaled::from_raw(33))
        .expect("global maximum fontdimen is writable");
    assert!(universe.leave_group().is_empty());
    assert_eq!(
        universe.font_dimen(font, MAX_FONT_DIMEN),
        Scaled::from_raw(33)
    );
    universe.rollback(&baseline);
    assert_eq!(
        universe.font_dimen(font, MAX_FONT_DIMEN),
        Scaled::from_raw(0)
    );
    assert_eq!(universe.testing_state_hash(), baseline_hash);
    assert_eq!(universe.snapshot().state_hash(), baseline_snapshot_hash);

    universe
        .set_font_dimen(font, MAX_FONT_DIMEN, Scaled::from_raw(44))
        .expect("maximum fontdimen is format-visible");
    let bytes = universe.dump_format().expect("boundary format encodes");
    let mut restored =
        Universe::from_format(World::memory(), &bytes).expect("boundary format restores");
    let restored_identifier = restored.intern("boundaryfont");
    let Meaning::Font(restored_font) = restored.meaning(restored_identifier) else {
        panic!("restored font identifier meaning");
    };
    assert_eq!(
        restored.font_dimen(restored_font, MAX_FONT_DIMEN),
        Scaled::from_raw(44)
    );
    assert_eq!(restored.font_dimen(restored_font, 1), Scaled::from_raw(11));
    assert_eq!(
        restored.dump_format().expect("boundary format redumps"),
        bytes
    );
    let restored_snapshot = restored.snapshot();
    let restored_hash = restored_snapshot.state_hash();
    restored
        .set_font_dimen(restored_font, MAX_FONT_DIMEN, Scaled::from_raw(55))
        .expect("restored maximum fontdimen remains writable");
    restored.rollback(&restored_snapshot);
    assert_eq!(restored.snapshot().state_hash(), restored_hash);
}

#[test]
fn oversized_immutable_font_parameter_table_is_rejected_before_publication() {
    let mut universe = Universe::new();
    let before = universe.testing_state_hash();
    let oversized = crate::font::LoadedFont::new(
        "oversized",
        "oversized.tfm",
        ContentHash::from_bytes(b"oversized").bytes(),
        0,
        Scaled::from_raw(Scaled::UNITY),
        Scaled::from_raw(Scaled::UNITY),
        vec![Scaled::from_raw(0); MAX_FONT_DIMEN as usize + 1],
        crate::font::FontMetrics::default(),
    );

    assert!(matches!(
        universe.try_intern_font(oversized),
        Err(super::FontParameterError::ParameterCountOutOfRange {
            count,
            maximum: MAX_FONT_DIMEN,
        }) if count == MAX_FONT_DIMEN as usize + 1
    ));
    assert_eq!(universe.testing_state_hash(), before);
}

#[test]
fn universe_is_send() {
    fn assert_send<T: Send>() {}

    assert_send::<Universe>();
}

#[test]
fn traced_list_finish_reuses_semantics_but_preserves_each_origin_instance() {
    let mut universe = Universe::new();
    let symbol = universe.intern("traced-list-cs");
    let first_origin = universe.synthetic_origin(SyntheticOriginKind::Test);
    let second_origin = universe.synthetic_origin(SyntheticOriginKind::Engine);
    let tokens = [
        Token::Char {
            ch: '🦀',
            cat: Catcode::Other,
        },
        Token::Cs(symbol.symbol()),
        Token::param(9),
        Token::frozen_end_template(),
        Token::frozen_endv(),
    ];
    let first: Vec<_> = tokens
        .iter()
        .copied()
        .map(|token| TracedTokenWord::pack(token, first_origin))
        .collect();
    let second: Vec<_> = tokens
        .iter()
        .copied()
        .map(|token| TracedTokenWord::pack(token, second_origin))
        .collect();

    let bulk = universe.intern_token_list(&tokens);
    let first_list = universe.finish_traced_token_list(&first);
    let second_list = universe.finish_traced_token_list(&second);

    assert_eq!(first_list.token_list(), bulk);
    assert_eq!(second_list.token_list(), bulk);
    assert_ne!(first_list.origin_list(), second_list.origin_list());
    assert_eq!(universe.tokens(first_list.token_list()), tokens);
    assert_eq!(
        universe.origin_list(first_list.origin_list()),
        vec![first_origin; tokens.len()]
    );
    assert_eq!(
        universe.origin_list(second_list.origin_list()),
        vec![second_origin; tokens.len()]
    );

    let empty = universe.finish_traced_token_list(&[]);
    assert_eq!(empty.token_list(), crate::ids::TokenListId::EMPTY);
    assert_eq!(empty.origin_list(), crate::ids::OriginListId::EMPTY);
}

#[test]
fn traced_list_finish_validates_every_word_before_publishing() {
    let mut universe = Universe::new();
    let valid = TracedTokenWord::pack(
        Token::Char {
            ch: 'v',
            cat: Catcode::Letter,
        },
        OriginId::UNKNOWN,
    );
    let invalid = TracedTokenWord::from_raw(2_u64 << 62);

    assert!(
        catch_unwind(AssertUnwindSafe(|| {
            universe.finish_traced_token_list(&[valid, invalid]);
        }))
        .is_err()
    );

    let finished = universe.finish_traced_token_list(&[valid]);
    assert_eq!(finished.token_list().raw(), 1);
    assert_eq!(finished.origin_list().raw(), 1);
}

#[test]
fn traced_list_finish_rejects_rolled_back_origins_before_publishing() {
    let mut universe = Universe::new();
    let snapshot = universe.snapshot();
    let stale = universe.synthetic_origin(SyntheticOriginKind::Test);
    universe.rollback(&snapshot);
    let traced = TracedTokenWord::pack(
        Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        },
        stale,
    );

    assert!(
        catch_unwind(AssertUnwindSafe(|| {
            universe.finish_traced_token_list(&[traced]);
        }))
        .is_err()
    );

    let valid = TracedTokenWord::pack(
        Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        },
        OriginId::UNKNOWN,
    );
    let finished = universe.finish_traced_token_list(&[valid]);
    assert_eq!(finished.token_list().raw(), 1);
    assert_eq!(finished.origin_list().raw(), 1);
}

#[test]
fn semantic_format_is_deterministic_validated_and_world_independent() {
    let mut universe = Universe::with_world(World::memory());
    let name = universe.intern("answer");
    universe.set_meaning(name, Meaning::CountRegister(42));
    universe.set_count(42, 1234);
    let body = universe.intern_token_list(&[
        Token::Cs(name.symbol()),
        Token::Char {
            ch: '!',
            cat: Catcode::Other,
        },
    ]);
    let macro_name = universe.intern("m");
    universe.set_macro_meaning(
        macro_name,
        MacroMeaning::new(MeaningFlags::LONG, crate::ids::TokenListId::EMPTY, body),
    );
    universe
        .world_mut()
        .write_text(PrintSink::TerminalAndLog, "must not enter format");
    let child = universe.freeze_node_list(&[Node::Rule {
        width: Some(Scaled::from_raw(10)),
        height: Some(Scaled::from_raw(20)),
        depth: None,
    }]);
    let root = universe.freeze_node_list(&[Node::HList(BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(10),
        height: Scaled::from_raw(20),
        depth: Scaled::from_raw(0),
        shift: Scaled::from_raw(0),
        display: false,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: child,
    }))]);
    universe.set_box_reg(7, root);
    let semantic_id = universe
        .stores
        .node_semantic_id(universe.box_reg(7).expect("promoted box register"));

    let first = universe.dump_format().expect("format encode");
    let _retained_checkpoint = universe.snapshot();
    let second = universe.dump_format().expect("deterministic format encode");
    assert_eq!(first, second, "retained checkpoints are not format state");

    let restored = Universe::from_format(World::memory(), &first).expect("format decode");
    let restored_name = restored.symbol("answer").expect("restored name");
    assert_eq!(restored.meaning(restored_name), Meaning::CountRegister(42));
    assert_eq!(restored.count(42), 1234);
    let restored_macro = restored.symbol("m").expect("restored macro name");
    assert!(matches!(
        restored.meaning(restored_macro),
        Meaning::Macro { .. }
    ));
    let restored_root = restored.box_reg(7).expect("restored box register");
    assert_eq!(restored.stores.node_semantic_id(restored_root), semantic_id);
    let restored_nodes = restored.nodes(restored_root).to_vec();
    let Node::HList(restored_box) = restored_nodes[0] else {
        panic!("restored box node kind");
    };
    assert_eq!(
        restored.nodes(restored_box.children).to_vec(),
        [Node::Rule {
            width: Some(Scaled::from_raw(10)),
            height: Some(Scaled::from_raw(20)),
            depth: None,
        }]
    );
    assert!(restored.world().effect_records().is_empty());

    let mut corrupted = first.clone();
    *corrupted.last_mut().expect("nonempty format") ^= 1;
    assert!(matches!(
        Universe::from_format(World::memory(), &corrupted),
        Err(super::FormatError::Checksum)
    ));
}

#[test]
fn semantic_format_round_trips_sparse_unicode_code_tables() {
    let mut universe = Universe::new();
    let ch = '\u{1f642}';
    universe.set_catcode(ch, Catcode::Active);
    universe.set_lccode(ch, 'a' as u32);
    universe.set_uccode(ch, 'A' as u32);
    universe.set_sfcode(ch, 2345);
    universe.set_mathcode(ch, 0x12_3456);
    universe.set_delcode(ch, 0x123_456);

    let image = universe.dump_format().expect("quiescent unicode format");
    let restored = Universe::from_format(World::memory(), &image).expect("unicode format restore");
    assert_eq!(restored.catcode(ch), Catcode::Active);
    assert_eq!(restored.lccode(ch), 'a' as u32);
    assert_eq!(restored.uccode(ch), 'A' as u32);
    assert_eq!(restored.sfcode(ch), 2345);
    assert_eq!(restored.mathcode(ch), 0x12_3456);
    assert_eq!(restored.delcode(ch), 0x123_456);
}

#[test]
fn frozen_non_node_sections_are_deterministic_and_keep_mutable_overlays() {
    let mut universe = Universe::new();
    universe.set_catcode('\u{1f642}', Catcode::Active);
    universe.add_hyphenation_pattern(PatternSpec {
        letters: "alpha".chars().collect(),
        values: vec![0, 0, 1, 0, 0, 0],
    });
    universe.add_hyphenation_exception(ExceptionSpec {
        word: "hyphen".to_owned(),
        positions: vec![2],
    });
    universe.add_hyphenation_exception(ExceptionSpec {
        word: "edge".to_owned(),
        positions: vec![0, 4, 4],
    });
    let image = universe.dump_format().expect("frozen non-node format");
    assert_eq!(universe.dump_format().expect("deterministic redump"), image);

    let mut loaded = Universe::from_format(World::memory(), &image).expect("direct frozen load");
    assert_eq!(loaded.catcode('\u{1f642}'), Catcode::Active);
    assert_eq!(loaded.hyphen_positions("alpha", 1, 1), vec![2]);
    assert_eq!(loaded.hyphenation_exception("hyphen"), Some(&[2][..]));
    assert_eq!(loaded.hyphenation_exception("edge"), Some(&[0, 4, 4][..]));
    let baseline = loaded.snapshot();
    loaded.set_catcode('\u{1f642}', Catcode::Letter);
    loaded.add_hyphenation_exception(ExceptionSpec {
        word: "overlay".to_owned(),
        positions: vec![3],
    });
    assert_eq!(loaded.catcode('\u{1f642}'), Catcode::Letter);
    assert_eq!(loaded.hyphenation_exception("overlay"), Some(&[3][..]));
    loaded.rollback(&baseline);
    assert_eq!(loaded.catcode('\u{1f642}'), Catcode::Active);
    assert_eq!(loaded.hyphenation_exception("overlay"), None);
    assert_eq!(loaded.dump_format().expect("rollback redump"), image);
}

#[test]
fn checksum_valid_non_node_section_corruption_fails_closed() {
    let valid = Universe::new().dump_format().expect("valid core format");
    for (kind, offset, expected) in [
        (crate::stores::FONTS_SECTION, 28, "font header"),
        (crate::stores::CODE_TABLES_SECTION, 12, "code-table header"),
        (crate::stores::HYPHENATION_SECTION, 12, "hyphenation header"),
    ] {
        let mut bytes = valid.clone();
        replace_format_section(&mut bytes, kind, |section| section[offset] ^= 1);
        let error = Universe::from_format(World::memory(), &bytes)
            .expect_err("checksum-valid frozen corruption");
        assert!(
            matches!(error, FormatError::InvalidState(ref message) if message.contains(expected)),
            "section {kind} returned {error:?}"
        );
    }
}

#[test]
fn frozen_foundational_sections_restore_ids_and_accept_job_local_additions() {
    let mut universe = Universe::new();
    universe.set_count(7, 41);
    let base = universe.intern("frozen-base");
    let base_tokens = universe.intern_token_list(&[
        Token::Cs(base.symbol()),
        Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        },
    ]);
    let base_macro = universe.intern_macro(MacroMeaning::new(
        MeaningFlags::LONG,
        crate::ids::TokenListId::EMPTY,
        base_tokens,
    ));
    universe.set_meaning(
        base,
        Meaning::Macro {
            flags: MeaningFlags::LONG,
            definition: base_macro,
        },
    );
    let base_glue = universe.intern_glue(GlueSpec {
        width: Scaled::from_raw(11),
        stretch: Scaled::from_raw(22),
        stretch_order: Order::Fil,
        shrink: Scaled::from_raw(33),
        shrink_order: Order::Normal,
    });

    let image = universe.dump_format().expect("frozen core format");
    let container = crate::format_container::decode(&image).expect("decode container");
    assert_eq!(
        container
            .sections
            .iter()
            .map(|section| section.kind)
            .collect::<Vec<_>>(),
        [
            crate::format_container::TRANSITIONAL_SEMANTIC_SECTION,
            crate::stores::NAMES_SECTION,
            crate::stores::NAMES_LOOKUP_SECTION,
            crate::stores::TOKEN_LISTS_SECTION,
            crate::stores::MACROS_SECTION,
            crate::stores::GLUE_SECTION,
            crate::stores::FONTS_SECTION,
            crate::stores::CODE_TABLES_SECTION,
            crate::stores::HYPHENATION_SECTION,
            crate::stores::FROZEN_NODES_SECTION,
            crate::stores::FROZEN_ENV_SECTION,
        ]
    );
    let environment = container
        .section(crate::stores::FROZEN_ENV_SECTION)
        .expect("frozen environment section");
    let env_entries = crate::stores::testing_frozen_environment_shape(environment.bytes.as_ref());
    assert!(env_entries > 0);

    let _ = crate::stores::testing_take_transitional_format_work();
    let mut loaded = Universe::from_format(World::memory(), &image).expect("load frozen core");
    assert_eq!(
        crate::stores::testing_take_transitional_format_work(),
        crate::stores::TestingFormatLoadWork {
            graph_key_remaps: 0,
            semantic_reseals: 0,
            assignment_replays: 0,
        },
        "normal schema-10 loading must not remap graphs, reseal semantic identities, or replay environment assignments"
    );
    assert_eq!(loaded.dump_format().expect("canonical redump"), image);
    let immutable_base = loaded.stores.env().testing_format_base().to_vec();
    let environment_snapshot = loaded.snapshot();
    loaded.enter_group();
    loaded.set_count(7, 99);
    assert_eq!(loaded.count(7), 99);
    assert!(loaded.leave_group().is_empty());
    assert_eq!(loaded.count(7), 41);
    loaded.enter_group();
    loaded.set_count(7, 100);
    loaded.set_count_global(7, 77);
    assert!(loaded.leave_group().is_empty());
    assert_eq!(loaded.count(7), 77);
    loaded.rollback(&environment_snapshot);
    assert_eq!(loaded.count(7), 41);
    assert_eq!(loaded.stores.env().testing_format_base(), immutable_base);
    let restored_base = loaded.symbol("frozen-base").expect("restored name");
    assert_eq!(restored_base.raw(), base.raw());
    assert_eq!(
        loaded
            .intern_token_list(&[
                Token::Cs(restored_base.symbol()),
                Token::Char {
                    ch: 'x',
                    cat: Catcode::Letter,
                },
            ])
            .raw(),
        base_tokens.raw()
    );
    let restored_glue = crate::ids::GlueId::testing_new(base_glue.raw());
    assert_eq!(
        loaded.intern_glue(loaded.glue(restored_glue)).raw(),
        base_glue.raw()
    );
    let Meaning::Macro {
        definition: restored_macro,
        ..
    } = loaded.meaning(restored_base)
    else {
        panic!("restored macro meaning");
    };
    assert_eq!(restored_macro.raw(), base_macro.raw());

    let baseline = loaded.snapshot();
    let added = loaded.intern("job-local-name");
    let added_tokens = loaded.intern_token_list(&[Token::Cs(added.symbol())]);
    let added_macro = loaded.intern_macro(MacroMeaning::new(
        MeaningFlags::EMPTY,
        crate::ids::TokenListId::EMPTY,
        added_tokens,
    ));
    loaded.set_meaning(
        added,
        Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition: added_macro,
        },
    );
    let added_glue = loaded.intern_glue(GlueSpec {
        width: Scaled::from_raw(-7),
        stretch: Scaled::from_raw(0),
        stretch_order: Order::Normal,
        shrink: Scaled::from_raw(4),
        shrink_order: Order::Fill,
    });
    assert_eq!(loaded.resolve(added), "job-local-name");
    assert_eq!(loaded.tokens(added_tokens), [Token::Cs(added.symbol())]);
    assert_eq!(
        loaded.macro_definition(added_macro).replacement_text(),
        added_tokens
    );
    assert_eq!(loaded.glue(added_glue).width, Scaled::from_raw(-7));

    loaded.rollback(&baseline);
    assert!(loaded.symbol("job-local-name").is_none());
    assert_eq!(loaded.dump_format().expect("rollback redump"), image);
}

#[test]
fn frozen_node_arena_installs_outside_job_epoch_and_rejects_corrupt_metadata() {
    let mut universe = Universe::new();
    let child = universe.freeze_node_list(&[Node::Penalty(17)]);
    let root = universe.freeze_node_list(&[Node::Adjust(child)]);
    universe.set_box_reg(8, root);
    let image = universe.dump_format().expect("frozen node format");

    let mut loaded = Universe::from_format(World::memory(), &image).expect("load frozen nodes");
    assert_eq!(loaded.testing_epoch_node_count(), 0);
    let frozen_root = loaded.box_reg(8).expect("frozen box root");
    let local = loaded.freeze_node_list(&[Node::Adjust(frozen_root)]);
    assert_eq!(loaded.testing_epoch_node_count(), 1);
    assert!(
        matches!(loaded.nodes(local).testing_decoded(), [Node::Adjust(id)] if *id == frozen_root)
    );

    for offset in [12_usize, 32 + 24] {
        let mut corrupt = image.clone();
        replace_format_section(
            &mut corrupt,
            crate::stores::FROZEN_NODES_SECTION,
            |section| {
                section[offset] ^= 1;
            },
        );
        assert!(Universe::from_format(World::memory(), &corrupt).is_err());
    }
}

#[test]
fn checksum_valid_foundational_section_corruption_fails_structural_validation() {
    let mut universe = Universe::new();
    let name = universe.intern("corrupt-me");
    let tokens = universe.intern_token_list(&[Token::Cs(name.symbol())]);
    let definition = universe.intern_macro(MacroMeaning::new(
        MeaningFlags::EMPTY,
        crate::ids::TokenListId::EMPTY,
        tokens,
    ));
    universe.set_meaning(
        name,
        Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition,
        },
    );
    let glue = universe.intern_glue(GlueSpec {
        width: Scaled::from_raw(1),
        stretch: Scaled::from_raw(2),
        stretch_order: Order::Normal,
        shrink: Scaled::from_raw(3),
        shrink_order: Order::Fil,
    });
    universe.set_skip(0, glue);
    let valid = universe.dump_format().expect("valid frozen core");

    for (kind, offset, expected) in [
        (crate::stores::NAMES_SECTION, 24 + 16, "semantic atom"),
        (crate::stores::NAMES_LOOKUP_SECTION, 0, "lookup header"),
        (
            crate::stores::TOKEN_LISTS_SECTION,
            24 + 8,
            "semantic identity",
        ),
        (crate::stores::MACROS_SECTION, 16 + 4, "parameter reference"),
        (crate::stores::GLUE_SECTION, 16 + 14, "reserved bytes"),
    ] {
        let mut bytes = valid.clone();
        replace_format_section(&mut bytes, kind, |section| {
            if kind == crate::stores::MACROS_SECTION {
                section[offset..offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
            } else {
                section[offset] ^= 1;
            }
        });
        let error = Universe::from_format(World::memory(), &bytes)
            .expect_err("checksum-valid malformed frozen section");
        assert!(
            matches!(error, FormatError::InvalidState(ref message) if message.contains(expected)),
            "section {kind} returned {error:?}"
        );
    }
}

#[test]
fn frozen_environment_references_are_validated_against_frozen_stores() {
    let mut universe = Universe::new();
    let symbol = universe.intern("overlay-cross-store");
    let tokens = universe.intern_token_list(&[Token::Cs(symbol.symbol())]);
    let definition = universe.intern_macro(MacroMeaning::new(
        MeaningFlags::EMPTY,
        crate::ids::TokenListId::EMPTY,
        tokens,
    ));
    universe.set_meaning(
        symbol,
        Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition,
        },
    );
    let mut image = universe.dump_format().expect("cross-store format");
    let container = crate::format_container::decode(&image).expect("decode format container");
    let environment = container
        .section(crate::stores::FROZEN_ENV_SECTION)
        .expect("frozen environment section");
    let corrupt =
        crate::stores::testing_corrupt_environment_macro_reference(environment.bytes.as_ref());
    replace_format_section(&mut image, crate::stores::FROZEN_ENV_SECTION, |section| {
        *section = corrupt;
    });
    let error = Universe::from_format(World::memory(), &image)
        .expect_err("overlay reference outside frozen macro store");
    assert!(
        matches!(error, FormatError::InvalidState(ref message) if message.contains("meaning macro is not live")),
        "unexpected cross-store validation error: {error:?}"
    );
}

#[test]
fn frozen_environment_rejects_global_cells_and_bad_box_references() {
    let mut universe = Universe::new();
    let list = universe.freeze_node_list(&[Node::Penalty(12)]);
    universe.set_box_reg(3, list);
    let valid = universe.dump_format().expect("format with frozen box");
    for (corrupt, expected) in [
        (
            crate::stores::testing_corrupt_environment_global_cell as fn(&[u8]) -> Vec<u8>,
            "global environment cell",
        ),
        (
            crate::stores::testing_corrupt_environment_box_reference as fn(&[u8]) -> Vec<u8>,
            "missing box node list",
        ),
    ] {
        let container = crate::format_container::decode(&valid).expect("decode valid format");
        let environment = container
            .section(crate::stores::FROZEN_ENV_SECTION)
            .expect("frozen environment section");
        let payload = corrupt(environment.bytes.as_ref());
        let mut image = valid.clone();
        replace_format_section(&mut image, crate::stores::FROZEN_ENV_SECTION, |section| {
            *section = payload;
        });
        let error =
            Universe::from_format(World::memory(), &image).expect_err("invalid frozen environment");
        assert!(
            matches!(error, FormatError::InvalidState(ref message) if message.contains(expected)),
            "unexpected environment validation error: {error:?}"
        );
    }
}

#[test]
fn checksum_valid_frozen_environment_corruption_fails_closed() {
    let valid = Universe::new()
        .dump_format()
        .expect("valid environment format");
    for (offset, expected) in [
        (12_usize, "reserved header"),
        (16 + 8, "value tag"),
        (16 + 9, "reserved record"),
    ] {
        let mut corrupt = valid.clone();
        replace_format_section(&mut corrupt, crate::stores::FROZEN_ENV_SECTION, |section| {
            section[offset] = u8::MAX
        });
        let error = Universe::from_format(World::memory(), &corrupt)
            .expect_err("checksum-valid environment corruption");
        assert!(
            matches!(error, FormatError::InvalidState(ref message) if message.contains(expected)),
            "offset {offset} returned {error:?}"
        );
    }
}

#[test]
fn pdf_font_codes_round_trip_and_change_checkpoint_identity() {
    use crate::font::{NULL_FONT, PdfFontCode};

    let mut universe = Universe::new();
    let baseline = universe.snapshot().state_hash();
    universe.set_pdf_font_code(PdfFontCode::Lp, NULL_FONT, 255, -1_500);
    universe.set_pdf_font_code(PdfFontCode::Ef, NULL_FONT, 0, 321);
    universe.set_pdf_font_code(PdfFontCode::Knac, NULL_FONT, 128, 456);
    universe.disable_pdf_font_ligatures(NULL_FONT);
    assert_eq!(
        universe.pdf_font_code(PdfFontCode::Lp, NULL_FONT, 255),
        -1000
    );
    assert_eq!(universe.pdf_font_code(PdfFontCode::Ef, NULL_FONT, 0), 321);
    assert_ne!(universe.snapshot().state_hash(), baseline);

    let mut equivalent = Universe::new();
    equivalent.set_pdf_font_code(PdfFontCode::Lp, NULL_FONT, 255, -1_500);
    equivalent.set_pdf_font_code(PdfFontCode::Ef, NULL_FONT, 0, 321);
    equivalent.set_pdf_font_code(PdfFontCode::Knac, NULL_FONT, 128, 456);
    equivalent.disable_pdf_font_ligatures(NULL_FONT);
    assert_eq!(
        equivalent.snapshot().state_hash(),
        universe.snapshot().state_hash()
    );

    let image = universe.dump_format().expect("pdf font-code format");
    let restored = Universe::from_format(World::memory(), &image).expect("pdf font-code restore");
    assert_eq!(
        restored.pdf_font_code(PdfFontCode::Lp, NULL_FONT, 255),
        -1000
    );
    assert_eq!(restored.pdf_font_code(PdfFontCode::Ef, NULL_FONT, 0), 321);
    assert_eq!(
        restored.pdf_font_code(PdfFontCode::Knac, NULL_FONT, 128),
        456
    );
    assert!(restored.pdf_font_ligatures_disabled(NULL_FONT));
    assert_eq!(restored.dump_format().expect("canonical redump"), image);
}

#[test]
fn pdf_glyph_to_unicode_mappings_round_trip_through_formats() {
    let mut universe = Universe::new();
    universe.set_pdf_glyph_to_unicode(crate::PdfGlyphToUnicode {
        tfm_name: None,
        glyph_name: b"Digamma".to_vec(),
        unicode: vec![0x2_D7CB],
    });
    universe.set_pdf_glyph_to_unicode(crate::PdfGlyphToUnicode {
        tfm_name: Some(b"cmr10".to_vec()),
        glyph_name: b"ffi".to_vec(),
        unicode: vec![0x66, 0x66, 0x69],
    });

    let image = universe.dump_format().expect("PDF glyph format");
    let restored = Universe::from_format(World::memory(), &image).expect("PDF glyph restore");
    assert_eq!(
        restored.pdf_glyph_to_unicode(b"cmr10", b"Digamma"),
        Some([0x2_D7CB].as_slice())
    );
    assert_eq!(
        restored.pdf_glyph_to_unicode(b"cmr10", b"ffi"),
        Some([0x66, 0x66, 0x69].as_slice())
    );
    assert_eq!(restored.dump_format().expect("canonical redump"), image);
}

#[test]
fn semantic_format_rejects_live_input_and_page_state() {
    let mut with_input = Universe::new();
    with_input.set_input_summary(InputSummary::new(
        vec![InputFrameSummary::TokenList {
            token_list: crate::ids::TokenListId::EMPTY,
            origin_list: crate::ids::OriginListId::EMPTY,
            replay_kind: TokenListReplayKind::Inserted,
            index: 0,
            macro_arguments: MacroArguments::new(),
            macro_invocation: OriginId::UNKNOWN,
            parent_macro_invocation: OriginId::UNKNOWN,
        }],
        None,
        None,
    ));
    assert_eq!(with_input.dump_format(), Err(FormatError::NonEmptyInput));

    let mut with_page = Universe::new();
    with_page.set_page_integer(PageInteger::DeadCycles, 1);
    assert_eq!(with_page.dump_format(), Err(FormatError::NonEmptyPage));

    let mut with_pdf_object = Universe::new();
    with_pdf_object.enable_pdf_output();
    with_pdf_object
        .reserve_pdf_raw_object()
        .expect("reserve PDF object");
    assert_eq!(
        with_pdf_object.dump_format(),
        Err(FormatError::NonEmptyPdfDocument)
    );
}

#[test]
fn semantic_format_uses_dto_local_survivor_root_keys() {
    fn boxed_universe() -> Universe {
        let mut universe = Universe::new();
        let list = universe.freeze_node_list(&[Node::Penalty(123)]);
        universe.set_box_reg(0, list);
        universe
    }

    let first = boxed_universe();
    let second = boxed_universe();
    assert_ne!(
        first.box_reg(0).expect("first box").arena(),
        second.box_reg(0).expect("second box").arena()
    );
    assert_eq!(
        first.dump_format().expect("first format"),
        second.dump_format().expect("second format")
    );
}

#[test]
fn semantic_format_and_hash_share_permanent_symbol_keys() {
    fn symbolic_universe() -> (Universe, crate::interner::Symbol) {
        let mut universe = Universe::new();
        let symbol = universe.intern("symbolic");
        universe.set_meaning(symbol, Meaning::CountRegister(17));
        let tokens = universe.intern_token_list(&[Token::Cs(symbol.symbol())]);
        universe.set_toks(3, tokens);
        universe.set_current_font_selector(symbol, NULL_FONT);
        (universe, symbol.symbol())
    }

    let (mut first, first_key) = symbolic_universe();
    let (mut second, second_key) = symbolic_universe();
    assert_eq!(first_key, second_key);
    assert_eq!(
        first.snapshot().state_hash(),
        second.snapshot().state_hash()
    );
    assert_eq!(
        first.dump_format().expect("first symbolic format"),
        second.dump_format().expect("second symbolic format")
    );
}

#[test]
fn token_semantic_id_converges_across_cold_restore_and_fork() {
    let mut cold = Universe::new();
    let target = cold.intern("target");
    let body = cold.intern_token_list(&[
        Token::Cs(target.symbol()),
        Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        },
        Token::param(1),
    ]);
    cold.set_toks(7, body);

    let bytes = cold.dump_format().expect("token format encodes");
    let fork = cold.clone();
    let mut restored =
        Universe::from_format(World::memory(), &bytes).expect("token format restores");

    let fork_body = fork.toks(7);
    let restored_body = restored.toks(7);
    let restored_target = restored.symbol("target").expect("target restores");
    assert_eq!(
        restored.intern_token_list(&[
            Token::Cs(restored_target.symbol()),
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            },
            Token::param(1),
        ]),
        restored_body,
        "direct frozen lookup must reuse the authoritative arena record",
    );
    let semantic_id = cold.stores.testing_token_semantic_id(body);
    assert_eq!(
        fork.stores.testing_token_semantic_id(fork_body),
        semantic_id
    );
    assert_eq!(
        restored.stores.testing_token_semantic_id(restored_body),
        semantic_id
    );
    assert_eq!(fork.testing_state_hash(), cold.testing_state_hash());
    assert_eq!(restored.testing_state_hash(), cold.testing_state_hash());
    assert_eq!(restored.dump_format().expect("token format redumps"), bytes);
}

#[test]
fn paragraph_input_identity_uses_token_and_symbol_semantics_across_restore() {
    let mut cold = Universe::new();
    let target = cold.intern("target");
    let body = cold.intern_token_list(&[
        Token::Cs(target.symbol()),
        Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        },
    ]);
    cold.set_toks(7, body);
    let bytes = cold.dump_format().expect("token format encodes");
    let restored = Universe::from_format(World::memory(), &bytes).expect("format restores");
    let restored_target = restored.symbol("target").expect("target restores");

    let summary = |token_list, symbol| {
        InputSummary::new_with_source_records(
            vec![InputFrameSummary::TokenList {
                token_list,
                origin_list: crate::ids::OriginListId::EMPTY,
                replay_kind: TokenListReplayKind::MacroBody,
                index: 1,
                macro_arguments: MacroArguments::from_parts(
                    Arc::from([TracedTokenWord::pack(Token::Cs(symbol), OriginId::UNKNOWN)]),
                    [
                        Some(MacroArgumentRange::new(0, 1)),
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                    ],
                ),
                macro_invocation: OriginId::UNKNOWN,
                parent_macro_invocation: OriginId::UNKNOWN,
            }],
            None,
            None,
            None,
        )
    };
    let cold_summary = summary(body, target.symbol());
    let restored_summary = summary(restored.toks(7), restored_target.symbol());

    assert_eq!(
        cold_summary.paragraph_boundary_identity(&cold),
        restored_summary.paragraph_boundary_identity(&restored)
    );
}

#[test]
fn semantic_format_restores_validated_fonts_banks_hashes_and_rollback_exactly() {
    let mut universe = Universe::new();
    let null_identifier = universe.intern("nullfont");
    universe.set_font_identifier_symbol(NULL_FONT, null_identifier);
    let identifier = universe.intern("structuredfont");
    let font = universe.intern_font_with_identifier(structured_format_font(), identifier);
    universe.set_current_font_selector(identifier, font);
    universe.set_math_family_font(crate::math::MathFontSize::Text, 3, font, true);
    universe
        .set_font_dimen(font, 7, Scaled::from_raw(777))
        .expect("guaranteed parameter is writable");
    let font_fragment = universe.stores.testing_font_semantic_fingerprint(font);

    let bytes = universe.dump_format().expect("valid format encodes");
    let mut restored =
        Universe::from_format(World::memory(), &bytes).expect("valid format restores");
    assert_eq!(restored.dump_format().expect("format redumps"), bytes);
    let restored_font = restored.current_font();
    assert_eq!(
        restored
            .stores
            .testing_font_semantic_fingerprint(restored_font),
        font_fragment
    );
    assert_eq!(
        restored
            .font_identifier_symbol(NULL_FONT)
            .map(|symbol| restored.resolve(symbol)),
        Some("nullfont")
    );
    assert_eq!(
        restored
            .font_identifier_symbol(restored_font)
            .map(|symbol| restored.resolve(symbol)),
        Some("structuredfont")
    );
    assert_eq!(restored.font_parameter_count(restored_font), 7);
    assert_eq!(
        restored.font_parameter(restored_font, 7),
        Scaled::from_raw(777)
    );
    assert_eq!(
        restored.math_family_font(crate::math::MathFontSize::Text, 3),
        restored_font
    );
    restored
        .font_metrics(restored_font)
        .validate()
        .expect("restored metrics retain canonical invariants");

    let snapshot = restored.snapshot();
    let before_hash = snapshot.state_hash();
    restored
        .set_font_dimen(restored_font, 7, Scaled::from_raw(-9))
        .expect("font parameter mutation");
    restored.set_current_font(NULL_FONT);
    restored.set_math_family_font(crate::math::MathFontSize::Text, 3, NULL_FONT, false);
    restored.rollback(&snapshot);
    assert_eq!(restored.snapshot().state_hash(), before_hash);
    assert_eq!(restored.dump_format().expect("rollback redump"), bytes);
}

#[test]
fn checksum_valid_malformed_font_formats_fail_with_structured_errors() {
    use crate::stores::TestingFontFormatCorruption as Corruption;

    let mut universe = Universe::new();
    let identifier = universe.intern("structuredfont");
    let font = universe.intern_font_with_identifier(structured_format_font(), identifier);
    universe.set_current_font_selector(identifier, font);
    let valid = universe.dump_format().expect("valid format encodes");

    for (corruption, expected) in [
        (Corruption::TooManyCharacters, "metrics"),
        (Corruption::OversizedLigKernProgram, "cursor capacity"),
        (Corruption::LigKernStart, "lig/kern"),
        (Corruption::ExtensibleRecipeIndex, "extensible recipe"),
        (Corruption::FontIdentifier, "identifier"),
        (Corruption::FontParameterCount, "parameter count"),
        (Corruption::FontDimenSlot, "fontdimen slot"),
        (Corruption::CurrentFont, "current font"),
        (Corruption::LastLoadedFont, "last loaded font"),
    ] {
        let mut bytes = valid.clone();
        corrupt_font_format(&mut bytes, corruption);
        let error = Universe::from_format(World::memory(), &bytes)
            .expect_err("malformed font format must fail closed");
        assert!(
            matches!(error, super::FormatError::InvalidState(ref message) if message.contains(expected)),
            "{corruption:?} returned unexpected structured error: {error:?}"
        );
    }
}

#[test]
fn checksum_valid_font_formats_accept_both_lig_kern_cursor_length_edges() {
    use crate::stores::TestingFontFormatCorruption as Corruption;

    let mut universe = Universe::new();
    let identifier = universe.intern("structuredfont");
    universe.intern_font_with_identifier(structured_format_font(), identifier);
    let valid = universe.dump_format().expect("valid format encodes");

    for (len, start) in [
        (usize::from(u16::MAX), u16::MAX - 1),
        (tex_fonts::metrics::MAX_LIG_KERN_PROGRAM_LEN, u16::MAX),
    ] {
        let mut bytes = valid.clone();
        corrupt_font_format(&mut bytes, Corruption::LigKernProgramLength { len, start });
        let restored = Universe::from_format(World::memory(), &bytes)
            .expect("addressable lig/kern program restores");
        assert_eq!(restored.dump_format().expect("format redumps"), bytes);
    }
}

#[test]
fn semantic_format_validates_and_canonicalizes_glue_set_ratios() {
    const CANONICAL: (i32, i32) = (123_457, 765_431);

    let canonical =
        format_with_box_glue_set(GlueSetRatio::from_ratio_parts(CANONICAL.0, CANONICAL.1));
    let mut reducible = canonical.clone();
    replace_format_ratio(
        &mut reducible,
        CANONICAL,
        (CANONICAL.0 * 2, CANONICAL.1 * 2),
    );
    refresh_format_checksum(&mut reducible);
    let restored = Universe::from_format(World::memory(), &reducible)
        .expect("reducible glue-set ratio restores");
    assert_eq!(restored.dump_format().expect("canonical redump"), canonical);

    for malformed in [
        (CANONICAL.0, 0),
        (CANONICAL.0, -CANONICAL.1),
        (i32::MIN, CANONICAL.1),
    ] {
        let mut bytes = canonical.clone();
        replace_format_ratio(&mut bytes, CANONICAL, malformed);
        refresh_format_checksum(&mut bytes);
        let error = Universe::from_format(World::memory(), &bytes)
            .expect_err("invalid glue-set ratio must fail format restore");
        assert!(
            matches!(error, super::FormatError::InvalidState(ref message) if message.contains("glue-set ratio")),
            "unexpected structured format error: {error:?}"
        );
    }
}

fn format_with_box_glue_set(glue_set: GlueSetRatio) -> Vec<u8> {
    let mut universe = Universe::with_world(World::memory());
    let children = universe.freeze_node_list(&[]);
    let root = universe.freeze_node_list(&[Node::HList(BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(1),
        height: Scaled::from_raw(2),
        depth: Scaled::from_raw(3),
        shift: Scaled::from_raw(4),
        display: false,
        glue_set,
        glue_sign: Sign::Stretching,
        glue_order: Order::Normal,
        children,
    }))]);
    universe.set_box_reg(19, root);
    universe.dump_format().expect("format encodes")
}

#[test]
fn format_v10_round_trips_tex_web_box_shift_and_rejects_legacy_v9() {
    let mut universe = Universe::with_world(World::memory());
    let children = universe.freeze_node_list(&[]);
    let root = universe.freeze_node_list(&[Node::HList(BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(1),
        height: Scaled::from_raw(2),
        depth: Scaled::from_raw(3),
        shift: Scaled::from_raw(-4),
        display: false,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children,
    }))]);
    universe.set_box_reg(19, root);

    let bytes = universe.dump_format().expect("format encodes");
    assert_eq!(&bytes[8..12], &10_u32.to_le_bytes());
    let restored = Universe::from_format(World::memory(), &bytes).expect("v10 format restores");
    let restored_root = restored.box_reg(19).expect("box register restores");
    let [Node::HList(boxed)] = restored.nodes(restored_root).testing_decoded() else {
        panic!("box register should contain an hlist");
    };
    assert_eq!(boxed.shift, Scaled::from_raw(-4));

    let mut v9 = bytes;
    v9[8..12].copy_from_slice(&9_u32.to_le_bytes());
    assert!(matches!(
        Universe::from_format(World::memory(), &v9),
        Err(super::FormatError::UnsupportedVersion(9))
    ));
}

fn replace_format_ratio(bytes: &mut Vec<u8>, old: (i32, i32), new: (i32, i32)) {
    replace_format_section(bytes, crate::stores::FROZEN_NODES_SECTION, |section| {
        let old = [old.0.to_le_bytes(), old.1.to_le_bytes()].concat();
        let replacement = [new.0.to_le_bytes(), new.1.to_le_bytes()].concat();
        let offsets: Vec<_> = section
            .windows(old.len())
            .enumerate()
            .filter_map(|(offset, window)| (window == old).then_some(offset))
            .collect();
        assert_eq!(offsets.len(), 1, "ratio wire must occur exactly once");
        section[offsets[0]..offsets[0] + replacement.len()].copy_from_slice(&replacement);
    });
}

fn refresh_format_checksum(bytes: &mut [u8]) {
    crate::format_container::refresh_checksum(bytes);
}

fn replace_format_section(bytes: &mut Vec<u8>, kind: u32, mutate: impl FnOnce(&mut Vec<u8>)) {
    let container = crate::format_container::decode(bytes).expect("decode test container");
    let mut sections: Vec<_> = container
        .sections
        .iter()
        .map(|section| (section.kind, section.alignment, section.bytes.to_vec()))
        .collect();
    let section = sections
        .iter_mut()
        .find(|section| section.0 == kind)
        .expect("target format section");
    mutate(&mut section.2);
    let inputs: Vec<_> = sections
        .iter()
        .map(
            |(kind, alignment, bytes)| crate::format_container::SectionInput {
                kind: *kind,
                alignment: *alignment,
                bytes,
            },
        )
        .collect();
    *bytes = crate::format_container::encode(&inputs).expect("re-encode test container");
}

fn corrupt_font_format(
    bytes: &mut Vec<u8>,
    corruption: crate::stores::TestingFontFormatCorruption,
) {
    let container = crate::format_container::decode(bytes).expect("decode test container");
    let environment = container
        .section(crate::stores::FROZEN_ENV_SECTION)
        .expect("frozen environment section");
    let frozen = container
        .section(crate::stores::FONTS_SECTION)
        .expect("frozen font section");
    let (environment, frozen) = crate::stores::testing_corrupt_font_format(
        environment.bytes.as_ref(),
        frozen.bytes.as_ref(),
        corruption,
    );
    replace_format_section(bytes, crate::stores::FROZEN_ENV_SECTION, |section| {
        *section = environment;
    });
    replace_format_section(bytes, crate::stores::FONTS_SECTION, |section| {
        *section = frozen;
    });
}

#[cfg(feature = "profiling-stats")]
#[test]
fn node_memory_measurement_is_nonsemantic_and_covers_recycled_storage() {
    let mut universe = Universe::new();
    let before = universe.snapshot().state_hash();
    let empty = universe.node_memory_columns();
    assert!(empty.iter().any(|column| column.name == "epoch.words"));
    assert!(
        empty
            .iter()
            .any(|column| column.name == "epoch.identity_tags")
    );
    assert!(empty.iter().any(|column| column.name == "epoch.spans"));
    assert!(empty.iter().any(|column| {
        column.name == "epoch.semantic_ids"
            && column.element_bytes == core::mem::size_of::<crate::node_arena::NodeSemanticId>()
    }));
    assert_eq!(before, universe.snapshot().state_hash());

    for amount in 0..32 {
        let children = universe.freeze_node_list(&[Node::Kern {
            amount: Scaled::from_raw(amount),
            kind: KernKind::Explicit,
        }]);
        let root = universe.freeze_node_list(&[Node::HList(BoxNode::new(BoxNodeFields {
            width: Scaled::from_raw(amount),
            height: Scaled::from_raw(0),
            depth: Scaled::from_raw(0),
            shift: Scaled::from_raw(0),
            display: false,
            glue_set: GlueSetRatio::ZERO,
            glue_sign: Sign::Normal,
            glue_order: Order::Normal,
            children,
        }))]);
        universe.set_box_reg(0, root);
    }

    let semantic_hash = universe.snapshot().state_hash();
    let columns = universe.node_memory_columns();
    assert!(
        columns.iter().any(|column| {
            column.name == "survivor.live.boxes.rows" && column.logical_bytes > 0
        })
    );
    assert!(columns.iter().any(|column| {
        column.name == "survivor.recycled.words"
            && column.logical_bytes == 0
            && column.retained_payload_bytes > 0
    }));
    assert!(columns.iter().any(|column| {
        column.name == "survivor.root_lookup_entries"
            && column.element_bytes == core::mem::size_of::<(crate::ids::SurvivorRootId, usize)>()
            && column.logical_bytes > 0
    }));
    assert_eq!(semantic_hash, universe.snapshot().state_hash());

    let timing = crate::survivor::survivor_measurement();
    assert!(timing.fresh_promotions > 0);
    assert!(timing.recycled_promotions > 0);
    assert!(timing.releases_to_recycling > 0);
    assert!(timing.peak_promotion_scratch_retained_bytes > 0);
    let append = crate::measurement::node_append_measurement();
    assert!(append.calls > 0);
    assert!(append.words > 0);
    let hash = crate::measurement::state_hash_measurement();
    assert!(hash.calls > 0);
    assert_eq!(semantic_hash, universe.snapshot().state_hash());
}

#[test]
#[should_panic(expected = "Universe snapshot belongs to a different Universe instance")]
fn rollback_rejects_snapshot_from_different_universe() {
    let mut first = Universe::new();
    let mut second = Universe::new();
    let snapshot = first.snapshot();

    second.rollback(&snapshot);
}

#[test]
fn frozen_generation_forks_once_at_an_owner_exact_snapshot() {
    let mut universe = Universe::new();
    universe.set_count(0, 11);
    let selected = universe.snapshot();
    universe.set_count(0, 22);
    let substrate = universe.freeze_generation();

    let fork = substrate
        .fork_at(&selected)
        .expect("retained fork succeeds");
    assert_eq!(fork.count(0), 11);

    let mut foreign = Universe::new();
    let foreign = foreign.snapshot();
    assert_eq!(
        substrate
            .fork_at(&foreign)
            .expect_err("foreign root rejected"),
        GenerationForkError::ForeignSnapshot
    );
}

#[test]
fn generation_charge_covers_source_backing_and_releases_it_with_the_substrate() {
    let empty_charge = Universe::new().freeze_generation().charged_bytes();
    let bytes: Arc<[u8]> = Arc::from(vec![b'x'; 16 * 1024]);
    let mut universe = Universe::new();
    universe
        .register_source(
            SourceId::new(0),
            SourceDescriptor::generated(Arc::clone(&bytes)),
        )
        .expect("generated source registration");
    let substrate = universe.freeze_generation();

    assert!(substrate.charged_bytes() >= empty_charge + bytes.len());
    assert!(Arc::strong_count(&bytes) > 1);
    drop(substrate);
    assert_eq!(Arc::strong_count(&bytes), 1);
}

#[test]
fn accepted_paragraph_mount_owns_shared_root_across_rollback() {
    let mut universe = Universe::new();
    let before = universe.snapshot();
    let glue = universe.intern_glue(GlueSpec {
        width: Scaled::from_raw(11),
        ..GlueSpec::ZERO
    });
    let old_epoch = universe.freeze_node_list(&[
        crate::node::Node::Penalty(11),
        crate::node::Node::Glue {
            spec: glue,
            kind: GlueKind::Normal,
            leader: None,
        },
    ]);
    let old = universe.retain_paragraph_result(old_epoch);
    let old_semantic = universe.stores.node_list_semantic_fragment(old.id());
    assert_eq!(
        universe.stores.testing_survivor_payload_strong_count(&old),
        2,
        "the local survivor slot and accepted-history mount share one payload"
    );

    universe.rollback(&before);
    assert_eq!(
        universe.stores.testing_survivor_payload_strong_count(&old),
        1,
        "rollback releases only the ordinary local survivor pin"
    );
    let carried = old.clone();
    assert_eq!(
        universe.stores.testing_survivor_payload_strong_count(&old),
        2,
        "carried accepted history adds one shared payload owner"
    );
    drop(carried);
    assert_eq!(
        universe.stores.testing_survivor_payload_strong_count(&old),
        1,
        "discarding accepted history is independent of graph size"
    );
    assert!(universe.can_mount_retained_paragraph_result(&old));
    let mounted_old = universe
        .mount_retained_paragraph_result(&old, &[], &[])
        .expect("accepted-history mount must outlive checkpoint rollback");
    assert_eq!(
        universe.stores.node_list_semantic_fragment(mounted_old),
        old_semantic,
        "mounting must preserve the sealed semantic identity"
    );
}

#[test]
fn restarted_universe_mounts_shared_paragraph_graph_with_local_provenance() {
    let mut universe = Universe::new();
    let before = universe.snapshot();
    let old_origin = universe.synthetic_origin(SyntheticOriginKind::Test);
    let glue = universe.intern_glue(GlueSpec {
        width: Scaled::from_raw(17),
        ..GlueSpec::ZERO
    });
    let children = universe.freeze_node_list(&[
        Node::Char {
            font: NULL_FONT,
            ch: 'x',
            origin: old_origin,
        },
        Node::Glue {
            spec: glue,
            kind: GlueKind::Normal,
            leader: None,
        },
    ]);
    let Some(crate::node_arena::NodeRef::Glue {
        spec: stored_glue, ..
    }) = universe.nodes(children).get(1)
    else {
        panic!("child glue must be stored");
    };
    let lines = universe.freeze_node_list(&[Node::HList(BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(17),
        height: Scaled::from_raw(7),
        depth: Scaled::from_raw(2),
        shift: Scaled::from_raw(0),
        display: false,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children,
    }))]);
    let retained = universe.retain_paragraph_result(lines);
    let semantic = universe.stores.node_list_semantic_fragment(retained.id());
    universe.rollback(&before);
    let mut restarted = universe.clone();

    assert_eq!(
        universe
            .stores
            .testing_survivor_payload_strong_count(&retained),
        1,
        "accepted history alone owns the shared payload after rollback"
    );
    assert!(restarted.can_mount_retained_paragraph_result(&retained));
    let current_origin = restarted.synthetic_origin(SyntheticOriginKind::Engine);
    assert_eq!(
        restarted.mount_retained_paragraph_result(&retained, &[current_origin], &[0]),
        Some(retained.id()),
        "mounting preserves the ordinary node-list handle"
    );
    assert_eq!(
        restarted
            .stores
            .testing_survivor_payload_strong_count(&retained),
        2,
        "the mounted local slot shares accepted-history storage"
    );
    let Some(crate::node_arena::NodeRef::HList(line)) = restarted.nodes(retained.id()).first()
    else {
        panic!("mounted root must remain a line box");
    };
    let mounted = restarted.nodes(line.children).to_vec();
    assert!(matches!(
        mounted.first(),
        Some(Node::Char { origin, .. }) if *origin == current_origin
    ));
    assert_eq!(restarted.glue(stored_glue).width, Scaled::from_raw(17));
    assert_eq!(
        restarted.stores.node_list_semantic_fragment(retained.id()),
        semantic,
        "mount-local provenance must not change semantic identity"
    );
}

#[test]
fn paragraph_mount_rejects_unsupported_handle_bearing_nodes() {
    let mut universe = Universe::new();
    let graph = universe.freeze_node_list(&[Node::Mark {
        class: 0,
        tokens: crate::ids::TokenListId::EMPTY,
    }]);
    let retained = universe.retain_paragraph_result(graph);
    assert!(!universe.can_mount_retained_paragraph_result(&retained));
}

#[test]
fn paragraph_mount_rejects_a_font_missing_from_the_current_store() {
    let mut universe = Universe::new();
    let before_font = universe.snapshot();
    let font = universe.intern_font(test_font("transient", b"transient"));
    let origin = universe.synthetic_origin(SyntheticOriginKind::Test);
    let graph = universe.freeze_node_list(&[Node::Char {
        font,
        ch: 'x',
        origin,
    }]);
    let retained = universe.retain_paragraph_result(graph);

    assert!(universe.can_mount_retained_paragraph_result(&retained));
    universe.rollback(&before_font);
    assert!(!universe.can_mount_retained_paragraph_result(&retained));
}

#[test]
fn generation_fork_detaches_the_accepted_effect_prefix() {
    let mut universe = Universe::new();
    universe.begin_retained_session().expect("retained session");
    universe
        .world_mut()
        .write_text(PrintSink::Log, "accepted prefix");
    let selected_pos = universe.world().effect_pos();
    let selected = universe.snapshot();
    universe
        .world_mut()
        .write_text(PrintSink::Log, "accepted suffix");
    let substrate = universe.freeze_generation();

    let mut fork = substrate
        .fork_at(&selected)
        .expect("retained fork succeeds");
    assert_eq!(fork.world().effect_pos(), selected_pos);
    assert!(fork.world().effect_records().is_empty());
    fork.world_mut().write_text(PrintSink::Log, "scratch tail");
    assert_eq!(fork.world().effect_pos().raw(), selected_pos.raw() + 1);
    assert!(matches!(
        fork.world().effect_records(),
        [EffectRecord::StreamWrite { text, .. }] if text == "scratch tail"
    ));
    assert_eq!(substrate.world().effect_records().len(), 2);
}

#[test]
fn generation_fork_detaches_the_accepted_artifact_prefix() {
    let mut universe = Universe::new();
    universe.begin_retained_session().expect("retained session");
    let first = universe.begin_shipout();
    let effect_pos = first.world().effect_pos();
    first
        .commit(
            crate::VerifiedArtifact::new(b"accepted page".to_vec()),
            effect_pos,
        )
        .expect("accepted shipout");
    let selected = universe.snapshot();
    let selected_pos = universe.world().artifact_pos();
    let substrate = universe.freeze_generation();

    let mut fork = substrate
        .fork_at(&selected)
        .expect("retained fork succeeds");
    assert_eq!(fork.world().artifact_pos(), selected_pos);
    assert!(fork.world().committed_artifacts().is_empty());
    let scratch = fork.begin_shipout();
    let effect_pos = scratch.world().effect_pos();
    scratch
        .commit(
            crate::VerifiedArtifact::new(b"scratch page".to_vec()),
            effect_pos,
        )
        .expect("scratch shipout");
    assert_eq!(fork.world().artifact_pos(), selected_pos + 1);
    assert!(matches!(
        fork.world().committed_artifacts(),
        [artifact] if artifact.bytes() == b"scratch page"
    ));
    assert!(matches!(
        substrate.world().committed_artifacts(),
        [artifact] if artifact.bytes() == b"accepted page"
    ));
}

#[test]
fn generation_retains_related_fork_diagnostic_graph_and_foreign_location() {
    let mut universe = Universe::new();
    let anchor = universe.snapshot();
    let mut substrate = universe.freeze_generation();
    let mut fork = substrate.fork_at(&anchor).expect("related fork");
    fork.register_source(
        SourceId::new(0),
        SourceDescriptor::named_generated("scratch.tex", Arc::from(&b"abc"[..])),
    )
    .expect("scratch source registration");
    let source = fork.source_range_origin(SourceId::new(0), 0, 3);
    let derived = fork.synthesized_origin(SynthesizedOriginKind::ValueRendering, source);

    substrate
        .retain_artifact_origins_from_fork(&fork, &[derived])
        .expect("related diagnostics retained");
    assert_eq!(
        substrate
            .resolve_origin(derived)
            .expect("owned scratch location"),
        crate::ResolvedSourceLocation {
            path: "scratch.tex".to_owned(),
            start: 0,
            end: 3,
            line: 1,
            column: 1,
        }
    );

    let unrelated = Universe::new();
    assert_eq!(
        substrate
            .retain_artifact_origins_from_fork(&unrelated, &[derived])
            .expect_err("unrelated universe rejected"),
        GenerationForkError::UnrelatedFork
    );
}

#[test]
fn promoted_fork_retargets_only_the_bit_identical_prefix() {
    let mut universe = Universe::new();
    let prefix = universe.snapshot();
    universe.set_count(0, 1);
    let after_anchor = universe.snapshot();
    let source = universe.freeze_generation();

    let mut fork = source.fork_at(&prefix).expect("fork at prefix");
    fork.set_count(1, 2);
    let target = fork.freeze_generation();
    let retargeted = target
        .retarget_prefix_from(&source, &prefix)
        .expect("prefix retargets");
    let restored = target
        .fork_at(&retargeted)
        .expect("retargeted root restores");
    assert_eq!(restored.count(0), 0);
    assert_eq!(
        target
            .fork_at(&prefix)
            .expect_err("cross-substrate checkpoint rejected"),
        GenerationForkError::ForeignSnapshot
    );
    assert_eq!(
        target
            .retarget_prefix_from(&source, &after_anchor)
            .expect_err("post-anchor record rejected"),
        GenerationForkError::PrefixBeyondForkAnchor
    );

    let unrelated = Universe::new().freeze_generation();
    assert_eq!(
        unrelated
            .retarget_prefix_from(&source, &prefix)
            .expect_err("unrelated target rejected"),
        GenerationForkError::UnrelatedFork
    );
}

#[test]
fn rollback_restores_store_tuple_and_placeholder_scalars() {
    let mut universe = Universe::new();
    let symbol = universe.intern("x");
    let snapshot = universe.snapshot();

    universe.set_meaning(symbol, Meaning::Relax);
    universe.rollback(&snapshot);

    assert_eq!(universe.meaning(symbol), Meaning::Undefined);
}

#[test]
fn snapshot_round_trip_keeps_active_and_named_meanings_independent() {
    let mut universe = Universe::new();
    let named = universe.intern("~");
    let active = universe.intern_active_character('~');
    universe.set_meaning(named, Meaning::CharGiven('N'));
    universe.set_meaning(active, Meaning::CharGiven('A'));
    let snapshot = universe.snapshot();

    universe.set_meaning(named, Meaning::Relax);
    universe.set_meaning(active, Meaning::Undefined);
    universe.rollback(&snapshot);

    assert_eq!(universe.meaning(named), Meaning::CharGiven('N'));
    assert_eq!(universe.meaning(active), Meaning::CharGiven('A'));
}

#[test]
fn provenance_is_accessible_through_universe_boundary() {
    let mut universe = Universe::new();
    let source = universe.source_origin(crate::input::SourceId::new(11), 80, 6, 4);
    let synthetic = universe.synthetic_origin(SyntheticOriginKind::Engine);
    let list = universe.allocate_origin_list(&[source, synthetic]);

    assert_eq!(universe.bootstrap_origin(), OriginId::UNKNOWN);
    assert_eq!(
        universe.origin(source),
        OriginRecord::Source(SourceOrigin::new(crate::input::SourceId::new(11), 80, 6, 4))
    );
    assert_eq!(universe.origin_list(list), &[source, synthetic]);
}

#[test]
fn semantic_hash_ignores_provenance_allocations() {
    let mut universe = Universe::new();
    let base_snapshot = universe.snapshot();
    let base_checkpoint_hash = base_snapshot.state_hash();
    let base_testing_hash = universe.testing_state_hash();

    let source = universe.source_origin(crate::input::SourceId::new(1), 0, 1, 1);
    let synthetic = universe.synthetic_origin(SyntheticOriginKind::Engine);
    let _list = universe.allocate_origin_list(&[source, synthetic]);
    let after_snapshot = universe.snapshot();

    assert_eq!(after_snapshot.state_hash(), base_checkpoint_hash);
    assert_eq!(universe.testing_state_hash(), base_testing_hash);
}

#[test]
fn semantic_hash_ignores_source_map_identities_and_generated_backing() {
    let mut universe = Universe::new();
    let baseline = universe.snapshot().state_hash();
    universe
        .register_source(
            crate::SourceId::new(4),
            SourceDescriptor::generated(Arc::from(&b"diagnostic only"[..])),
        )
        .expect("source-map integration operation succeeds");

    assert_eq!(universe.snapshot().state_hash(), baseline);
}

#[test]
fn world_and_source_map_rollback_reuse_ids_and_positions_atomically() {
    let mut world = World::memory();
    world
        .set_memory_file("input.tex", b"old".to_vec())
        .expect("source-map integration operation succeeds");
    let mut universe = Universe::with_world(world);
    let snapshot = universe.snapshot();

    let old = universe
        .world_mut()
        .read_file("input.tex")
        .expect("source-map integration operation succeeds");
    let old_record = old.record();
    let old_start = universe
        .register_source(
            crate::SourceId::new(0),
            SourceDescriptor::world(old.record(), old.bytes().len() as u64),
        )
        .expect("source-map integration operation succeeds");
    universe.rollback(&snapshot);
    assert!(universe.world().input_record(old_record).is_none());
    assert_eq!(
        universe.source_position(crate::SourceId::new(0), 0),
        Err(SourceMapError::UnknownSource)
    );

    universe
        .world_mut()
        .set_memory_file("input.tex", b"new".to_vec())
        .expect("source-map integration operation succeeds");
    let new = universe
        .world_mut()
        .read_file("input.tex")
        .expect("source-map integration operation succeeds");
    assert_eq!(new.record().raw(), old_record.raw());
    assert_ne!(new.record(), old_record);
    assert!(universe.world().input_record(old_record).is_none());
    assert_eq!(
        universe.register_source(
            crate::SourceId::new(0),
            SourceDescriptor::world(old_record, old.bytes().len() as u64),
        ),
        Err(SourceMapError::MissingWorldInput)
    );
    let new_start = universe
        .register_source(
            crate::SourceId::new(0),
            SourceDescriptor::world(new.record(), new.bytes().len() as u64),
        )
        .expect("source-map integration operation succeeds");
    assert_ne!(new_start, old_start);
    assert_eq!(
        universe.source_backing_bytes(
            universe
                .source_region(crate::SourceId::new(0))
                .expect("source-map integration operation succeeds")
        ),
        Some(&b"new"[..])
    );
}

#[test]
fn world_registration_checks_record_liveness_and_length() {
    let mut missing = Universe::new();
    assert_eq!(
        missing.register_source(
            crate::SourceId::new(0),
            SourceDescriptor::world(crate::InputRecordId::new(0), 0),
        ),
        Err(SourceMapError::MissingWorldInput)
    );

    let mut world = World::memory();
    world
        .set_memory_file("input.tex", b"abc".to_vec())
        .expect("source-map integration operation succeeds");
    let mut universe = Universe::with_world(world);
    let content = universe
        .world_mut()
        .read_file("input.tex")
        .expect("source-map integration operation succeeds");
    assert_eq!(
        universe.register_source(
            crate::SourceId::new(0),
            SourceDescriptor::world(content.record(), 99),
        ),
        Err(SourceMapError::WorldInputLengthMismatch)
    );
}

#[test]
fn semantic_hash_ignores_pending_source_token_origins() {
    let mut universe = Universe::new();
    let registration = universe
        .register_input_source(
            crate::input::SourceId::new(1),
            SourceDescriptor::generated(std::sync::Arc::from(&b"x"[..])),
        )
        .expect("pending source summary needs a live generated backing");
    let token = Token::Char {
        ch: 'x',
        cat: Catcode::Letter,
    };
    let left_origin = universe.source_origin(crate::input::SourceId::new(1), 0, 1, 1);
    let right_origin = universe.source_origin(crate::input::SourceId::new(1), 14, 3, 9);
    let left_summary = pending_source_summary(token, left_origin, registration);
    let right_summary = pending_source_summary(token, right_origin, registration);
    assert_eq!(left_summary, right_summary);

    universe.set_input_summary(left_summary);
    let left_hash = universe.snapshot().state_hash();
    universe.set_input_summary(right_summary);
    let right_hash = universe.snapshot().state_hash();

    assert_eq!(left_hash, right_hash);
}

#[test]
fn transient_input_hash_uses_stable_control_sequence_atoms_and_ignores_origins() {
    let mut first = Universe::new();
    let first_symbol = first.intern("transient-name");
    let first_origin = first.source_origin(SourceId::new(1), 10, 2, 3);
    first.set_input_summary(transient_summary(TracedTokenWord::pack(
        Token::Cs(first_symbol.symbol()),
        first_origin,
    )));

    let mut second = Universe::new();
    second.intern("different-allocation-order");
    let second_symbol = second.intern("transient-name");
    let second_origin = second.source_origin(SourceId::new(9), 90, 8, 7);
    second.set_input_summary(transient_summary(TracedTokenWord::pack(
        Token::Cs(second_symbol.symbol()),
        second_origin,
    )));

    assert_eq!(
        first.snapshot().state_hash(),
        second.snapshot().state_hash()
    );
}

#[test]
fn transient_input_validation_rejects_stale_packed_symbols_atomically() {
    let mut universe = Universe::new();
    let mark = universe.snapshot();
    let stale = universe.intern("rolled-back-transient");
    universe.rollback(&mark);
    universe.intern("replacement-transient");
    let invalid = transient_summary(TracedTokenWord::pack(
        Token::Cs(stale.symbol()),
        OriginId::UNKNOWN,
    ));

    assert!(catch_unwind(AssertUnwindSafe(|| universe.set_input_summary(invalid))).is_err());
    assert_eq!(universe.input_summary(), &InputSummary::default());
}

#[test]
fn input_hash_ignores_source_ids_and_allocator_history() {
    let mut universe = Universe::new();
    let first_registration = universe
        .register_input_source(
            SourceId::new(1),
            SourceDescriptor::generated(Arc::from(&b"x"[..])),
        )
        .expect("first generated source");
    let second_registration = universe
        .register_input_source(
            SourceId::new(99),
            SourceDescriptor::generated(Arc::from(&b"x"[..])),
        )
        .expect("second generated source");
    let token = Token::Char {
        ch: 'x',
        cat: Catcode::Letter,
    };
    let first = source_summary_with_identity(token, SourceId::new(1), first_registration, 2);
    let second =
        source_summary_with_identity(token, SourceId::new(99), second_registration, 10_000);

    universe.set_input_summary(first);
    let first_hash = universe.snapshot().state_hash();
    universe.set_input_summary(second);

    assert_eq!(universe.snapshot().state_hash(), first_hash);
}

#[test]
fn input_summary_validation_is_recursive_and_atomic_after_reuse() {
    let mut universe = Universe::new();
    let mark = universe.snapshot();
    let stale_registration = universe
        .register_input_source(
            crate::SourceId::new(1),
            SourceDescriptor::generated(Arc::from(&b"x"[..])),
        )
        .expect("register discarded source");
    let stale_symbol = universe.intern("discarded");
    let stale_origin = universe.synthetic_origin(SyntheticOriginKind::Test);
    let stale_word = TracedTokenWord::pack(Token::Cs(stale_symbol.symbol()), stale_origin);
    let stale_list = universe.finish_traced_token_list(&[stale_word]);
    universe.rollback(&mark);

    let registration = universe
        .register_input_source(
            crate::SourceId::new(1),
            SourceDescriptor::generated(Arc::from(&b"x"[..])),
        )
        .expect("register replacement source");
    let symbol = universe.intern("replacement");
    let origin = universe.synthetic_origin(SyntheticOriginKind::Engine);
    let word = TracedTokenWord::pack(Token::Cs(symbol.symbol()), origin);
    let list = universe.finish_traced_token_list(&[word]);
    assert_ne!(registration, stale_registration);
    assert_ne!(list, stale_list);

    let source = |registration, pending| {
        SourceFrameSummary::new(
            0,
            1,
            1,
            0,
            LexerState::MidLine,
            "x".to_owned(),
            0,
            vec![pending],
            false,
        )
        .with_registration(Some(registration))
    };
    let token_frame = |traced: TracedTokenList, arguments: MacroArguments, invocation| {
        InputFrameSummary::TokenList {
            token_list: traced.token_list(),
            origin_list: traced.origin_list(),
            replay_kind: TokenListReplayKind::MacroBody,
            index: 0,
            macro_arguments: arguments,
            macro_invocation: invocation,
            parent_macro_invocation: OriginId::UNKNOWN,
        }
    };

    let stale_argument = one_macro_argument(stale_word, 1);
    let mut invalid = vec![
        InputSummary::new(
            vec![InputFrameSummary::Source {
                source_id: crate::SourceId::new(1),
                input_record: None,
                source: source(stale_registration, word),
            }],
            None,
            None,
        ),
        InputSummary::new(
            vec![token_frame(
                stale_list,
                MacroArguments::new(),
                OriginId::UNKNOWN,
            )],
            None,
            None,
        ),
        InputSummary::new(
            vec![token_frame(list, stale_argument, OriginId::UNKNOWN)],
            None,
            None,
        ),
        InputSummary::new(
            vec![token_frame(list, MacroArguments::new(), stale_origin)],
            None,
            None,
        ),
        InputSummary::new(
            vec![InputFrameSummary::Condition {
                token: crate::input::ConditionFrameToken::new(7),
                condition: crate::input::ConditionFrameSummary::evaluating_if(stale_word),
            }],
            None,
            None,
        ),
        InputSummary::new(
            Vec::new(),
            Some(crate::SourceId::new(1)),
            Some(source(stale_registration, word)),
        ),
    ];

    let mut foreign = Universe::new();
    let foreign_registration = foreign
        .register_input_source(
            crate::SourceId::new(1),
            SourceDescriptor::generated(Arc::from(&b"x"[..])),
        )
        .expect("register foreign source");
    let foreign_symbol = foreign.intern("foreign");
    let foreign_origin = foreign.synthetic_origin(SyntheticOriginKind::Test);
    let foreign_word = TracedTokenWord::pack(Token::Cs(foreign_symbol.symbol()), foreign_origin);
    let foreign_list = foreign.finish_traced_token_list(&[foreign_word]);
    invalid.extend([
        InputSummary::new(
            vec![InputFrameSummary::Source {
                source_id: crate::SourceId::new(1),
                input_record: None,
                source: source(foreign_registration, word),
            }],
            None,
            None,
        ),
        InputSummary::new(
            vec![token_frame(
                foreign_list,
                MacroArguments::new(),
                OriginId::UNKNOWN,
            )],
            None,
            None,
        ),
        InputSummary::new(
            vec![InputFrameSummary::Condition {
                token: crate::input::ConditionFrameToken::new(9),
                condition: crate::input::ConditionFrameSummary::evaluating_if(foreign_word),
            }],
            None,
            None,
        ),
    ]);
    for summary in invalid {
        assert!(catch_unwind(AssertUnwindSafe(|| universe.set_input_summary(summary))).is_err());
        assert_eq!(universe.input_summary(), &InputSummary::default());
    }

    let arguments = one_macro_argument(word, 9);
    let valid = InputSummary::new(
        vec![
            InputFrameSummary::Source {
                source_id: crate::SourceId::new(1),
                input_record: None,
                source: source(registration, word),
            },
            token_frame(list, arguments, origin),
            InputFrameSummary::Condition {
                token: crate::input::ConditionFrameToken::new(8),
                condition: crate::input::ConditionFrameSummary::evaluating_if(word),
            },
        ],
        None,
        None,
    );
    universe.set_input_summary(valid.clone());
    assert_eq!(universe.input_summary(), &valid);
    let checkpoint = universe.snapshot();
    universe.set_input_summary(InputSummary::default());
    universe.rollback(&checkpoint);
    assert_eq!(universe.input_summary(), &valid);
}

#[test]
fn semantic_hash_distinguishes_evaluating_conditional_state() {
    let mut universe = Universe::new();
    let token = crate::input::ConditionFrameToken::new(0);
    let context = TracedTokenWord::pack(Token::frozen_end_template(), OriginId::UNKNOWN);
    universe.set_input_summary(InputSummary::new(
        vec![InputFrameSummary::Condition {
            token,
            condition: crate::input::ConditionFrameSummary::evaluating_if(context),
        }],
        None,
        None,
    ));
    let evaluating = universe.snapshot().state_hash();
    universe.set_input_summary(InputSummary::new(
        vec![InputFrameSummary::Condition {
            token,
            condition: crate::input::ConditionFrameSummary::new_if(context, false),
        }],
        None,
        None,
    ));

    assert_ne!(universe.snapshot().state_hash(), evaluating);
}

#[test]
fn semantic_hash_ignores_conditional_frame_identity() {
    let mut universe = Universe::new();
    let context = TracedTokenWord::pack(Token::frozen_end_template(), OriginId::UNKNOWN);
    let summary = |raw| {
        InputSummary::new(
            vec![InputFrameSummary::Condition {
                token: crate::input::ConditionFrameToken::new(raw),
                condition: crate::input::ConditionFrameSummary::new_if(context, true),
            }],
            None,
            None,
        )
    };
    universe.set_input_summary(summary(3));
    let first = universe.snapshot().state_hash();
    universe.set_input_summary(summary(91));

    assert_eq!(universe.snapshot().state_hash(), first);
}

#[test]
fn snapshot_reuses_hash_base_for_origin_only_input_summary_changes() {
    let mut universe = Universe::new();
    let body_token = Token::Char {
        ch: 'm',
        cat: Catcode::Letter,
    };
    let body = universe.intern_token_list(&[body_token]);
    let params = universe.intern_token_list(&[]);
    let definition = universe.intern_macro(MacroMeaning::new(MeaningFlags::EMPTY, params, body));
    let left_origin = universe.source_origin(crate::input::SourceId::new(1), 10, 2, 3);
    let right_origin = universe.source_origin(crate::input::SourceId::new(2), 20, 4, 5);
    let left_origins = universe.allocate_origin_list(&[left_origin]);
    let right_origins = universe.allocate_origin_list(&[right_origin]);
    let left_invocation =
        universe.macro_invocation_origin(definition, left_origin, left_origin, OriginId::UNKNOWN);
    let right_invocation =
        universe.macro_invocation_origin(definition, right_origin, right_origin, OriginId::UNKNOWN);
    let left_summary = macro_replay_summary(body, left_origins, left_invocation, left_origin);
    let right_summary = macro_replay_summary(body, right_origins, right_invocation, right_origin);
    assert_eq!(left_summary, right_summary);

    universe.set_input_summary(left_summary);
    let first = universe.snapshot();
    universe.set_input_summary(right_summary);
    let second = universe.snapshot();

    assert_eq!(first.state_hash(), second.state_hash());
}

#[test]
fn universe_rollback_truncates_provenance_without_reviving_origin_ids() {
    let mut universe = Universe::new();
    let mark = universe.snapshot();

    let stale = universe.source_origin(crate::input::SourceId::new(7), 70, 8, 9);
    let stale_list = universe.allocate_origin_list(&[stale]);
    assert!(universe.origin_if_live(stale).is_some());
    assert!(universe.origin_list_if_live(stale_list).is_some());

    universe.rollback(&mark);
    assert_eq!(universe.origin_if_live(stale), None);
    assert_eq!(universe.origin_list_if_live(stale_list), None);

    let replayed = universe.source_origin(crate::input::SourceId::new(7), 70, 8, 9);
    let replayed_list = universe.allocate_origin_list(&[replayed]);
    assert_ne!(replayed.raw(), stale.raw());
    assert_eq!(replayed_list.raw(), stale_list.raw());
    assert_ne!(replayed_list, stale_list);
    assert_eq!(
        universe.origin(replayed),
        OriginRecord::Source(SourceOrigin::new(crate::input::SourceId::new(7), 70, 8, 9))
    );
    assert_eq!(universe.origin_list(replayed_list), &[replayed]);
}

#[test]
fn rollback_rejects_dropped_effect_snapshot_before_mutating_stores() {
    let mut universe = Universe::new();
    let symbol = universe.intern("x");
    let snapshot = universe.snapshot();

    universe.set_meaning(symbol, Meaning::Relax);
    universe
        .world_mut()
        .write_text(PrintSink::TerminalAndLog, "committed\n");
    let effect_pos = universe.world().effect_pos();
    universe
        .commit_effects(effect_pos)
        .expect("memory world commit succeeds");
    let live_hash = universe.testing_state_hash();

    let result = catch_unwind(AssertUnwindSafe(|| universe.rollback(&snapshot)));

    assert!(result.is_err());
    assert_eq!(universe.meaning(symbol), Meaning::Relax);
    assert_eq!(universe.testing_state_hash(), live_hash);
}

#[test]
fn rollback_restores_page_builder_state_and_hash() {
    let mut universe = Universe::new();
    let base_hash = universe.testing_state_hash();
    let snapshot = universe.snapshot();
    let glue = universe.intern_glue(GlueSpec {
        width: Scaled::from_raw(3),
        stretch: Scaled::from_raw(1),
        stretch_order: Order::Normal,
        shrink: Scaled::from_raw(0),
        shrink_order: Order::Normal,
    });

    universe.set_page_dimension(PageDimension::Goal, Scaled::from_raw(100));
    universe.set_page_dimension(PageDimension::Total, Scaled::from_raw(25));
    universe.set_page_integer(PageInteger::InsertPenalties, 7);
    universe.append_page_contribution(Node::Glue {
        spec: glue,
        kind: GlueKind::Normal,

        leader: None,
    });
    universe.push_current_page_node(Node::Penalty(42));
    universe.record_best_page_break(1, Scaled::from_raw(100), 12);
    universe.record_page_fire_up(1);

    assert_ne!(universe.testing_state_hash(), base_hash);
    universe.rollback(&snapshot);

    assert_eq!(universe.testing_state_hash(), base_hash);
    assert!(universe.page_contributions().is_empty());
    assert_eq!(universe.current_page_len(), 0);
    assert_eq!(
        universe.page_dimension(PageDimension::Goal),
        Scaled::MAX_DIMEN
    );
    assert_eq!(universe.page_integer(PageInteger::InsertPenalties), 0);
    assert!(universe.page_fire_up().is_none());
}

#[test]
fn replay_probe_drop_restores_semantic_page_store_and_world_state() {
    let mut universe = Universe::with_world(World::memory());
    let base_hash = universe.testing_state_hash();

    {
        let mut probe = universe.begin_replay_probe();
        probe.set_count(7, 91);
        probe.append_page_contribution(Node::Penalty(17));
        probe.record_page_fire_up(3);
        probe
            .world_mut()
            .write_text(PrintSink::TerminalAndLog, "speculative\n");
    }

    assert_eq!(universe.testing_state_hash(), base_hash);
    assert_eq!(universe.count(7), 0);
    assert!(universe.page_contributions().is_empty());
    assert!(universe.page_fire_up().is_none());
    assert!(universe.world().effect_records().is_empty());
}

#[test]
fn replay_probe_commit_keeps_semantic_transition() {
    let mut universe = Universe::new();
    let mut probe = universe.begin_replay_probe();
    probe.set_count(7, 91);
    probe.append_page_contribution(Node::Penalty(17));
    probe.record_page_fire_up(3);
    probe.commit();

    assert_eq!(universe.count(7), 91);
    assert_eq!(universe.page_contributions(), &[Node::Penalty(17)]);
    assert_eq!(
        universe.page_fire_up().map(|fire| fire.trigger().index()),
        Some(3)
    );
}

#[test]
fn rollback_bumps_epoch_past_previous_live_epoch() {
    let mut universe = Universe::new();
    let snapshot = universe.snapshot();
    let before_rollback = universe.stores.env().epoch();

    universe.rollback(&snapshot);

    assert!(snapshot.epoch() < before_rollback);
    assert!(before_rollback < universe.stores.env().epoch());
}

#[test]
fn job_clock_initializes_tex_clock_parameters_once() {
    let clock = JobClock {
        time: 721,
        second: 37,
        day: 8,
        month: 7,
        year: 2026,
    };
    let universe = Universe::with_world(World::memory_with_clock(clock));

    assert_eq!(universe.int_param(crate::env::banks::IntParam::TIME), 721);
    assert_eq!(universe.int_param(crate::env::banks::IntParam::DAY), 8);
    assert_eq!(universe.int_param(crate::env::banks::IntParam::MONTH), 7);
    assert_eq!(universe.int_param(crate::env::banks::IntParam::YEAR), 2026);
}

#[test]
fn format_load_refreshes_tex_clock_parameters_for_the_new_job() {
    let format_clock = JobClock {
        time: 721,
        second: 37,
        day: 8,
        month: 7,
        year: 2026,
    };
    let format = Universe::with_world(World::memory_with_clock(format_clock))
        .dump_format()
        .expect("format encodes");
    let job_clock = JobClock {
        time: 15,
        second: 0,
        day: 1,
        month: 11,
        year: 2024,
    };

    let restored = Universe::from_format(World::memory_with_clock(job_clock), &format)
        .expect("format restores for a new job");

    assert_eq!(restored.int_param(crate::env::banks::IntParam::TIME), 15);
    assert_eq!(restored.int_param(crate::env::banks::IntParam::DAY), 1);
    assert_eq!(restored.int_param(crate::env::banks::IntParam::MONTH), 11);
    assert_eq!(restored.int_param(crate::env::banks::IntParam::YEAR), 2024);
}

#[test]
fn rollback_restores_world_inputs_stream_buffers_and_rng() {
    let mut universe = Universe::new();
    universe
        .world_mut()
        .set_memory_file("main.tex", b"abc".to_vec())
        .expect("seed memory file");
    let slot = StreamSlot::new(2);
    let snapshot = universe.snapshot();

    let read = universe
        .world_mut()
        .open_in(slot, "main.tex")
        .expect("read file through world");
    universe.world_mut().open_out(slot, "main.aux");
    universe
        .world_mut()
        .write_text(PrintSink::Stream(slot), "partial");
    let random = universe.world_mut().next_random_u64();
    assert_eq!(read.hash(), ContentHash::from_bytes(b"abc"));
    assert_eq!(universe.world().input_records().len(), 1);

    universe.rollback(&snapshot);

    assert!(universe.world().input_records().is_empty());
    assert_eq!(universe.world().stream_bufs().partial_line(slot), "");
    assert!(
        universe
            .world()
            .stream_bufs()
            .read_stream_path(slot)
            .is_none()
    );
    assert_eq!(universe.world_mut().next_random_u64(), random);
}

#[test]
fn shipout_commit_flushes_releases_then_checkpoints() {
    let mut universe = Universe::new();
    let base = universe.snapshot();
    let mut transaction = universe.begin_shipout();
    let children = transaction.freeze_node_list(&[Node::Kern {
        amount: Scaled::from_raw(7),
        kind: KernKind::Explicit,
    }]);
    let page = Node::HList(BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(7),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
        shift: Scaled::from_raw(0),
        display: false,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children,
    }));
    assert!(matches!(page, Node::HList(_)));
    assert_eq!(transaction.testing_epoch_node_count(), 1);

    transaction
        .world_mut()
        .write_text(PrintSink::TerminalAndLog, "shipout\n");
    let effect_pos = transaction.world().effect_pos();
    let hash = transaction
        .commit(
            crate::VerifiedArtifact::new(b"detached page artifact".to_vec()),
            effect_pos,
        )
        .expect("shipout commit succeeds");

    assert_eq!(
        hash,
        ContentHash::for_domain(ContentDomain::Artifact, b"detached page artifact")
    );
    assert_eq!(universe.world().artifact_commits(), &[hash]);
    let committed = &universe.world().committed_artifacts()[0];
    assert_eq!(committed.hash(), hash);
    assert_eq!(committed.bytes(), b"detached page artifact");
    assert!(universe.world().effect_records().is_empty());
    assert_eq!(
        universe.world().memory_terminal_output(),
        Some(&b"shipout\n"[..])
    );
    assert_eq!(universe.testing_epoch_node_count(), 0);
    assert_eq!(universe.snapshot().state_hash(), base.state_hash());
}

#[test]
fn repeated_shipout_commits_do_not_retain_epoch_page_nodes() {
    let mut universe = Universe::new();

    for page in 0..32 {
        let mut transaction = universe.begin_shipout();
        let children = transaction.freeze_node_list(&[Node::Kern {
            amount: Scaled::from_raw(page),
            kind: KernKind::Explicit,
        }]);
        let _page = Node::HList(BoxNode::new(BoxNodeFields {
            width: Scaled::from_raw(page),
            height: Scaled::from_raw(0),
            depth: Scaled::from_raw(0),
            shift: Scaled::from_raw(0),
            display: false,
            glue_set: GlueSetRatio::ZERO,
            glue_sign: Sign::Normal,
            glue_order: Order::Normal,
            children,
        }));
        let effect_pos = transaction.world().effect_pos();
        transaction
            .commit(
                crate::VerifiedArtifact::new(format!("page {page}").into_bytes()),
                effect_pos,
            )
            .expect("shipout commit succeeds");
        assert_eq!(universe.testing_epoch_node_count(), 0);
    }
}

#[test]
fn retained_shipout_rolls_back_logical_output_without_published_host_bytes() {
    let mut universe = Universe::new();
    universe
        .begin_retained_session()
        .expect("retained session starts");
    let before = universe.snapshot();
    let mut transaction = universe.begin_shipout();
    transaction
        .world_mut()
        .write_text(PrintSink::TerminalAndLog, "logical shipout\n");
    let effect_pos = transaction.world().effect_pos();
    transaction
        .commit(
            crate::VerifiedArtifact::new(b"logical page".to_vec()),
            effect_pos,
        )
        .expect("logical shipout succeeds");

    assert_eq!(universe.world().artifact_commits().len(), 1);
    assert_eq!(universe.world().effect_records().len(), 1);
    assert_eq!(universe.world().memory_terminal_output(), Some(&b""[..]));
    assert_eq!(universe.testing_epoch_node_count(), 0);

    universe.rollback(&before);
    assert!(universe.world().artifact_commits().is_empty());
    assert!(universe.world().effect_records().is_empty());
    assert_eq!(universe.world().memory_terminal_output(), Some(&b""[..]));
}

#[test]
fn pdf_page_allocation_replays_identical_object_ids_and_hashes() {
    let mut universe = Universe::new();
    universe.enable_pdf_output();
    universe.set_int_param(IntParam::PDF_OUTPUT, 1);
    universe
        .begin_retained_session()
        .expect("retained session starts");
    let before = universe.snapshot();

    let effect_pos = universe.world().effect_pos();
    let first_hash = universe
        .begin_shipout()
        .commit(
            crate::VerifiedArtifact::new(b"checkpointed PDF page".to_vec()),
            effect_pos,
        )
        .expect("first shipout succeeds");
    let first_page = universe.pdf_pages()[0];
    let first_state_hash = universe.snapshot().state_hash();
    assert_eq!(first_page.artifact(), first_hash);
    assert_eq!(first_page.resources_object(), 1);
    assert_eq!(first_page.page_object(), 2);
    assert_eq!(first_page.contents_object(), 3);
    assert_eq!(universe.pdf_next_object_id(), 4);

    universe.rollback(&before);
    assert!(universe.pdf_pages().is_empty());
    assert_eq!(universe.pdf_next_object_id(), 1);

    let effect_pos = universe.world().effect_pos();
    let replay_hash = universe
        .begin_shipout()
        .commit(
            crate::VerifiedArtifact::new(b"checkpointed PDF page".to_vec()),
            effect_pos,
        )
        .expect("replayed shipout succeeds");
    assert_eq!(replay_hash, first_hash);
    assert_eq!(universe.pdf_pages(), &[first_page]);
    assert_eq!(universe.snapshot().state_hash(), first_state_hash);
}

#[test]
fn first_shipout_freezes_pdf_controls_and_dvi_mode_allocates_no_pdf_page() {
    let mut universe = Universe::new();
    universe.enable_pdf_output();
    universe.set_int_param(IntParam::PDF_OUTPUT, 0);
    universe.set_int_param(IntParam::PDF_MAJOR_VERSION, 1);
    universe.set_int_param(IntParam::PDF_MINOR_VERSION, 7);
    universe.set_int_param(IntParam::PDF_COMPRESS_LEVEL, 6);
    universe.set_int_param(IntParam::PDF_OBJ_COMPRESS_LEVEL, 3);
    universe.set_int_param(IntParam::PDF_DECIMAL_DIGITS, 4);
    let before = universe.snapshot();

    let effect_pos = universe.world().effect_pos();
    universe
        .begin_shipout()
        .commit(
            crate::VerifiedArtifact::new(b"DVI-mode page".to_vec()),
            effect_pos,
        )
        .expect("DVI-mode shipout succeeds");

    let fixed = universe
        .fixed_pdf_output_parameters()
        .expect("first shipout freezes controls");
    assert_eq!(fixed.output, 0);
    assert_eq!(fixed.major_version, 1);
    assert_eq!(fixed.minor_version, 7);
    assert_eq!(fixed.compress_level, 6);
    assert_eq!(fixed.object_compress_level, 3);
    assert_eq!(fixed.decimal_digits, 4);
    assert!(universe.pdf_pages().is_empty());
    assert_eq!(universe.pdf_next_object_id(), 1);

    universe.set_int_param(IntParam::PDF_OUTPUT, 1);
    assert_eq!(
        universe.fixed_pdf_output_parameters(),
        Some(fixed),
        "later assignments do not change the fixed output policy"
    );

    universe.rollback(&before);
    assert_eq!(universe.fixed_pdf_output_parameters(), None);
    assert!(universe.pdf_pages().is_empty());
}

#[test]
fn failed_shipout_does_not_allocate_pdf_objects() {
    let mut universe = Universe::new();
    universe.enable_pdf_output();
    let mut transaction = universe.begin_shipout();
    transaction
        .world_mut()
        .write_text(PrintSink::TerminalAndLog, "uncommitted effect");
    let effect_pos = transaction.world().effect_pos();
    transaction
        .world_mut()
        .fail_effect_commit_before(effect_pos);
    transaction
        .commit(
            crate::VerifiedArtifact::new(b"failed PDF page".to_vec()),
            effect_pos,
        )
        .expect_err("effect failure rejects shipout");

    assert!(universe.pdf_pages().is_empty());
    assert_eq!(universe.pdf_next_object_id(), 1);
}

#[test]
fn snapshot_state_hash_is_deterministic_for_same_program() {
    assert_eq!(
        checkpoint_hashes_for_program(),
        checkpoint_hashes_for_program()
    );
}

#[test]
fn exact_snapshots_reuse_immutable_store_projection_across_forks() {
    let mut universe = Universe::new();
    universe.intern("cached-name");

    assert!(
        universe
            .snapshot_with_exact_identity()
            .has_exact_state_identity()
    );
    universe.set_count(0, 42);
    let checkpoint = universe.snapshot();
    let substrate = universe.freeze_generation();
    let mut fork = substrate.fork_at(&checkpoint).expect("retained fork");
    assert!(
        fork.snapshot_with_exact_identity()
            .has_exact_state_identity()
    );

    assert_eq!(
        fork.stores.testing_exact_immutable_encodes(),
        1,
        "related forks must not reserialize interned content"
    );
}

#[test]
fn retained_snapshot_restores_exact_component_projections_into_forks() {
    let mut universe = Universe::new();
    universe.set_input_summary(condition_input_summary(0));
    universe.add_hyphenation_pattern(PatternSpec {
        letters: "retained".chars().collect(),
        values: vec![0, 0, 1, 0, 0, 0, 0, 0, 0],
    });
    let checkpoint = universe.snapshot();
    let substrate = universe.freeze_generation();
    let mut fork = substrate.fork_at(&checkpoint).expect("retained fork");
    let input_calls = fork.testing_input_projection_hash_calls();
    let hyphenation_calls = fork.stores.testing_hyphenation_projection_hash_calls();

    let _ = fork.snapshot_with_exact_identity();

    assert_eq!(fork.testing_input_projection_hash_calls(), input_calls);
    assert_eq!(
        fork.stores.testing_hyphenation_projection_hash_calls(),
        hyphenation_calls,
        "exact comparison must compose the retained roots without rebuilding them"
    );

    fork.set_input_summary(condition_input_summary(1));
    fork.add_hyphenation_exception(ExceptionSpec {
        word: "retained".to_owned(),
        positions: vec![2],
    });
    let _ = fork.snapshot_with_exact_identity();
    assert_eq!(fork.testing_input_projection_hash_calls(), input_calls + 1);
    assert_eq!(
        fork.stores.testing_hyphenation_projection_hash_calls(),
        hyphenation_calls + 1,
        "only dirty component roots are projected"
    );
}

#[test]
fn exact_immutable_store_growth_hashes_only_new_append_entries() {
    let mut universe = Universe::new();
    let _ = universe.snapshot_with_exact_identity();
    let before = universe.stores.testing_exact_immutable_leaves();

    universe.intern("one-new-name");
    let _ = universe.snapshot_with_exact_identity();

    assert_eq!(
        universe.stores.testing_exact_immutable_leaves(),
        before + 1,
        "one append must hash one leaf instead of recapturing every immutable store"
    );
}

#[test]
fn exact_immutable_store_cache_tracks_live_accepted_and_scratch_lineages() {
    let mut universe = Universe::new();
    let anchor = universe.snapshot_with_exact_identity();
    let accepted_before = universe.stores.testing_exact_immutable_leaves();
    for index in 0..8 {
        universe.intern(&format!("accepted-{index}"));
        let _ = universe.snapshot_with_exact_identity();
    }
    assert_eq!(
        universe.stores.testing_exact_immutable_leaves() - accepted_before,
        8,
        "live accepted boundaries must hash each new append entry once"
    );

    let substrate = universe.freeze_generation();
    let mut scratch = substrate.fork_at(&anchor).expect("scratch fork");
    let scratch_before = scratch.stores.testing_exact_immutable_leaves();
    for index in 0..8 {
        scratch.intern(&format!("scratch-{index}"));
        let _ = scratch.snapshot_with_exact_identity();
    }
    assert_eq!(
        scratch.stores.testing_exact_immutable_leaves() - scratch_before,
        8,
        "live scratch boundaries must hash each new append entry once"
    );
}

#[test]
fn exact_immutable_store_root_survives_format_reconstruction() {
    let mut original = Universe::new();
    let name = original.intern("format-root-name");
    let tokens = original.intern_token_list(&[Token::Cs(name.symbol())]);
    let definition = original.intern_macro(MacroMeaning::new(
        MeaningFlags::EMPTY,
        crate::ids::TokenListId::EMPTY,
        tokens,
    ));
    original.set_meaning(
        name,
        Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition,
        },
    );
    let expected = original
        .snapshot_with_exact_identity()
        .exact_state_identity
        .expect("closed state has exact identity");
    let format = original.dump_format().expect("format capture");

    let mut restored = Universe::from_format(World::memory(), &format).expect("format restore");
    let actual = restored
        .snapshot_with_exact_identity()
        .exact_state_identity
        .expect("restored state has exact identity");

    assert_eq!(actual, expected);
}

#[test]
fn format_dump_compacts_to_the_reachable_name_macro_and_token_closure() {
    let mut universe = Universe::new();
    let dead_name = universe.intern("dead-format-history");
    let dead_tokens = universe.intern_token_list(&[Token::Cs(dead_name.symbol())]);
    universe.intern_macro(MacroMeaning::new(
        MeaningFlags::EMPTY,
        crate::ids::TokenListId::EMPTY,
        dead_tokens,
    ));

    let live_name = universe.intern("live-format-root");
    let live_tokens = universe.intern_token_list(&[Token::Cs(live_name.symbol())]);
    let live_macro = universe.intern_macro(MacroMeaning::new(
        MeaningFlags::EMPTY,
        crate::ids::TokenListId::EMPTY,
        live_tokens,
    ));
    universe.set_meaning(
        live_name,
        Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition: live_macro,
        },
    );

    let image = universe.dump_format().expect("compact format dump");
    let container = crate::format_container::decode(&image).expect("decode compact format");
    let count = |kind| {
        let bytes = container
            .section(kind)
            .expect("format section")
            .bytes
            .as_ref();
        u32::from_le_bytes(bytes[4..8].try_into().expect("count field"))
    };
    assert_eq!(count(crate::stores::NAMES_SECTION), 1);
    assert_eq!(count(crate::stores::TOKEN_LISTS_SECTION), 2);
    assert_eq!(count(crate::stores::MACROS_SECTION), 1);

    let restored = Universe::from_format(World::memory(), &image).expect("restore compact format");
    let restored_live = restored
        .symbol("live-format-root")
        .expect("reachable name remains interned");
    assert!(matches!(
        restored.meaning(restored_live),
        Meaning::Macro { .. }
    ));
    assert!(restored.symbol("dead-format-history").is_none());
}

#[test]
fn exact_immutable_store_root_rebuilds_after_divergent_rollback_allocation() {
    let mut replayed = Universe::new();
    let baseline = replayed.snapshot();
    replayed.intern("discarded-branch-name");
    let _ = identity_of(&mut replayed);
    replayed.rollback(&baseline);
    replayed.intern("replacement-name");

    let mut cold = Universe::new();
    cold.intern("replacement-name");
    assert_eq!(identity_of(&mut replayed), identity_of(&mut cold));
}

#[test]
fn exact_immutable_font_collection_ignores_allocation_order() {
    let mut first = Universe::new();
    let first_target_name = first.intern("target-font");
    first.intern_font(test_font("filler", b"filler"));
    let first_target = first.intern_font(test_font("target", b"target"));
    first.intern_font(test_font("target", b"target"));
    first.set_meaning(first_target_name, Meaning::Font(first_target));

    let mut second = Universe::new();
    let second_target = second.intern_font(test_font("target", b"target"));
    second.intern_font(test_font("filler", b"filler"));
    second.intern_font(test_font("target", b"target"));
    let second_target_name = second.intern("target-font");
    second.set_meaning(second_target_name, Meaning::Font(second_target));

    assert_ne!(first_target.raw(), second_target.raw());
    assert_eq!(identity_of(&mut first), identity_of(&mut second));
}

#[test]
fn exact_checkpoint_identity_composes_every_future_state_root() {
    fn identity(universe: &mut Universe) -> u64 {
        universe
            .snapshot_with_exact_identity()
            .exact_state_identity
            .expect("closed checkpoint has exact identity")
    }

    fn assert_change(mut change: impl FnMut(&mut Universe)) {
        let mut universe = Universe::new();
        let baseline = identity(&mut universe);
        change(&mut universe);
        assert_ne!(identity(&mut universe), baseline);
    }

    assert_change(|universe| {
        universe.intern("immutable-component");
    });
    assert_change(|universe| universe.set_count(0, 1));
    assert_change(|universe| universe.set_catcode('x', Catcode::Active));
    assert_change(|universe| {
        universe.add_hyphenation_pattern(PatternSpec {
            letters: "identity".chars().collect(),
            values: vec![0, 0, 1, 0, 0, 0, 0, 0, 0],
        });
    });
    assert_change(|universe| universe.set_input_summary(condition_input_summary(1)));
    assert_change(|universe| {
        universe
            .world_mut()
            .open_out(StreamSlot::new(2), "identity.aux");
    });
    assert_change(|universe| {
        universe.set_page_dimension(PageDimension::Total, Scaled::from_raw(17));
    });
    assert_change(|universe| universe.set_interaction_mode(super::InteractionMode::Batch));
    assert_change(Universe::enable_pdf_output);
}

#[test]
fn exact_checkpoint_identity_restores_after_inverse_mutation() {
    let mut universe = Universe::new();
    let original = universe.snapshot();
    let baseline = identity_of(&mut universe);
    universe.set_count(9, 99);
    universe.set_pdf_return_value(17);
    assert_ne!(identity_of(&mut universe), baseline);
    universe.rollback(&original);
    assert_eq!(identity_of(&mut universe), baseline);
}

fn identity_of(universe: &mut Universe) -> u64 {
    universe
        .snapshot_with_exact_identity()
        .exact_state_identity
        .expect("closed checkpoint has exact identity")
}

#[test]
fn snapshot_state_hash_ignores_content_intern_order() {
    let mut first = Universe::new();
    let first_zed = first.intern("z");
    let alpha = first.intern("alpha");
    let macro_target = first.intern("macro_target");
    first.set_meaning(first_zed, Meaning::Relax);
    let filler_tokens = first.intern_token_list(&[Token::param(1)]);
    let target_parameters = first.intern_token_list(&[Token::param(1)]);
    let target_replacement = first.intern_token_list(&[
        Token::Cs(alpha.symbol()),
        Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        },
    ]);
    let filler_glue = first.intern_glue(glue(99));
    let target_glue = first.intern_glue(glue(7));
    let filler_macro = first.intern_macro(MacroMeaning::new(
        MeaningFlags::LONG,
        filler_tokens,
        filler_tokens,
    ));
    let target_macro = first.intern_macro(MacroMeaning::new(
        MeaningFlags::PROTECTED,
        target_parameters,
        target_replacement,
    ));
    first.set_toks(0, target_replacement);
    first.set_skip(0, target_glue);
    first.set_meaning(
        macro_target,
        Meaning::Macro {
            flags: MeaningFlags::PROTECTED,
            definition: target_macro,
        },
    );
    assert_ne!(filler_glue, target_glue);
    assert_ne!(filler_macro, target_macro);
    let first_hash = first.snapshot().state_hash();

    let mut second = Universe::new();
    let macro_target = second.intern("macro_target");
    let alpha = second.intern("alpha");
    let target_replacement = second.intern_token_list(&[
        Token::Cs(alpha.symbol()),
        Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        },
    ]);
    let filler_tokens = second.intern_token_list(&[Token::param(1)]);
    let target_parameters = second.intern_token_list(&[Token::param(1)]);
    let target_glue = second.intern_glue(glue(7));
    let filler_glue = second.intern_glue(glue(99));
    let target_macro = second.intern_macro(MacroMeaning::new(
        MeaningFlags::PROTECTED,
        target_parameters,
        target_replacement,
    ));
    let filler_macro = second.intern_macro(MacroMeaning::new(
        MeaningFlags::LONG,
        filler_tokens,
        filler_tokens,
    ));
    let second_zed = second.intern("z");
    second.set_meaning(second_zed, Meaning::Relax);
    second.set_toks(0, target_replacement);
    second.set_skip(0, target_glue);
    second.set_meaning(
        macro_target,
        Meaning::Macro {
            flags: MeaningFlags::PROTECTED,
            definition: target_macro,
        },
    );
    assert_ne!(filler_glue, target_glue);
    assert_ne!(filler_macro, target_macro);

    assert_eq!(first_hash, second.snapshot().state_hash());
    assert_eq!(
        identity_of(&mut first),
        identity_of(&mut second),
        "exact identity must ignore immutable allocation order and child handles"
    );

    // The next slice reads these keys from the incremental baseline cache.
    // Dense symbol ids differ between the two stores, but semantic ordering
    // and the resulting checkpoint hash must remain name based.
    first.set_meaning(first_zed, Meaning::Undefined);
    second.set_meaning(second_zed, Meaning::Undefined);
    assert_eq!(
        first.snapshot().state_hash(),
        second.snapshot().state_hash()
    );
}

#[test]
fn snapshot_state_hash_keys_same_spelling_namespaces_independently() {
    fn build(active_first: bool, active_meaning: Meaning) -> u64 {
        let mut universe = Universe::new();
        let (named, active) = if active_first {
            let active = universe.intern_active_character('~');
            (universe.intern("~"), active)
        } else {
            let named = universe.intern("~");
            (named, universe.intern_active_character('~'))
        };
        universe.set_meaning(named, Meaning::CharGiven('N'));
        universe.set_meaning(active, active_meaning);
        universe.snapshot().state_hash()
    }

    assert_eq!(
        build(false, Meaning::CharGiven('A')),
        build(true, Meaning::CharGiven('A'))
    );
    assert_ne!(
        build(false, Meaning::CharGiven('A')),
        build(false, Meaning::CharGiven('B'))
    );
}

#[test]
fn snapshot_state_hash_changes_for_one_register_bit() {
    let mut unchanged = Universe::new();
    let mut changed = Universe::new();
    changed.set_count(0, 1);

    assert_ne!(
        unchanged.snapshot().state_hash(),
        changed.snapshot().state_hash()
    );
}

#[test]
fn clone_preserves_pending_state_hash_slice() {
    let mut original = Universe::new();
    original.set_count(0, 41);
    let _base = original.snapshot();
    original.set_count(0, 42);
    let mut fork = original.clone();

    assert_eq!(fork.count(0), 42);
    assert_eq!(
        original.snapshot().state_hash(),
        fork.snapshot().state_hash()
    );
}

#[test]
fn snapshot_state_hash_changes_for_rng_only_change() {
    let mut unchanged = Universe::new();
    let mut changed = Universe::new();
    let _ = changed.world_mut().next_random_u64();

    assert_ne!(
        unchanged.snapshot().state_hash(),
        changed.snapshot().state_hash()
    );
}

#[test]
fn nonjournal_state_is_complete_in_hash_cursors() {
    let mut first = Universe::new();
    let mut second = Universe::new();
    first.set_catcode('x', Catcode::Letter);
    second.set_catcode('x', Catcode::Active);
    assert_ne!(
        first.snapshot().state_hash(),
        second.snapshot().state_hash()
    );

    let mut first = Universe::new();
    let mut second = Universe::new();
    first.add_hyphenation_pattern(PatternSpec {
        letters: "alpha".chars().collect(),
        values: vec![0, 1, 0, 0, 0, 0],
    });
    second.add_hyphenation_exception(ExceptionSpec {
        word: "alpha".to_owned(),
        positions: vec![2],
    });
    assert_ne!(
        first.snapshot().state_hash(),
        second.snapshot().state_hash()
    );

    let mut first = Universe::new();
    let mut second = Universe::new();
    first.set_int_param(crate::env::banks::IntParam::MAG, 1000);
    second.set_int_param(crate::env::banks::IntParam::MAG, 1200);
    let _ = first.prepare_mag();
    let _ = second.prepare_mag();
    assert_ne!(
        first.snapshot().state_hash(),
        second.snapshot().state_hash()
    );
}

#[test]
fn projection_cache_clearing_preserves_named_boundary_hashes() {
    fn prepare(universe: &mut Universe) {
        universe.set_catcode('~', Catcode::Active);
        universe.add_hyphenation_pattern(PatternSpec {
            letters: "cache".chars().collect(),
            values: vec![0, 0, 1, 0, 0, 0],
        });
        universe
            .world_mut()
            .open_out(StreamSlot::new(3), "cache.aux");
        for value in 0..130 {
            universe.push_current_page_node(Node::Kern {
                amount: Scaled::from_raw(19 + value),
                kind: KernKind::Explicit,
            });
        }
        universe.push_page_discard(Node::Penalty(27));
    }

    let mut warm = Universe::new();
    let mut cleared = Universe::new();
    prepare(&mut warm);
    prepare(&mut cleared);

    for value in 1..=4 {
        warm.set_count(0, value);
        cleared.set_count(0, value);
        cleared.testing_clear_state_hash_caches();
        assert_eq!(
            warm.snapshot().state_hash(),
            cleared.snapshot().state_hash(),
            "discardable projection caches changed boundary {value}"
        );
    }
}

fn condition_input_summary(value: u32) -> InputSummary {
    InputSummary::new(
        vec![InputFrameSummary::Condition {
            token: ConditionFrameToken::new(u64::from(value) + 1),
            condition: ConditionFrameSummary::new_if(
                TracedTokenWord::pack(
                    Token::Char {
                        ch: char::from_u32(b'a' as u32 + value).expect("small test character"),
                        cat: Catcode::Letter,
                    },
                    OriginId::UNKNOWN,
                ),
                value.is_multiple_of(2),
            ),
        }],
        None,
        None,
    )
}

#[test]
fn unchanged_input_root_reuses_its_projection_without_frame_comparison() {
    let mut universe = Universe::new();
    universe.set_input_summary(condition_input_summary(0));
    let _ = universe.snapshot();
    let calls = universe.testing_input_projection_hash_calls();

    universe.set_count(0, 1);
    let _ = universe.snapshot();

    assert_eq!(universe.testing_input_projection_hash_calls(), calls);
}

#[test]
fn rebuilt_equal_input_roots_hash_canonically_across_allocation_identities() {
    let mut first = Universe::new();
    let mut second = Universe::new();
    let first_base = first.snapshot().state_hash();
    let second_base = second.snapshot().state_hash();
    assert_eq!(first_base, second_base);

    first.set_input_summary(condition_input_summary(0));
    second.set_input_summary(condition_input_summary(0));
    assert_eq!(
        first.snapshot().state_hash(),
        second.snapshot().state_hash()
    );
}

#[test]
fn every_component_change_is_cache_clear_differential() {
    fn assert_change(mut change: impl FnMut(&mut Universe)) {
        let mut warm = Universe::new();
        let mut cleared = Universe::new();
        let baseline = warm.snapshot().state_hash();
        assert_eq!(cleared.snapshot().state_hash(), baseline);
        change(&mut warm);
        change(&mut cleared);
        cleared.testing_clear_state_hash_caches();
        let warm_hash = warm.snapshot().state_hash();
        let cleared_hash = cleared.snapshot().state_hash();
        assert_ne!(warm_hash, baseline);
        assert_eq!(warm_hash, cleared_hash);
    }

    assert_change(|universe| universe.set_count(0, 1));
    assert_change(|universe| universe.set_catcode('x', Catcode::Active));
    assert_change(|universe| {
        universe.add_hyphenation_pattern(PatternSpec {
            letters: "component".chars().collect(),
            values: vec![0, 0, 1, 0, 0, 0, 0, 0, 0, 0],
        });
    });
    assert_change(|universe| {
        universe
            .world_mut()
            .write_text(PrintSink::TerminalAndLog, "effect\n");
    });
    assert_change(|universe| {
        universe
            .world_mut()
            .open_out(StreamSlot::new(2), "component.aux");
    });
    assert_change(|universe| universe.set_input_summary(condition_input_summary(1)));
    assert_change(|universe| {
        universe.set_page_dimension(PageDimension::Total, Scaled::from_raw(17));
    });
    assert_change(|universe| {
        universe.push_current_page_node(Node::Kern {
            amount: Scaled::from_raw(23),
            kind: KernKind::Explicit,
        });
    });
    assert_change(|universe| universe.set_interaction_mode(super::InteractionMode::Batch));
}

#[test]
fn two_forks_group_compaction_and_shipout_retargeting_are_cache_differential() {
    let mut root = Universe::new();
    root.set_catcode('~', Catcode::Active);
    for value in 0..192 {
        root.push_current_page_node(Node::Kern {
            amount: Scaled::from_raw(value),
            kind: KernKind::Explicit,
        });
    }
    let _ = root.snapshot();
    let mut warm = root.clone();
    let mut cleared = root.clone();
    cleared.testing_clear_state_hash_caches();

    for universe in [&mut warm, &mut cleared] {
        universe.enter_group();
        universe.set_count(7, 77);
        let _ = universe.leave_group();
        universe
            .world_mut()
            .write_text(PrintSink::TerminalAndLog, "shipout\n");
        let effect_pos = universe.world().effect_pos();
        universe
            .begin_shipout()
            .commit(
                crate::VerifiedArtifact::new(b"component projection page".to_vec()),
                effect_pos,
            )
            .expect("memory shipout succeeds");
        universe.set_count(8, 88);
    }
    cleared.testing_clear_state_hash_caches();

    assert_eq!(
        warm.snapshot().state_hash(),
        cleared.snapshot().state_hash()
    );
    assert_eq!(
        warm.world().memory_terminal_output(),
        cleared.world().memory_terminal_output()
    );
}

#[test]
fn randomized_incremental_hash_matches_cold_projection_rebuilds() {
    fn next(seed: &mut u64) -> u64 {
        *seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        *seed
    }

    for initial_seed in 0..8_u64 {
        let mut seed = initial_seed + 1;
        let mut warm = Universe::new();
        let mut cold = Universe::new();
        let mut retained = Vec::new();

        for step in 0..256_u32 {
            let operation = next(&mut seed) % 11;
            let mut retain_boundary = false;
            match operation {
                0 => {
                    let index = (next(&mut seed) % 32) as u16;
                    let value = next(&mut seed) as i32;
                    warm.set_count(index, value);
                    cold.set_count(index, value);
                }
                1 => {
                    let ch = char::from_u32(b'a' as u32 + (next(&mut seed) % 26) as u32)
                        .expect("ASCII test character");
                    let catcode = if next(&mut seed).is_multiple_of(2) {
                        Catcode::Letter
                    } else {
                        Catcode::Other
                    };
                    warm.set_catcode(ch, catcode);
                    cold.set_catcode(ch, catcode);
                }
                2 => {
                    let value = Scaled::from_raw(next(&mut seed) as i32);
                    warm.set_page_dimension(PageDimension::Total, value);
                    cold.set_page_dimension(PageDimension::Total, value);
                }
                3 => {
                    let node = Node::Kern {
                        amount: Scaled::from_raw(next(&mut seed) as i32),
                        kind: KernKind::Explicit,
                    };
                    warm.push_current_page_node(node.clone());
                    cold.push_current_page_node(node);
                }
                4 => {
                    let value = (next(&mut seed) % 20) as u32;
                    warm.set_input_summary(condition_input_summary(value));
                    cold.set_input_summary(condition_input_summary(value));
                }
                5 => {
                    let text = format!("random effect {initial_seed}:{step}\n");
                    warm.world_mut()
                        .write_text(PrintSink::TerminalAndLog, &text);
                    cold.world_mut()
                        .write_text(PrintSink::TerminalAndLog, &text);
                }
                6 => {
                    let index = (next(&mut seed) % 16) as u16;
                    let value = next(&mut seed) as i32;
                    for universe in [&mut warm, &mut cold] {
                        universe.enter_group();
                        universe.set_count(index, value);
                        let _ = universe.leave_group();
                    }
                }
                7 => retain_boundary = true,
                8 if !retained.is_empty() => {
                    let index = (next(&mut seed) as usize) % retained.len();
                    let (warm_mark, cold_mark) = retained.swap_remove(index);
                    warm.rollback(&warm_mark);
                    cold.rollback(&cold_mark);
                    retained.clear();
                }
                9 => {
                    warm = warm.clone();
                    cold = cold.clone();
                    retained.clear();
                }
                10 => {
                    for universe in [&mut warm, &mut cold] {
                        universe
                            .world_mut()
                            .write_text(PrintSink::TerminalAndLog, "random shipout\n");
                        let effect_pos = universe.world().effect_pos();
                        universe
                            .begin_shipout()
                            .commit(
                                crate::VerifiedArtifact::new(
                                    b"randomized differential page".to_vec(),
                                ),
                                effect_pos,
                            )
                            .expect("memory shipout succeeds");
                    }
                    retained.clear();
                }
                8 => {}
                _ => unreachable!("operation is reduced modulo eleven"),
            }

            cold.testing_clear_state_hash_caches();
            let warm_boundary = warm.snapshot();
            let cold_boundary = cold.snapshot();
            assert_eq!(
                warm_boundary.state_hash(),
                cold_boundary.state_hash(),
                "seed {initial_seed}, step {step}, operation {operation}"
            );
            assert_eq!(
                warm.world().memory_terminal_output(),
                cold.world().memory_terminal_output(),
                "effect divergence at seed {initial_seed}, step {step}"
            );
            if retain_boundary {
                retained.push((warm_boundary, cold_boundary));
            }
        }
    }
}

#[test]
fn already_interned_last_font_selection_changes_hash_semantically() {
    let mut universe = Universe::new();
    let first_font = test_font("first", b"first");
    let second_font = test_font("second", b"second");
    universe.intern_font(first_font.clone());
    universe.intern_font(second_font.clone());
    let baseline = universe.snapshot();

    universe.intern_font(first_font);
    let first = universe.snapshot().state_hash();
    universe.rollback(&baseline);
    universe.intern_font(second_font);

    assert_ne!(universe.snapshot().state_hash(), first);
}

#[test]
fn snapshot_state_hash_distinguishes_font_content_identity() {
    let mut first = Universe::new();
    let mut second = Universe::new();
    let first_symbol = first.intern("font");
    let second_symbol = second.intern("font");

    let first_font = first.intern_font(test_font("cmr10", b"same"));
    let second_font = second.intern_font(test_font("cmr10", b"different"));
    assert_eq!(first_font.raw(), second_font.raw());

    first.set_meaning(first_symbol, Meaning::Font(first_font));
    second.set_meaning(second_symbol, Meaning::Font(second_font));

    assert_ne!(
        first.snapshot().state_hash(),
        second.snapshot().state_hash()
    );
}

#[test]
fn font_host_path_is_provenance_not_semantic_identity() {
    let bytes = b"identical tfm bytes";
    let mut first = Universe::new();
    let mut second = Universe::new();
    let first_symbol = first.intern("font");
    let second_symbol = second.intern("font");
    let make_font = |path: &str| {
        crate::font::LoadedFont::new(
            "cmr10",
            path,
            ContentHash::from_bytes(bytes).bytes(),
            0,
            Scaled::from_raw(10 * Scaled::UNITY),
            Scaled::from_raw(10 * Scaled::UNITY),
            vec![Scaled::from_raw(0); 7],
            crate::font::FontMetrics::default(),
        )
    };

    let first_font = first.intern_font(make_font("/texlive/a/cmr10.tfm"));
    let second_font = second.intern_font(make_font("/vendor/b/cmr10.tfm"));
    first.set_meaning(first_symbol, Meaning::Font(first_font));
    second.set_meaning(second_symbol, Meaning::Font(second_font));

    assert_ne!(
        first.font(first_font).path(),
        second.font(second_font).path()
    );
    assert_eq!(
        first.snapshot().state_hash(),
        second.snapshot().state_hash()
    );
    assert_eq!(
        first.dump_format().expect("first semantic format"),
        second.dump_format().expect("second semantic format")
    );
}

#[test]
fn short_loaded_font_parameters_seed_seven_snapshot_covered_env_values() {
    let mut universe = Universe::new();
    let loaded = crate::font::LoadedFont::new(
        "short",
        "short.tfm",
        ContentHash::from_bytes(b"short").bytes(),
        0,
        Scaled::from_raw(10 * Scaled::UNITY),
        Scaled::from_raw(10 * Scaled::UNITY),
        vec![Scaled::from_raw(-1)],
        crate::font::FontMetrics::default(),
    );
    assert_eq!(loaded.parameters().len(), 7);

    let short = universe.intern_font(loaded);
    let _later = universe.intern_font(test_font("later", b"later"));
    assert_eq!(universe.font_parameter_count(short), 7);
    assert_eq!(universe.font_parameter(short, 1), Scaled::from_raw(-1));
    for number in 2..=7 {
        assert_eq!(universe.font_parameter(short, number), Scaled::from_raw(0));
    }

    let snapshot = universe.snapshot();
    universe
        .set_font_dimen(short, 7, Scaled::from_raw(77))
        .expect("guaranteed fontdimen remains writable after another font loads");
    assert_eq!(universe.font_parameter(short, 7), Scaled::from_raw(77));
    universe.rollback(&snapshot);
    assert_eq!(universe.font_parameter(short, 7), Scaled::from_raw(0));
}

#[test]
fn snapshot_state_hash_distinguishes_font_identifier_identity() {
    let mut first = Universe::new();
    let mut second = Universe::new();
    let first_a = first.intern("a");
    let first_b = first.intern("b");
    let second_a = second.intern("a");
    let second_b = second.intern("b");

    let first_font = first.intern_font_with_identifier(test_font("cmr10", b"same"), first_a);
    let second_font = second.intern_font_with_identifier(test_font("cmr10", b"same"), second_b);
    first.set_meaning(first_b, Meaning::Font(first_font));
    second.set_meaning(second_a, Meaning::Font(second_font));

    assert_ne!(
        first.snapshot().state_hash(),
        second.snapshot().state_hash()
    );
}

#[test]
fn generated_fonts_rollback_replay_and_format_with_source_links() {
    let mut universe = Universe::with_world(World::memory());
    universe.set_int_param_global(crate::env::banks::IntParam::DEFAULT_HYPHEN_CHAR, 45);
    universe.set_int_param_global(crate::env::banks::IntParam::DEFAULT_SKEW_CHAR, -1);
    let base_name = universe.intern("base");
    let copy_name = universe.intern("copy");
    let spaced_name = universe.intern("spaced");
    let base = universe.intern_font_with_identifier(test_font("cmr10", b"metrics"), base_name);
    universe
        .set_font_dimen(base, 2, Scaled::from_raw(9 * Scaled::UNITY))
        .expect("current source fontdimen write");
    universe.set_font_hyphen_char(base, 99);
    let before = universe.snapshot();

    let copy = universe
        .try_copy_font_with_identifier(base, copy_name)
        .expect("copy font");
    let spaced = universe
        .try_letterspace_font_with_identifier(base, spaced_name, 100, true)
        .expect("letterspace font");
    assert_eq!(universe.font_parameter(copy, 2).raw(), 9 * Scaled::UNITY);
    assert_eq!(universe.font_parameter(spaced, 2).raw(), 0);
    assert_eq!(universe.font_hyphen_char(copy), 99);
    assert_eq!(universe.font_hyphen_char(spaced), 45);
    assert!(universe.pdf_font_ligatures_disabled(spaced));
    let source = match universe.font(spaced).construction() {
        crate::font::FontConstruction::Letterspaced { source, .. } => *source,
        construction => panic!("unexpected construction {construction:?}"),
    };
    assert_eq!(universe.font_by_source_identity(source), Some(base));
    let generated_hash = universe.snapshot().state_hash();
    let format = universe.dump_format().expect("generated font format");
    let restored =
        Universe::from_format(World::memory(), &format).expect("restore generated fonts");
    assert_eq!(restored.dump_format().expect("canonical redump"), format);
    assert_eq!(
        restored.font_by_source_identity(source).map(FontId::raw),
        Some(base.raw())
    );

    universe.rollback(&before);
    let replay_copy = universe
        .try_copy_font_with_identifier(base, copy_name)
        .expect("replay copy font");
    let replay_spaced = universe
        .try_letterspace_font_with_identifier(base, spaced_name, 100, true)
        .expect("replay letterspace font");
    assert_eq!(replay_copy.raw(), copy.raw());
    assert_eq!(replay_spaced.raw(), spaced.raw());
    assert_ne!(replay_copy, copy);
    assert_ne!(replay_spaced, spaced);
    assert_eq!(universe.snapshot().state_hash(), generated_hash);
}

#[test]
fn complete_font_fragments_include_identifier_namespace_and_survive_fork() {
    let mut named = Universe::new();
    let mut active = Universe::new();
    let named_identifier = named.intern("x");
    let active_identifier = active.intern_active_character('x');
    let named_font =
        named.intern_font_with_identifier(test_font("cmr10", b"same"), named_identifier);
    let active_font =
        active.intern_font_with_identifier(test_font("cmr10", b"same"), active_identifier);

    let named_fragment = named.stores.testing_font_semantic_fingerprint(named_font);
    assert_ne!(
        named_fragment,
        active.stores.testing_font_semantic_fingerprint(active_font)
    );

    let fork = named.clone();
    assert_eq!(
        fork.stores.testing_font_semantic_fingerprint(named_font),
        named_fragment
    );
}

#[test]
fn compact_stored_font_id_resolves_its_identifier() {
    let mut universe = Universe::new();
    let identifier = universe.intern("tenrm");
    let font = universe.intern_font_with_identifier(test_font("cmr10", b"same"), identifier);
    let stored = FontId::new(font.raw());

    assert_ne!(stored, font);
    assert_eq!(universe.font_identifier_symbol(stored), Some(identifier));
}

#[test]
fn rollback_restores_font_identifier_registration() {
    let mut universe = Universe::new();
    let snapshot = universe.snapshot();
    let unnamed_fragment = universe.stores.testing_font_semantic_fingerprint(NULL_FONT);
    let nullfont = universe.intern("nullfont");
    universe.set_font_identifier_symbol(NULL_FONT, nullfont);
    assert_eq!(universe.font_identifier_symbol(NULL_FONT), Some(nullfont));
    assert_ne!(
        universe.stores.testing_font_semantic_fingerprint(NULL_FONT),
        unnamed_fragment
    );

    universe.rollback(&snapshot);
    assert_eq!(universe.font_identifier_symbol(NULL_FONT), None);
    assert_eq!(
        universe.stores.testing_font_semantic_fingerprint(NULL_FONT),
        unnamed_fragment
    );
}

#[test]
fn rollback_reuse_does_not_revive_stale_font_identity() {
    let mut universe = Universe::new();
    let snapshot = universe.snapshot();
    let stale = universe.intern_font(test_font("stale", b"stale"));
    let stale_fragment = universe.stores.testing_font_semantic_fingerprint(stale);

    universe.rollback(&snapshot);
    let reused = universe.intern_font(test_font("reused", b"reused"));

    assert_eq!(reused.raw(), stale.raw());
    assert_ne!(reused, stale);
    assert!(std::panic::catch_unwind(|| universe.font(stale)).is_err());
    assert_eq!(universe.font(reused).name(), "reused");
    assert_ne!(
        universe.stores.testing_font_semantic_fingerprint(reused),
        stale_fragment
    );
}

#[test]
fn rollback_restores_state_hash_cursor() {
    let mut universe = Universe::new();
    let base = universe.snapshot();
    universe.set_count(0, 10);
    let first = universe.snapshot();

    universe.rollback(&base);
    universe.set_count(0, 10);
    let second = universe.snapshot();

    assert_eq!(first.state_hash(), second.state_hash());
}

#[test]
fn rollback_rebuilds_incremental_hash_baselines_after_node_span_reuse() {
    let mut reused = Universe::new();
    let base = reused.snapshot();
    let first_list = reused.freeze_node_list(&[Node::Char {
        font: NULL_FONT,
        ch: 'x',
        origin: crate::token::OriginId::UNKNOWN,
    }]);
    reused.set_box_reg(0, first_list);
    let first_hash = reused.snapshot().state_hash();

    reused.rollback(&base);
    let second_list = reused.freeze_node_list(&[Node::Char {
        font: NULL_FONT,
        ch: 'y',
        origin: crate::token::OriginId::UNKNOWN,
    }]);
    assert_ne!(
        first_list, second_list,
        "rollback must retag the reused epoch node span"
    );
    reused.set_box_reg(0, second_list);
    let reused_hash = reused.snapshot().state_hash();

    let mut fresh = Universe::new();
    let _ = fresh.snapshot();
    let fresh_list = fresh.freeze_node_list(&[Node::Char {
        font: NULL_FONT,
        ch: 'y',
        origin: crate::token::OriginId::UNKNOWN,
    }]);
    fresh.set_box_reg(0, fresh_list);
    let fresh_hash = fresh.snapshot().state_hash();

    assert_ne!(first_hash, reused_hash);
    assert_eq!(reused_hash, fresh_hash);
}

#[test]
fn mixed_arena_box_promotion_replays_with_resolvable_equal_hashes() {
    let mut universe = Universe::new();
    let child = universe.freeze_node_list(&[Node::Char {
        font: NULL_FONT,
        ch: 'x',
        origin: crate::token::OriginId::UNKNOWN,
    }]);
    universe.set_box_reg(0, child);
    let base = universe.snapshot();

    let first = promote_survivor_wrapped_box(&mut universe);
    let first_hash = universe.snapshot().state_hash();
    assert_promoted_wrapper_is_resolvable(&universe, first);

    universe.rollback(&base);
    let second = promote_survivor_wrapped_box(&mut universe);
    let second_hash = universe.snapshot().state_hash();
    assert_promoted_wrapper_is_resolvable(&universe, second);

    assert_eq!(first_hash, second_hash);
}

fn promote_survivor_wrapped_box(universe: &mut Universe) -> NodeListId {
    let child = universe
        .box_reg(0)
        .expect("survivor child should remain live");
    let wrapper = universe.freeze_node_list(&[Node::VList(BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(10),
        height: Scaled::from_raw(7),
        depth: Scaled::from_raw(3),
        shift: Scaled::from_raw(0),
        display: false,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: child,
    }))]);
    universe.set_box_reg_global(255, wrapper);
    universe.box_reg(255).expect("wrapper should be promoted")
}

#[test]
fn grouped_box_take_pins_nested_survivor_children_before_coalesced_release() {
    let mut universe = Universe::new();
    let leader_children = universe.freeze_node_list(&[Node::Char {
        font: NULL_FONT,
        ch: 'x',
        origin: crate::token::OriginId::UNKNOWN,
    }]);
    let leader = BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(10),
        height: Scaled::from_raw(7),
        depth: Scaled::from_raw(3),
        shift: Scaled::from_raw(0),
        display: false,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: leader_children,
    });
    let glue = universe.intern_glue(GlueSpec::ZERO);
    let value = universe.freeze_node_list(&[Node::Glue {
        spec: glue,
        kind: GlueKind::Leaders,
        leader: Some(LeaderPayload::HList(leader)),
    }]);

    universe.enter_group();
    universe.set_box_reg(0, value);
    let before = universe.testing_epoch_clone_counts();
    let taken = universe
        .take_box_reg_same_level(0)
        .expect("local box should move out of the register");

    let ArenaRef::Survivor(root) = taken.arena() else {
        panic!("taken value should remain survivor-backed")
    };
    let Some(crate::node_arena::NodeRef::Glue {
        leader: Some(LeaderPayload::HList(leader)),
        ..
    }) = universe.nodes(taken).first()
    else {
        panic!("taken value should preserve its leader box");
    };
    assert_eq!(leader.children.arena(), ArenaRef::Survivor(root));
    assert_eq!(
        universe.nodes(leader.children),
        &[Node::Char {
            font: NULL_FONT,
            ch: 'x',
            origin: crate::token::OriginId::UNKNOWN,
        }]
    );
    let _ = universe.leave_group();
    assert_eq!(universe.testing_epoch_clone_counts(), before);
    assert_eq!(universe.testing_survivor_pin_count(), 1);
}

#[test]
fn same_level_box_take_crosses_nested_group_but_restores_at_owner_group() {
    let mut universe = Universe::new();
    let baseline = universe.freeze_node_list(&[Node::Char {
        font: NULL_FONT,
        ch: 'o',
        origin: crate::token::OriginId::UNKNOWN,
    }]);
    universe.set_box_reg(0, baseline);
    let baseline = universe.box_reg(0).expect("root box");

    universe.enter_group();
    let local = universe.freeze_node_list(&[Node::Char {
        font: NULL_FONT,
        ch: 'l',
        origin: crate::token::OriginId::UNKNOWN,
    }]);
    universe.set_box_reg(0, local);
    universe.enter_group();
    assert!(universe.take_box_reg_same_level(0).is_some());
    assert!(universe.box_reg(0).is_none());

    let _ = universe.leave_group();
    assert!(
        universe.box_reg(0).is_none(),
        "the destructive take must survive the nested construction group"
    );
    let _ = universe.leave_group();
    assert_eq!(universe.box_reg(0), Some(baseline));
}

#[test]
fn destructive_unbox_transfers_only_children_before_same_level_clear() {
    let mut universe = Universe::new();
    let baseline = universe.freeze_node_list(&[Node::Char {
        font: NULL_FONT,
        ch: 'b',
        origin: crate::token::OriginId::UNKNOWN,
    }]);
    universe.set_box_reg(0, baseline);
    let baseline = universe.box_reg(0).expect("baseline box");

    universe.enter_group();
    let leaf = universe.freeze_node_list(&[Node::Char {
        font: NULL_FONT,
        ch: 'x',
        origin: crate::token::OriginId::UNKNOWN,
    }]);
    let nested = universe.freeze_node_list(&[Node::HList(BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(10),
        height: Scaled::from_raw(7),
        depth: Scaled::from_raw(3),
        shift: Scaled::from_raw(0),
        display: false,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: leaf,
    }))]);
    let wrapper = universe.freeze_node_list(&[Node::HList(BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(10),
        height: Scaled::from_raw(7),
        depth: Scaled::from_raw(3),
        shift: Scaled::from_raw(0),
        display: false,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: nested,
    }))]);
    universe.set_box_reg(0, wrapper);
    let before = universe.testing_epoch_clone_counts();

    let TakeUnboxResult::Children(children) =
        universe.take_unbox_children_same_level(0, UnboxKind::Horizontal)
    else {
        panic!("compatible hbox should transfer its children")
    };

    assert!(universe.box_reg(0).is_none());
    let ArenaRef::Survivor(root) = children.arena() else {
        panic!("unboxed children should remain survivor-backed")
    };
    let Some(crate::node_arena::NodeRef::HList(nested)) = universe.nodes(children).first() else {
        panic!("nested hbox should survive the transfer")
    };
    assert_eq!(nested.children.arena(), ArenaRef::Survivor(root));
    assert!(matches!(
        universe.nodes(nested.children).first(),
        Some(crate::node_arena::NodeRef::Char { ch: 'x', .. })
    ));
    let after = universe.testing_epoch_clone_counts();
    assert_eq!(after, before, "survivor transfer performs no epoch clone");

    let _ = universe.leave_group();
    assert_eq!(universe.box_reg(0), Some(baseline));
}

#[test]
fn destructive_unbox_rejects_incompatible_kind_without_mutation() {
    let mut universe = Universe::new();
    let children = universe.freeze_node_list(&[Node::Kern {
        amount: Scaled::from_raw(1),
        kind: KernKind::Explicit,
    }]);
    let wrapper = universe.freeze_node_list(&[Node::VList(BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(0),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
        shift: Scaled::from_raw(0),
        display: false,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children,
    }))]);
    universe.set_box_reg(4, wrapper);
    let survivor = universe.box_reg(4);
    let before = universe.testing_epoch_clone_counts();

    assert_eq!(
        universe.take_unbox_children_same_level(4, UnboxKind::Horizontal),
        TakeUnboxResult::Incompatible
    );
    assert_eq!(universe.box_reg(4), survivor);
    assert_eq!(universe.testing_epoch_clone_counts(), before);
}

fn assert_promoted_wrapper_is_resolvable(universe: &Universe, wrapper: NodeListId) {
    let Some(crate::node_arena::NodeRef::VList(box_node)) = universe.nodes(wrapper).first() else {
        panic!("promoted wrapper should contain a vlist");
    };
    let (ArenaRef::Survivor(wrapper_root), ArenaRef::Survivor(child_root)) =
        (wrapper.arena(), box_node.children.arena())
    else {
        panic!("promoted wrapper and child should be survivor-owned");
    };
    assert_eq!(wrapper_root, child_root);
    assert_eq!(
        universe.nodes(box_node.children),
        &[Node::Char {
            font: NULL_FONT,
            ch: 'x',
            origin: crate::token::OriginId::UNKNOWN,
        }]
    );
}

#[test]
fn snapshot_state_hash_walks_deep_node_lists_iteratively() {
    let mut universe = Universe::new();
    let mut current = universe.freeze_node_list(&[Node::Char {
        font: NULL_FONT,
        ch: 'x',
        origin: crate::token::OriginId::UNKNOWN,
    }]);

    for _ in 0..5000 {
        current = universe.freeze_node_list(&[Node::HList(BoxNode::new(BoxNodeFields {
            width: Scaled::from_raw(1),
            height: Scaled::from_raw(2),
            depth: Scaled::from_raw(3),
            shift: Scaled::from_raw(0),
            display: false,
            glue_set: GlueSetRatio::ZERO,
            glue_sign: Sign::Normal,
            glue_order: Order::Normal,
            children: current,
        }))]);
    }

    universe.set_box_reg(0, current);
    assert_ne!(universe.snapshot().state_hash(), 0);
}

#[test]
fn snapshot_state_hash_ignores_unreachable_epoch_node_allocations() {
    let mut without_discarded_nodes = Universe::new();
    let mut with_discarded_nodes = Universe::new();
    let _ = without_discarded_nodes.snapshot();
    let _ = with_discarded_nodes.snapshot();

    for amount in 0..1_000 {
        let child = with_discarded_nodes.freeze_node_list(&[Node::Kern {
            amount: Scaled::from_raw(amount),
            kind: KernKind::Explicit,
        }]);
        let _discarded =
            with_discarded_nodes.freeze_node_list(&[Node::HList(BoxNode::new(BoxNodeFields {
                width: Scaled::from_raw(amount),
                height: Scaled::from_raw(0),
                depth: Scaled::from_raw(0),
                shift: Scaled::from_raw(0),
                display: false,
                glue_set: GlueSetRatio::ZERO,
                glue_sign: Sign::Normal,
                glue_order: Order::Normal,
                children: child,
            }))]);
    }

    assert_eq!(
        without_discarded_nodes.snapshot().state_hash(),
        with_discarded_nodes.snapshot().state_hash()
    );
}

#[test]
fn snapshot_state_hash_depends_on_live_box_content_not_overwritten_construction_history() {
    let mut direct = Universe::new();
    let mut overwritten = Universe::new();
    let _ = direct.snapshot();
    let _ = overwritten.snapshot();

    for amount in 0..1_000 {
        let transient = overwritten.freeze_node_list(&[Node::Kern {
            amount: Scaled::from_raw(amount),
            kind: KernKind::Explicit,
        }]);
        overwritten.set_box_reg(0, transient);
    }

    let direct_final = direct.freeze_node_list(&[Node::Char {
        font: NULL_FONT,
        ch: 'x',
        origin: crate::token::OriginId::UNKNOWN,
    }]);
    direct.set_box_reg(0, direct_final);
    let overwritten_final = overwritten.freeze_node_list(&[Node::Char {
        font: NULL_FONT,
        ch: 'x',
        origin: crate::token::OriginId::UNKNOWN,
    }]);
    overwritten.set_box_reg(0, overwritten_final);

    assert_eq!(
        direct.snapshot().state_hash(),
        overwritten.snapshot().state_hash()
    );
}

#[test]
fn finished_box_assignment_reclaims_only_its_epoch_construction_suffix() {
    let mut universe = Universe::new();
    let older = universe.freeze_node_list(&[Node::Char {
        font: NULL_FONT,
        ch: 'a',
        origin: crate::token::OriginId::UNKNOWN,
    }]);
    let mut transaction = universe.begin_box_build();
    let children = transaction.freeze_node_list(&[Node::Kern {
        amount: Scaled::from_raw(17),
        kind: KernKind::Explicit,
    }]);
    let root = transaction.freeze_node_list(&[Node::HList(BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(17),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
        shift: Scaled::from_raw(0),
        display: false,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children,
    }))]);
    assert_eq!(transaction.testing_epoch_node_count(), 3);

    transaction.finish(0, Some(root), false);

    assert_eq!(universe.testing_epoch_node_count(), 1);
    assert_eq!(
        universe.nodes(older).first(),
        Some(crate::node_arena::NodeRef::Char {
            font: NULL_FONT,
            ch: 'a',
            origin: crate::token::OriginId::UNKNOWN,
        })
    );
    let stored = universe.box_reg(0).expect("box assignment should be live");
    let Some(crate::node_arena::NodeRef::HList(box_node)) = universe.nodes(stored).first() else {
        panic!("stored value should be an hbox");
    };
    assert_eq!(
        universe.nodes(box_node.children),
        &[Node::Kern {
            amount: Scaled::from_raw(17),
            kind: KernKind::Explicit,
        }]
    );
}

#[test]
fn cancelled_box_build_reclaims_its_epoch_construction_suffix() {
    let mut universe = Universe::new();
    {
        let mut transaction = universe.begin_box_build();
        let _discarded = transaction.freeze_node_list(&[Node::Char {
            font: NULL_FONT,
            ch: 'x',
            origin: crate::token::OriginId::UNKNOWN,
        }]);
    }

    assert_eq!(universe.testing_epoch_node_count(), 0);
}

fn checkpoint_hashes_for_program() -> Vec<u64> {
    let mut universe = Universe::new();
    let mut hashes = Vec::new();
    hashes.push(universe.snapshot().state_hash());

    universe.set_count(0, 42);
    universe.set_catcode('@', Catcode::Letter);
    hashes.push(universe.snapshot().state_hash());

    let symbol = universe.intern("foo");
    let tokens = universe.intern_token_list(&[Token::Cs(symbol.symbol())]);
    universe.set_toks(2, tokens);
    universe.record_deferred_write(StreamSlot::new(1), tokens);
    hashes.push(universe.snapshot().state_hash());

    let _ = universe.world_mut().next_random_u64();
    hashes.push(universe.snapshot().state_hash());
    hashes
}

#[test]
fn deferred_write_admission_preserves_unexpanded_tokens_and_effect_order() {
    let mut universe = Universe::new();
    let escape = universe.intern("the");
    let tokens = universe.intern_token_list(&[
        Token::Cs(escape.symbol()),
        Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        },
    ]);
    let slot = StreamSlot::new(5);

    universe.world_mut().write_text(PrintSink::Log, "before");
    universe.record_deferred_write(slot, tokens);
    universe.world_mut().write_text(PrintSink::Log, "after");

    assert!(matches!(
        universe.world().effect_records(),
        [
            EffectRecord::StreamWrite { text: before, .. },
            EffectRecord::DeferredWrite { stream, tokens: recorded },
            EffectRecord::StreamWrite { text: after, .. },
        ] if before == "before" && *stream == slot && *recorded == tokens && after == "after"
    ));
}

#[test]
fn deferred_write_rejects_stale_foreign_and_reused_token_lists_before_mutation() {
    let mut universe = Universe::new();
    let snapshot = universe.snapshot();
    let stale = universe.intern_token_list(&[Token::Char {
        ch: 's',
        cat: Catcode::Letter,
    }]);
    universe.rollback(&snapshot);
    let replacement = universe.intern_token_list(&[Token::Char {
        ch: 'r',
        cat: Catcode::Letter,
    }]);
    assert_eq!(stale.raw(), replacement.raw());
    assert_ne!(stale, replacement);

    let effect_pos = universe.world().effect_pos();
    assert!(
        catch_unwind(AssertUnwindSafe(|| {
            universe.record_deferred_write(StreamSlot::new(1), stale);
        }))
        .is_err()
    );
    assert_eq!(universe.world().effect_pos(), effect_pos);
    assert!(universe.world().effect_records().is_empty());

    let mut owner = Universe::new();
    let foreign = owner.intern_token_list(&[Token::Char {
        ch: 'f',
        cat: Catcode::Letter,
    }]);
    assert!(
        catch_unwind(AssertUnwindSafe(|| {
            universe.record_deferred_write(StreamSlot::new(2), foreign);
        }))
        .is_err()
    );
    assert_eq!(universe.world().effect_pos(), effect_pos);
    assert!(universe.world().effect_records().is_empty());
}

fn glue(width: i32) -> GlueSpec {
    GlueSpec {
        width: Scaled::from_raw(width),
        stretch: Scaled::from_raw(1),
        stretch_order: Order::Fil,
        shrink: Scaled::from_raw(2),
        shrink_order: Order::Normal,
    }
}

fn test_font(name: &str, bytes: &[u8]) -> crate::font::LoadedFont {
    crate::font::LoadedFont::new(
        name,
        format!("{name}.tfm"),
        ContentHash::from_bytes(bytes).bytes(),
        0,
        Scaled::from_raw(10 * Scaled::UNITY),
        Scaled::from_raw(10 * Scaled::UNITY),
        vec![Scaled::from_raw(0); 7],
        crate::font::FontMetrics::default(),
    )
}

fn structured_format_font() -> crate::font::LoadedFont {
    use crate::font::{
        CharMetrics, CharTag, ExtensibleRecipe, FontMetrics, LigKernCommand, LigKernInstruction,
        LigatureCommand, LoadedFont,
    };

    let mut characters = vec![None; 256];
    let metric = |tag| {
        Some(CharMetrics {
            width: Scaled::from_raw(500),
            height: Scaled::from_raw(300),
            depth: Scaled::from_raw(100),
            italic_correction: Scaled::from_raw(25),
            tag,
        })
    };
    characters[usize::from(b'A')] = metric(CharTag::LigKern {
        program_index: 0,
        start_index: 0,
    });
    characters[usize::from(b'B')] = metric(CharTag::Extensible(0));
    characters[usize::from(b'C')] = metric(CharTag::None);
    let metrics = FontMetrics::new(
        characters,
        vec![LigKernInstruction {
            skip_byte: 128,
            next_char: b'C',
            command: Some(LigKernCommand::Ligature(LigatureCommand {
                replacement: b'C',
                delete_current: true,
                delete_next: true,
                pass_over: 0,
            })),
        }],
        None,
        None,
        vec![ExtensibleRecipe {
            top: None,
            middle: None,
            bottom: Some(b'B'),
            repeated: b'C',
        }],
    );
    metrics.validate().expect("test metric structure is valid");
    LoadedFont::new(
        "structuredfont",
        "structuredfont.tfm",
        ContentHash::from_bytes(b"structuredfont").bytes(),
        0x1234_5678,
        Scaled::from_raw(10 * Scaled::UNITY),
        Scaled::from_raw(10 * Scaled::UNITY),
        (1..=7).map(Scaled::from_raw).collect(),
        metrics,
    )
}

fn pending_source_summary(
    token: Token,
    origin: OriginId,
    registration: crate::source_map::RegisteredSource,
) -> InputSummary {
    InputSummary::new(
        vec![InputFrameSummary::Source {
            source_id: crate::input::SourceId::new(1),
            input_record: None,
            source: SourceFrameSummary::new(
                0,
                1,
                1,
                0,
                LexerState::MidLine,
                "x".to_owned(),
                0,
                vec![TracedTokenWord::pack(token, origin)],
                false,
            )
            .with_registration(Some(registration)),
        }],
        None,
        None,
    )
}

fn source_summary_with_identity(
    token: Token,
    source_id: SourceId,
    registration: crate::source_map::RegisteredSource,
    next_source_id: u32,
) -> InputSummary {
    InputSummary::new_with_resume_state(
        vec![InputFrameSummary::Source {
            source_id,
            input_record: None,
            source: SourceFrameSummary::new(
                0,
                1,
                1,
                0,
                LexerState::MidLine,
                "x".to_owned(),
                0,
                vec![TracedTokenWord::pack(token, OriginId::UNKNOWN)],
                false,
            )
            .with_registration(Some(registration)),
        }],
        None,
        None,
        None,
        next_source_id,
        true,
    )
}

fn macro_replay_summary(
    body: crate::ids::TokenListId,
    origins: crate::ids::OriginListId,
    invocation: OriginId,
    argument_origin: OriginId,
) -> InputSummary {
    let arguments = one_macro_argument(TracedTokenWord::pack(Token::param(1), argument_origin), 1);
    InputSummary::new(
        vec![InputFrameSummary::TokenList {
            token_list: body,
            origin_list: origins,
            replay_kind: TokenListReplayKind::MacroBody,
            index: 0,
            macro_arguments: arguments,
            macro_invocation: invocation,
            parent_macro_invocation: OriginId::UNKNOWN,
        }],
        None,
        None,
    )
}

fn one_macro_argument(word: TracedTokenWord, slot: u8) -> MacroArguments {
    let mut ranges = [None; crate::input::MACRO_ARGUMENT_SLOTS];
    ranges[usize::from(slot - 1)] = Some(MacroArgumentRange::new(0, 1));
    MacroArguments::from_parts(Arc::from([word]), ranges)
}

fn transient_summary(word: TracedTokenWord) -> InputSummary {
    InputSummary::new(
        vec![InputFrameSummary::TransientTokenList {
            tokens: Arc::from([word]),
            replay_kind: TokenListReplayKind::Inserted,
            macro_invocation: OriginId::UNKNOWN,
            parent_macro_invocation: OriginId::UNKNOWN,
        }],
        None,
        None,
    )
}
