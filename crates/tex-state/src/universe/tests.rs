use super::{
    BoxDimension, FormatError, GenerationForkError, TakeUnboxResult, UnboxKind, Universe,
    utf8_scalar_len_at,
};
use crate::PdfDocumentFragmentKind;
use crate::env::banks::{IntParam, TokParam};
use crate::font::{
    FONT_INFO_CAPACITY, FontExpansion, MAX_FONT_DIMEN, NULL_FONT, WEB2C_FONT_INFO_CAPACITY,
};
use crate::glue::{GlueSpec, Order};
use crate::hyphenation::{ExceptionSpec, PatternSpec};
use crate::ids::{ArenaRef, FontId, TokenListId};
use crate::input::{
    ConditionFrameSummary, ConditionFrameToken, InputFrameSummary, InputSummary, LexerState,
    MacroArgumentRange, MacroArguments, SourceFrameSummary, SourceId, TokenListReplayKind,
    TracedTokenList,
};
use crate::macro_store::{MacroDefinitionProvenance, MacroMeaning};
use crate::math::{
    FractionThickness, MathChar, MathChoice, MathField, MathFraction, MathListNode, MathNoad,
    MathStyle, NoadClass, NoadKind,
};
use crate::meaning::{Meaning, MeaningFlags, RawMeaning};
use crate::node::{
    AdjustNode, BoxLr, BoxNode, BoxNodeFields, GlueKind, KernKind, LeaderPayload, MarginKernSide,
    Node, PdfLiteralMode, Sign, Whatsit,
};
use crate::node_arena::NodeListRef;
use crate::page::{PageDimension, PageInteger, PageMark};
use crate::provenance::{
    InsertedOriginKind, OriginRecord, SourceOrigin, SynthesizedOriginKind, SyntheticOriginKind,
};
use crate::scaled::{GlueSetRatio, Scaled};
use crate::source_fragments::{EditorLayout, FragmentStore, LayoutGeneration, Piece};
use crate::source_map::{SourceDescriptor, SourceMapError};
use crate::token::{
    Catcode, OriginId, RootedTracedTokenBuffer, RootedTracedTokenWord, Token, TracedTokenWord,
};
use crate::world::{
    ContentDomain, ContentHash, EffectRecord, InputDependencyAccess, InputDependencyOutcome,
    JobClock, PrintSink, ShellEscapePolicy, StreamSlot, World,
};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

fn zero_box(children: NodeListRef) -> BoxNode {
    BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(0),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
        shift: Scaled::from_raw(0),
        box_lr: BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children,
    })
}

fn freeze_ref(universe: &mut Universe, nodes: &[Node]) -> NodeListRef {
    universe.freeze_node_list(nodes)
}

#[test]
fn rejected_direct_operation_discards_unpublished_page_node_builders_exactly() {
    let mut universe = Universe::new();
    universe.begin_private_revision();

    let operation = universe.begin_direct_operation();
    let rejected = universe.freeze_node_list(&[Node::Penalty(17)]);
    drop(rejected);
    universe.discard_direct_operation_allocations(operation);

    assert!(universe.page_contributions().is_empty());
}

#[test]
fn committed_page_nodes_remain_owned_across_checkpoint_rollback() {
    let mut universe = Universe::new();
    universe.begin_private_revision();

    let operation = universe.begin_direct_operation();
    let children = universe.freeze_node_list(&[Node::Penalty(23)]);
    universe.append_page_contribution(Node::HList(zero_box(children)));
    universe.commit_direct_operation(operation);

    let promoted = match universe.page_contribution_front() {
        Some(Node::HList(node)) => node.children.clone(),
        other => panic!("expected committed page hlist, got {other:?}"),
    };
    assert_eq!(promoted.to_vec(), [Node::Penalty(23)]);

    let checkpoint = universe.snapshot();
    let operation = universe.begin_direct_operation();
    let _ = universe.pop_page_contribution_front();
    universe.commit_direct_operation(operation);

    universe.rollback(&checkpoint);
    assert_eq!(promoted.to_vec(), [Node::Penalty(23)]);
    let operation = universe.begin_direct_operation();
    let _ = universe.pop_page_contribution_front();
    universe.commit_direct_operation(operation);
    drop(checkpoint);

    let operation = universe.begin_direct_operation();
    universe.commit_direct_operation(operation);
}

#[test]
fn committed_level_zero_operations_retire_journal_history_at_a_bounded_baseline() {
    let mut universe = Universe::new();
    let first = universe.begin_direct_operation();
    universe.set_count_global(0, 1);
    universe.commit_direct_operation(first);
    assert_eq!(universe.env_journal_entry_count(), 0);
    let retained_bytes = universe.env_journal_bytes();

    for value in 2..=10_000 {
        let operation = universe.begin_direct_operation();
        universe.set_count_global(0, value);
        universe.set_count_global(1, -value);
        universe.commit_direct_operation(operation);
        assert_eq!(universe.env_journal_entry_count(), 0);
        assert_eq!(universe.stores.testing_exact_env_undo_entries(), 0);
    }

    assert_eq!(universe.count(0), 10_000);
    assert_eq!(universe.count(1), -10_000);
    assert_eq!(universe.env_journal_bytes(), retained_bytes);
}

#[test]
fn closed_groups_retire_but_open_groups_and_retained_checkpoints_keep_exact_history() {
    let mut universe = Universe::new();
    let retained = universe.snapshot();
    let after_x = Token::Char {
        ch: 'x',
        cat: Catcode::Other,
    };
    let after_y = Token::Char {
        ch: 'y',
        cat: Catcode::Other,
    };

    let committed = universe.begin_direct_operation();
    universe.enter_group();
    universe.set_count(0, 11);
    universe.push_aftergroup(after_x);
    assert_eq!(universe.leave_group(), vec![after_x]);
    universe.set_count_global(1, 22);
    universe.commit_direct_operation(committed);
    assert!(universe.env_journal_entry_count() > 0);
    universe.rollback(&retained);
    assert_eq!(universe.count(0), 0);
    assert_eq!(universe.count(1), 0);
    drop(retained);

    universe.enter_group();
    let open = universe.begin_direct_operation();
    universe.set_count(0, 33);
    universe.push_aftergroup(after_y);
    let invalidated_inside_group = universe.snapshot();
    universe.commit_direct_operation(open);
    assert!(universe.env_journal_entry_count() > 0);
    assert_eq!(universe.leave_group(), vec![after_y]);
    assert_eq!(universe.count(0), 0);
    assert!(!universe.can_rollback_to(&invalidated_inside_group));

    let baseline = universe.begin_direct_operation();
    universe.set_count_global(2, 44);
    universe.commit_direct_operation(baseline);
    assert_eq!(universe.env_journal_entry_count(), 0);
    assert_eq!(universe.count(2), 44);
    drop(invalidated_inside_group);

    universe.enter_group();
    let open_without_retained_snapshot = universe.begin_direct_operation();
    universe.set_count(3, 55);
    universe.commit_direct_operation(open_without_retained_snapshot);
    assert_eq!(
        universe.stores.testing_exact_env_undo_entries(),
        0,
        "an open TeX group alone must not retain derived snapshot deltas"
    );
    let _ = universe.leave_group();
}

#[test]
fn journal_retirement_preserves_the_named_checkpoint_hash_schedule() {
    let mut retired = Universe::new();
    let mut uninterrupted = Universe::new();

    for value in 1..=128 {
        let operation = retired.begin_direct_operation();
        retired.set_count_global((value % 4) as u16, value);
        retired.commit_direct_operation(operation);
        uninterrupted.set_count_global((value % 4) as u16, value);
    }

    assert_eq!(retired.env_journal_entry_count(), 0);
    let retired_hash = retired.snapshot().state_hash();
    let uninterrupted_hash = uninterrupted.snapshot().state_hash();
    assert_eq!(retired_hash, uninterrupted_hash);

    let original = retired.count(0);
    let first = retired.begin_direct_operation();
    retired.set_count_global(0, 999);
    retired.commit_direct_operation(first);
    let second = retired.begin_direct_operation();
    retired.set_count_global(0, original);
    retired.commit_direct_operation(second);
    uninterrupted.set_count_global(0, 999);
    uninterrupted.set_count_global(0, original);
    assert_eq!(
        retired.snapshot().state_hash(),
        uninterrupted.snapshot().state_hash()
    );
}

#[test]
fn journal_retirement_releases_superseded_reachability_owned_values() {
    let mut universe = Universe::new();

    let old_tokens_root = universe.intern_token_list_ref(&[Token::Char {
        ch: 'a',
        cat: Catcode::Other,
    }]);
    let old_tokens = old_tokens_root.id();
    let old_glue = universe.intern_glue(glue(71));
    let old_glue_id = old_glue.id();
    let old_macro = universe.intern_macro(MacroMeaning::new(
        MeaningFlags::EMPTY,
        TokenListId::EMPTY,
        TokenListId::EMPTY,
    ));
    let old_macro_id = old_macro.id();
    let name = universe.intern("retired-macro");
    let first = universe.begin_direct_operation();
    universe.set_toks(7, old_tokens);
    universe.set_skip(7, old_glue);
    universe.set_meaning(
        name,
        Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition: old_macro_id,
        },
    );
    drop(old_tokens_root);
    drop(old_macro);
    universe.commit_direct_operation(first);

    let new_tokens_root = universe.intern_token_list_ref(&[Token::Char {
        ch: 'b',
        cat: Catcode::Other,
    }]);
    let new_tokens = new_tokens_root.id();
    let new_glue = universe.intern_glue(glue(72));
    let new_macro = universe.intern_macro(MacroMeaning::new(
        MeaningFlags::LONG,
        TokenListId::EMPTY,
        TokenListId::EMPTY,
    ));
    let second = universe.begin_direct_operation();
    universe.set_toks(7, new_tokens);
    universe.set_skip(7, new_glue);
    universe.set_meaning(
        name,
        Meaning::Macro {
            flags: MeaningFlags::LONG,
            definition: new_macro.id(),
        },
    );
    drop(new_tokens_root);
    drop(new_macro);
    universe.commit_direct_operation(second);

    assert!(catch_unwind(AssertUnwindSafe(|| universe.tokens(old_tokens))).is_err());
    assert!(catch_unwind(AssertUnwindSafe(|| universe.glue(old_glue_id))).is_err());
    assert!(
        catch_unwind(AssertUnwindSafe(|| {
            universe.macro_definition(old_macro_id)
        }))
        .is_err()
    );
}

#[test]
fn private_token_roots_accept_and_rejected_direct_suffixes_do_not_publish() {
    let mut universe = Universe::new();
    universe.begin_private_revision();

    let first_operation = universe.begin_direct_operation();
    let retained = universe.intern_token_list(&[Token::Char {
        ch: 'k',
        cat: Catcode::Letter,
    }]);
    universe.set_toks(7, retained);
    universe.commit_direct_operation(first_operation);
    let retained_hash = universe.snapshot().state_hash();
    let retained_effects = universe.world.effect_records().len();
    let retained_stats = universe
        .testing_private_revision_domain_stats()
        .expect("private domain is live");
    assert_eq!(retained_stats.0, 1);

    let failed_operation = universe.begin_direct_operation();
    let failed = universe.intern_token_list(&[
        Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        },
        Token::Char {
            ch: 'y',
            cat: Catcode::Letter,
        },
    ]);
    universe.discard_direct_operation_allocations(failed_operation);
    assert_eq!(
        universe.testing_private_revision_domain_stats(),
        Some(retained_stats)
    );
    assert_eq!(universe.tokens(retained).len(), 1);
    assert_eq!(universe.toks(7), retained);
    assert_eq!(universe.snapshot().state_hash(), retained_hash);
    assert_eq!(universe.world.effect_records().len(), retained_effects);
    assert!(catch_unwind(AssertUnwindSafe(|| universe.tokens(failed))).is_err());

    universe
        .accept_private_revision()
        .expect("typed compatibility root transfers");
    assert!(universe.testing_private_revision_domain_stats().is_none());
    assert_eq!(
        universe.tokens(retained)[0],
        Token::Char {
            ch: 'k',
            cat: Catcode::Letter,
        }
    );
}

#[test]
fn private_glue_roots_accept_and_rejected_direct_suffixes_do_not_publish() {
    let mut universe = Universe::new();
    universe.begin_private_revision();

    let retained_operation = universe.begin_direct_operation();
    let retained = universe.intern_glue(glue(101));
    let retained_id = retained.id();
    universe.set_skip(7, retained);
    universe.commit_direct_operation(retained_operation);
    let retained_stats = universe
        .testing_private_revision_domain_stats()
        .expect("private glue domain is live");
    assert_eq!(retained_stats.0, 1);

    let failed_operation = universe.begin_direct_operation();
    let failed = universe.intern_glue(glue(102));
    let failed_id = failed.id();
    drop(failed);
    universe.discard_direct_operation_allocations(failed_operation);
    assert_eq!(universe.skip(7), retained_id);
    assert_eq!(universe.glue(retained_id), glue(101));
    assert!(catch_unwind(AssertUnwindSafe(|| universe.glue(failed_id))).is_err());
    assert_eq!(
        universe.testing_private_revision_domain_stats(),
        Some(retained_stats)
    );

    let unselected_operation = universe.begin_direct_operation();
    let unselected = universe.intern_glue(glue(103));
    let unselected_id = unselected.id();
    drop(unselected);
    universe.commit_direct_operation(unselected_operation);
    assert_eq!(
        universe
            .testing_private_revision_domain_stats()
            .expect("private glue domain remains live")
            .0,
        2
    );

    universe
        .accept_private_revision()
        .expect("only the Env-owned glue allocation transfers");
    assert!(universe.testing_private_revision_domain_stats().is_none());
    assert_eq!(universe.glue(universe.skip(7)), glue(101));
    assert!(catch_unwind(AssertUnwindSafe(|| universe.glue(unselected_id))).is_err());
}

#[test]
fn glue_current_undo_page_and_checkpoint_edges_are_structural_roots() {
    let mut universe = Universe::new();
    assert_eq!(universe.stores.testing_glue_live_totals().0, 1);

    let outer = universe.intern_glue(glue(201));
    universe.set_skip(0, &outer);
    drop(outer);
    assert_eq!(universe.stores.testing_glue_live_totals().0, 2);

    universe.enter_group();
    let local = universe.intern_glue(glue(202));
    let local_id = local.id();
    universe.set_skip(0, local);
    assert_eq!(universe.stores.testing_glue_live_totals().0, 3);
    let _ = universe.leave_group();
    assert_eq!(universe.stores.testing_glue_live_totals().0, 2);
    assert!(catch_unwind(AssertUnwindSafe(|| universe.glue(local_id))).is_err());

    let checkpoint = universe.snapshot();
    let page = universe.intern_glue(glue(203));
    let page_id = page.id();
    universe.append_page_contribution(Node::Glue {
        spec: page,
        kind: GlueKind::Normal,
        leader: None,
    });
    assert_eq!(universe.stores.testing_glue_live_totals().0, 3);

    universe.rollback(&checkpoint);
    assert_eq!(universe.stores.testing_glue_live_totals().0, 2);
    assert!(catch_unwind(AssertUnwindSafe(|| universe.glue(page_id))).is_err());
    assert_eq!(universe.glue(universe.skip(0)), glue(201));
}

#[test]
fn private_macro_roots_accept_and_rejected_direct_suffixes_do_not_publish() {
    let mut universe = Universe::new();
    let name = universe.intern("private-macro");
    universe.begin_private_revision();

    let retained_operation = universe.begin_direct_operation();
    let retained_body = universe.intern_token_list(&[Token::Char {
        ch: 'r',
        cat: Catcode::Letter,
    }]);
    let retained = universe.intern_macro(MacroMeaning::new(
        MeaningFlags::LONG,
        TokenListId::EMPTY,
        retained_body,
    ));
    universe.set_meaning(
        name,
        Meaning::Macro {
            flags: MeaningFlags::LONG,
            definition: retained.id(),
        },
    );
    drop(retained);
    let retained = universe.intern_macro(MacroMeaning::new(
        MeaningFlags::LONG,
        TokenListId::EMPTY,
        retained_body,
    ));
    let retained_id = retained.id();
    universe.set_meaning(
        name,
        Meaning::Macro {
            flags: MeaningFlags::LONG,
            definition: retained_id,
        },
    );
    drop(retained);
    universe.commit_direct_operation(retained_operation);
    let retained_stats = universe
        .testing_private_revision_domain_stats()
        .expect("private macro domain is live");
    assert_eq!(
        retained_stats.0, 4,
        "token, body, and two occurrences are private"
    );

    let failed_operation = universe.begin_direct_operation();
    let failed_body = universe.intern_token_list(&[Token::Char {
        ch: 'f',
        cat: Catcode::Letter,
    }]);
    let failed = universe.intern_macro(MacroMeaning::new(
        MeaningFlags::OUTER,
        TokenListId::EMPTY,
        failed_body,
    ));
    let failed_id = failed.id();
    drop(failed);
    universe.discard_direct_operation_allocations(failed_operation);
    assert_eq!(
        universe.testing_private_revision_domain_stats(),
        Some(retained_stats)
    );
    assert_eq!(
        universe.meaning(name),
        Meaning::Macro {
            flags: MeaningFlags::LONG,
            definition: retained_id,
        }
    );
    assert!(
        catch_unwind(AssertUnwindSafe(|| {
            universe.macro_definition(failed_id)
        }))
        .is_err()
    );

    let mut substrate = universe.freeze_generation();
    substrate
        .accept_private_revision()
        .expect("typed macro roots transfer");
    assert!(substrate.testing_private_revision_domain_stats().is_none());
    assert_eq!(
        substrate
            .universe
            .macro_definition(retained_id)
            .replacement_text(),
        retained_body
    );
}

