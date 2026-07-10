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
use crate::node::{BoxNode, BoxNodeFields, GlueKind, KernKind, Node, Sign};
use crate::page::{PageDimension, PageInteger};
use crate::provenance::{OriginRecord, SourceOrigin, SyntheticOriginKind};
use crate::scaled::{GlueSetRatio, Scaled};
use crate::token::{Catcode, OriginId, Token, TracedTokenWord};
use crate::world::{ContentHash, JobClock, PrintSink, StreamSlot, World};
use std::panic::{AssertUnwindSafe, catch_unwind};

#[test]
fn universe_is_send() {
    fn assert_send<T: Send>() {}

    assert_send::<Universe>();
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
    let left_invocation = universe.macro_invocation_origin(definition, left_origin, left_origin);
    let right_invocation = universe.macro_invocation_origin(definition, right_origin, right_origin);
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
fn universe_rollback_truncates_provenance_and_replay_reuses_origin_ids() {
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
    assert_eq!(replayed.raw(), stale.raw());
    assert_eq!(replayed_list.raw(), stale_list.raw());
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
    let zed = first.intern("z");
    let alpha = first.intern("alpha");
    let macro_target = first.intern("macro_target");
    first.set_meaning(zed, Meaning::Relax);
    let filler_tokens = first.intern_token_list(&[Token::param(1)]);
    let target_tokens = first.intern_token_list(&[
        Token::Cs(alpha),
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
        Token::Cs(alpha),
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
    let zed = second.intern("z");
    second.set_meaning(zed, Meaning::Relax);
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

fn assert_promoted_wrapper_is_resolvable(universe: &Universe, wrapper: NodeListId) {
    let [Node::VList(box_node)] = universe.nodes(wrapper) else {
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

fn checkpoint_hashes_for_program() -> Vec<u64> {
    let mut universe = Universe::new();
    let mut hashes = Vec::new();
    hashes.push(universe.snapshot().state_hash());

    universe.set_count(0, 42);
    universe.set_catcode('@', Catcode::Letter);
    hashes.push(universe.snapshot().state_hash());

    let symbol = universe.intern("foo");
    let tokens = universe.intern_token_list(&[Token::Cs(symbol)]);
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
        }],
        None,
        None,
    )
}
