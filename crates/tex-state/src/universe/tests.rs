use super::{CheckpointResumeKind, ResumeFallback, Universe};
use crate::font::NULL_FONT;
use crate::glue::{GlueSpec, Order};
use crate::ids::{ArenaRef, NodeListId};
use crate::input::{
    InputFrameSummary, InputSummary, LexerState, MacroArguments, SourceFrameSummary,
    TokenListReplayKind, TracedTokenList,
};
use crate::macro_store::MacroMeaning;
use crate::meaning::{Meaning, MeaningFlags};
use crate::node::{BoxNode, BoxNodeFields, GlueKind, KernKind, LeaderPayload, Node, Sign};
use crate::page::{PageDimension, PageInteger};
use crate::provenance::{OriginRecord, SourceOrigin, SyntheticOriginKind};
use crate::scaled::{GlueSetRatio, Scaled};
use crate::source_map::{SourceDescriptor, SourceMapError};
use crate::token::{Catcode, OriginId, Token, TracedTokenWord};
use crate::world::{ContentHash, JobClock, PrintSink, StreamSlot, World};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

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

    let first_list = universe.finish_traced_token_list(&first);
    let second_list = universe.finish_traced_token_list(&second);

    assert_eq!(first_list.token_list(), second_list.token_list());
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

    let first = universe.dump_format().expect("format encode");
    let second = universe.dump_format().expect("deterministic format encode");
    assert_eq!(first, second);

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
fn semantic_format_restores_validated_fonts_banks_hashes_and_rollback_exactly() {
    let mut universe = Universe::new();
    let null_identifier = universe.intern("nullfont");
    universe.set_font_identifier_symbol(NULL_FONT, null_identifier);
    let identifier = universe.intern("structuredfont");
    let font = universe.intern_font_with_identifier(structured_format_font(), identifier);
    universe.set_current_font_selector(identifier, font);
    universe.set_math_family_font(crate::math::MathFontSize::Text, 3, font, true);
    universe
        .set_font_dimen(font, 7, Scaled::from_raw(777), true)
        .expect("guaranteed parameter is writable");

    let bytes = universe.dump_format().expect("valid format encodes");
    let mut restored =
        Universe::from_format(World::memory(), &bytes).expect("valid format restores");
    assert_eq!(restored.dump_format().expect("format redumps"), bytes);
    let restored_font = restored.current_font();
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
        .set_font_dimen(restored_font, 7, Scaled::from_raw(-9), false)
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
        replace_store_format_payload(
            &mut bytes,
            crate::stores::testing_corrupt_font_format(&valid[29..], corruption),
        );
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
        replace_store_format_payload(
            &mut bytes,
            crate::stores::testing_corrupt_font_format(
                &valid[29..],
                Corruption::LigKernProgramLength { len, start },
            ),
        );
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

fn replace_format_ratio(bytes: &mut [u8], old: (i32, i32), new: (i32, i32)) {
    const HEADER: usize = 29;
    let old = [old.0.to_le_bytes(), old.1.to_le_bytes()].concat();
    let replacement = [new.0.to_le_bytes(), new.1.to_le_bytes()].concat();
    let offsets: Vec<_> = bytes[HEADER..]
        .windows(old.len())
        .enumerate()
        .filter_map(|(offset, window)| (window == old).then_some(HEADER + offset))
        .collect();
    assert_eq!(offsets.len(), 1, "ratio wire must occur exactly once");
    bytes[offsets[0]..offsets[0] + replacement.len()].copy_from_slice(&replacement);
}

fn refresh_format_checksum(bytes: &mut [u8]) {
    const HEADER: usize = 29;
    let checksum = super::format_checksum(bytes[12], &bytes[HEADER..]);
    bytes[21..29].copy_from_slice(&checksum.to_le_bytes());
}

fn replace_store_format_payload(bytes: &mut Vec<u8>, payload: Vec<u8>) {
    const HEADER: usize = 29;
    bytes.truncate(HEADER);
    bytes[13..21].copy_from_slice(&(payload.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&payload);
    refresh_format_checksum(bytes);
}

#[cfg(feature = "node-stats")]
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
            column.name == "survivor.live.boxes.width" && column.logical_bytes > 0
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
    let token = Token::Char {
        ch: 'x',
        cat: Catcode::Letter,
    };
    let left_origin = universe.source_origin(crate::input::SourceId::new(1), 0, 1, 1);
    let right_origin = universe.source_origin(crate::input::SourceId::new(1), 14, 3, 9);
    let left_summary = pending_source_summary(token, left_origin);
    let right_summary = pending_source_summary(token, right_origin);
    assert_eq!(left_summary, right_summary);

    universe.set_input_summary(left_summary);
    let left_hash = universe.snapshot().state_hash();
    universe.set_input_summary(right_summary);
    let right_hash = universe.snapshot().state_hash();

    assert_eq!(left_hash, right_hash);
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
    let argument = universe.intern_token_list(&[Token::param(1)]);
    let left_origin = universe.source_origin(crate::input::SourceId::new(1), 10, 2, 3);
    let right_origin = universe.source_origin(crate::input::SourceId::new(2), 20, 4, 5);
    let left_origins = universe.allocate_origin_list(&[left_origin]);
    let right_origins = universe.allocate_origin_list(&[right_origin]);
    let left_invocation =
        universe.macro_invocation_origin(definition, left_origin, left_origin, OriginId::UNKNOWN);
    let right_invocation =
        universe.macro_invocation_origin(definition, right_origin, right_origin, OriginId::UNKNOWN);
    let left_summary = macro_replay_summary(body, argument, left_origins, left_invocation);
    let right_summary = macro_replay_summary(body, argument, right_origins, right_invocation);
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
fn hash_only_checkpoint_records_previous_resume_boundary() {
    let mut universe = Universe::new();
    let symbol = universe.intern("x");
    let resume = universe.snapshot();
    let resume_fallback = resume
        .resume_fallback()
        .expect("resume-valid snapshot is its own resume fallback");
    let resume_boundary = resume_fallback.boundary();

    let hash_only = universe.with_hash_only_checkpoints(|universe| {
        universe.set_meaning(symbol, Meaning::Relax);
        universe.snapshot()
    });

    assert_eq!(resume.resume_kind(), CheckpointResumeKind::ResumeValid);
    assert_eq!(
        resume_fallback,
        ResumeFallback::DirectRollback(resume_boundary)
    );
    assert_eq!(hash_only.resume_kind(), CheckpointResumeKind::HashOnly);
    assert_eq!(
        hash_only.resume_fallback(),
        Some(ResumeFallback::DirectRollback(resume_boundary))
    );
    assert_eq!(
        universe.last_checkpoint(),
        Some(hash_only.checkpoint_metadata())
    );

    universe.rollback(&resume);

    let replayed = universe.with_hash_only_checkpoints(|universe| {
        universe.set_meaning(symbol, Meaning::Relax);
        universe.snapshot()
    });
    assert_eq!(replayed.resume_kind(), CheckpointResumeKind::HashOnly);
    assert_eq!(
        replayed.resume_fallback(),
        Some(ResumeFallback::DirectRollback(resume_boundary))
    );
    assert_eq!(replayed.state_hash(), hash_only.state_hash());
}

#[test]
fn effectful_hash_only_commit_marks_resume_fallback_unavailable() {
    let mut universe = Universe::new();
    let resume = universe.snapshot();
    let resume_boundary = resume
        .resume_fallback()
        .expect("resume-valid snapshot is its own resume fallback")
        .boundary();

    universe.with_hash_only_checkpoints(|universe| {
        universe
            .world_mut()
            .write_text(PrintSink::TerminalAndLog, "nested shipout effect\n");
        let effect_pos = universe.world().effect_pos();
        universe
            .commit_effects(effect_pos)
            .expect("memory world commit succeeds");
    });

    let checkpoint = universe
        .last_checkpoint()
        .expect("hash-only commit should checkpoint");
    assert_eq!(checkpoint.resume_kind(), CheckpointResumeKind::HashOnly);
    assert_eq!(
        checkpoint.resume_fallback(),
        Some(ResumeFallback::Unavailable(resume_boundary))
    );
    assert!(
        !checkpoint
            .resume_fallback()
            .expect("fallback should be recorded")
            .direct_rollback_available()
    );
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
    assert!(universe.current_page_nodes().is_empty());
    assert_eq!(
        universe.page_dimension(PageDimension::Goal),
        Scaled::MAX_DIMEN
    );
    assert_eq!(universe.page_integer(PageInteger::InsertPenalties), 0);
    assert!(universe.page_fire_up().is_none());
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
    let boundary = universe.begin_shipout();
    let children = universe.freeze_node_list(&[Node::Kern {
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
    assert_eq!(universe.testing_epoch_node_count(), 1);

    universe
        .world_mut()
        .write_text(PrintSink::TerminalAndLog, "shipout\n");
    let effect_pos = universe.world().effect_pos();
    let hash = universe
        .commit_shipout(boundary, b"detached page artifact", effect_pos)
        .expect("shipout commit succeeds");

    assert_eq!(hash, ContentHash::from_bytes(b"detached page artifact"));
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
        let boundary = universe.begin_shipout();
        let children = universe.freeze_node_list(&[Node::Kern {
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
        let effect_pos = universe.world().effect_pos();
        universe
            .commit_shipout(boundary, format!("page {page}").as_bytes(), effect_pos)
            .expect("shipout commit succeeds");
        assert_eq!(universe.testing_epoch_node_count(), 0);
    }
}

#[test]
fn snapshot_state_hash_is_deterministic_for_same_program() {
    assert_eq!(
        checkpoint_hashes_for_program(),
        checkpoint_hashes_for_program()
    );
}

#[test]
fn snapshot_state_hash_ignores_content_intern_order() {
    let mut first = Universe::new();
    let first_zed = first.intern("z");
    let alpha = first.intern("alpha");
    let macro_target = first.intern("macro_target");
    first.set_meaning(first_zed, Meaning::Relax);
    let filler_tokens = first.intern_token_list(&[Token::param(1)]);
    let target_tokens = first.intern_token_list(&[
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
        target_tokens,
        target_tokens,
    ));
    first.set_toks(0, target_tokens);
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
    let target_tokens = second.intern_token_list(&[
        Token::Cs(alpha.symbol()),
        Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        },
    ]);
    let filler_tokens = second.intern_token_list(&[Token::param(1)]);
    let target_glue = second.intern_glue(glue(7));
    let filler_glue = second.intern_glue(glue(99));
    let target_macro = second.intern_macro(MacroMeaning::new(
        MeaningFlags::PROTECTED,
        target_tokens,
        target_tokens,
    ));
    let filler_macro = second.intern_macro(MacroMeaning::new(
        MeaningFlags::LONG,
        filler_tokens,
        filler_tokens,
    ));
    let second_zed = second.intern("z");
    second.set_meaning(second_zed, Meaning::Relax);
    second.set_toks(0, target_tokens);
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
        .set_font_dimen(short, 7, Scaled::from_raw(77), false)
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
fn rollback_restores_font_identifier_registration() {
    let mut universe = Universe::new();
    let snapshot = universe.snapshot();
    let nullfont = universe.intern("nullfont");
    universe.set_font_identifier_symbol(NULL_FONT, nullfont);
    assert_eq!(universe.font_identifier_symbol(NULL_FONT), Some(nullfont));

    universe.rollback(&snapshot);
    assert_eq!(universe.font_identifier_symbol(NULL_FONT), None);
}

#[test]
fn rollback_reuse_does_not_revive_stale_font_identity() {
    let mut universe = Universe::new();
    let snapshot = universe.snapshot();
    let stale = universe.intern_font(test_font("stale", b"stale"));

    universe.rollback(&snapshot);
    let reused = universe.intern_font(test_font("reused", b"reused"));

    assert_eq!(reused.raw(), stale.raw());
    assert_ne!(reused, stale);
    assert!(std::panic::catch_unwind(|| universe.font(stale)).is_err());
    assert_eq!(universe.font(reused).name(), "reused");
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
    }]);
    reused.set_box_reg(0, first_list);
    let first_hash = reused.snapshot().state_hash();

    reused.rollback(&base);
    let second_list = reused.freeze_node_list(&[Node::Char {
        font: NULL_FONT,
        ch: 'y',
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
fn grouped_box_take_copies_nested_survivor_children_before_coalesced_release() {
    let mut universe = Universe::new();
    let leader_children = universe.freeze_node_list(&[Node::Char {
        font: NULL_FONT,
        ch: 'x',
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
    let taken = universe
        .take_box_reg_same_level(0)
        .expect("local box should move out of the register");

    assert!(matches!(taken.arena(), ArenaRef::Epoch));
    let Some(crate::node_arena::NodeRef::Glue {
        leader: Some(LeaderPayload::HList(leader)),
        ..
    }) = universe.nodes(taken).first()
    else {
        panic!("taken value should preserve its leader box");
    };
    assert!(matches!(leader.children.arena(), ArenaRef::Epoch));
    assert_eq!(
        universe.nodes(leader.children),
        &[Node::Char {
            font: NULL_FONT,
            ch: 'x'
        }]
    );
    let _ = universe.leave_group();
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
            ch: 'x'
        }]
    );
}

#[test]
fn snapshot_state_hash_walks_deep_node_lists_iteratively() {
    let mut universe = Universe::new();
    let mut current = universe.freeze_node_list(&[Node::Char {
        font: NULL_FONT,
        ch: 'x',
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
    }]);
    direct.set_box_reg(0, direct_final);
    let overwritten_final = overwritten.freeze_node_list(&[Node::Char {
        font: NULL_FONT,
        ch: 'x',
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
    }]);
    let boundary = universe.begin_box_build();
    let children = universe.freeze_node_list(&[Node::Kern {
        amount: Scaled::from_raw(17),
        kind: KernKind::Explicit,
    }]);
    let root = universe.freeze_node_list(&[Node::HList(BoxNode::new(BoxNodeFields {
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
    assert_eq!(universe.testing_epoch_node_count(), 3);

    universe.finish_box_assignment(boundary, 0, Some(root), false);

    assert_eq!(universe.testing_epoch_node_count(), 1);
    assert_eq!(
        universe.nodes(older).first(),
        Some(crate::node_arena::NodeRef::Char {
            font: NULL_FONT,
            ch: 'a'
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
    let boundary = universe.begin_box_build();
    let _discarded = universe.freeze_node_list(&[Node::Char {
        font: NULL_FONT,
        ch: 'x',
    }]);

    universe.cancel_box_build(boundary);

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
    universe
        .world_mut()
        .record_deferred_write(StreamSlot::new(1), tokens);
    hashes.push(universe.snapshot().state_hash());

    let _ = universe.world_mut().next_random_u64();
    hashes.push(universe.snapshot().state_hash());
    hashes
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

fn pending_source_summary(token: Token, origin: OriginId) -> InputSummary {
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
            ),
        }],
        None,
        None,
    )
}

fn macro_replay_summary(
    body: crate::ids::TokenListId,
    argument: crate::ids::TokenListId,
    origins: crate::ids::OriginListId,
    invocation: OriginId,
) -> InputSummary {
    let mut arguments = MacroArguments::new();
    arguments.set_traced(1, TracedTokenList::new(argument, origins));
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