#[test]
fn engine_usage_statistics_retain_one_word_extent_across_rollback() {
    let mut universe = Universe::new();
    let baseline = universe.snapshot();
    let before = universe.engine_usage_statistics();
    assert_eq!(before.memory_words, 1_045);
    assert_eq!(before.memory_word_capacity, 250_000);
    universe.intern("allocator-high-water-probe");
    let tokens = universe.intern_token_list(&[Token::Char {
        ch: 'x',
        cat: crate::token::Catcode::Other,
    }]);
    universe.set_toks_global(0, tokens);
    let peak = universe.engine_usage_statistics();
    assert!(peak.strings > before.strings);
    assert!(peak.string_characters > before.string_characters);
    universe.rollback(&baseline);
    let rolled_back = universe.engine_usage_statistics();
    assert_eq!(rolled_back.strings, peak.strings);
    assert_eq!(rolled_back.string_characters, peak.string_characters);
    // §§125/1334 return the token words to `avail`, but the allocator's
    // low/high coordinate extent survives the rollback.
    assert_eq!(rolled_back.memory_words, peak.memory_words);
    assert_eq!(
        rolled_back.memory_word_capacity,
        before.memory_word_capacity
    );
}

#[test]
fn main_memory_extent_is_recorded_at_allocation_before_group_restore() {
    let mut positive = Universe::new();
    positive.enter_group();
    // TeX82 §§125/127 move the one-word allocator coordinate when the
    // scanner allocates the list, not when §283 later walks `unsave`.
    positive.observe_transient_token_words(601);
    let tokens = positive.intern_token_list(&vec![
        Token::Char {
            ch: 'x',
            cat: Catcode::Other,
        };
        600
    ]);
    positive.set_toks(0, tokens);
    let _ = positive.leave_group();
    // Section 283 releases the local token list, while §1334 retains the
    // allocator coordinate recorded at the allocation event.
    assert_eq!(positive.engine_usage_statistics().memory_words, 1_642);

    let mut negative = Universe::new();
    negative.enter_group();
    negative.intern_token_list(&vec![
        Token::Char {
            ch: 'x',
            cat: Catcode::Other,
        };
        600
    ]);
    let _ = negative.leave_group();
    // An immutable host-store allocation without a canonical allocator event
    // does not occupy TeX's allocator and must not move the coordinate.
    assert_eq!(negative.engine_usage_statistics().memory_words, 1_045);
}

#[test]
fn main_memory_projection_separates_variable_and_character_nodes() {
    let mut variable = Universe::new();
    let penalties = variable.freeze_node_list(&vec![Node::Penalty(0); 501]);
    variable.set_box_reg_ref_global(0, penalties);
    // TeX82 §§127/157 allocate two low-memory words per penalty. The 1002
    // live words cross §127's initial 1000-word free block exactly once.
    assert_eq!(variable.engine_usage_statistics().memory_words, 2_045);

    let mut characters = Universe::new();
    let chars = characters.freeze_node_list(&vec![
        Node::Char {
            font: NULL_FONT,
            ch: 'x',
            origin: crate::provenance::OriginRef::unknown(),
        };
        501
    ]);
    characters.set_box_reg_ref_global(0, chars);
    // Section 135's character nodes come from §125's one-word arena, so the
    // same logical node count must not grow the variable-size low arena.
    assert_eq!(characters.engine_usage_statistics().memory_words, 1_546);
}

#[test]
fn main_memory_extent_observes_transient_frozen_node_lists() {
    let mut variable = Universe::new();
    variable.freeze_node_list(&vec![Node::Penalty(0); 501]);
    // TeX82 §§127/157: conversion allocates these two-word nodes before a
    // completed list is installed in any semantic owner. The §1334 low-arena
    // coordinate survives even if the temporary result is never retained.
    assert_eq!(variable.engine_usage_statistics().memory_words, 2_045);

    let mut dynamic = Universe::new();
    dynamic.freeze_node_list(&vec![
        Node::Char {
            font: NULL_FONT,
            ch: 'x',
            origin: crate::provenance::OriginRef::unknown(),
        };
        501
    ]);
    // Section 135's character nodes use the one-word high arena, so the same
    // transient list must not invent a low-memory growth block.
    assert_eq!(dynamic.engine_usage_statistics().memory_words, 1_542);
}

#[test]
fn main_memory_extent_observes_scanner_owned_token_words() {
    let mut positive = Universe::new();
    positive.observe_transient_token_words(600);
    // TeX82 §§200/384 allocate scanner results from the one-word arena even
    // before a semantic owner installs the completed token list.
    assert_eq!(positive.engine_usage_statistics().memory_words, 1_641);

    let mut negative = Universe::new();
    negative.intern_token_list(&vec![
        Token::Char {
            ch: 'x',
            cat: Catcode::Other,
        };
        600
    ]);
    // Unbound immutable host-store history is not a TeX allocation owner.
    assert_eq!(negative.engine_usage_statistics().memory_words, 1_045);
}

#[test]
fn transient_memory_observations_reuse_the_unchanged_allocator_base() {
    let mut universe = Universe::new();
    universe.observe_transient_token_words(600);
    universe.observe_transient_token_words(601);
    assert_eq!(universe.testing_transient_memory_base_projections(), 1);
    assert_eq!(universe.engine_usage_statistics().memory_words, 1_642);

    let unowned = universe.intern_token_list(&vec![
        Token::Char {
            ch: 'x',
            cat: Catcode::Other,
        };
        100
    ]);
    universe.observe_transient_token_words(602);
    // Immutable store history is not an allocator owner, so it neither
    // invalidates nor enters the cached live-root base.
    assert_eq!(universe.testing_transient_memory_base_projections(), 1);
    assert_eq!(universe.engine_usage_statistics().memory_words, 1_643);

    universe.intern_glue(GlueSpec {
        width: Scaled::from_raw(123),
        stretch: Scaled::from_raw(0),
        stretch_order: Order::Normal,
        shrink: Scaled::from_raw(0),
        shrink_order: Order::Normal,
    });
    universe.observe_transient_token_words(602);
    // Glue allocation advances its TeX82 low-arena projection in O(1), but
    // immutable glue-store growth is not a reason to rebuild unrelated roots.
    assert_eq!(universe.testing_transient_memory_base_projections(), 1);

    universe.set_toks_global(0, unowned);
    universe.observe_transient_token_words(603);
    // Installing the list as a canonical root updates that allocator base
    // incrementally; it does not reconstruct unrelated macro/token roots.
    assert_eq!(universe.testing_transient_memory_base_projections(), 1);
    assert_eq!(universe.engine_usage_statistics().memory_words, 1_745);

    let body = universe.intern_token_list(&vec![
        Token::Char {
            ch: 'y',
            cat: Catcode::Other,
        };
        50
    ]);
    let symbol = universe.intern("cached-allocator-macro");
    universe.set_macro_meaning_global(
        symbol,
        MacroMeaning::new(MeaningFlags::EMPTY, TokenListId::EMPTY, body),
    );
    universe.observe_transient_token_words(604);
    assert_eq!(universe.testing_transient_memory_base_projections(), 1);
    assert_eq!(universe.engine_usage_statistics().memory_words, 1_798);

    universe.set_meaning_global(symbol, Meaning::Relax);
    universe.observe_transient_token_words(700);
    // Removing the macro releases its definition words from the live base;
    // the later transient allocation still advances the canonical high-water.
    assert_eq!(universe.testing_transient_memory_base_projections(), 1);
    assert_eq!(universe.engine_usage_statistics().memory_words, 1_842);
}

#[test]
fn executor_operation_boundaries_retain_the_unchanged_allocator_base() {
    let mut universe = Universe::new();
    universe.observe_transient_token_words(600);
    assert_eq!(universe.testing_transient_memory_base_projections(), 1);

    let committed = universe.begin_direct_operation();
    universe.commit_direct_operation(committed);
    universe.observe_transient_token_words(601);
    assert_eq!(universe.testing_transient_memory_base_projections(), 1);

    let discarded = universe.begin_direct_operation();
    universe.discard_direct_operation_allocations(discarded);
    universe.observe_transient_token_words(602);
    assert_eq!(universe.testing_transient_memory_base_projections(), 1);

    let rolled_back = universe.snapshot();
    universe.observe_transient_token_words(603);
    universe.rollback(&rolled_back);
    universe.observe_transient_token_words(0);
    // The rollback receipt updates the projection before rejected dynamic
    // handles are truncated. Its pre-rollback allocator coordinate still
    // contributes to §1334's high-water diagnostic.
    assert_eq!(universe.testing_transient_memory_base_projections(), 1);
    assert_eq!(universe.engine_usage_statistics().memory_words, 1_644);
}

#[test]
fn rollback_updates_allocator_roots_before_truncating_rejected_values() {
    let mut universe = Universe::new();
    universe.observe_transient_token_words(0);
    let symbol = universe.intern("rolled-back-memory-macro");
    let snapshot = universe.snapshot();

    let tokens = universe.intern_token_list(&vec![
        Token::Char {
            ch: 'r',
            cat: Catcode::Other,
        };
        100
    ]);
    universe.set_toks_global(0, tokens);
    universe.set_macro_meaning_global(
        symbol,
        MacroMeaning::new(MeaningFlags::EMPTY, TokenListId::EMPTY, tokens),
    );
    universe.observe_transient_token_words(0);
    assert_eq!(universe.testing_transient_memory_base_projections(), 1);

    universe.rollback(&snapshot);
    universe.observe_transient_token_words(0);
    assert_eq!(universe.testing_transient_memory_base_projections(), 1);
    assert_eq!(universe.toks(0), TokenListId::EMPTY);
    assert_eq!(universe.meaning(symbol), Meaning::Undefined);
    // The rejected roots are gone, but their live pre-rollback coordinate is
    // still part of §1334's high-water extent.
    assert_eq!(universe.engine_usage_statistics().memory_words, 1_147);
}

#[test]
fn transient_node_allocations_reuse_roots_and_preserve_the_high_water() {
    let mut universe = Universe::new();
    universe.observe_transient_token_words(0);
    assert_eq!(universe.testing_transient_memory_base_projections(), 1);

    universe.intern_token_list(&vec![
        Token::Char {
            ch: 'x',
            cat: Catcode::Other,
        };
        600
    ]);
    universe.freeze_node_list(&vec![Node::Penalty(0); 501]);
    // The frozen list is a real §§127/157 allocation event, so it advances
    // §1334's low-arena high-water even while no semantic root owns it. The
    // unrelated unowned token history is absent, and neither event rebuilds
    // the live macro/token/box closure.
    assert_eq!(universe.testing_transient_memory_base_projections(), 1);
    assert_eq!(universe.engine_usage_statistics().memory_words, 2_045);
}

#[test]
fn shape_preserving_box_rewrites_rebuild_borrowed_projection_and_preserve_high_water() {
    let mut universe = Universe::new();
    let children = universe.freeze_node_list(&vec![Node::Penalty(0); 494]);
    let root = universe.freeze_node_list(&[Node::HList(BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(1),
        height: Scaled::from_raw(2),
        depth: Scaled::from_raw(3),
        shift: Scaled::from_raw(0),
        box_lr: BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children,
    }))]);
    universe.set_box_reg_ref_global(0, root);
    assert_eq!(universe.engine_usage_statistics().memory_words, 1_045);
    universe.observe_transient_token_words(0);
    assert_eq!(universe.testing_transient_memory_base_projections(), 2);

    universe.set_box_dimension(0, BoxDimension::Width, Scaled::from_raw(4));
    universe.set_box_dimension(0, BoxDimension::Height, Scaled::from_raw(5));
    // Each immutable rewrite allocates a temporary replacement while the old
    // box is live, so §§125--130/1334 still retain the crossed 1,000-word
    // low-arena boundary after rebuilding the borrowed projection once.
    assert_eq!(universe.testing_transient_memory_base_projections(), 3);
    assert_eq!(universe.engine_usage_statistics().memory_words, 2_045);
}

#[test]
fn box_root_changes_rebuild_the_borrowed_allocator_projection() {
    let mut universe = Universe::new();
    let root = universe.freeze_node_list(&[Node::Penalty(1)]);
    universe.set_box_reg_ref_global(0, root);
    universe.observe_transient_token_words(0);
    assert_eq!(universe.testing_transient_memory_base_projections(), 2);

    let replacement = universe.freeze_node_list(&[Node::Penalty(2), Node::Penalty(3)]);
    universe.set_box_reg_ref_global(0, replacement);
    universe.observe_transient_token_words(0);
    // A direct root replacement discards the borrowed diagnostic projection;
    // no independent graph-lifetime registry survives the mutation.
    assert_eq!(universe.testing_transient_memory_base_projections(), 3);

    universe.enter_group();
    let local = universe.freeze_node_list(&[Node::Penalty(5), Node::Penalty(6), Node::Penalty(7)]);
    universe.set_box_reg_ref(0, local);
    universe.observe_transient_token_words(0);
    let _ = universe.leave_group();
    universe.observe_transient_token_words(0);
    assert_eq!(universe.testing_transient_memory_base_projections(), 5);

    let taken = universe.take_box_reg_ref(0).expect("box is present");
    universe.observe_transient_token_words(0);
    assert_eq!(universe.testing_transient_memory_base_projections(), 6);
    universe.set_box_reg_ref_global(0, taken);
    universe.observe_transient_token_words(0);
    assert_eq!(universe.testing_transient_memory_base_projections(), 7);

    let alias = universe.box_reg_ref(0).expect("replacement box");
    universe.set_box_reg_ref_global(1, alias);
    universe.observe_transient_token_words(0);
    assert_eq!(universe.testing_transient_memory_base_projections(), 8);

    let snapshot = universe.snapshot();
    let divergent = universe.freeze_node_list(&[Node::Penalty(4)]);
    universe.set_box_reg_ref_global(0, divergent);
    universe.observe_transient_token_words(0);
    assert_eq!(universe.testing_transient_memory_base_projections(), 9);
    universe.rollback(&snapshot);
    universe.observe_transient_token_words(0);
    // A box restore remains the negative control: the derived projection has
    // no independent graph-lifetime registry, so both replacement and restore
    // rebuild lazily while their Env owners remain authoritative.
    assert_eq!(universe.testing_transient_memory_base_projections(), 10);
}

#[test]
fn box_alias_handoffs_do_not_revisit_unrelated_allocator_roots() {
    let mut universe = Universe::new();
    for index in 0..1_000 {
        let symbol = universe.intern(&format!("unrelated-root-{index}"));
        universe.set_meaning_global(symbol, Meaning::Relax);
    }
    let root = universe.freeze_node_list(&[Node::Penalty(1)]);
    universe.set_box_reg_ref_global(0, root);
    universe.observe_transient_token_words(0);
    assert_eq!(universe.testing_main_memory_root_traversals(), 2);

    let alias = universe.box_reg_ref(0).expect("box root");
    universe.set_box_reg_ref_global(1, alias);
    universe.clear_box_reg_global(0);
    universe.clear_box_reg_global(1);
    universe.observe_transient_token_words(0);

    // TeX82 §§125--130 update the allocator owner at each box handoff.
    // Alias multiplicities belong to that retained projection, so §638's
    // later observation does not rescan unrelated environment roots.
    assert_eq!(universe.testing_main_memory_root_traversals(), 3);
    assert_eq!(universe.testing_transient_memory_base_projections(), 3);

    let snapshot = universe.snapshot();
    universe.freeze_node_list(&[Node::Penalty(2)]);
    let traversals_before_rollback = universe.testing_main_memory_root_traversals();
    universe.rollback(&snapshot);
    universe.observe_transient_token_words(0);
    // An unrooted allocation suffix changes no allocator root. Rollback keeps
    // the base and therefore does not revisit unrelated Env cells.
    assert_eq!(
        universe.testing_main_memory_root_traversals(),
        traversals_before_rollback
    );
}

#[test]
fn refiled_global_box_restore_rebuilds_the_borrowed_allocator_projection() {
    let mut universe = Universe::new();
    let baseline = universe.freeze_node_list(&[Node::Penalty(1)]);
    universe.set_box_reg_ref_global(0, baseline);
    universe.observe_transient_token_words(0);
    assert_eq!(universe.testing_transient_memory_base_projections(), 2);

    universe.enter_group();
    universe.enter_group();
    let local = universe.freeze_node_list(&[Node::Penalty(2), Node::Penalty(3)]);
    universe.set_box_reg_ref(0, local);
    let retained = universe.freeze_node_list(&[Node::Penalty(4)]);
    universe.set_box_reg_ref_global(0, retained);
    let retained = universe
        .box_reg_ref(0)
        .expect("global box remains installed");

    // TeX82 §§275/283 refile the global save into the outer group while the
    // inner local value retires. The first exit invalidates the borrowed
    // allocator projection without reading the refiled record's non-owning old
    // coordinate; the outer exit retains the same direct owner.
    let _ = universe.leave_group();
    universe.observe_transient_token_words(0);
    assert_eq!(universe.testing_transient_memory_base_projections(), 4);
    let _ = universe.leave_group();
    universe.observe_transient_token_words(0);
    assert_eq!(universe.testing_transient_memory_base_projections(), 4);
    assert_eq!(universe.box_reg_ref(0), Some(retained));
}

#[test]
fn string_pool_accounting_keeps_control_sequences_and_typed_allocations_distinct() {
    let mut universe = Universe::new();
    universe.intern("control-name");
    universe.record_string_pool_allocations(3, 17);

    let usage = universe.engine_usage_statistics();
    assert_eq!(usage.strings, 4);
    assert_eq!(usage.string_characters, "control-name".len() + 17);
    assert_eq!(usage.control_sequences, 1);
}

#[test]
fn string_pool_format_baselines_and_capacities_round_trip() {
    let mut source = Universe::new();
    source.intern("format-control");
    source.intern_internal_control_sequence("nullfont");
    source.record_string_pool_allocations(2, 9);
    let image = source.dump_format().expect("format dumps");
    let baseline = source.string_pool_accounting();

    let mut loaded = Universe::from_format(World::default(), &image).expect("format loads");
    let initially_used = loaded.engine_usage_statistics();
    assert_eq!(initially_used.strings, 0);
    assert_eq!(initially_used.string_characters, 0);
    assert_eq!(initially_used.string_capacity, 15_000 - (1_027 + 3));
    assert_eq!(
        initially_used.string_character_capacity,
        125_000 - (106_808 + "format-control".len() + 9)
    );

    loaded.intern("job-control");
    loaded.record_string_pool_allocations(4, 23);
    let used = loaded.engine_usage_statistics();
    assert_eq!(used.strings, 5);
    assert_eq!(used.string_characters, "job-control".len() + 23);
    assert_eq!(source.string_pool_accounting(), baseline);
}

#[test]
fn loaded_string_pool_recycles_components_but_retains_fresh_strings() {
    let mut source = Universe::new();
    source.remember_string_pool_string("etrip");
    source.remember_string_pool_string(".out");
    let image = source.dump_format().expect("string-pool format");
    let mut loaded = Universe::from_format(World::memory(), &image).expect("loaded string pool");
    let baseline = loaded.engine_usage_statistics();

    // Web2C tex.ch [29.517] reuses filename components already present in the
    // format, while a new component is retained exactly once.
    loaded.slow_make_string_pool_string("etrip");
    loaded.slow_make_string_pool_string(".out");
    assert_eq!(loaded.engine_usage_statistics(), baseline);
    loaded.slow_make_string_pool_string("tripos");
    loaded.slow_make_string_pool_string("tripos");
    let after_components = loaded.engine_usage_statistics();
    assert_eq!(after_components.strings, 1);
    assert_eq!(after_components.string_characters, "tripos".len());

    // TeX82 §§525/532/536/537 retain every made output or opened-input name.
    loaded.make_string_pool_string("etrip.out");
    loaded.make_string_pool_string("etrip.out");
    let after_names = loaded.engine_usage_statistics();
    assert_eq!(after_names.strings, 3);
    assert_eq!(
        after_names.string_characters,
        "tripos".len() + 2 * "etrip.out".len()
    );

    // §§341/372's direct one-character namespace does not call make_string;
    // §934 retains one word-and-language string, and tex.ch [42.941] flushes
    // that just-made string when the same exception is replaced.
    loaded.intern("3");
    loaded.add_hyphenation_exception_for_language(
        7,
        ExceptionSpec {
            word: "hyphen".to_owned(),
            positions: vec![2],
        },
    );
    loaded.add_hyphenation_exception_for_language(
        7,
        ExceptionSpec {
            word: "hyphen".to_owned(),
            positions: vec![3],
        },
    );
    let final_usage = loaded.engine_usage_statistics();
    assert_eq!(final_usage.strings, 4);
    assert_eq!(
        final_usage.string_characters,
        "tripos".len() + 2 * "etrip.out".len() + "hyphen".len() + 1
    );
}

#[test]
fn string_pool_ownership_distinguishes_physical_and_fixed_names() {
    let image = Universe::new().dump_format().expect("empty format");
    let mut loaded = Universe::from_format(World::memory(), &image).expect("loaded string pool");
    let baseline = loaded.engine_usage_statistics();

    // TeX82 §1252 calls `make_string` for every active/null font identifier,
    // even when two physical strings have the same spelling.
    let first = loaded.intern_retained_pool_string("FONT?");
    let second = loaded.intern_retained_pool_string("FONT?");
    assert_eq!(first, second);
    let retained = loaded.engine_usage_statistics();
    assert_eq!(retained.control_sequences, baseline.control_sequences);
    assert_eq!(retained.strings - baseline.strings, 2);
    assert_eq!(retained.string_characters - baseline.string_characters, 10);

    // TeX82 §1215's `inaccessible` slot and §§341/372's one-character
    // namespace are fixed eqtb identities, not string-pool allocations. A
    // multi-character hash spelling remains the negative control.
    loaded.intern_internal_control_sequence("inaccessible");
    loaded.intern("»");
    assert_eq!(loaded.engine_usage_statistics(), retained);
    loaded.intern("ab");
    let hashed = loaded.engine_usage_statistics();
    assert_eq!(hashed.strings - retained.strings, 1);
    assert_eq!(hashed.string_characters - retained.string_characters, 2);
}

#[test]
fn etex_string_pool_profile_selects_the_static_web_vocabulary_once() {
    let mut universe = Universe::new();
    let before = universe.engine_usage_statistics();
    universe.select_string_pool_profile(crate::StringPoolProfile::Etex26);
    let selected = universe.engine_usage_statistics();

    assert_eq!(selected.strings, before.strings);
    assert_eq!(selected.string_characters, before.string_characters);
    assert_eq!(before.string_capacity - selected.string_capacity, 52);
    assert_eq!(
        before.string_character_capacity - selected.string_character_capacity,
        882
    );

    universe.select_string_pool_profile(crate::StringPoolProfile::Etex26);
    assert_eq!(universe.engine_usage_statistics(), selected);
}

#[test]
fn memory_usage_counts_reachable_lists_without_immutable_store_history() {
    let mut source = Universe::new();
    let format_tokens = source.intern_token_list(&[
        Token::Char {
            ch: 'f',
            cat: crate::token::Catcode::Other,
        },
        Token::Char {
            ch: 'm',
            cat: crate::token::Catcode::Other,
        },
        Token::Char {
            ch: 't',
            cat: crate::token::Catcode::Other,
        },
    ]);
    source.set_toks_global(0, format_tokens);
    let image = source.dump_format().expect("format dumps");
    let mut loaded = Universe::from_format(World::default(), &image).expect("format loads");
    let baseline = loaded.engine_usage_statistics().memory_words;

    let job_tokens = loaded.intern_token_list(&[
        Token::Char {
            ch: 'j',
            cat: crate::token::Catcode::Other,
        },
        Token::Char {
            ch: 'o',
            cat: crate::token::Catcode::Other,
        },
        Token::Char {
            ch: 'b',
            cat: crate::token::Catcode::Other,
        },
    ]);
    loaded.intern_token_list(&[
        Token::Char {
            ch: 'r',
            cat: crate::token::Catcode::Other,
        },
        Token::Char {
            ch: 'u',
            cat: crate::token::Catcode::Other,
        },
        Token::Char {
            ch: 'n',
            cat: crate::token::Catcode::Other,
        },
    ]);

    // Hash-consed immutable history is not a WEB allocator coordinate. Neither
    // unattached list is live, so it cannot affect §1334's extent.
    assert_eq!(loaded.engine_usage_statistics().memory_words, baseline);

    loaded.set_toks_global(1, job_tokens);
    // The reachable three-token list owns its §200 reference-count head.
    assert_eq!(loaded.engine_usage_statistics().memory_words, baseline + 4);
}

#[test]
fn format_round_trip_preserves_multiletter_hash_accounting() {
    let mut source = Universe::new();
    source.intern("");
    // TeX82 §§356/372 route one-character spellings to fixed `eqtb` slots;
    // §259 owns only the multiletter negative control.
    source.intern_hash_control_sequence("x");
    source.intern_active_character('x');
    source.intern("format-control");
    let image = source.dump_format().expect("format dumps");

    let mut loaded = Universe::from_format(World::default(), &image).expect("format loads");
    loaded.intern_internal_control_sequence("nullfont");
    assert_eq!(loaded.engine_usage_statistics().control_sequences, 1);

    loaded.intern("y");
    loaded.intern_active_character('y');
    loaded.intern("job-control");
    assert_eq!(loaded.engine_usage_statistics().control_sequences, 2);
}

#[test]
fn font_info_accounting_preserves_typed_allocation_and_runtime_growth() {
    let mut universe = Universe::new();
    assert_eq!(universe.engine_usage_statistics().font_info_words, 7);

    let font = test_font("arena-font", b"arena").with_font_info_words(100);
    let id = universe.intern_font(font);
    assert_eq!(universe.engine_usage_statistics().font_info_words, 107);

    universe
        .set_font_dimen(id, 10, Scaled::from_raw(42))
        .expect("last loaded font may grow");
    assert_eq!(universe.engine_usage_statistics().font_info_words, 110);

    let image = universe.dump_format().expect("format dumps");
    let mut restored = Universe::from_format(World::default(), &image).expect("format loads");
    assert_eq!(restored.engine_usage_statistics().font_info_words, 110);
}

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
fn pdf_match_capture_state_is_not_serialized_into_formats() {
    let mut universe = Universe::new();
    universe.set_pdf_match_state(b"session".to_vec(), vec![Some((0, 7))], 1, true);
    let format = universe.dump_format().expect("dump format");
    let restored = Universe::from_format(World::memory(), &format).expect("restore format");
    assert_eq!(restored.pdf_match_capture(0), None);
}

#[test]
fn page_insertion_heights_are_checkpointed_live_state_and_forbid_format_dump() {
    let mut universe = Universe::new();
    let baseline = universe.snapshot();
    universe.upsert_page_insertion(crate::page::PageInsertion::new(
        254,
        Scaled::from_raw(12 * Scaled::UNITY),
    ));
    assert_eq!(
        universe.page_insertion_height(254),
        Some(Scaled::from_raw(12 * Scaled::UNITY))
    );
    assert_ne!(universe.snapshot().state_hash(), baseline.state_hash());
    assert_eq!(universe.dump_format(), Err(FormatError::NonEmptyPage));
    universe.rollback(&baseline);
    assert_eq!(universe.page_insertion_height(254), None);
    assert_eq!(universe.snapshot().state_hash(), baseline.state_hash());
}

#[test]
fn ignore_primitive_error_parameter_rolls_back_with_the_environment() {
    let mut universe = Universe::new();
    let baseline = universe.snapshot();
    universe.set_int_param(IntParam::IGNORE_PRIMITIVE_ERROR, 3);
    assert_eq!(universe.int_param(IntParam::IGNORE_PRIMITIVE_ERROR), 3);
    assert_ne!(universe.snapshot().state_hash(), baseline.state_hash());
    universe.rollback(&baseline);
    assert_eq!(universe.int_param(IntParam::IGNORE_PRIMITIVE_ERROR), 0);
    assert_eq!(universe.snapshot().state_hash(), baseline.state_hash());
}

#[test]
fn restore_tracing_preserves_save_stack_order_for_extended_register_banks() {
    // TeX82 §§252/283 pop ordinary entries in LIFO order; e-TeX
    // [49.1221--1224] drains extended registers at their shared sparse-array
    // save-stack boundary.
    let mut universe = Universe::new();
    universe.set_int_param(IntParam::TRACING_RESTORES, 1);
    universe.set_int_param(IntParam::TRACING_ONLINE, 1);
    universe.set_int_param(IntParam::ESCAPE_CHAR, i32::from(b'\\'));
    let glue = universe.intern_glue(GlueSpec {
        width: Scaled::from_raw(5 * Scaled::UNITY),
        ..GlueSpec::ZERO
    });
    let toks = universe.intern_token_list(&[Token::Char {
        ch: 'x',
        cat: Catcode::Other,
    }]);

    universe.enter_group();
    universe.set_count(20, 5);
    universe.set_count(2000, 5);
    universe.set_dimen(21, Scaled::from_raw(5 * Scaled::UNITY));
    universe.set_dimen(2100, Scaled::from_raw(5 * Scaled::UNITY));
    universe.set_skip(22, &glue);
    universe.set_muskip(2200, glue);
    universe.set_toks(32767, toks);
    let _ = universe.leave_group();

    let output: String = universe
        .world()
        .effect_records()
        .iter()
        .filter_map(|effect| match effect {
            EffectRecord::StreamWrite { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    let Some(skip_pos) = output.find("{restoring \\skip22=0.0pt}") else {
        panic!("missing skip restore in {output:?}");
    };
    let Some(dimen21_pos) = output.find("{restoring \\dimen21=0.0pt}") else {
        panic!("missing dimen21 restore in {output:?}");
    };
    let toks_pos = output
        .find("{restoring \\toks32767=}")
        .unwrap_or_else(|| panic!("missing toks restore in {output:?}"));
    let Some(muskip_pos) = output.find("{restoring \\muskip2200=0.0mu}") else {
        panic!("missing muskip2200 restore in {output:?}");
    };
    let Some(dimen_pos) = output.find("{restoring \\dimen2100=0.0pt}") else {
        panic!("missing dimen2100 restore in {output:?}");
    };
    let Some(count2000_pos) = output.find("{restoring \\count2000=0}") else {
        panic!("missing count2000 restore in {output:?}");
    };
    let Some(count20_pos) = output.find("{restoring \\count20=0}") else {
        panic!("missing count20 restore in {output:?}");
    };
    assert!(
        skip_pos < dimen21_pos
            && dimen21_pos < toks_pos
            && toks_pos < muskip_pos
            && muskip_pos < dimen_pos
            && dimen_pos < count2000_pos
            && count2000_pos < count20_pos
    );
}

#[test]
fn restore_tracing_interleaves_code_tables_with_eqtb_in_save_stack_order() {
    // TeX82 §§252/283: code-table entries and ordinary eqtb entries occupy
    // one save stack, so `unsave` reports them in one strict LIFO sequence.
    fn traced_universe() -> Universe {
        let mut universe = Universe::new();
        universe.set_int_param(IntParam::TRACING_RESTORES, 1);
        universe.set_int_param(IntParam::TRACING_ONLINE, 1);
        universe.set_int_param(IntParam::ESCAPE_CHAR, i32::from(b'\\'));
        universe
    }

    fn restore_output(universe: &Universe) -> String {
        universe
            .world()
            .effect_records()
            .iter()
            .filter_map(|effect| match effect {
                EffectRecord::StreamWrite { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    let mut interleaved = traced_universe();
    interleaved.enter_group();
    interleaved.set_count(1, 11);
    interleaved.set_catcode('J', Catcode::Active);
    interleaved.set_catcode('j', Catcode::Active);
    interleaved.set_count(2, 22);
    let _ = interleaved.leave_group();
    assert_eq!(interleaved.count(1), 0);
    assert_eq!(interleaved.count(2), 0);
    assert_eq!(interleaved.catcode('J'), Catcode::Letter);
    assert_eq!(interleaved.catcode('j'), Catcode::Letter);
    let output = restore_output(&interleaved);
    let count2 = output
        .find("{restoring \\count2=0}")
        .expect("count2 restore");
    let cat_j = output
        .find("{restoring \\catcode106=11}")
        .expect("lowercase j catcode restore");
    let cat_upper_j = output
        .find("{restoring \\catcode74=11}")
        .expect("uppercase J catcode restore");
    let count1 = output
        .find("{restoring \\count1=0}")
        .expect("count1 restore");
    assert!(count2 < cat_j && cat_j < cat_upper_j && cat_upper_j < count1);

    // Negative control: without intervening code-table saves, ordinary eqtb
    // restoration remains the same LIFO sequence.
    let mut ordinary = traced_universe();
    ordinary.enter_group();
    ordinary.set_count(1, 11);
    ordinary.set_count(2, 22);
    let _ = ordinary.leave_group();
    let output = restore_output(&ordinary);
    assert!(
        output
            .find("{restoring \\count2=0}")
            .expect("negative-control count2 restore")
            < output
                .find("{restoring \\count1=0}")
                .expect("negative-control count1 restore")
    );
    assert!(!output.contains("catcode"));
}

fn assert_parshape_restore_trace(mut universe: Universe) {
    universe.set_int_param(IntParam::TRACING_RESTORES, 1);
    universe.set_int_param(IntParam::TRACING_ONLINE, 1);
    universe.set_int_param(IntParam::ESCAPE_CHAR, i32::from(b'\\'));
    assert_eq!(universe.paragraph_shape_len(), 2);

    universe.enter_group();
    universe.set_count(20, 5);
    universe.set_paragraph_shape(&[], false);
    let _ = universe.leave_group();

    assert_eq!(universe.paragraph_shape_len(), 2);
    let output: String = universe
        .world()
        .effect_records()
        .iter()
        .filter_map(|effect| match effect {
            EffectRecord::StreamWrite { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    let parshape_pos = output
        .find("{restoring \\parshape=2}")
        .unwrap_or_else(|| panic!("missing parshape restore in {output:?}"));
    let count_pos = output
        .find("{restoring \\count20=0}")
        .unwrap_or_else(|| panic!("missing count restore in {output:?}"));
    assert!(
        parshape_pos < count_pos,
        "restore order changed: {output:?}"
    );
}

#[test]
fn parshape_restore_trace_resolves_live_token_list_handle() {
    let mut universe = Universe::new();
    universe.set_paragraph_shape(
        &[
            super::ParagraphShapeLine {
                indent: Scaled::from_raw(Scaled::UNITY),
                width: Scaled::from_raw(10 * Scaled::UNITY),
            },
            super::ParagraphShapeLine {
                indent: Scaled::from_raw(2 * Scaled::UNITY),
                width: Scaled::from_raw(9 * Scaled::UNITY),
            },
        ],
        true,
    );
    assert_parshape_restore_trace(universe);
}

#[test]
fn parshape_restore_trace_resolves_loaded_format_token_list_handle() {
    let mut initex = Universe::new();
    initex.set_paragraph_shape(
        &[
            super::ParagraphShapeLine {
                indent: Scaled::from_raw(Scaled::UNITY),
                width: Scaled::from_raw(10 * Scaled::UNITY),
            },
            super::ParagraphShapeLine {
                indent: Scaled::from_raw(2 * Scaled::UNITY),
                width: Scaled::from_raw(9 * Scaled::UNITY),
            },
        ],
        true,
    );
    let format = initex.dump_format().expect("parshape format serializes");
    let loaded = Universe::from_format(World::memory(), &format).expect("parshape format loads");

    assert_parshape_restore_trace(loaded);
}

#[test]
fn parshape_restore_trace_materializes_loaded_format_base_after_zero_overlay() {
    let mut initex = Universe::new();
    initex.set_paragraph_shape(
        &[super::ParagraphShapeLine {
            indent: Scaled::from_raw(Scaled::UNITY),
            width: Scaled::from_raw(10 * Scaled::UNITY),
        }; 10],
        true,
    );
    let format = initex.dump_format().expect("parshape format serializes");
    let mut loaded =
        Universe::from_format(World::memory(), &format).expect("parshape format loads");
    let cell = crate::cell::CellId::new(
        crate::cell::BankTag::TokParam,
        u32::from(TokParam::PAR_SHAPE_INTERNAL.raw()),
    );
    assert!(
        loaded
            .stores
            .env()
            .testing_format_base()
            .iter()
            .any(|entry| entry.cell == cell && entry.word != 0),
        "fixture requires a serialized parshape in the immutable format base"
    );

    // Schema-11's loaded-format journal can represent restoration by deleting
    // a mutable overlay (word zero), exposing the immutable base entry. Build
    // that exact journal shape instead of assigning the frozen payload into
    // the live token-list arena before the group starts.
    loaded.stores.testing_restore_env_word(cell, 0);
    loaded.set_int_param(IntParam::TRACING_RESTORES, 1);
    loaded.set_int_param(IntParam::TRACING_ONLINE, 1);
    loaded.set_int_param(IntParam::ESCAPE_CHAR, i32::from(b'\\'));
    loaded.enter_group();
    loaded.set_paragraph_shape(&[], false);
    let _ = loaded.leave_group();

    let output: String = loaded
        .world()
        .effect_records()
        .iter()
        .filter_map(|effect| match effect {
            EffectRecord::StreamWrite { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        output.contains("{restoring \\parshape=10}"),
        "loaded format base was not materialized for restore tracing: {output:?}"
    );
}

#[test]
fn absent_internal_parshape_restore_is_not_traced() {
    let mut universe = Universe::new();
    universe.set_int_param(IntParam::TRACING_RESTORES, 1);
    universe.set_int_param(IntParam::TRACING_ONLINE, 1);
    universe.set_int_param(IntParam::ESCAPE_CHAR, i32::from(b'\\'));
    assert_eq!(universe.paragraph_shape_len(), 0);

    universe.enter_group();
    universe.set_paragraph_shape(&[], false);
    let _ = universe.leave_group();

    let output: String = universe
        .world()
        .effect_records()
        .iter()
        .filter_map(|effect| match effect {
            EffectRecord::StreamWrite { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        !output.contains("{restoring \\parshape=0}"),
        "absent implementation backing cell acquired a TeX restore trace: {output:?}"
    );
}

#[test]
fn exact_environment_identity_tracks_null_parshape_representation_rewrite() {
    let mut universe = Universe::new();
    let baseline = universe.stores.exact_env_identity();
    assert_eq!(
        baseline,
        universe.stores.testing_recomputed_exact_env_identity()
    );

    // Paragraph completion canonically clears `par_shape_ptr`. The already
    // null case still materializes Umber's private empty-list representation
    // without a TeX save-stack word, so the aggregate raw-restore seam must
    // update the exact Env accumulator and retain only snapshot-rollback state.
    universe.set_paragraph_shape(&[], false);
    assert_eq!(universe.paragraph_shape_len(), 0);
    assert_ne!(universe.stores.exact_env_identity(), baseline);
    assert_eq!(
        universe.stores.exact_env_identity(),
        universe.stores.testing_recomputed_exact_env_identity()
    );

    universe.set_paragraph_shape(
        &[super::ParagraphShapeLine {
            indent: Scaled::from_raw(Scaled::UNITY),
            width: Scaled::from_raw(10 * Scaled::UNITY),
        }],
        false,
    );
    let _ = universe.snapshot_with_exact_identity();
    universe.set_paragraph_shape(&[], false);
    let _ = universe.snapshot_with_exact_identity();
    assert_eq!(universe.paragraph_shape_len(), 0);
    assert_eq!(
        universe.stores.exact_env_identity(),
        universe.stores.testing_recomputed_exact_env_identity(),
        "nonnull-to-null paragraph completion must converge through the journaled path"
    );
}

#[test]
fn exact_environment_identity_tracks_loaded_format_null_parshape_rewrite() {
    let initex = Universe::new();
    let format = initex.dump_format().expect("empty format serializes");
    let mut loaded = Universe::from_format(World::memory(), &format).expect("format loads");
    assert_eq!(
        loaded.stores.exact_env_identity(),
        loaded.stores.testing_recomputed_exact_env_identity()
    );

    loaded.set_paragraph_shape(&[], false);
    assert_eq!(loaded.paragraph_shape_len(), 0);
    assert_eq!(
        loaded.stores.exact_env_identity(),
        loaded.stores.testing_recomputed_exact_env_identity()
    );
}

#[test]
fn format_round_trip_preserves_profile_state_but_not_pending_transients() {
    // TeX82 §§1299--1329 serialize the semantic tables, not the live job's
    // input, page-building, diagnostic, or host-effect machinery.  e-TeX's
    // change at §1307 additionally clears `TeXXeTstate`, while retaining the
    // neighboring extended profile parameters.
    let mut universe = Universe::new_with_plain_catcodes();
    universe.set_count(17, 68);
    universe.set_int_param(IntParam::SAVING_V_DISCARDS, 2);
    universe.set_int_param(IntParam::TEX_XET_STATE, 1);
    let every_job = universe.intern_token_list(&[Token::Char {
        ch: 'J',
        cat: Catcode::Other,
    }]);
    universe.set_tok_param(TokParam::EVERY_JOB, every_job);

    assert_eq!(universe.take_pending_every_job(), TokenListId::EMPTY);
    universe.set_current_input_line(1299);
    universe.set_pack_begin_line(1307);
    universe.set_output_routine_active(true);
    universe
        .world_mut()
        .write_text(PrintSink::Terminal, "pending-initex-output");

    let bytes = universe.dump_format().expect("semantic format serializes");
    let mut restored = Universe::from_format(World::memory(), &bytes)
        .expect("semantic format restores into a fresh job");

    assert_eq!(restored.count(17), 68);
    assert_eq!(restored.int_param(IntParam::SAVING_V_DISCARDS), 2);
    assert_eq!(restored.int_param(IntParam::TEX_XET_STATE), 0);
    let restored_every_job = restored.take_pending_every_job();
    assert_eq!(
        restored.tokens(restored_every_job).tokens(),
        universe.tokens(every_job).tokens()
    );
    assert_eq!(restored.take_pending_every_job(), TokenListId::EMPTY);
    assert_eq!(restored.current_input_line(), 0);
    assert_eq!(restored.pack_begin_line(), 0);
    assert!(!restored.output_routine_is_active());
    assert!(restored.world().effect_records().is_empty());
}

#[test]
fn initial_code_tables_survive_format_round_trip() {
    let dumped_clock = JobClock {
        time: 61,
        second: 2,
        day: 3,
        month: 4,
        year: 2005,
    };
    let loaded_clock = JobClock {
        time: 126,
        second: 7,
        day: 8,
        month: 9,
        year: 2010,
    };
    let initial = Universe::with_world(World::memory_with_clock(dumped_clock));
    let image = initial.dump_format().expect("INITEX format serializes");
    let restored = Universe::from_format(World::memory_with_clock(loaded_clock), &image)
        .expect("INITEX format restores");

    // tex.web §§232 and 240 make the code tables portable format state.
    for code in 0_u8..=u8::MAX {
        let ch = char::from(code);
        assert_eq!(restored.catcode(ch), initial.catcode(ch), "catcode {code}");
        assert_eq!(
            restored.mathcode(ch),
            initial.mathcode(ch),
            "mathcode {code}"
        );
        assert_eq!(restored.lccode(ch), initial.lccode(ch), "lccode {code}");
        assert_eq!(restored.uccode(ch), initial.uccode(ch), "uccode {code}");
        assert_eq!(restored.sfcode(ch), initial.sfcode(ch), "sfcode {code}");
        assert_eq!(restored.delcode(ch), initial.delcode(ch), "delcode {code}");
    }
    for raw in 0..crate::env::banks::PARAMETER_COUNT as u16 {
        let param = IntParam::new(raw);
        if [
            IntParam::TIME,
            IntParam::DAY,
            IntParam::MONTH,
            IntParam::YEAR,
        ]
        .contains(&param)
        {
            continue;
        }
        assert_eq!(
            restored.int_param(param),
            initial.int_param(param),
            "integer parameter {raw}"
        );
    }
    for register in 0..=u8::MAX {
        assert_eq!(
            restored.count(register.into()),
            initial.count(register.into()),
            "count register {register}"
        );
    }

    // tex.web's `fix_date_and_time` values are job inputs, so loading replaces
    // them with the new World's clock rather than restoring them.
    assert_eq!(restored.int_param(IntParam::TIME), loaded_clock.time);
    assert_eq!(restored.int_param(IntParam::DAY), loaded_clock.day);
    assert_eq!(restored.int_param(IntParam::MONTH), loaded_clock.month);
    assert_eq!(restored.int_param(IntParam::YEAR), loaded_clock.year);
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
fn font_info_capacity_boundary_is_grouped_rollback_safe_and_format_stable() {
    // `nullfont` permanently owns seven words, leaving this many shared
    // `font_info` words for the only loaded font in this test.
    const LAST_PARAMETER: u32 = (FONT_INFO_CAPACITY - 7) as u32;
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
    let baseline_hash = universe.snapshot().state_hash();

    universe.enter_group();
    universe
        .set_font_dimen(font, LAST_PARAMETER, Scaled::from_raw(22))
        .expect("last shared font-info word is writable");
    assert_eq!(
        universe.font_dimen(font, LAST_PARAMETER),
        Scaled::from_raw(22)
    );
    assert_ne!(universe.snapshot().state_hash(), baseline_hash);
    assert!(universe.leave_group().is_empty());
    assert_eq!(
        universe.font_dimen(font, LAST_PARAMETER),
        Scaled::from_raw(22)
    );
    let grouped_write_hash = universe.snapshot().state_hash();
    assert_ne!(grouped_write_hash, baseline_hash);

    let invalid = universe
        .set_font_dimen(font, LAST_PARAMETER + 1, Scaled::from_raw(99))
        .expect_err("fontdimen above the shared capacity is rejected");
    assert!(matches!(
        invalid,
        super::FontParameterError::FontInfoCapacity { .. }
    ));
    assert_eq!(
        universe.font_dimen(font, LAST_PARAMETER + 1),
        Scaled::from_raw(0)
    );
    assert_eq!(universe.font_dimen(font, 1), Scaled::from_raw(11));
    assert_eq!(universe.snapshot().state_hash(), grouped_write_hash);

    universe.rollback(&baseline);
    assert_eq!(
        universe.font_dimen(font, LAST_PARAMETER),
        Scaled::from_raw(0)
    );
    assert_eq!(universe.snapshot().state_hash(), baseline_hash);

    universe.enter_group();
    universe
        .set_font_dimen(font, LAST_PARAMETER, Scaled::from_raw(33))
        .expect("global capacity-boundary fontdimen is writable");
    assert!(universe.leave_group().is_empty());
    assert_eq!(
        universe.font_dimen(font, LAST_PARAMETER),
        Scaled::from_raw(33)
    );
    universe.rollback(&baseline);
    assert_eq!(
        universe.font_dimen(font, LAST_PARAMETER),
        Scaled::from_raw(0)
    );
    assert_eq!(universe.snapshot().state_hash(), baseline_hash);
    assert_eq!(universe.snapshot().state_hash(), baseline_snapshot_hash);

    universe
        .set_font_dimen(font, LAST_PARAMETER, Scaled::from_raw(44))
        .expect("capacity-boundary fontdimen is format-visible");
    let bytes = universe.dump_format().expect("boundary format encodes");
    let mut restored =
        Universe::from_format(World::memory(), &bytes).expect("boundary format restores");
    let restored_identifier = restored.intern("boundaryfont");
    let Meaning::Font(restored_font) = restored.meaning(restored_identifier) else {
        panic!("restored font identifier meaning");
    };
    assert_eq!(
        restored.font_dimen(restored_font, LAST_PARAMETER),
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
        .set_font_dimen(restored_font, LAST_PARAMETER, Scaled::from_raw(55))
        .expect("restored capacity-boundary fontdimen remains writable");
    restored.rollback(&restored_snapshot);
    assert_eq!(restored.snapshot().state_hash(), restored_hash);
}

#[test]
fn web2c_font_memory_configuration_accepts_large_pdftex_fontdimen_banks() {
    let mut universe = Universe::new();
    let font = universe.intern_font(test_font("web2c-font", b"web2c-font"));
    let error = universe
        .set_font_dimen(font, 65_536, Scaled::from_raw(1))
        .expect_err("TeX82's compiled font-memory default is smaller");
    assert_eq!(
        error,
        super::FontParameterError::FontInfoCapacity {
            capacity: FONT_INFO_CAPACITY,
        }
    );

    universe.configure_font_info_capacity(WEB2C_FONT_INFO_CAPACITY);
    universe
        .set_font_dimen(font, 65_536, Scaled::from_raw(1))
        .expect("the pinned Web2C configuration admits expl3's intarray bank");
    assert_eq!(universe.font_dimen(font, 65_536), Scaled::from_raw(1));
}

#[test]
fn oversized_immutable_font_parameter_table_is_rejected_before_publication() {
    let mut universe = Universe::new();
    let before = universe.snapshot().state_hash();
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
    assert_eq!(universe.snapshot().state_hash(), before);
}

#[test]
fn universe_is_send() {
    fn assert_send<T: Send>() {}

    assert_send::<Universe>();
}

#[test]
fn traced_list_finish_uses_fresh_runtime_coordinates_with_equal_semantics() {
    let mut universe = Universe::new();
    let symbol = universe.intern("traced-list-cs");
    let first_origin = universe.synthetic_origin_ref(SyntheticOriginKind::Test);
    let second_origin = universe.synthetic_origin_ref(SyntheticOriginKind::Engine);
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
    let first = RootedTracedTokenBuffer::new(
        tokens
            .iter()
            .copied()
            .map(|token| RootedTracedTokenWord::new(token, first_origin.clone())),
    );
    let second = RootedTracedTokenBuffer::new(
        tokens
            .iter()
            .copied()
            .map(|token| RootedTracedTokenWord::new(token, second_origin.clone())),
    );

    let bulk = universe.intern_token_list(&tokens);
    let first_list = universe.finish_rooted_traced_token_list(&first);
    let second_list = universe.finish_rooted_traced_token_list(&second);

    assert_ne!(first_list.token_list(), bulk);
    assert_ne!(second_list.token_list(), bulk);
    assert_ne!(first_list.token_list(), second_list.token_list());
    let semantic = universe.stores.testing_token_semantic_id(bulk);
    assert_eq!(
        universe
            .stores
            .testing_token_semantic_id(first_list.token_list()),
        semantic
    );
    assert_eq!(
        universe
            .stores
            .testing_token_semantic_id(second_list.token_list()),
        semantic
    );
    assert_ne!(first_list.origin_list(), second_list.origin_list());
    assert_eq!(universe.tokens(first_list.token_list()), tokens);
    assert_eq!(
        first_list.origin_ref().origins(),
        vec![first_origin.id(); tokens.len()]
    );
    assert_eq!(
        second_list.origin_ref().origins(),
        vec![second_origin.id(); tokens.len()]
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
fn detached_format_strips_structural_origin_lists_without_retaining_runtime_ids() {
    let mut source = Universe::new();
    let name = source.intern("format-provenance-negative");
    let body = source.intern_token_list(&[Token::Char {
        ch: 'x',
        cat: Catcode::Letter,
    }]);
    let definition = source.intern_macro(MacroMeaning::new(
        MeaningFlags::EMPTY,
        TokenListId::EMPTY,
        body,
    ));
    source.set_meaning(
        name,
        Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition: definition.id(),
        },
    );
    let origin = source.synthetic_origin_ref(SyntheticOriginKind::Format);
    let repeated: [crate::provenance::OriginRef; 32] = std::array::from_fn(|_| origin.clone());
    let replacement = source.allocate_origin_list_ref(&repeated);
    source.set_macro_definition_provenance(
        definition.id(),
        MacroDefinitionProvenance::new(
            origin,
            crate::provenance::OriginListRef::empty(),
            replacement,
        ),
    );

    let image = source
        .dump_format()
        .expect("format with live provenance dumps");
    let loaded = Universe::from_format(World::memory(), &image).expect("detached format loads");
    let restored_name = loaded
        .symbol("format-provenance-negative")
        .expect("macro name is restored");
    let Meaning::Macro {
        definition: restored,
        ..
    } = loaded.meaning(restored_name)
    else {
        panic!("restored meaning is a macro");
    };
    assert_eq!(
        loaded.macro_definition_provenance(restored),
        MacroDefinitionProvenance::unknown()
    );
    assert_eq!(loaded.provenance_stats().origin_records(), 0);
    assert_eq!(loaded.provenance_stats().origin_list_entries(), 0);
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
        box_lr: crate::node::BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: child,
    }))]);
    universe.set_box_reg_ref(7, root);
    let semantic_id = universe
        .box_reg_ref(7)
        .expect("promoted box register")
        .semantic_id();

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
    let restored_root = restored.box_reg_ref(7).expect("restored box register");
    assert_eq!(restored_root.semantic_id(), semantic_id);
    let restored_nodes = restored_root.to_vec();
    let Node::HList(ref restored_box) = restored_nodes[0] else {
        panic!("restored box node kind");
    };
    assert_eq!(
        restored_box.children.to_vec(),
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

/// TeX82 §§681, 683, 688--689 (`tex.web:13366-13426`,
/// `tex.web:13457-13497`, `tex.web:13551-13599`) define the child-bearing
/// math records that a format must retain without collapsing field variants.
#[test]
fn format_roundtrip_complete_math_graph() {
    let mut universe = Universe::new();
    let leaf = freeze_ref(&mut universe, &[Node::Penalty(17)]);
    let empty = NodeListRef::empty();
    let math_char = |character| MathChar {
        family: 3,
        character,
        origin: OriginId::UNKNOWN,
    };
    let noad = MathNoad {
        kind: NoadKind::Accent {
            accent: math_char('^'),
        },
        nucleus: MathField::SubMlist(empty.clone()),
        subscript: MathField::MathTextChar(math_char('t')),
        superscript: MathField::SubBox(leaf),
    };
    let display = freeze_ref(&mut universe, &[Node::MathNoad(noad.clone())]);
    let text = freeze_ref(
        &mut universe,
        &[Node::MathNoad(MathNoad::new(
            NoadKind::Normal(NoadClass::Ord),
            MathField::Empty,
        ))],
    );
    let script = freeze_ref(
        &mut universe,
        &[Node::MathNoad(MathNoad::new(
            NoadKind::Normal(NoadClass::Bin),
            MathField::MathChar(math_char('+')),
        ))],
    );
    let script_script = freeze_ref(&mut universe, &[Node::MathStyle(MathStyle::ScriptScript)]);
    let nested = freeze_ref(
        &mut universe,
        &[Node::MathChoice(MathChoice {
            display,
            text,
            script,
            script_script,
        })],
    );
    let root = universe.freeze_node_list(&[
        Node::MathOn(Scaled::from_raw(1)),
        Node::MathNoad(noad),
        Node::FractionNoad(MathFraction {
            numerator: nested.clone(),
            denominator: empty,
            thickness: FractionThickness::Explicit(Scaled::from_raw(2)),
            left_delimiter: Some(0x12345),
            right_delimiter: Some(0x23456),
        }),
        Node::Nonscript,
        Node::MathStyle(MathStyle::Display),
        Node::MathList(MathListNode {
            display: true,
            content: nested,
        }),
        Node::MathOff(Scaled::from_raw(3)),
    ]);
    universe.set_box_reg_ref(23, root);
    let expected_semantic_id = universe
        .box_reg_ref(23)
        .expect("promoted math graph")
        .semantic_id();
    let image = universe.dump_format().expect("complete math graph encodes");

    let restored =
        Universe::from_format(World::memory(), &image).expect("complete math graph restores");
    let restored_root = restored.box_reg_ref(23).expect("math graph root restores");
    assert_eq!(restored_root.semantic_id(), expected_semantic_id);
    assert_eq!(restored_root.len(), 7);
    assert_eq!(
        restored.dump_format().expect("complete math graph redumps"),
        image
    );
}

#[test]
fn pdftex_margin_kern_query_owns_the_complete_skipable_edge_rule() {
    fn hbox(universe: &mut Universe, children: Vec<Node>) -> NodeListRef {
        let children = universe.freeze_node_list(&children);
        universe.freeze_node_list(&[Node::HList(BoxNode::new(BoxNodeFields {
            width: Scaled::from_raw(0),
            height: Scaled::from_raw(0),
            depth: Scaled::from_raw(0),
            shift: Scaled::from_raw(0),
            box_lr: BoxLr::Normal,
            glue_set: GlueSetRatio::ZERO,
            glue_sign: Sign::Normal,
            glue_order: Order::Normal,
            children,
        }))])
    }

    // pdftex.web §470's `cp_skipable` predicate is typed list policy: the
    // command layer must not duplicate compact-node classification details.
    let mut universe = Universe::new();
    let empty = NodeListRef::empty();
    let replacement = freeze_ref(&mut universe, &[Node::Penalty(1)]);
    let zero_glue = universe.intern_glue(GlueSpec::ZERO);
    let nonzero_glue = universe.intern_glue(GlueSpec {
        width: Scaled::from_raw(Scaled::UNITY),
        ..GlueSpec::ZERO
    });
    let empty_hlist = Node::HList(BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(0),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
        shift: Scaled::from_raw(0),
        box_lr: BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: empty.clone(),
    }));
    let expected = Scaled::from_raw(-3 * Scaled::UNITY);
    let skipable = vec![
        Node::Ins {
            class: 0,
            size: Scaled::from_raw(0),
            split_top_skip: zero_glue.clone(),
            split_max_depth: Scaled::from_raw(0),
            floating_penalty: 0,
            content: empty.clone(),
        },
        Node::Mark {
            class: 0,
            tokens: universe.token_list_ref(TokenListId::EMPTY),
        },
        Node::Adjust(AdjustNode::ordinary(empty.clone())),
        Node::Penalty(1),
        Node::Whatsit(Whatsit::PdfLiteral {
            mode: PdfLiteralMode::Origin,
            payload: Vec::new(),
        }),
        Node::Disc {
            kind: crate::node::DiscKind::Discretionary,
            pre: empty.clone(),
            post: empty.clone(),
            replace: empty.clone(),
            physical_replace_count: 0,
        },
        Node::MathOn(Scaled::from_raw(0)),
        Node::MathOff(Scaled::from_raw(0)),
        Node::Kern {
            amount: Scaled::from_raw(0),
            kind: KernKind::Explicit,
        },
        Node::Kern {
            amount: Scaled::from_raw(Scaled::UNITY),
            kind: KernKind::Font,
        },
        Node::Kern {
            amount: Scaled::from_raw(Scaled::UNITY),
            kind: KernKind::Auto,
        },
        Node::Glue {
            spec: zero_glue,
            kind: GlueKind::Normal,
            leader: None,
        },
        empty_hlist,
        Node::Glue {
            spec: nonzero_glue.clone(),
            kind: GlueKind::LeftSkip,
            leader: None,
        },
        Node::MarginKern {
            amount: expected,
            side: MarginKernSide::Left,
            font: NULL_FONT,
            ch: b'x',
        },
    ];
    let root = hbox(&mut universe, skipable);
    universe.set_box_reg_ref(0, root);
    assert_eq!(
        universe.box_margin_kern(0, MarginKernSide::Left),
        Some(expected)
    );

    let blockers = [
        Node::Whatsit(Whatsit::PdfRefXImage {
            object: 1,
            width: Scaled::from_raw(0),
            height: Scaled::from_raw(0),
            depth: Scaled::from_raw(0),
        }),
        Node::Disc {
            kind: crate::node::DiscKind::Discretionary,
            pre: empty.clone(),
            post: empty.clone(),
            replace: replacement,
            physical_replace_count: 1,
        },
        Node::MathOn(Scaled::from_raw(1)),
        Node::Kern {
            amount: Scaled::from_raw(1),
            kind: KernKind::Explicit,
        },
        Node::Glue {
            spec: nonzero_glue,
            kind: GlueKind::Normal,
            leader: None,
        },
        Node::HList(BoxNode::new(BoxNodeFields {
            width: Scaled::from_raw(1),
            height: Scaled::from_raw(0),
            depth: Scaled::from_raw(0),
            shift: Scaled::from_raw(0),
            box_lr: BoxLr::Normal,
            glue_set: GlueSetRatio::ZERO,
            glue_sign: Sign::Normal,
            glue_order: Order::Normal,
            children: empty,
        })),
    ];
    for (offset, blocker) in blockers.into_iter().enumerate() {
        let root = hbox(
            &mut universe,
            vec![
                blocker,
                Node::MarginKern {
                    amount: expected,
                    side: MarginKernSide::Left,
                    font: NULL_FONT,
                    ch: b'x',
                },
            ],
        );
        let index = u16::try_from(offset + 1).expect("small test register");
        universe.set_box_reg_ref(index, root);
        assert_eq!(
            universe.box_margin_kern(index, MarginKernSide::Left),
            Some(Scaled::from_raw(0)),
            "blocker {offset} must terminate the edge scan"
        );
    }
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
    universe
        .add_hyphenation_pattern(PatternSpec {
            letters: "alpha".chars().collect(),
            values: vec![0, 0, 1, 0, 0, 0],
        })
        .expect("pattern fits the default trie capacity");
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
fn hyphenation_exception_occupancy_and_capacity_survive_format_replacement() {
    // TeX82 §§934/1334: one-letter words never occupy the table, while a
    // language-qualified replacement updates one occupied entry in place.
    let mut universe = Universe::new();
    universe.set_hyphenation_exception_capacity(659);
    universe.add_hyphenation_exception(ExceptionSpec {
        word: "t".to_owned(),
        positions: Vec::new(),
    });
    universe.add_hyphenation_exception(ExceptionSpec {
        word: "bbbbbb".to_owned(),
        positions: vec![2],
    });
    let image = universe.dump_format().expect("hyphenation usage format");
    let mut loaded = Universe::from_format(World::memory(), &image).expect("loaded usage");
    loaded.add_hyphenation_exception(ExceptionSpec {
        word: "bbbbbb".to_owned(),
        positions: vec![3],
    });

    let usage = loaded.engine_usage_statistics();
    assert_eq!(usage.hyphenation_exceptions, 1);
    assert_eq!(usage.hyphenation_exception_capacity, 659);
    assert_eq!(loaded.hyphenation_exception("bbbbbb"), Some(&[3][..]));
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
            definition: base_macro.id(),
        },
    );
    let base_glue = universe.intern_glue(GlueSpec {
        width: Scaled::from_raw(11),
        stretch: Scaled::from_raw(22),
        stretch_order: Order::Fil,
        shrink: Scaled::from_raw(33),
        shrink_order: Order::Normal,
    });
    universe.set_skip(0, &base_glue);

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

    let mut loaded = Universe::from_format(World::memory(), &image).expect("load frozen core");
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
            definition: added_macro.id(),
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
        loaded.macro_definition(added_macro.id()).replacement_text(),
        added_tokens
    );
    assert_eq!(loaded.glue(added_glue).width, Scaled::from_raw(-7));

    loaded.rollback(&baseline);
    assert!(loaded.symbol("job-local-name").is_none());
    assert_eq!(loaded.dump_format().expect("rollback redump"), image);
}

#[test]
fn frozen_node_graph_restores_and_rejects_corrupt_metadata() {
    let mut universe = Universe::new();
    let child = universe.freeze_node_list(&[Node::Penalty(17)]);
    let root = universe.freeze_node_list(&[Node::Adjust(crate::node::AdjustNode::ordinary(child))]);
    universe.set_box_reg_ref(8, root);
    let image = universe.dump_format().expect("frozen node format");

    let mut loaded = Universe::from_format(World::memory(), &image).expect("load frozen nodes");
    let frozen_root = loaded.box_reg_ref(8).expect("frozen box root");
    let local = loaded.freeze_node_list(&[Node::Adjust(crate::node::AdjustNode::ordinary(
        frozen_root.clone(),
    ))]);
    assert!(matches!(
        local.nodes().first(),
        Some(crate::node_arena::NodeRef::Adjust(adjust))
            if local.resolve(adjust.content).is_some_and(|child| child == frozen_root)
    ));

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
            definition: definition.id(),
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
            definition: definition.id(),
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
    universe.set_box_reg_ref(3, list);
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
fn semantic_format_rejects_live_input_page_and_job_only_pdf_state() {
    let mut with_input = Universe::new();
    with_input.set_input_summary(InputSummary::new(
        vec![InputFrameSummary::TokenList {
            token_list: with_input.token_list_ref(crate::ids::TokenListId::EMPTY),
            origin_list: crate::provenance::OriginListRef::empty(),
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
    with_pdf_object.set_pdf_space_font_name(b"job-only-space-font".to_vec());
    assert_eq!(
        with_pdf_object.dump_format(),
        Err(FormatError::NonEmptyPdfDocument)
    );
}

#[test]
fn pdf_format_resources_round_trip_and_remain_usable() {
    let mut universe = Universe::new();
    universe.enable_pdf_output();
    let body = universe.intern_token_list(&[Token::Char {
        ch: '4',
        cat: Catcode::Other,
    }]);
    let object = universe
        .reserve_pdf_raw_object()
        .expect("reserve raw object");
    universe
        .initialize_pdf_raw_object(object, false, None, false, body, false)
        .expect("initialize raw object");
    universe
        .reference_pdf_raw_object(object.raw())
        .expect("reference raw object");

    let form_nodes = universe.freeze_node_list(&[Node::Penalty(2718)]);
    universe.set_box_reg_ref(0, form_nodes);
    let form_nodes = universe
        .take_box_reg_ref_same_level(0)
        .expect("take form nodes");
    let form_identity = universe.reserve_pdf_form().expect("reserve form");
    universe
        .initialize_pdf_form(
            form_identity,
            form_nodes,
            (
                Scaled::from_raw(11),
                Scaled::from_raw(12),
                Scaled::from_raw(13),
            ),
            Some(body),
            None,
            false,
        )
        .expect("initialize form");

    let image = universe
        .allocate_pdf_external_image(
            crate::PdfExternalImageSource {
                identity: ContentHash::new([7; 32]),
                metadata: crate::PdfExternalImageMetadata::Raster(crate::PdfRasterImageMetadata {
                    format: crate::PdfRasterFormat::Png,
                    width: 2,
                    height: 3,
                    bits_per_component: 8,
                    color_space: crate::PdfRasterColorSpace::Rgb,
                    alpha: false,
                    png_color_type: Some(2),
                }),
                natural_width: Scaled::from_raw(20),
                natural_height: Scaled::from_raw(30),
                bytes: Arc::from([1_u8, 2, 3]),
            },
            crate::PdfExternalImageDimensions {
                width: Scaled::from_raw(40),
                height: Scaled::from_raw(50),
                depth: Scaled::from_raw(0),
            },
            0,
        )
        .expect("allocate image");
    let next_object = universe.pdf_next_object_id();

    let bytes = universe.dump_format().expect("PDF resource format");
    let mut restored =
        Universe::from_format(World::memory(), &bytes).expect("restore PDF resources");
    assert!(restored.pdf_output_enabled());
    assert_eq!(restored.pdf_next_object_id(), next_object);
    let raw = restored
        .pdf_raw_object(object.raw())
        .expect("restored raw object");
    assert!(raw.is_referenced());
    assert_eq!(
        restored.tokens(raw.data().expect("raw payload").data()),
        [Token::Char {
            ch: '4',
            cat: Catcode::Other
        }]
    );
    let form = restored.pdf_form(form_identity.0).expect("restored form");
    assert_eq!(form.width(), Scaled::from_raw(11));
    assert!(matches!(
        form.box_list_ref().nodes().first(),
        Some(crate::node_arena::NodeRef::Penalty(2718))
    ));
    assert_eq!(
        restored
            .pdf_external_image_record(image.id())
            .expect("restored image")
            .bytes(),
        [1, 2, 3]
    );
    restored
        .reference_pdf_raw_object(object.raw())
        .expect("post-load raw reference");
    let later = restored
        .reserve_pdf_raw_object()
        .expect("post-load allocation");
    assert_eq!(later.raw(), next_object);
}

#[test]
fn semantic_format_uses_dto_local_payload_root_keys() {
    fn boxed_universe() -> Universe {
        let mut universe = Universe::new();
        let list = universe.freeze_node_list(&[Node::Penalty(123)]);
        universe.set_box_reg_ref(0, list);
        universe
    }

    let first = boxed_universe();
    let second = boxed_universe();
    assert_ne!(
        first.box_reg_ref(0).expect("first box").id().arena(),
        second.box_reg_ref(0).expect("second box").id().arena()
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
    assert_eq!(restored.dump_format().expect("token format redumps"), bytes);
}

#[test]
fn token_parameter_presence_is_grouped_checkpointed_and_format_stable() {
    // e-TeX 2.6 etex.ch §24.362 distinguishes a null \everyeof pointer from
    // an explicitly assigned empty token list.
    let mut universe = Universe::new();
    let parameter = TokParam::EVERY_EOF;
    assert_eq!(universe.tok_param_option(parameter), None);
    let null_hash = universe.snapshot().state_hash();
    universe.set_tok_param(parameter, TokenListId::EMPTY);
    assert_ne!(
        universe.snapshot().state_hash(),
        null_hash,
        "observation identity must distinguish null from assigned empty"
    );
    universe = Universe::new();

    universe.enter_group();
    universe.set_tok_param(parameter, TokenListId::EMPTY);
    assert_eq!(
        universe.tok_param_option(parameter),
        Some(TokenListId::EMPTY)
    );
    let _ = universe.leave_group();
    assert_eq!(universe.tok_param_option(parameter), None);

    universe.set_tok_param_global(parameter, TokenListId::EMPTY);
    universe.enter_group();
    universe.set_tok_param_option(parameter, None);
    assert_eq!(
        universe.tok_param_option(parameter),
        None,
        "a null pointer is distinct from the outer present-empty pointer"
    );
    let _ = universe.leave_group();
    assert_eq!(
        universe.tok_param_option(parameter),
        Some(TokenListId::EMPTY),
        "TeX82 §§275--283 restore the exact saved token-list pointer"
    );

    universe.enter_group();
    let nonempty = universe.intern_token_list(&[Token::Char {
        ch: 'x',
        cat: Catcode::Letter,
    }]);
    universe.set_tok_param(parameter, nonempty);
    let _ = universe.leave_group();
    assert_eq!(
        universe.tok_param_option(parameter),
        Some(TokenListId::EMPTY)
    );

    let format = universe
        .dump_format()
        .expect("present-empty format encodes");
    let restored =
        Universe::from_format(World::memory(), &format).expect("present-empty format restores");
    assert_eq!(
        restored.tok_param_option(parameter),
        Some(TokenListId::EMPTY)
    );
    assert_eq!(
        restored
            .dump_format()
            .expect("present-empty format redumps"),
        format
    );

    let checkpoint = universe.snapshot();
    universe.set_tok_param(parameter, nonempty);
    universe.rollback(&checkpoint);
    assert_eq!(
        universe.tok_param_option(parameter),
        Some(TokenListId::EMPTY)
    );
}

#[test]
fn checkpoint_hash_schema_receipts_token_presence_and_box_lr_vocabularies() {
    assert_eq!(
        crate::CHECKPOINT_STATE_HASH_SCHEMA_VERSION,
        28,
        "v26 receipts token-cell presence; v27 receipts canonical box_lr identity; v28 receipts ligature left/right boundary hits"
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
    let children = NodeListRef::empty();
    let root = universe.freeze_node_list(&[Node::HList(BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(1),
        height: Scaled::from_raw(2),
        depth: Scaled::from_raw(3),
        shift: Scaled::from_raw(4),
        box_lr: crate::node::BoxLr::Normal,
        glue_set,
        glue_sign: Sign::Stretching,
        glue_order: Order::Normal,
        children,
    }))]);
    universe.set_box_reg_ref(19, root);
    universe.dump_format().expect("format encodes")
}

#[test]
fn format_v11_round_trips_tex_web_box_shift_and_rejects_legacy_v10() {
    let mut universe = Universe::with_world(World::memory());
    let children = NodeListRef::empty();
    let root = universe.freeze_node_list(&[Node::HList(BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(1),
        height: Scaled::from_raw(2),
        depth: Scaled::from_raw(3),
        shift: Scaled::from_raw(-4),
        box_lr: crate::node::BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children,
    }))]);
    universe.set_box_reg_ref(19, root);

    let bytes = universe.dump_format().expect("format encodes");
    assert_eq!(&bytes[8..12], &11_u32.to_le_bytes());
    let restored = Universe::from_format(World::memory(), &bytes).expect("v11 format restores");
    let restored_root = restored.box_reg_ref(19).expect("box register restores");
    let Some(Node::HList(boxed)) = restored_root.get(0) else {
        panic!("box register should contain an hlist");
    };
    assert_eq!(boxed.shift, Scaled::from_raw(-4));

    let mut v10 = bytes;
    v10[8..12].copy_from_slice(&10_u32.to_le_bytes());
    assert!(matches!(
        Universe::from_format(World::memory(), &v10),
        Err(super::FormatError::UnsupportedVersion(10))
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
fn generation_fork_retargets_page_pdf_and_effect_token_roots() {
    let mut universe = Universe::new();
    let root = universe.intern_token_list_ref(&[Token::param(8)]);
    let retained = root.clone();
    let id = root.id();
    universe.set_page_mark(PageMark::Bot, id);
    universe.set_page_mark_class(PageMark::SplitBot, 11, id);
    universe.record_deferred_write(StreamSlot::new(4), id);
    universe.append_pdf_document_fragment(PdfDocumentFragmentKind::Names, id);
    let checkpoint = universe.snapshot();
    drop(root);

    universe.clear_page_mark(PageMark::Bot);
    universe.clear_page_mark_class(PageMark::SplitBot, 11);
    universe
        .world_mut()
        .record_special("suffix", b"discarded".to_vec());
    let substrate = universe.freeze_generation();
    let fork = substrate
        .fork_at(&checkpoint)
        .expect("typed token roots are forkable at the exact checkpoint");
    drop(substrate);

    assert_eq!(fork.page_mark(PageMark::Bot), id);
    assert_eq!(fork.page_mark_class(PageMark::SplitBot, 11), id);
    assert!(
        fork.world()
            .page_effect_prefix()
            .iter()
            .any(|effect| matches!(
                effect,
                EffectRecord::DeferredWrite { tokens, .. } if tokens.tokens() == [Token::param(8)]
            )),
        "{:?}",
        fork.world().page_effect_prefix()
    );
    assert_eq!(
        fork.pdf_document_fragments(PdfDocumentFragmentKind::Names)
            .collect::<Vec<_>>(),
        vec![id]
    );
    assert!(retained.strong_count() > 1);
    drop(fork);
    drop(checkpoint);
    assert_eq!(retained.strong_count(), 1);
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
    let mut first = universe.begin_shipout();
    let effect_pos = first.world().effect_pos();
    let reservation = first.world_mut().reserve_artifact_publication_at(0);
    first
        .commit(
            crate::VerifiedArtifact::new(b"accepted page".to_vec()),
            effect_pos,
            reservation,
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
    let mut scratch = fork.begin_shipout();
    let effect_pos = scratch.world().effect_pos();
    let reservation = scratch.world_mut().reserve_artifact_publication_at(0);
    scratch
        .commit(
            crate::VerifiedArtifact::new(b"scratch page".to_vec()),
            effect_pos,
            reservation,
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
fn artifact_root_resolves_after_scratch_fork_is_dropped_without_import() {
    let mut universe = Universe::new();
    let anchor = universe.snapshot();
    let substrate = universe.freeze_generation();
    let mut fork = substrate.fork_at(&anchor).expect("related fork");
    fork.register_source(
        SourceId::new(0),
        SourceDescriptor::named_generated("scratch.tex", Arc::from(&b"abc"[..])),
    )
    .expect("scratch source registration");
    let source = fork.source_range_origin_ref(SourceId::new(0), 0, 3);
    let derived = fork.synthesized_origin_ref(SynthesizedOriginKind::ValueRendering, source);
    drop(fork);

    assert_eq!(
        substrate
            .resolve_rooted_origin(&derived)
            .expect("artifact-owned scratch location"),
        crate::ResolvedSourceLocation {
            path: "scratch.tex".to_owned(),
            start: 0,
            end: 3,
            line: 1,
            column: 1,
        }
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
fn provenance_records_are_accessible_through_universe_boundary() {
    let mut universe = Universe::new();
    let source = universe.source_origin(crate::input::SourceId::new(11), 80, 6, 4);
    assert_eq!(universe.bootstrap_origin(), OriginId::UNKNOWN);
    assert_eq!(
        universe.origin(source),
        OriginRecord::Source(SourceOrigin::new(crate::input::SourceId::new(11), 80, 6, 4))
    );
}

#[test]
fn semantic_hash_ignores_provenance_allocations() {
    let mut universe = Universe::new();
    let base_snapshot = universe.snapshot();
    let base_checkpoint_hash = base_snapshot.state_hash();
    let base_testing_hash = universe.snapshot().state_hash();

    let source = universe.synthetic_origin_ref(SyntheticOriginKind::Test);
    let synthetic = universe.synthetic_origin_ref(SyntheticOriginKind::Engine);
    let _list = universe.allocate_origin_list_ref(&[source, synthetic]);
    let after_snapshot = universe.snapshot();

    assert_eq!(after_snapshot.state_hash(), base_checkpoint_hash);
    assert_eq!(universe.snapshot().state_hash(), base_testing_hash);
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
fn repeated_generated_and_world_registration_reuses_line_indexes() {
    let mut universe = Universe::new();
    let generated_source = SourceId::new(3);
    let generated = SourceDescriptor::generated(Arc::from(&b"one\n\ntwo\n"[..]));
    let generated_start = universe
        .register_source(generated_source, generated.clone())
        .expect("generated source registers");
    let generated_region = universe
        .source_region(generated_source)
        .expect("generated source is live");
    let generated_index = universe
        .source_line_starts(generated_region)
        .expect("generated source has a line index")
        .as_ptr();
    for _ in 0..32 {
        assert_eq!(
            universe
                .register_source(generated_source, generated.clone())
                .expect("identical generated registration is idempotent"),
            generated_start
        );
        assert_eq!(
            universe
                .source_line_starts(generated_region)
                .expect("generated index remains live")
                .as_ptr(),
            generated_index
        );
    }
    assert_eq!(
        universe.source_line_starts(generated_region),
        Some(&[0, 4, 5, 9][..])
    );

    let mut world = World::memory();
    world
        .set_memory_file("lines.tex", b"alpha\nbeta".to_vec())
        .expect("memory input installs");
    let mut universe = Universe::with_world(world);
    let content = universe
        .world_mut()
        .read_file("lines.tex")
        .expect("memory input reads");
    let world_source = SourceId::new(9);
    let descriptor = SourceDescriptor::world(content.record(), content.bytes().len() as u64);
    let world_start = universe
        .register_source(world_source, descriptor.clone())
        .expect("world source registers");
    let world_region = universe
        .source_region(world_source)
        .expect("world source is live");
    let world_index = universe
        .source_line_starts(world_region)
        .expect("world source has a line index")
        .as_ptr();
    for _ in 0..32 {
        assert_eq!(
            universe
                .register_source(world_source, descriptor.clone())
                .expect("identical world registration is idempotent"),
            world_start
        );
        assert_eq!(
            universe
                .source_line_starts(world_region)
                .expect("world index remains live")
                .as_ptr(),
            world_index
        );
    }
    assert_eq!(universe.source_line_starts(world_region), Some(&[0, 6][..]));
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
    let stale_origin = universe.synthetic_origin_ref(SyntheticOriginKind::Test);
    let stale_origin_id = stale_origin.id();
    let stale_word = TracedTokenWord::pack(Token::Cs(stale_symbol.symbol()), stale_origin_id);
    let stale_buffer = RootedTracedTokenBuffer::new([RootedTracedTokenWord::new(
        Token::Char {
            ch: 's',
            cat: Catcode::Other,
        },
        stale_origin.clone(),
    )]);
    let stale_list = universe.finish_rooted_traced_token_list(&stale_buffer);
    drop(stale_buffer);
    drop(stale_origin);
    universe.rollback(&mark);

    let registration = universe
        .register_input_source(
            crate::SourceId::new(1),
            SourceDescriptor::generated(Arc::from(&b"x"[..])),
        )
        .expect("register replacement source");
    let symbol = universe.intern("replacement");
    let origin = universe.synthetic_origin_ref(SyntheticOriginKind::Engine);
    let word = TracedTokenWord::pack(Token::Cs(symbol.symbol()), origin.id());
    let buffer = RootedTracedTokenBuffer::new([RootedTracedTokenWord::new(
        Token::Cs(symbol.symbol()),
        origin.clone(),
    )]);
    let list = universe.finish_rooted_traced_token_list(&buffer);
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
            token_list: traced.token_ref().clone(),
            origin_list: traced.origin_ref().clone(),
            replay_kind: TokenListReplayKind::MacroBody,
            index: 0,
            macro_arguments: arguments,
            macro_invocation: invocation,
            parent_macro_invocation: OriginId::UNKNOWN,
        }
    };

    let stale_argument = one_macro_argument(stale_word, 1);
    let structurally_retained = InputSummary::new(
        vec![token_frame(
            list.clone(),
            MacroArguments::new(),
            OriginId::UNKNOWN,
        )],
        None,
        None,
    );
    universe.set_input_summary(structurally_retained.clone());
    assert_eq!(universe.input_summary(), &structurally_retained);
    universe.set_input_summary(InputSummary::default());
    drop(structurally_retained);
    let mut invalid = vec![
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
            vec![InputFrameSummary::Source {
                source_id: crate::SourceId::new(1),
                input_record: None,
                source: source(stale_registration, word),
            }],
            None,
            None,
        ),
        InputSummary::new(
            vec![token_frame(list.clone(), stale_argument, OriginId::UNKNOWN)],
            None,
            None,
        ),
        InputSummary::new(
            vec![token_frame(
                list.clone(),
                MacroArguments::new(),
                stale_origin_id,
            )],
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
    let foreign_origin = foreign.synthetic_origin_ref(SyntheticOriginKind::Test);
    let foreign_word =
        TracedTokenWord::pack(Token::Cs(foreign_symbol.symbol()), foreign_origin.id());
    let foreign_buffer = RootedTracedTokenBuffer::new([RootedTracedTokenWord::new(
        Token::Cs(foreign_symbol.symbol()),
        foreign_origin,
    )]);
    let foreign_list = foreign.finish_rooted_traced_token_list(&foreign_buffer);
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
            token_frame(list, arguments, origin.id()),
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
    let left_origin = universe.synthetic_origin_ref(SyntheticOriginKind::Test);
    let right_origin = universe.synthetic_origin_ref(SyntheticOriginKind::Engine);
    let left_origins = universe.allocate_origin_list_ref(std::slice::from_ref(&left_origin));
    let right_origins = universe.allocate_origin_list_ref(std::slice::from_ref(&right_origin));
    let left_invocation = universe.macro_invocation_origin(
        definition.id(),
        left_origin.id(),
        left_origin.id(),
        OriginId::UNKNOWN,
    );
    let right_invocation = universe.macro_invocation_origin(
        definition.id(),
        right_origin.id(),
        right_origin.id(),
        OriginId::UNKNOWN,
    );
    let body_root = universe.token_list_ref(body);
    let left_summary = macro_replay_summary(
        body_root.clone(),
        left_origins,
        left_invocation,
        left_origin.id(),
    );
    let right_summary = macro_replay_summary(
        body_root,
        right_origins,
        right_invocation,
        right_origin.id(),
    );
    assert_eq!(left_summary, right_summary);

    universe.set_input_summary(left_summary);
    let first = universe.snapshot();
    universe.set_input_summary(right_summary);
    let second = universe.snapshot();

    assert_eq!(first.state_hash(), second.state_hash());
}

#[test]
fn universe_rollback_truncates_origin_records_without_reviving_ids() {
    let mut universe = Universe::new();
    let mark = universe.snapshot();

    let stale = universe.source_origin(crate::input::SourceId::new(7), 70, 8, 9);
    assert!(universe.origin_if_live(stale).is_some());

    universe.rollback(&mark);
    assert_eq!(universe.origin_if_live(stale), None);

    let replayed = universe.source_origin(crate::input::SourceId::new(7), 70, 8, 9);
    assert_ne!(replayed.raw(), stale.raw());
    assert_eq!(
        universe.origin(replayed),
        OriginRecord::Source(SourceOrigin::new(crate::input::SourceId::new(7), 70, 8, 9))
    );
}

#[test]
fn rollback_rejects_dropped_effect_snapshot_before_mutating_stores() {
    let mut universe = Universe::new();
    let symbol = universe.intern("x");
    let snapshot = universe.snapshot();

    universe.set_meaning(symbol, Meaning::Relax);
    let origin = universe.source_origin(crate::input::SourceId::new(7), 70, 8, 9);
    universe
        .world_mut()
        .write_text(PrintSink::TerminalAndLog, "committed\n");
    let effect_pos = universe.world().effect_pos();
    universe
        .commit_effects(effect_pos)
        .expect("memory world commit succeeds");
    let live_hash = universe.snapshot().state_hash();
    let provenance = universe.provenance_stats();

    assert!(!universe.can_rollback_to(&snapshot));

    let result = catch_unwind(AssertUnwindSafe(|| universe.rollback(&snapshot)));

    assert!(result.is_err());
    assert_eq!(universe.meaning(symbol), Meaning::Relax);
    assert!(universe.origin_if_live(origin).is_some());
    assert_eq!(universe.provenance_stats(), provenance);
    assert_eq!(universe.snapshot().state_hash(), live_hash);
}

#[test]
fn generation_fork_accepts_persistent_snapshot_behind_effect_barrier() {
    let mut universe = Universe::new();
    universe.set_count(0, 11);
    let checkpoint = universe.snapshot();

    universe.set_count(0, 22);
    universe
        .world_mut()
        .write_text(PrintSink::TerminalAndLog, "committed\n");
    let effect_pos = universe.world().effect_pos();
    universe
        .commit_effects(effect_pos)
        .expect("memory world commit succeeds");
    assert!(!universe.can_rollback_to(&checkpoint));

    let substrate = universe.freeze_generation();
    let fork = substrate
        .fork_at(&checkpoint)
        .expect("persistent checkpoint remains forkable");

    assert_eq!(fork.count(0), 11);
    assert_eq!(fork.world().effect_pos().raw(), 0);
}

#[test]
fn fixed_size_direct_operation_mark_does_not_add_a_checkpoint_hash_boundary() {
    let mut with_mark = Universe::new();
    let mut without_mark = Universe::new();

    with_mark.set_count(0, 11);
    without_mark.set_count(0, 11);
    let snapshot_serial = with_mark.next_snapshot_serial;
    let checkpoint_hash = with_mark.state_hash_base.checkpoint_hash;
    let mark = with_mark.begin_direct_operation();
    assert!(
        std::mem::size_of_val(&mark) <= 20 * std::mem::size_of::<usize>(),
        "direct operation cursor must remain fixed and small"
    );
    with_mark.commit_direct_operation(mark);
    assert_eq!(with_mark.next_snapshot_serial, snapshot_serial);
    assert_eq!(with_mark.state_hash_base.checkpoint_hash, checkpoint_hash);
    with_mark.set_count(1, 22);
    without_mark.set_count(1, 22);

    assert_eq!(
        with_mark.snapshot().state_hash(),
        without_mark.snapshot().state_hash(),
        "direct command delivery must not alter the named checkpoint schedule"
    );
}

#[test]
fn rollback_restores_page_builder_state_and_hash() {
    let mut universe = Universe::new();
    let base_hash = universe.snapshot().state_hash();
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

    assert_ne!(universe.snapshot().state_hash(), base_hash);
    universe.rollback(&snapshot);

    assert_eq!(universe.snapshot().state_hash(), base_hash);
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
fn empty_page_mark_presence_invalidates_dependencies_and_survives_rollback() {
    use crate::{DependencyKey, DependencyValue};

    let marks = [
        PageMark::Top,
        PageMark::First,
        PageMark::Bot,
        PageMark::SplitFirst,
        PageMark::SplitBot,
    ];
    let mut universe = Universe::new();

    for (ordinal, mark) in marks.into_iter().enumerate() {
        let direct = DependencyKey::PageMark(mark.index());
        let class_zero = DependencyKey::PageMarkClass {
            mark: mark.index(),
            class: 0,
        };
        let sparse_class = u16::try_from(ordinal + 1).expect("bounded class");
        let sparse = DependencyKey::PageMarkClass {
            mark: mark.index(),
            class: sparse_class,
        };
        let snapshot = universe.snapshot();

        for key in [direct, class_zero, sparse] {
            assert_eq!(
                universe.semantic_dependency_value(key),
                Some(DependencyValue::Absent)
            );
        }

        let region = universe
            .begin_tracked_region()
            .expect("start tracked region");
        for key in [direct, class_zero, sparse] {
            universe.record_dependency(key, DependencyValue::Absent);
        }
        let observations = universe
            .finish_tracked_region(region)
            .expect("finish tracked region")
            .observations()
            .to_vec();

        universe.set_page_mark(mark, TokenListId::EMPTY);
        universe.set_page_mark_class(mark, sparse_class, TokenListId::EMPTY);
        let empty = universe
            .semantic_dependency_value(direct)
            .expect("direct mark dependency");
        assert_ne!(empty, DependencyValue::Absent);
        assert_eq!(
            universe.semantic_dependency_value(class_zero),
            Some(empty.clone())
        );
        assert_eq!(universe.semantic_dependency_value(sparse), Some(empty));

        for observation in observations {
            let key = observation.key;
            assert_eq!(
                universe.validate_dependencies_with_failure_readonly(
                    std::slice::from_ref(&observation),
                    |key| universe
                        .semantic_dependency_value(key)
                        .expect("page mark dependency"),
                ),
                Some(key),
                "absent-to-present-empty must reject checkpoint reuse for {key:?}"
            );
        }

        let region = universe
            .begin_tracked_region()
            .expect("start tracked region");
        for key in [direct, class_zero, sparse] {
            universe.record_dependency(
                key,
                universe
                    .semantic_dependency_value(key)
                    .expect("page mark dependency"),
            );
        }
        let observations = universe
            .finish_tracked_region(region)
            .expect("finish tracked region")
            .observations()
            .to_vec();

        universe.clear_page_mark_class(mark, 0);
        universe.clear_page_mark_class(mark, sparse_class);
        for observation in observations {
            let key = observation.key;
            assert_eq!(
                universe.validate_dependencies_with_failure_readonly(
                    std::slice::from_ref(&observation),
                    |key| universe
                        .semantic_dependency_value(key)
                        .expect("page mark dependency"),
                ),
                Some(key),
                "present-empty-to-absent must reject checkpoint reuse for {key:?}"
            );
        }

        universe.rollback(&snapshot);
        for key in [direct, class_zero, sparse] {
            assert_eq!(
                universe.semantic_dependency_value(key),
                Some(DependencyValue::Absent),
                "rollback must restore absence for {key:?}"
            );
        }
    }
}

#[test]
fn execution_facade_reads_record_exact_and_aggregate_dependencies() {
    use crate::{DependencyEngineField, DependencyKey};

    let mut universe = Universe::new_with_plain_catcodes();
    let region = universe
        .begin_tracked_region()
        .expect("start execution read region");
    let _ = universe.count(3);
    let _ = universe.current_font();
    let _ = universe.catcode('A');
    let _ = universe.hyphen_positions_for_language(0, "letters", 2, 3);
    let _ = universe.page_contents();
    let _ = universe.innermost_group_kind();
    let record = universe
        .finish_tracked_region(region)
        .expect("execution reads are projectable");

    for key in [
        DependencyKey::Cell(crate::cell::CellId::new(crate::cell::BankTag::Count, 3)),
        DependencyKey::Cell(crate::cell::CellId::new(
            crate::cell::BankTag::CurrentFont,
            0,
        )),
        DependencyKey::Code {
            table: crate::DependencyCodeTable::Catcode,
            scalar: 'A'.into(),
        },
        DependencyKey::HyphenationPatterns(0),
        DependencyKey::HyphenationExceptions(0),
        DependencyKey::HyphenationCodes(0),
        DependencyKey::Page(crate::DependencyPageField::Contents),
        DependencyKey::Engine(DependencyEngineField::GroupType),
    ] {
        assert!(
            record
                .observations()
                .iter()
                .any(|observation| observation.key == key),
            "missing execution dependency {key:?}"
        );
    }

    let mut observations = record.observations().to_vec();
    universe.set_count(4, 99);
    assert!(universe.validate_dependencies(&mut observations, |key| {
        universe
            .semantic_dependency_value(key)
            .expect("recorded execution dependency remains projectable")
    }));
    universe.set_count(3, 17);
    assert!(!universe.validate_dependencies(&mut observations, |key| {
        universe
            .semantic_dependency_value(key)
            .expect("recorded execution dependency remains projectable")
    }));
}

fn record_world_observation(
    universe: &mut Universe,
    key: crate::DependencyKey,
) -> crate::ObservedDependency {
    let region = universe
        .begin_tracked_region()
        .expect("start World dependency region");
    universe.observe_semantic_dependency(key);
    universe
        .finish_tracked_region(region)
        .expect("World fact is projectable")
        .observations()
        .iter()
        .find(|observation| observation.key == key)
        .expect("World fact was observed")
        .clone()
}

fn world_observation_validates(
    universe: &Universe,
    observation: &crate::ObservedDependency,
) -> bool {
    universe
        .validate_dependencies_with_failure_readonly(std::slice::from_ref(observation), |key| {
            universe
                .semantic_dependency_value(key)
                .expect("tracked World fact remains projectable")
        })
        .is_none()
}

#[test]
fn every_world_projection_stays_green_after_an_unrelated_world_mutation() {
    use crate::{DependencyKey, DependencyWorldField};

    let request = World::input_resource_dependency_identity("tracked.tex");
    for key in [
        DependencyKey::World {
            field: DependencyWorldField::InputResource,
            index: request,
        },
        DependencyKey::World {
            field: DependencyWorldField::OutputStream,
            index: 0,
        },
        DependencyKey::World {
            field: DependencyWorldField::InputStream,
            index: 0,
        },
        DependencyKey::World {
            field: DependencyWorldField::TerminalInputCursor,
            index: 0,
        },
        DependencyKey::World {
            field: DependencyWorldField::EffectPolicy,
            index: 0,
        },
        DependencyKey::World {
            field: DependencyWorldField::ShellEscapePolicy,
            index: 0,
        },
        DependencyKey::World {
            field: DependencyWorldField::JobClock,
            index: 0,
        },
        DependencyKey::World {
            field: DependencyWorldField::Rng,
            index: 0,
        },
        DependencyKey::World {
            field: DependencyWorldField::LoadedResources,
            index: 0,
        },
        DependencyKey::World {
            field: DependencyWorldField::MaterializationBarrier,
            index: 0,
        },
    ] {
        let mut universe = Universe::new();
        let observation = record_world_observation(&mut universe, key);
        universe
            .world_mut()
            .open_out(StreamSlot::new(15), "unrelated.aux");
        assert!(
            world_observation_validates(&universe, &observation),
            "unrelated output slot changed {key:?}"
        );
    }
}

#[test]
fn exact_world_keys_reject_relevant_mutations_without_cross_talk() {
    use crate::{DependencyKey, DependencyWorldField};

    let path = std::path::Path::new("tracked.tex");
    let request = World::input_resource_dependency_identity(path);
    let other_request = World::input_resource_dependency_identity("other.tex");
    let resource = DependencyKey::World {
        field: DependencyWorldField::InputResource,
        index: request,
    };
    let other_resource = DependencyKey::World {
        field: DependencyWorldField::InputResource,
        index: other_request,
    };
    let mut universe = Universe::new();
    let resource_observation = record_world_observation(&mut universe, resource);
    let other_resource_observation = record_world_observation(&mut universe, other_resource);
    universe
        .world_mut()
        .record_input_dependency(
            path,
            InputDependencyOutcome::Missing,
            InputDependencyAccess::AuthoritativeProbe,
        )
        .expect("record exact input request");
    assert!(!world_observation_validates(
        &universe,
        &resource_observation
    ));
    assert!(world_observation_validates(
        &universe,
        &other_resource_observation
    ));

    let output = DependencyKey::World {
        field: DependencyWorldField::OutputStream,
        index: 0,
    };
    let other_output = DependencyKey::World {
        field: DependencyWorldField::OutputStream,
        index: 1,
    };
    let mut universe = Universe::new();
    let output_observation = record_world_observation(&mut universe, output);
    let other_output_observation = record_world_observation(&mut universe, other_output);
    universe
        .world_mut()
        .open_out(StreamSlot::new(0), "tracked.aux");
    assert!(!world_observation_validates(&universe, &output_observation));
    assert!(world_observation_validates(
        &universe,
        &other_output_observation
    ));

    let input = DependencyKey::World {
        field: DependencyWorldField::InputStream,
        index: 0,
    };
    let other_input = DependencyKey::World {
        field: DependencyWorldField::InputStream,
        index: 1,
    };
    let mut universe = Universe::new();
    let input_observation = record_world_observation(&mut universe, input);
    let other_input_observation = record_world_observation(&mut universe, other_input);
    universe
        .world_mut()
        .set_memory_file(path, b"line\n".to_vec())
        .expect("seed input stream");
    let content = universe.world_mut().read_file(path).expect("read input");
    universe
        .world_mut()
        .open_in_content(StreamSlot::new(0), &content)
        .expect("open input stream");
    assert!(!world_observation_validates(&universe, &input_observation));
    assert!(world_observation_validates(
        &universe,
        &other_input_observation
    ));
}

#[test]
fn scalar_and_aggregate_world_keys_reject_their_relevant_mutations() {
    use crate::{DependencyKey, DependencyWorldField};

    type WorldMutation = (DependencyWorldField, fn(&mut Universe));
    let cases: &[WorldMutation] = &[
        (DependencyWorldField::TerminalInputCursor, |universe| {
            universe
                .world_mut()
                .push_memory_terminal_line("typed")
                .expect("supply terminal line");
        }),
        (DependencyWorldField::EffectPolicy, |universe| {
            universe
                .begin_retained_session()
                .expect("enter retained mode");
        }),
        (DependencyWorldField::ShellEscapePolicy, |universe| {
            universe
                .world_mut()
                .set_shell_escape_policy(ShellEscapePolicy::Restricted);
        }),
        (DependencyWorldField::Rng, |universe| {
            let _ = universe.world_mut().next_random_u64();
        }),
        (DependencyWorldField::LoadedResources, |universe| {
            universe
                .world_mut()
                .set_memory_file("loaded.tex", b"loaded".to_vec())
                .expect("seed loaded resource");
            universe
                .world_mut()
                .read_file("loaded.tex")
                .expect("load resource");
        }),
        (DependencyWorldField::MaterializationBarrier, |universe| {
            universe
                .world_mut()
                .write_text(PrintSink::Log, "materialize");
            let end = universe.world().effect_pos();
            universe.commit_effects(end).expect("materialize effects");
        }),
    ];
    for &(field, mutate) in cases {
        let key = DependencyKey::World { field, index: 0 };
        let mut universe = Universe::new();
        let observation = record_world_observation(&mut universe, key);
        mutate(&mut universe);
        assert!(
            !world_observation_validates(&universe, &observation),
            "relevant mutation left {field:?} green"
        );
    }

    let mut baseline = Universe::with_world(World::memory_with_clock(JobClock::DEFAULT));
    let mut changed = Universe::with_world(World::memory_with_clock(JobClock {
        year: 2027,
        ..JobClock::DEFAULT
    }));
    let key = DependencyKey::World {
        field: DependencyWorldField::JobClock,
        index: 0,
    };
    assert_ne!(
        baseline.semantic_dependency_value(key),
        changed.semantic_dependency_value(key),
        "job clock construction input must have an exact projection"
    );
    let observation = record_world_observation(&mut baseline, key);
    changed.track_dependency(key);
    changed.mark_dependency_changed(key);
    assert!(
        !world_observation_validates(&changed, &observation),
        "a different fixed job clock must reject the dependency"
    );
}

#[test]
fn world_projections_are_allocation_independent_across_universes() {
    use crate::{DependencyKey, DependencyWorldField};

    fn configured_world(reverse: bool) -> World {
        let mut world = World::memory();
        world
            .set_memory_file("a.tex", b"a\n".to_vec())
            .expect("seed a");
        world
            .set_memory_file("b.tex", b"b\n".to_vec())
            .expect("seed b");
        let paths = if reverse {
            ["b.tex", "a.tex"]
        } else {
            ["a.tex", "b.tex"]
        };
        let mut a = None;
        for path in paths {
            let content = world.read_file(path).expect("read configured input");
            world
                .record_input_dependency(
                    path,
                    InputDependencyOutcome::Present(content.hash()),
                    InputDependencyAccess::RequiredRead,
                )
                .expect("record configured dependency");
            if path == "a.tex" {
                a = Some(content);
            }
        }
        world
            .open_in_content(StreamSlot::new(3), &a.expect("a was read"))
            .expect("open configured input stream");
        world.open_out(StreamSlot::new(2), "same.aux");
        world
            .push_memory_terminal_line("same terminal line")
            .expect("supply configured terminal line");
        world
    }

    let left = Universe::with_world(configured_world(false));
    let right = Universe::with_world(configured_world(true));
    let request = World::input_resource_dependency_identity("a.tex");
    for key in [
        DependencyKey::World {
            field: DependencyWorldField::InputResource,
            index: request,
        },
        DependencyKey::World {
            field: DependencyWorldField::OutputStream,
            index: 2,
        },
        DependencyKey::World {
            field: DependencyWorldField::InputStream,
            index: 3,
        },
        DependencyKey::World {
            field: DependencyWorldField::TerminalInputCursor,
            index: 0,
        },
        DependencyKey::World {
            field: DependencyWorldField::EffectPolicy,
            index: 0,
        },
        DependencyKey::World {
            field: DependencyWorldField::ShellEscapePolicy,
            index: 0,
        },
        DependencyKey::World {
            field: DependencyWorldField::JobClock,
            index: 0,
        },
        DependencyKey::World {
            field: DependencyWorldField::Rng,
            index: 0,
        },
        DependencyKey::World {
            field: DependencyWorldField::LoadedResources,
            index: 0,
        },
        DependencyKey::World {
            field: DependencyWorldField::MaterializationBarrier,
            index: 0,
        },
    ] {
        assert_eq!(
            left.semantic_dependency_value(key),
            right.semantic_dependency_value(key),
            "allocation order leaked into {key:?}"
        );
    }
}

#[test]
fn replay_probe_drop_restores_semantic_page_store_and_world_state() {
    let mut universe = Universe::with_world(World::memory());
    let base_hash = universe.snapshot().state_hash();

    {
        let mut probe = universe.begin_replay_probe();
        probe.set_count(7, 91);
        probe.append_page_contribution(Node::Penalty(17));
        probe.record_page_fire_up(3);
        probe
            .world_mut()
            .write_text(PrintSink::TerminalAndLog, "speculative\n");
    }

    assert_eq!(universe.snapshot().state_hash(), base_hash);
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
        box_lr: crate::node::BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children,
    }));
    assert!(matches!(page, Node::HList(_)));

    transaction
        .world_mut()
        .write_text(PrintSink::TerminalAndLog, "shipout\n");
    let effect_pos = transaction.world().effect_pos();
    let reservation = transaction.world_mut().reserve_artifact_publication_at(0);
    let (hash, _) = transaction
        .commit(
            crate::VerifiedArtifact::new(b"detached page artifact".to_vec()),
            effect_pos,
            reservation,
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
            box_lr: crate::node::BoxLr::Normal,
            glue_set: GlueSetRatio::ZERO,
            glue_sign: Sign::Normal,
            glue_order: Order::Normal,
            children,
        }));
        let effect_pos = transaction.world().effect_pos();
        let reservation = transaction.world_mut().reserve_artifact_publication_at(0);
        transaction
            .commit(
                crate::VerifiedArtifact::new(format!("page {page}").into_bytes()),
                effect_pos,
                reservation,
            )
            .expect("shipout commit succeeds");
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
    let reservation = transaction.world_mut().reserve_artifact_publication_at(0);
    transaction
        .commit(
            crate::VerifiedArtifact::new(b"logical page".to_vec()),
            effect_pos,
            reservation,
        )
        .expect("logical shipout succeeds");

    assert_eq!(universe.world().artifact_commits().len(), 1);
    assert_eq!(universe.world().effect_records().len(), 1);
    assert_eq!(universe.world().memory_terminal_output(), Some(&b""[..]));

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

    let reservation = universe.world_mut().reserve_artifact_publication_at(0);
    let (first_hash, _) = universe
        .begin_shipout()
        .commit(
            crate::VerifiedArtifact::new(b"checkpointed PDF page".to_vec()),
            effect_pos,
            reservation,
        )
        .expect("first shipout succeeds");
    let first_page = universe.pdf_pages()[0].clone();
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

    let reservation = universe.world_mut().reserve_artifact_publication_at(0);
    let (replay_hash, _) = universe
        .begin_shipout()
        .commit(
            crate::VerifiedArtifact::new(b"checkpointed PDF page".to_vec()),
            effect_pos,
            reservation,
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

    let reservation = universe.world_mut().reserve_artifact_publication_at(0);
    universe
        .begin_shipout()
        .commit(
            crate::VerifiedArtifact::new(b"DVI-mode page".to_vec()),
            effect_pos,
            reservation,
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
    let reservation = transaction.world_mut().reserve_artifact_publication_at(0);
    transaction
        .world_mut()
        .fail_effect_commit_before(effect_pos);
    transaction
        .commit(
            crate::VerifiedArtifact::new(b"failed PDF page".to_vec()),
            effect_pos,
            reservation,
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
fn live_state_identity_ignores_dead_allocation_history_and_preserves_future_append() {
    let mut direct = Universe::new();
    let mut noisy = Universe::new();

    for index in 0..64 {
        let dead_name = noisy.intern(&format!("dead-{index}"));
        let dead_tokens = noisy.intern_token_list(&[
            Token::Cs(dead_name.symbol()),
            Token::Char {
                ch: char::from(b'a' + (index % 26) as u8),
                cat: Catcode::Letter,
            },
        ]);
        noisy.intern_macro(MacroMeaning::new(
            MeaningFlags::LONG,
            crate::ids::TokenListId::EMPTY,
            dead_tokens,
        ));
        noisy.intern_glue(glue(index));
        noisy.freeze_node_list(&[Node::Kern {
            amount: Scaled::from_raw(index),
            kind: KernKind::Explicit,
        }]);
    }

    fn install_live_root(universe: &mut Universe) {
        let name = universe.intern("live-root");
        let replacement = universe.intern_token_list(&[
            Token::Cs(name.symbol()),
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            },
        ]);
        let definition = universe.intern_macro(MacroMeaning::new(
            MeaningFlags::PROTECTED,
            crate::ids::TokenListId::EMPTY,
            replacement,
        ));
        universe.set_meaning(
            name,
            Meaning::Macro {
                flags: MeaningFlags::PROTECTED,
                definition: definition.id(),
            },
        );
        universe.set_toks(0, replacement);
        let skip = universe.intern_glue(glue(7));
        universe.set_skip(0, skip);
    }

    install_live_root(&mut direct);
    install_live_root(&mut noisy);
    assert_eq!(
        direct.snapshot().state_hash(),
        noisy.snapshot().state_hash()
    );
    assert_eq!(identity_of(&mut direct), identity_of(&mut noisy));
    noisy.testing_clear_state_hash_caches();
    assert_eq!(identity_of(&mut direct), identity_of(&mut noisy));

    let direct_future = direct.intern("future-root");
    let noisy_future = noisy.intern("future-root");
    direct.set_meaning(direct_future, Meaning::CharGiven('z'));
    noisy.set_meaning(noisy_future, Meaning::CharGiven('z'));
    assert_eq!(
        direct.snapshot().state_hash(),
        noisy.snapshot().state_hash()
    );
    assert_eq!(identity_of(&mut direct), identity_of(&mut noisy));
}

#[test]
fn retained_snapshot_restores_exact_component_projections_into_forks() {
    let mut universe = Universe::new();
    universe.set_input_summary(condition_input_summary(0));
    universe
        .add_hyphenation_pattern(PatternSpec {
            letters: "retained".chars().collect(),
            values: vec![0, 0, 1, 0, 0, 0, 0, 0, 0],
        })
        .expect("pattern fits the default trie capacity");
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
fn exact_reachable_store_root_survives_format_reconstruction() {
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
            definition: definition.id(),
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
fn format_dump_preserves_names_but_compacts_macro_token_and_glue_closure() {
    let mut universe = Universe::new();
    let dead_name = universe.intern("dead-format-history");
    let dead_tokens = universe.intern_token_list(&[Token::Cs(dead_name.symbol())]);
    universe.intern_macro(MacroMeaning::new(
        MeaningFlags::EMPTY,
        crate::ids::TokenListId::EMPTY,
        dead_tokens,
    ));
    let dead_glue = universe.intern_glue(glue(901));
    drop(dead_glue);

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
            definition: live_macro.id(),
        },
    );
    let live_glue = universe.intern_glue(glue(902));
    universe.set_skip(0, live_glue);
    let live_identity = identity_of(&mut universe);

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
    // TeX82 §§256 and 1309 retain the complete occupied control-sequence
    // namespace even when an entry has no reachable meaning.
    assert_eq!(count(crate::stores::NAMES_SECTION), 2);
    assert_eq!(count(crate::stores::TOKEN_LISTS_SECTION), 2);
    assert_eq!(count(crate::stores::MACROS_SECTION), 1);
    assert_eq!(count(crate::stores::GLUE_SECTION), 2);

    let mut restored =
        Universe::from_format(World::memory(), &image).expect("restore compact format");
    assert_eq!(identity_of(&mut restored), live_identity);
    let restored_live = restored
        .symbol("live-format-root")
        .expect("reachable name remains interned");
    assert!(matches!(
        restored.meaning(restored_live),
        Meaning::Macro { .. }
    ));
    let restored_dead = restored
        .symbol("dead-format-history")
        .expect("occupied undefined name remains interned");
    assert_eq!(restored.meaning(restored_dead), Meaning::Undefined);
    assert_eq!(restored.glue(restored.skip(0)), glue(902));
}

#[test]
fn exact_reachable_store_root_ignores_divergent_rollback_allocation() {
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
fn exact_reachable_font_identity_ignores_allocation_order() {
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
        let live = universe.intern("live-immutable-component");
        universe.set_meaning(live, Meaning::Relax);
    });
    assert_change(|universe| {
        universe
            .configure_font_expansion(
                NULL_FONT,
                FontExpansion {
                    stretch: 20,
                    shrink: 10,
                    step: 5,
                    auto_expand: true,
                },
            )
            .expect("null font accepts one expansion configuration");
    });
    assert_change(|universe| universe.set_count(0, 1));
    assert_change(|universe| universe.set_catcode('x', Catcode::Active));
    assert_change(|universe| {
        universe
            .add_hyphenation_pattern(PatternSpec {
                letters: "identity".chars().collect(),
                values: vec![0, 0, 1, 0, 0, 0, 0, 0, 0],
            })
            .expect("pattern fits the default trie capacity");
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

#[test]
fn replay_probe_rolls_back_interaction_mode_unless_committed() {
    let mut universe = Universe::new();
    universe.set_interaction_mode(super::InteractionMode::Nonstop);
    {
        let mut probe = universe.begin_replay_probe();
        probe.set_interaction_mode(super::InteractionMode::Batch);
    }
    assert_eq!(universe.interaction_mode(), super::InteractionMode::Nonstop);

    {
        let mut probe = universe.begin_replay_probe();
        probe.set_interaction_mode(super::InteractionMode::Scroll);
        probe.commit();
    }
    assert_eq!(universe.interaction_mode(), super::InteractionMode::Scroll);
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
    first.set_skip(0, &target_glue);
    first.set_meaning(
        macro_target,
        Meaning::Macro {
            flags: MeaningFlags::PROTECTED,
            definition: target_macro.id(),
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
    second.set_skip(0, &target_glue);
    second.set_meaning(
        macro_target,
        Meaning::Macro {
            flags: MeaningFlags::PROTECTED,
            definition: target_macro.id(),
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
    first
        .add_hyphenation_pattern(PatternSpec {
            letters: "alpha".chars().collect(),
            values: vec![0, 1, 0, 0, 0, 0],
        })
        .expect("pattern fits the default trie capacity");
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
        universe
            .add_hyphenation_pattern(PatternSpec {
                letters: "cache".chars().collect(),
                values: vec![0, 0, 1, 0, 0, 0],
            })
            .expect("pattern fits the default trie capacity");
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
        universe
            .add_hyphenation_pattern(PatternSpec {
                letters: "component".chars().collect(),
                values: vec![0, 0, 1, 0, 0, 0, 0, 0, 0, 0],
            })
            .expect("pattern fits the default trie capacity");
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
        let reservation = universe.world_mut().reserve_artifact_publication_at(0);
        universe
            .begin_shipout()
            .commit(
                crate::VerifiedArtifact::new(b"component projection page".to_vec()),
                effect_pos,
                reservation,
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
                        let reservation = universe.world_mut().reserve_artifact_publication_at(0);
                        universe
                            .begin_shipout()
                            .commit(
                                crate::VerifiedArtifact::new(
                                    b"randomized differential page".to_vec(),
                                ),
                                effect_pos,
                                reservation,
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
fn font_identifier_alias_replaces_and_rolls_back_the_previous_name() {
    let mut universe = Universe::new();
    let first = universe.intern("first");
    let second = universe.intern("second");
    let font = universe.intern_font_with_identifier(test_font("cmr10", b"same"), first);
    let snapshot = universe.snapshot();

    universe.set_font_identifier_symbol(font, second);
    assert_eq!(universe.font_identifier_symbol(font), Some(second));

    universe.rollback(&snapshot);
    assert_eq!(universe.font_identifier_symbol(font), Some(first));
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
        origin: crate::provenance::OriginRef::unknown(),
    }]);
    reused.set_box_reg_ref(0, first_list.clone());
    let first_hash = reused.snapshot().state_hash();

    reused.rollback(&base);
    let second_list = reused.freeze_node_list(&[Node::Char {
        font: NULL_FONT,
        ch: 'y',
        origin: crate::provenance::OriginRef::unknown(),
    }]);
    assert_ne!(
        first_list, second_list,
        "rollback must retag the reused epoch node span"
    );
    reused.set_box_reg_ref(0, second_list);
    let reused_hash = reused.snapshot().state_hash();

    let mut fresh = Universe::new();
    let _ = fresh.snapshot();
    let fresh_list = fresh.freeze_node_list(&[Node::Char {
        font: NULL_FONT,
        ch: 'y',
        origin: crate::provenance::OriginRef::unknown(),
    }]);
    fresh.set_box_reg_ref(0, fresh_list);
    let fresh_hash = fresh.snapshot().state_hash();

    assert_ne!(first_hash, reused_hash);
    assert_eq!(reused_hash, fresh_hash);
}

#[test]
fn structurally_owned_box_replays_with_resolvable_equal_hashes() {
    let mut universe = Universe::new();
    let child = universe.freeze_node_list(&[Node::Char {
        font: NULL_FONT,
        ch: 'x',
        origin: crate::provenance::OriginRef::unknown(),
    }]);
    universe.set_box_reg_ref(0, child);
    let base = universe.snapshot();

    let first = store_wrapped_box(&mut universe);
    let first_hash = universe.snapshot().state_hash();
    assert_promoted_wrapper_is_resolvable(&universe, first);

    universe.rollback(&base);
    let second = store_wrapped_box(&mut universe);
    let second_hash = universe.snapshot().state_hash();
    assert_promoted_wrapper_is_resolvable(&universe, second);

    assert_eq!(first_hash, second_hash);
}

fn store_wrapped_box(universe: &mut Universe) -> crate::node_arena::NodeListRef {
    let child = universe
        .box_reg_ref(0)
        .expect("child owner should remain live");
    let wrapper = universe.freeze_node_list(&[Node::VList(BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(10),
        height: Scaled::from_raw(7),
        depth: Scaled::from_raw(3),
        shift: Scaled::from_raw(0),
        box_lr: crate::node::BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: child,
    }))]);
    universe.set_box_reg_ref_global(255, wrapper);
    universe.box_reg_ref(255).expect("wrapper should be stored")
}

#[test]
fn grouped_box_take_owns_nested_children_before_coalesced_release() {
    let mut universe = Universe::new();
    let leader_children = universe.freeze_node_list(&[Node::Char {
        font: NULL_FONT,
        ch: 'x',
        origin: crate::provenance::OriginRef::unknown(),
    }]);
    let leader = BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(10),
        height: Scaled::from_raw(7),
        depth: Scaled::from_raw(3),
        shift: Scaled::from_raw(0),
        box_lr: crate::node::BoxLr::Normal,
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
    universe.set_box_reg_ref(0, value);
    let taken = universe
        .take_box_reg_ref_same_level(0)
        .expect("local box should move out of the register");

    let ArenaRef::Owned(root) = taken.id().arena() else {
        panic!("taken value should remain directly owned")
    };
    let Some(crate::node_arena::NodeRef::Glue {
        leader: Some(LeaderPayload::HList(leader)),
        ..
    }) = taken.nodes().first()
    else {
        panic!("taken value should preserve its leader box");
    };
    assert_ne!(leader.children.arena(), ArenaRef::Owned(root));
    assert_eq!(
        taken
            .resolve(leader.children)
            .expect("leader child belongs to the taken owner")
            .nodes(),
        &[Node::Char {
            font: NULL_FONT,
            ch: 'x',
            origin: crate::provenance::OriginRef::unknown(),
        }]
    );
    let _ = universe.leave_group();
    assert!(
        universe.box_reg_ref(0).is_none(),
        "§1079's direct voiding preserves the original void restoration"
    );
}

#[test]
fn same_level_box_take_crosses_nested_group_but_restores_at_owner_group() {
    let mut universe = Universe::new();
    let baseline = universe.freeze_node_list(&[Node::Char {
        font: NULL_FONT,
        ch: 'o',
        origin: crate::provenance::OriginRef::unknown(),
    }]);
    universe.set_box_reg_ref(0, baseline);
    let baseline = universe.box_reg_ref(0).expect("root box");

    universe.enter_group();
    let local = universe.freeze_node_list(&[Node::Char {
        font: NULL_FONT,
        ch: 'l',
        origin: crate::provenance::OriginRef::unknown(),
    }]);
    universe.set_box_reg_ref(0, local);
    universe.enter_group();
    assert!(universe.take_box_reg_ref_same_level(0).is_some());
    assert!(universe.box_reg_ref(0).is_none());

    let _ = universe.leave_group();
    assert!(
        universe.box_reg_ref(0).is_none(),
        "the destructive take must survive the nested construction group"
    );
    let _ = universe.leave_group();
    assert_eq!(universe.box_reg_ref(0), Some(baseline));
}

#[test]
fn destructive_unbox_transfers_only_children_before_same_level_clear() {
    let mut universe = Universe::new();
    let baseline = universe.freeze_node_list(&[Node::Char {
        font: NULL_FONT,
        ch: 'b',
        origin: crate::provenance::OriginRef::unknown(),
    }]);
    universe.set_box_reg_ref(0, baseline);
    let baseline = universe.box_reg_ref(0).expect("baseline box");

    universe.enter_group();
    let leaf = universe.freeze_node_list(&[Node::Char {
        font: NULL_FONT,
        ch: 'x',
        origin: crate::provenance::OriginRef::unknown(),
    }]);
    let nested = universe.freeze_node_list(&[Node::HList(BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(10),
        height: Scaled::from_raw(7),
        depth: Scaled::from_raw(3),
        shift: Scaled::from_raw(0),
        box_lr: crate::node::BoxLr::Normal,
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
        box_lr: crate::node::BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: nested,
    }))]);
    universe.set_box_reg_ref(0, wrapper);
    let TakeUnboxResult::Children(children) =
        universe.take_unbox_children_same_level(0, UnboxKind::Horizontal)
    else {
        panic!("compatible hbox should transfer its children")
    };

    assert!(universe.box_reg_ref(0).is_none());
    let ArenaRef::Owned(root) = children.id().arena() else {
        panic!("unboxed children should remain directly owned")
    };
    let Some(crate::node_arena::NodeRef::HList(nested)) = children.nodes().first() else {
        panic!("nested hbox should survive the transfer")
    };
    assert_ne!(nested.children.arena(), ArenaRef::Owned(root));
    assert!(matches!(
        children
            .resolve(nested.children)
            .expect("owned nested child")
            .nodes()
            .first(),
        Some(crate::node_arena::NodeRef::Char { ch: 'x', .. })
    ));
    let _ = universe.leave_group();
    assert_eq!(universe.box_reg_ref(0), Some(baseline));
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
        box_lr: crate::node::BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children,
    }))]);
    universe.set_box_reg_ref(4, wrapper);
    let stored = universe.box_reg_ref(4);

    assert_eq!(
        universe.take_unbox_children_same_level(4, UnboxKind::Horizontal),
        TakeUnboxResult::Incompatible
    );
    assert_eq!(universe.box_reg_ref(4), stored);
}

fn assert_promoted_wrapper_is_resolvable(
    _universe: &Universe,
    wrapper: crate::node_arena::NodeListRef,
) {
    let Some(crate::node_arena::NodeRef::VList(box_node)) = wrapper.nodes().first() else {
        panic!("stored wrapper should contain a vlist");
    };
    let (ArenaRef::Owned(wrapper_root), ArenaRef::Owned(child_root)) =
        (wrapper.id().arena(), box_node.children.arena())
    else {
        panic!("wrapper and child should each have an owned payload");
    };
    assert_ne!(wrapper_root, child_root);
    assert_eq!(
        wrapper
            .resolve(box_node.children)
            .expect("owned wrapper child")
            .nodes(),
        &[Node::Char {
            font: NULL_FONT,
            ch: 'x',
            origin: crate::provenance::OriginRef::unknown(),
        }]
    );
}

#[test]
fn snapshot_state_hash_walks_deep_node_lists_iteratively() {
    let mut universe = Universe::new();
    let mut current = universe.freeze_node_list(&[Node::Char {
        font: NULL_FONT,
        ch: 'x',
        origin: crate::provenance::OriginRef::unknown(),
    }]);

    for _ in 0..5000 {
        let children = current;
        current = universe.freeze_node_list(&[Node::HList(BoxNode::new(BoxNodeFields {
            width: Scaled::from_raw(1),
            height: Scaled::from_raw(2),
            depth: Scaled::from_raw(3),
            shift: Scaled::from_raw(0),
            box_lr: crate::node::BoxLr::Normal,
            glue_set: GlueSetRatio::ZERO,
            glue_sign: Sign::Normal,
            glue_order: Order::Normal,
            children,
        }))]);
    }

    universe.set_box_reg_ref(0, current);
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
                box_lr: crate::node::BoxLr::Normal,
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
        overwritten.set_box_reg_ref(0, transient);
    }

    let direct_final = direct.freeze_node_list(&[Node::Char {
        font: NULL_FONT,
        ch: 'x',
        origin: crate::provenance::OriginRef::unknown(),
    }]);
    direct.set_box_reg_ref(0, direct_final);
    let overwritten_final = overwritten.freeze_node_list(&[Node::Char {
        font: NULL_FONT,
        ch: 'x',
        origin: crate::provenance::OriginRef::unknown(),
    }]);
    overwritten.set_box_reg_ref(0, overwritten_final);

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
        origin: crate::provenance::OriginRef::unknown(),
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
        box_lr: crate::node::BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children,
    }))]);
    transaction.finish(0, Some(root), false);

    assert!(matches!(
        older.nodes().first(),
        Some(crate::node_arena::NodeRef::Char {
            font: NULL_FONT,
            ch: 'a',
            origin: crate::token::OriginId::UNKNOWN,
            ..
        })
    ));
    let stored = universe
        .box_reg_ref(0)
        .expect("box assignment should be live");
    let Some(crate::node_arena::NodeRef::HList(box_node)) = stored.nodes().first() else {
        panic!("stored value should be an hbox");
    };
    assert_eq!(
        stored
            .resolve(box_node.children)
            .expect("owned stored child")
            .nodes(),
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
            origin: crate::provenance::OriginRef::unknown(),
        }]);
    }

    assert!(universe.box_reg_ref(0).is_none());
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
        ] if before == "before" && *stream == slot && recorded.id() == tokens && after == "after"
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
    body: crate::token_store::TokenListRef,
    origins: crate::provenance::OriginListRef,
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
