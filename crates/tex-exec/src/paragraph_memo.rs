//! Recorder-driven paragraph front-end eligibility and transitional detached reuse.

use tex_lex::InputStack;
use tex_state::env::banks::{DimenParam, GlueParam, IntParam, TokParam};
use tex_state::{
    DetachedVirtualEffect, EffectRecord, MemoTimingPhase, MemoValueLimits, ParagraphRecordingPhase,
    ParagraphValidationFailure, PrintSink, Universe,
};

use crate::{ExecError, ExecutionContext, ModeNest};

const MAX_PARAGRAPH_DEPENDENCY_CACHE_ENTRIES: usize = 4_096;

struct ValidatedParagraphEntry {
    input: tex_lex::PreparedParagraphTransition,
    lines: tex_state::survivor::RetainedNodeList,
    provenance: std::sync::Arc<tex_state::ParagraphOriginResolver>,
}

#[cfg(feature = "profiling-stats")]
type PhaseStart = tex_state::ProfilingTimer;
#[cfg(not(feature = "profiling-stats"))]
struct PhaseStart;

#[inline]
fn start_phase() -> PhaseStart {
    #[cfg(feature = "profiling-stats")]
    {
        tex_state::World::start_profiling_timer()
    }
    #[cfg(not(feature = "profiling-stats"))]
    {
        PhaseStart
    }
}

#[inline]
fn finish_phase(stores: &mut Universe, phase: ParagraphRecordingPhase, started: PhaseStart) {
    #[cfg(feature = "profiling-stats")]
    stores.record_pure_paragraph_phase(phase, started.elapsed());
    #[cfg(not(feature = "profiling-stats"))]
    let _ = (stores, phase, started);
}

#[inline]
fn finish_memo_phase(stores: &mut Universe, phase: MemoTimingPhase, started: PhaseStart) {
    #[cfg(feature = "profiling-stats")]
    stores.record_pure_memo_timing(
        tex_state::PureMemoLayer::Paragraph,
        phase,
        started.elapsed(),
    );
    #[cfg(not(feature = "profiling-stats"))]
    let _ = (stores, phase, started);
}

pub(crate) fn try_reuse_aligned_paragraph(
    starting_span: Option<tex_state::RootSpanId>,
    starting_root_span: Option<tex_state::RootSpanId>,
    starting_input_identity: Option<u64>,
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut ExecutionContext<'_>,
) -> Result<bool, ExecError> {
    let Some(mut entry) = stores.align_recorded_paragraph_start(
        starting_span,
        starting_root_span,
        starting_input_identity,
    ) else {
        return Ok(false);
    };
    debug_assert!(entry.barriers.is_empty());
    let validation_started = start_phase();
    let validated = validate_paragraph_entry(
        &entry,
        input,
        stores,
        execution,
        starting_input_identity,
        // Every recorded outer paragraph crosses `start_paragraph`, which
        // resets the enclosing vertical prev_graf before line construction.
        // Validation runs just before that transition, so compare against the
        // value the cold path will actually observe rather than the stale
        // pre-start vertical value.
        0,
    );
    finish_memo_phase(stores, MemoTimingPhase::Validation, validation_started);
    let Some(ValidatedParagraphEntry {
        input: prepared_input,
        lines: retained_lines,
        provenance,
    }) = validated
    else {
        return Ok(false);
    };

    // Cold execution starts the paragraph before consuming or executing its
    // body. Preserve that ordering, but make the start speculative because a
    // changed prefix can make its parskip fire the output routine. In that
    // case normal dispatch must repeat the start from the original input and
    // pre-body state so the output routine observes cold-equivalent context.
    let nest_before_start = nest.clone();
    let effect_start = stores.world().effect_records().len();
    let mut start_probe = stores.begin_replay_probe();
    let start_result = crate::assignments::start_reused_paragraph(nest, input, &mut start_probe);
    let start_failure = if start_result.is_ok() {
        start_probe
            .page_fire_up()
            .is_some()
            .then_some(ParagraphValidationFailure::ParagraphStart)
            .or_else(|| {
                (start_probe.world().effect_records().len() != effect_start)
                    .then_some(ParagraphValidationFailure::Effect)
            })
    } else {
        None
    };
    if let Err(error) = start_result {
        drop(start_probe);
        *nest = nest_before_start;
        return Err(error);
    }
    if let Some(failure) = start_failure {
        drop(start_probe);
        *nest = nest_before_start;
        stores.record_pure_paragraph_validation_failure(failure);
        return Ok(false);
    }
    start_probe.commit();

    let input_origins = resolve_paragraph_provenance(stores, &entry.input_provenance);
    let input_suffix_token_lists = entry
        .input_suffix_token_lists
        .iter()
        .map(|tokens| stores.import_memo_token_list(tokens, MemoValueLimits::default()))
        .collect::<Result<Vec<_>, _>>()
        .expect("recorded paragraph input token list must remain importable");
    let transition_applied = input.apply_paragraph_transition(
        stores,
        prepared_input,
        &input_suffix_token_lists,
        &input_origins,
        &entry.input_provenance.origin_slots,
        &entry.input_origin_list_lengths,
    )?;
    assert!(
        transition_applied,
        "validated paragraph source transition must apply"
    );
    let current_input = input.publication_summary(stores);
    let _ = stores.finish_pure_paragraph_recording();
    replay_mutations(stores, &entry.mutations);
    replay_effects(stores, &entry.effects);
    let mount_started = start_phase();
    let mounted_lines = stores
        .mount_prevalidated_paragraph_result_lazy(&retained_lines, provenance)
        .expect("prevalidated paragraph line mount must remain valid");
    let lines = stores.nodes(mounted_lines).to_vec();
    finish_memo_phase(stores, MemoTimingPhase::Import, mount_started);
    execution.abandon_cold_paragraph_recording();
    entry.ending_input = current_input;
    entry.lines = Some(retained_lines);
    let line_count = entry.line_count;
    let line_last_badness = entry.line_last_badness;
    let mutation_count = entry.mutations.len();
    let delivered_tokens = entry.delivered_tokens;
    let continuation = if entry.display_active_directions.is_some() {
        crate::executor::ParagraphContinuation::Display
    } else {
        crate::executor::ParagraphContinuation::End
    };
    let display_active_directions = entry
        .display_active_directions
        .as_deref()
        .unwrap_or_default()
        .to_vec();
    stores.record_carried_paragraph(&entry);
    stores.record_paragraph_region(entry);
    execution.pending_paragraph_memo = None;
    let last_line = crate::assignments::install_reused_paragraph_hlist_after_start(
        nest,
        stores,
        execution,
        Vec::new(),
        Some((lines, line_count, line_last_badness)),
        continuation,
    )?;
    if continuation == crate::executor::ParagraphContinuation::Display {
        crate::math::enter_display_after_reused_paragraph(
            nest,
            input,
            stores,
            execution,
            last_line,
            display_active_directions,
        )?;
    }
    stores.record_pure_paragraph_hit(delivered_tokens, mutation_count);
    stores.record_pure_paragraph_line_hit();
    Ok(true)
}

/// Central fail-before-mutation validation boundary for an accepted paragraph.
/// Source/input alignment, exact entry identity, typed semantic fallback,
/// state-delta preconditions, effects, and retained line liveness are all
/// established before the caller advances input or replays state.
fn validate_paragraph_entry(
    entry: &tex_state::RecordedParagraphRegion,
    input: &InputStack,
    stores: &mut Universe,
    execution: &mut ExecutionContext<'_>,
    current_starting_input_identity: Option<u64>,
    current_prev_graf: i32,
) -> Option<ValidatedParagraphEntry> {
    let Some(observations) = entry.dependency_observations.as_deref() else {
        stores.record_pure_paragraph_validation_failure(ParagraphValidationFailure::RetainedResult);
        return None;
    };
    let prepared_input =
        entry
            .starting_root_span
            .zip(entry.ending_span)
            .and_then(|(start, end)| {
                input.prepare_paragraph_transition(
                    stores,
                    tex_lex::ParagraphSourceCoverage {
                        starting: start,
                        consumed: &entry.consumed_spans,
                        ending: end,
                    },
                    tex_lex::RecordedParagraphTransition::new(
                        entry.starting_input.as_ref(),
                        entry.starting_input_identity,
                        entry.input_transition_common_frames,
                        &entry.ending_input,
                    ),
                    current_starting_input_identity,
                )
            });
    let mutation_entry_class_changed = !same_mutation_entry_class(
        entry.mutation_entry_in_group,
        tex_state::ExpansionState::execution_group_depth(stores),
    );
    let dependency_failure = (!mutation_entry_class_changed)
        .then(|| {
            validate_paragraph_dependencies(
                stores,
                observations,
                &entry.dependency_ordinals,
                |key| paragraph_validation_value(stores, execution, key),
            )
        })
        .flatten();
    let validation_failure = mutation_entry_class_changed
        .then_some(ParagraphValidationFailure::Mutation)
        .or_else(|| dependency_failure.map(ParagraphValidationFailure::from_dependency))
        .or_else(|| {
            (!validate_mutations(stores, &entry.mutations))
                .then_some(ParagraphValidationFailure::Mutation)
        })
        .or_else(|| {
            (!validate_effects(&entry.effects)).then_some(ParagraphValidationFailure::Effect)
        })
        .or_else(|| {
            prepared_input
                .is_none()
                .then_some(ParagraphValidationFailure::InputTransition)
        });
    if let Some(failure) = validation_failure {
        stores.record_pure_paragraph_validation_failure(failure);
        return None;
    }
    if !validate_finished_lines(entry, observations, stores, execution, current_prev_graf) {
        return None;
    }
    let Some(lines) = entry.lines.clone() else {
        stores.record_pure_paragraph_validation_failure(ParagraphValidationFailure::RetainedResult);
        return None;
    };
    if !stores.can_mount_retained_paragraph_result(&lines) {
        stores.record_pure_paragraph_validation_failure(ParagraphValidationFailure::RetainedResult);
        return None;
    }
    let provenance = match &entry.line_provenance {
        tex_state::ParagraphLineProvenance::Accepted(resolver) => std::sync::Arc::clone(resolver),
        tex_state::ParagraphLineProvenance::Pending => {
            stores.record_pure_paragraph_validation_failure(
                ParagraphValidationFailure::RetainedResult,
            );
            return None;
        }
    };
    Some(ValidatedParagraphEntry {
        input: prepared_input?,
        lines,
        provenance,
    })
}

#[inline]
pub(crate) const fn same_mutation_entry_class(
    recorded_in_group: bool,
    execution_group_depth: u32,
) -> bool {
    recorded_in_group == (execution_group_depth != 0)
}

fn validate_finished_lines(
    entry: &tex_state::RecordedParagraphRegion,
    observations: &[tex_state::ObservedDependency],
    stores: &mut Universe,
    execution: &mut ExecutionContext<'_>,
    current_prev_graf: i32,
) -> bool {
    let line_validation_started = start_phase();
    let semantic_failure = validate_paragraph_dependencies(
        stores,
        observations,
        &entry.break_dependency_ordinals,
        |key| projected_break_validation_value(stores, execution, &entry.mutations, key),
    );
    let valid = entry
        .break_prev_graf
        .is_none_or(|expected| current_prev_graf == expected)
        && semantic_failure.is_none();
    finish_memo_phase(stores, MemoTimingPhase::Validation, line_validation_started);
    if !valid {
        stores
            .record_pure_paragraph_validation_failure(ParagraphValidationFailure::BreakDependency);
        // Finished lines are the only retained paragraph artifact. A real
        // line-breaking dependency change therefore makes the remainder of
        // this revision a cold run; retrying and rerecording at every
        // paragraph only adds linear overhead and cannot produce a hit.
        let abandoned = stores.abandon_pure_paragraph_recording();
        debug_assert!(
            abandoned,
            "paragraph validation owns a recording checkpoint"
        );
        execution.abandon_cold_paragraph_recording();
        stores.preserve_prior_paragraph_history();
        execution.paragraph_memo_disabled_for_run = true;
    }
    valid
}

fn validate_paragraph_dependencies(
    stores: &Universe,
    observations: &[tex_state::ObservedDependency],
    ordinals: &[u32],
    mut read_current: impl FnMut(tex_state::DependencyKey) -> tex_state::DependencyValue,
) -> Option<tex_state::DependencyKey> {
    ordinals.iter().find_map(|&ordinal| {
        let observation = observations
            .get(ordinal as usize)
            .expect("accepted paragraph observation ordinal is valid");
        let stamp_changed = stores.dependency_changed_at(observation.key) != observation.changed_at;
        (stamp_changed && read_current(observation.key) != observation.value)
            .then_some(observation.key)
    })
}

fn projected_break_validation_value(
    stores: &Universe,
    execution: &mut ExecutionContext<'_>,
    mutations: &[tex_state::PureParagraphMutation],
    key: tex_state::DependencyKey,
) -> tex_state::DependencyValue {
    if let Some(value) = mutations
        .iter()
        .rev()
        .find_map(|mutation| match (*mutation, key) {
            (
                tex_state::PureParagraphMutation::Count { index, value, .. },
                tex_state::DependencyKey::Cell {
                    bank: tex_state::DependencyBank::Count,
                    index: dependency_index,
                },
            ) if u32::from(index) == dependency_index => Some(value),
            (
                tex_state::PureParagraphMutation::IntParam { param, value, .. },
                tex_state::DependencyKey::Cell {
                    bank: tex_state::DependencyBank::IntParam,
                    index: dependency_index,
                },
            ) if u32::from(param.raw()) == dependency_index => Some(value),
            _ => None,
        })
    {
        return tex_state::DependencyValue::Integer(i64::from(value));
    }
    if let Some(font) = mutations
        .iter()
        .rev()
        .find_map(|mutation| match (*mutation, key) {
            (
                tex_state::PureParagraphMutation::CurrentFont { value_font, .. },
                tex_state::DependencyKey::Cell {
                    bank: tex_state::DependencyBank::CurrentFont,
                    index: 0,
                },
            ) => Some(value_font),
            _ => None,
        })
    {
        return stores.semantic_font_dependency_value(font);
    }
    paragraph_validation_value(stores, execution, key)
}

fn validate_effects(effects: &[DetachedVirtualEffect]) -> bool {
    effects.iter().all(|effect| {
        std::str::from_utf8(&effect.payload).is_ok()
            && matches!(
                (effect.operation.as_str(), effect.stream),
                ("terminal" | "log" | "terminal-and-log", None) | ("stream", Some(_))
            )
    })
}

fn validate_mutations(stores: &Universe, mutations: &[tex_state::PureParagraphMutation]) -> bool {
    let mut seen = ahash::AHashSet::new();
    mutations.iter().all(|mutation| {
        let key = match *mutation {
            tex_state::PureParagraphMutation::Count { index, .. } => (0_u8, index),
            tex_state::PureParagraphMutation::IntParam { param, .. } => (1_u8, param.raw()),
            tex_state::PureParagraphMutation::CurrentFont { .. } => (2_u8, 0),
        };
        if !seen.insert(key) {
            return true;
        }
        match *mutation {
            tex_state::PureParagraphMutation::Count {
                index, expected, ..
            } => stores.count(index) == expected,
            tex_state::PureParagraphMutation::IntParam {
                param, expected, ..
            } => stores.int_param(param) == expected,
            tex_state::PureParagraphMutation::CurrentFont {
                expected_font,
                expected_symbol,
                ..
            } => font_selector_matches(stores, expected_font, expected_symbol),
        }
    })
}

fn replay_mutations(stores: &mut Universe, mutations: &[tex_state::PureParagraphMutation]) {
    for mutation in mutations {
        match *mutation {
            tex_state::PureParagraphMutation::Count {
                index,
                value,
                global,
                ..
            } => {
                if global {
                    stores.set_count_global(index, value);
                } else {
                    stores.set_count(index, value);
                }
            }
            tex_state::PureParagraphMutation::IntParam {
                param,
                value,
                global,
                ..
            } => {
                if global {
                    stores.set_int_param_global(param, value);
                } else {
                    stores.set_int_param(param, value);
                }
            }
            tex_state::PureParagraphMutation::CurrentFont {
                value_font,
                value_symbol,
                global,
                ..
            } => match (global, value_symbol) {
                (true, Some(symbol)) => {
                    stores.set_current_font_selector_global(symbol, value_font);
                }
                (false, Some(symbol)) => stores.set_current_font_selector(symbol, value_font),
                (true, None) => stores.set_current_font_global(value_font),
                (false, None) => stores.set_current_font(value_font),
            },
        }
    }
}

fn font_selector_matches(
    stores: &Universe,
    expected_font: tex_state::ids::FontId,
    expected_symbol: Option<tex_state::interner::Symbol>,
) -> bool {
    stores.semantic_font_dependency_value(stores.current_font())
        == stores.semantic_font_dependency_value(expected_font)
        && match (
            stores.current_font_symbol().map(|symbol| symbol.symbol()),
            expected_symbol,
        ) {
            (Some(current), Some(expected)) => {
                stores.control_sequence_kind(current) == stores.control_sequence_kind(expected)
                    && stores.resolve(current) == stores.resolve(expected)
            }
            (None, None) => true,
            (Some(_), None) | (None, Some(_)) => false,
        }
}

fn replay_effects(stores: &mut Universe, effects: &[DetachedVirtualEffect]) {
    for effect in effects {
        let Ok(text) = std::str::from_utf8(&effect.payload) else {
            continue;
        };
        let sink = match effect.operation.as_str() {
            "terminal" => PrintSink::Terminal,
            "log" => PrintSink::Log,
            "terminal-and-log" => PrintSink::TerminalAndLog,
            "stream" => PrintSink::Stream(tex_state::StreamSlot::new(effect.stream.unwrap_or(0))),
            _ => continue,
        };
        stores.world_mut().write_text(sink, text);
    }
}

fn resolve_paragraph_provenance(
    stores: &mut Universe,
    recipe: &tex_state::ParagraphProvenanceRecipe,
) -> Vec<tex_state::token::OriginId> {
    recipe
        .root_spans
        .iter()
        .map(|span| {
            let Some(anchor) = usize::try_from(span.piece)
                .ok()
                .and_then(|piece| recipe.piece_anchors.get(piece))
            else {
                return tex_state::token::OriginId::UNKNOWN;
            };
            stores
                .origin_for_root_span(anchor.with_offsets(span.start, span.end))
                .unwrap_or(tex_state::token::OriginId::UNKNOWN)
        })
        .collect()
}

pub(crate) fn provenance_recipe_for_origins(
    stores: &Universe,
    origins: impl IntoIterator<Item = tex_state::token::OriginId>,
) -> tex_state::ParagraphProvenanceRecipe {
    let mut recipe = ParagraphProvenanceBuilder::default();
    for origin in origins {
        recipe.push_origin(stores, origin);
    }
    recipe.finish()
}

#[derive(Default)]
struct ParagraphProvenanceBuilder {
    piece_anchors: Vec<tex_state::RootSpanId>,
    root_spans: Vec<tex_state::ParagraphProvenanceSpan>,
    origin_slots: Vec<u32>,
    root_ordinals: ahash::AHashMap<tex_state::RootSpanId, u32>,
    piece_ordinals: ahash::AHashMap<tex_state::PieceId, u32>,
}

impl ParagraphProvenanceBuilder {
    fn push_origin(&mut self, stores: &Universe, origin: tex_state::token::OriginId) {
        let Some(span) = stores.root_span_for_origin(origin) else {
            self.origin_slots.push(u32::MAX);
            return;
        };
        let ordinal = if let Some(&ordinal) = self.root_ordinals.get(&span) {
            ordinal
        } else {
            let Ok(ordinal) = u32::try_from(self.root_spans.len()) else {
                self.origin_slots.push(u32::MAX);
                return;
            };
            let piece = span.piece();
            let piece_ordinal = if let Some(&piece_ordinal) = self.piece_ordinals.get(&piece) {
                piece_ordinal
            } else {
                let Ok(piece_ordinal) = u32::try_from(self.piece_anchors.len()) else {
                    self.origin_slots.push(u32::MAX);
                    return;
                };
                self.piece_anchors.push(span.start_anchor());
                self.piece_ordinals.insert(piece, piece_ordinal);
                piece_ordinal
            };
            self.root_spans.push(tex_state::ParagraphProvenanceSpan {
                piece: piece_ordinal,
                start: span.start(),
                end: span.end(),
            });
            self.root_ordinals.insert(span, ordinal);
            ordinal
        };
        self.origin_slots.push(ordinal);
    }

    fn finish(self) -> tex_state::ParagraphProvenanceRecipe {
        tex_state::ParagraphProvenanceRecipe {
            piece_anchors: self.piece_anchors.into(),
            root_spans: self.root_spans.into(),
            origin_slots: self.origin_slots.into(),
            node_slots: std::sync::Arc::from([]),
        }
    }
}

fn paragraph_input_provenance(
    stores: &Universe,
    frames: &[tex_state::InputFrameSummary],
) -> (tex_state::ParagraphProvenanceRecipe, Vec<u32>) {
    let mut recipe = ParagraphProvenanceBuilder::default();
    let mut origin_list_lengths = Vec::new();
    for frame in frames {
        match frame {
            tex_state::InputFrameSummary::TokenList {
                origin_list,
                macro_arguments,
                macro_invocation,
                parent_macro_invocation,
                ..
            } => {
                let origins = stores.origin_list(*origin_list);
                origin_list_lengths.push(u32::try_from(origins.len()).unwrap_or(u32::MAX));
                for &origin in origins {
                    recipe.push_origin(stores, origin);
                }
                for &word in macro_arguments.tokens().iter() {
                    recipe.push_origin(stores, word.origin());
                }
                recipe.push_origin(stores, *macro_invocation);
                recipe.push_origin(stores, *parent_macro_invocation);
            }
            tex_state::InputFrameSummary::TransientTokenList {
                tokens,
                macro_invocation,
                parent_macro_invocation,
                ..
            } => {
                for &word in tokens.iter() {
                    recipe.push_origin(stores, word.origin());
                }
                recipe.push_origin(stores, *macro_invocation);
                recipe.push_origin(stores, *parent_macro_invocation);
            }
            tex_state::InputFrameSummary::Condition { condition, .. } => {
                recipe.push_origin(stores, condition.context().origin());
            }
            tex_state::InputFrameSummary::Source { .. } => {
                unreachable!("paragraph input transition suffix cannot contain a source frame")
            }
        }
    }
    (recipe.finish(), origin_list_lengths)
}

pub(crate) fn publish_prepared_hlist(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut ExecutionContext<'_>,
    nodes: &[tex_state::node::Node],
    prev_graf: i32,
    continuation: crate::executor::ParagraphContinuation,
) {
    publish_recorded_region(input, stores, execution, continuation);
    if execution.pending_paragraph_memo.is_none() {
        return;
    }
    let dependencies_started = start_phase();
    let Some(break_dependency_ordinals) = paragraph_break_dependencies(stores, execution, nodes)
    else {
        execution.pending_paragraph_memo = None;
        return;
    };
    finish_phase(
        stores,
        ParagraphRecordingPhase::BreakDependencies,
        dependencies_started,
    );
    let prev_graf = (!stores.paragraph_shape().is_empty()
        || stores.dimen_param(DimenParam::HANG_INDENT).raw() != 0)
        .then_some(prev_graf);
    execution.pending_paragraph_memo = Some(crate::executor::PendingParagraphMemo {
        break_dependency_ordinals,
        prev_graf,
        continuation,
    });
}

fn publish_recorded_region(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut ExecutionContext<'_>,
    continuation: crate::executor::ParagraphContinuation,
) {
    let Some(mut recording) = execution.cold_paragraph_recording.take() else {
        let _ = stores.finish_pure_paragraph_recording();
        return;
    };
    let (mut keys, expansion_barriers) = execution.finish_paragraph_expansion_recording();
    keys.append(&mut recording.dependencies);
    keys.retain(|key| {
        let tex_state::DependencyKey::Query { domain, .. } = key else {
            return true;
        };
        let reason = match *domain {
            tex_expand::PARAGRAPH_SCANTOKENS_BARRIER_DOMAIN => {
                Some(tex_state::ParagraphBarrierReason::Scantokens)
            }
            tex_expand::PARAGRAPH_INPUT_OPEN_BARRIER_DOMAIN => {
                Some(tex_state::ParagraphBarrierReason::MidParagraphInputOpen)
            }
            tex_expand::PARAGRAPH_END_INPUT_BARRIER_DOMAIN => {
                Some(tex_state::ParagraphBarrierReason::EndInput)
            }
            _ => None,
        };
        if let Some(reason) = reason {
            recording.barriers.insert(reason);
            false
        } else {
            true
        }
    });
    for barrier in expansion_barriers {
        recording.barriers.insert(match barrier {
            tex_expand::ParagraphExpansionBarrier::InputOpen => {
                tex_state::ParagraphBarrierReason::MidParagraphInputOpen
            }
            tex_expand::ParagraphExpansionBarrier::EndInput => {
                tex_state::ParagraphBarrierReason::EndInput
            }
            tex_expand::ParagraphExpansionBarrier::Scantokens => {
                tex_state::ParagraphBarrierReason::Scantokens
            }
        });
    }
    if let Some(reads) = &mut recording.inline_math {
        append_inline_math_dependency_keys(stores, reads, &mut keys);
    }
    let input_started = start_phase();
    let effects = match detach_effects(&stores.world().effect_records()[recording.effect_start..]) {
        Some(effects) => effects,
        None => {
            recording
                .barriers
                .insert(tex_state::ParagraphBarrierReason::UntrackedWorldAccess);
            Vec::new()
        }
    };
    let mutation_summary = stores
        .finish_pure_paragraph_recording()
        .expect("cold paragraph recording has matching state checkpoint");
    let ending_span = input.root_source_checkpoint_anchor(stores);
    let consumed_spans = recording
        .starting_root_span
        .zip(ending_span)
        .and_then(|(start, end)| input.root_source_coverage(start, end, stores));
    let consumed_spans = match consumed_spans {
        Some(spans) => spans,
        None => {
            recording
                .barriers
                .insert(tex_state::ParagraphBarrierReason::UnsupportedInputTransition);
            Vec::new()
        }
    };
    let ending_input = input.publication_summary(stores);
    let ending_group_depth = tex_state::ExpansionState::execution_group_depth(stores);
    // Balanced child groups are represented by the exact live-group setter
    // script. Replacing or attaching payload to the entry frame itself is not.
    if recording.starting_group_depth != ending_group_depth
        || (recording.starting_group_depth != 0 && mutation_summary.unsupported_group_ownership)
    {
        recording
            .barriers
            .insert(tex_state::ParagraphBarrierReason::UnsupportedGroupTransition);
    }
    let input_transition_prefix = recording.starting_input.as_ref().map_or_else(
        // An anchored probe starts in the sole root source frame. Physical
        // root coverage above proves that frame's cursor transition; any
        // additional ending frames were introduced by the paragraph and are
        // handled by the existing suffix detachment/rebinding path.
        || Some(1),
        |starting| starting.paragraph_cursor_transition_prefix_to(&ending_input),
    );
    if input_transition_prefix.is_none() {
        recording
            .barriers
            .insert(tex_state::ParagraphBarrierReason::UnsupportedInputTransition);
    }
    finish_phase(
        stores,
        ParagraphRecordingPhase::InputTransition,
        input_started,
    );
    if stores.int_param(IntParam::PDF_ADJUST_SPACING) != 0
        || stores.int_param(IntParam::PDF_PROTRUDE_CHARS) != 0
        || stores.int_param(IntParam::PDF_ADJUST_INTERWORD_GLUE) != 0
        || stores.int_param(IntParam::PDF_PREPEND_KERN) != 0
        || stores.int_param(IntParam::PDF_APPEND_KERN) != 0
    {
        return;
    }
    if !recording.barriers.is_empty() {
        let barriers = recording.barriers.into_iter().collect::<Vec<_>>();
        stores.record_pure_paragraph_barriers(&barriers);
        return;
    }
    let input_transition_prefix = input_transition_prefix.expect("barrier-free input transition");
    let (input_provenance, input_origin_list_lengths) =
        paragraph_input_provenance(stores, &ending_input.frames()[input_transition_prefix..]);
    let input_suffix_token_lists = ending_input.frames()[input_transition_prefix..]
        .iter()
        .filter_map(|frame| match frame {
            tex_state::InputFrameSummary::TokenList { token_list, .. } => {
                Some(stores.detach_token_list(*token_list))
            }
            tex_state::InputFrameSummary::Source { .. }
            | tex_state::InputFrameSummary::TransientTokenList { .. }
            | tex_state::InputFrameSummary::Condition { .. } => None,
        })
        .collect::<Result<Vec<_>, _>>()
        .expect("live paragraph input token list must remain detachable");
    let dependency_started = start_phase();
    keys.extend([
        tex_state::DependencyKey::Cell {
            bank: tex_state::DependencyBank::CurrentFont,
            index: 0,
        },
        tex_state::DependencyKey::Cell {
            bank: tex_state::DependencyBank::TokParam,
            index: u32::from(TokParam::EVERY_PAR.raw()),
        },
        tex_state::DependencyKey::Cell {
            bank: tex_state::DependencyBank::DimenParam,
            index: u32::from(DimenParam::PAR_INDENT.raw()),
        },
        tex_state::DependencyKey::Cell {
            bank: tex_state::DependencyBank::GlueParam,
            index: u32::from(GlueParam::SPACE_SKIP.raw()),
        },
        tex_state::DependencyKey::Cell {
            bank: tex_state::DependencyBank::GlueParam,
            index: u32::from(GlueParam::XSPACE_SKIP.raw()),
        },
    ]);
    keys.sort_unstable();
    keys.dedup();
    let dependencies = keys
        .into_iter()
        .map(|key| paragraph_observed_dependency(stores, execution, key))
        .collect::<Option<Vec<_>>>();
    let Some(dependencies) = dependencies else {
        let barriers = [tex_state::ParagraphBarrierReason::UnsupportedInputTransition];
        stores.record_pure_paragraph_barriers(&barriers);
        finish_phase(
            stores,
            ParagraphRecordingPhase::FrontEndDependencies,
            dependency_started,
        );
        return;
    };
    finish_phase(
        stores,
        ParagraphRecordingPhase::FrontEndDependencies,
        dependency_started,
    );
    let publication_started = start_phase();
    stores.record_paragraph_region(tex_state::RecordedParagraphRegion {
        starting_span: recording.starting_span,
        starting_root_span: recording.starting_root_span,
        starting_input: recording.starting_input,
        starting_input_identity: recording.starting_input_identity,
        ending_span,
        consumed_spans: consumed_spans.into(),
        delivered_tokens: recording.delivered_tokens,
        dependency_ordinals: dependencies.into(),
        dependency_observations: None,
        mutation_entry_in_group: mutation_summary.entry_in_group,
        mutations: mutation_summary.mutations.into(),
        effects: effects.into(),
        ending_input,
        input_transition_common_frames: u32::try_from(input_transition_prefix)
            .expect("input frame count must fit u32"),
        input_provenance,
        input_origin_list_lengths: input_origin_list_lengths.into(),
        input_suffix_token_lists: input_suffix_token_lists.into(),
        barriers: recording.barriers.into_iter().collect::<Vec<_>>().into(),
        break_dependency_ordinals: Vec::new().into(),
        break_prev_graf: None,
        lines: None,
        line_count: 0,
        line_last_badness: 0,
        display_active_directions: (continuation
            == crate::executor::ParagraphContinuation::Display)
            .then(|| std::sync::Arc::from([])),
        line_provenance: tex_state::ParagraphLineProvenance::Pending,
    });
    finish_phase(
        stores,
        ParagraphRecordingPhase::RegionPublication,
        publication_started,
    );
    if execution.pending_paragraph_memo.is_none() {
        execution.pending_paragraph_memo = Some(crate::executor::PendingParagraphMemo {
            break_dependency_ordinals: Vec::new(),
            prev_graf: None,
            continuation,
        });
    }
}

pub(crate) fn publish_finished_lines(
    stores: &mut Universe,
    execution: &mut ExecutionContext<'_>,
    nodes: &[tex_state::node::Node],
    line_count: i32,
    active_directions: &[tex_state::node::Direction],
) {
    let Some(pending) = execution.pending_paragraph_memo.take() else {
        return;
    };
    let list = stores.freeze_node_list(nodes);
    let retention_started = start_phase();
    let retained = stores.retain_paragraph_result(list);
    finish_phase(
        stores,
        ParagraphRecordingPhase::LineRetention,
        retention_started,
    );
    let publication_started = start_phase();
    let last_badness = stores.last_badness();
    let display_active_directions = match pending.continuation {
        crate::executor::ParagraphContinuation::End => None,
        crate::executor::ParagraphContinuation::Display => Some(active_directions.into()),
    };
    stores.finish_recorded_paragraph_lines(tex_state::RecordedParagraphLines {
        dependency_ordinals: pending.break_dependency_ordinals,
        prev_graf: pending.prev_graf,
        lines: retained,
        line_count,
        last_badness,
        display_active_directions,
    });
    finish_phase(
        stores,
        ParagraphRecordingPhase::RegionPublication,
        publication_started,
    );
}

fn append_inline_math_dependency_keys(
    stores: &Universe,
    reads: &mut crate::executor::InlineMathReads,
    keys: &mut Vec<tex_state::DependencyKey>,
) {
    use tex_state::{
        DependencyBank as Bank, DependencyCodeTable as Code, DependencyFontField as Font,
        DependencyKey as Key,
    };

    reads.mathcodes.sort_unstable();
    reads.mathcodes.dedup();
    keys.extend(reads.mathcodes.iter().map(|ch| Key::Code {
        table: Code::Mathcode,
        scalar: u32::from(*ch),
    }));
    reads.delcodes.sort_unstable();
    reads.delcodes.dedup();
    keys.extend(reads.delcodes.iter().map(|ch| Key::Code {
        table: Code::Delcode,
        scalar: u32::from(*ch),
    }));
    for param in [
        IntParam::FAM,
        IntParam::DELIMITER_FACTOR,
        IntParam::BIN_OP_PENALTY,
        IntParam::REL_PENALTY,
    ] {
        keys.push(Key::Cell {
            bank: Bank::IntParam,
            index: u32::from(param.raw()),
        });
    }
    for param in [
        DimenParam::MATH_SURROUND,
        DimenParam::DELIMITER_SHORTFALL,
        DimenParam::NULL_DELIMITER_SPACE,
        // TeX's `\scriptspace` slot has no named bank constant yet.
        DimenParam::new(12),
    ] {
        keys.push(Key::Cell {
            bank: Bank::DimenParam,
            index: u32::from(param.raw()),
        });
    }
    for param in [
        // TeX's thin/medium/thick math glue slots.
        GlueParam::new(15),
        GlueParam::new(16),
        GlueParam::new(17),
    ] {
        keys.push(Key::Cell {
            bank: Bank::GlueParam,
            index: u32::from(param.raw()),
        });
    }
    keys.push(Key::Cell {
        bank: Bank::TokParam,
        index: u32::from(TokParam::EVERY_MATH.raw()),
    });

    let sizes = [
        tex_state::math::MathFontSize::Text,
        tex_state::math::MathFontSize::Script,
        tex_state::math::MathFontSize::ScriptScript,
    ];
    let mut fonts = Vec::with_capacity(reads.family_mask.count_ones() as usize);
    for index in 0_u32..48 {
        if reads.family_mask & (1_u64 << index) == 0 {
            continue;
        }
        keys.push(Key::Cell {
            bank: Bank::MathFamilyFont,
            index,
        });
        let size = sizes[usize::try_from(index / 16).expect("math size index fits usize")];
        let family = u8::try_from(index % 16).expect("math family index fits u8");
        fonts.push(stores.math_family_font(size, family));
    }
    fonts.sort_unstable_by_key(|font| font.raw());
    fonts.dedup();
    for font in fonts {
        for field in [Font::Metrics, Font::Parameters, Font::SkewChar] {
            keys.push(Key::Font {
                field,
                font: font.raw(),
                index: 0,
            });
        }
    }
}

fn paragraph_break_dependencies(
    stores: &mut Universe,
    execution: &mut ExecutionContext<'_>,
    nodes: &[tex_state::node::Node],
) -> Option<Vec<u32>> {
    use tex_state::{
        DependencyBank as Bank, DependencyCodeTable as Code, DependencyEngineField as Engine,
        DependencyFontField as Font, DependencyKey as Key,
    };

    let discovery_started = start_phase();
    let mut keys = Vec::new();
    for param in [
        IntParam::PRETOLERANCE,
        IntParam::TOLERANCE,
        IntParam::LINE_PENALTY,
        IntParam::HYPHEN_PENALTY,
        IntParam::EX_HYPHEN_PENALTY,
        IntParam::ADJ_DEMERITS,
        IntParam::DOUBLE_HYPHEN_DEMERITS,
        IntParam::FINAL_HYPHEN_DEMERITS,
        IntParam::LAST_LINE_FIT,
        IntParam::LOOSENESS,
        IntParam::INTERLINE_PENALTY,
        IntParam::CLUB_PENALTY,
        IntParam::WIDOW_PENALTY,
        IntParam::BROKEN_PENALTY,
        IntParam::HBADNESS,
        IntParam::HANG_AFTER,
        IntParam::TRACING_LOST_CHARS,
        IntParam::LANGUAGE,
        IntParam::LEFT_HYPHEN_MIN,
        IntParam::RIGHT_HYPHEN_MIN,
        IntParam::UC_HYPH,
        IntParam::PDF_ADJUST_SPACING,
        IntParam::PDF_PROTRUDE_CHARS,
        IntParam::PDF_ADJUST_INTERWORD_GLUE,
        IntParam::PDF_PREPEND_KERN,
        IntParam::PDF_APPEND_KERN,
    ] {
        keys.push(Key::Cell {
            bank: Bank::IntParam,
            index: u32::from(param.raw()),
        });
    }
    for param in [
        DimenParam::EMERGENCY_STRETCH,
        DimenParam::H_SIZE,
        DimenParam::HANG_INDENT,
        DimenParam::HFUZZ,
        DimenParam::OVERFULL_RULE,
        DimenParam::PDF_IGNORED_DIMEN,
        DimenParam::PDF_FIRST_LINE_HEIGHT,
        DimenParam::PDF_LAST_LINE_DEPTH,
        DimenParam::PDF_EACH_LINE_HEIGHT,
        DimenParam::PDF_EACH_LINE_DEPTH,
    ] {
        keys.push(Key::Cell {
            bank: Bank::DimenParam,
            index: u32::from(param.raw()),
        });
    }
    for param in [
        GlueParam::LEFT_SKIP,
        GlueParam::RIGHT_SKIP,
        GlueParam::PAR_FILL_SKIP,
    ] {
        keys.push(Key::Cell {
            bank: Bank::GlueParam,
            index: u32::from(param.raw()),
        });
    }
    keys.push(Key::Engine(Engine::ParShape));
    keys.push(Key::Engine(Engine::PenaltyArrays));

    let mut languages = vec![0_u8];
    let mut fonts = Vec::new();
    if collect_break_graph_facts(stores, nodes, &mut languages, &mut fonts) {
        return None;
    }
    fonts.sort_unstable_by_key(|font| font.raw());
    fonts.dedup();
    for font in fonts {
        for (field, index) in [
            (Font::Metrics, 0),
            (Font::HyphenChar, 0),
            (Font::Parameters, 0),
            (Font::PdfShaping, 0),
        ] {
            keys.push(Key::Font {
                field,
                font: font.raw(),
                index,
            });
        }
    }
    for table in [Code::Lccode, Code::Sfcode] {
        keys.push(Key::CodeGeneration(table));
    }
    languages.sort_unstable();
    languages.dedup();
    for language in languages {
        keys.push(Key::HyphenationPatterns(language));
        keys.push(Key::HyphenationExceptions(language));
        keys.push(Key::HyphenationCodes(language));
    }
    keys.sort_unstable();
    keys.dedup();
    finish_phase(
        stores,
        ParagraphRecordingPhase::BreakKeyDiscovery,
        discovery_started,
    );
    let registration_started = start_phase();
    let tracked = keys
        .into_iter()
        .map(|key| (key, stores.track_dependency(key)))
        .collect::<Vec<_>>();
    finish_phase(
        stores,
        ParagraphRecordingPhase::BreakStampRegistration,
        registration_started,
    );
    let projection_started = start_phase();
    let dependencies = tracked
        .into_iter()
        .map(|(key, changed_at)| {
            paragraph_observation_for_stamp(stores, execution, key, changed_at)
        })
        .collect();
    finish_phase(
        stores,
        ParagraphRecordingPhase::BreakValueProjection,
        projection_started,
    );
    dependencies
}

/// Collects facts for the complete supported horizontal graph and returns
/// true for construction whose executor-owned inputs are not modeled yet.
fn collect_break_graph_facts(
    stores: &Universe,
    nodes: &[tex_state::node::Node],
    languages: &mut Vec<u8>,
    fonts: &mut Vec<tex_state::ids::FontId>,
) -> bool {
    let child = |list, languages: &mut Vec<u8>, fonts: &mut Vec<_>| {
        collect_frozen_break_graph_facts(stores, list, languages, fonts)
    };
    for node in nodes {
        match node {
            tex_state::node::Node::Char { font, .. } => {
                fonts.push(*font);
            }
            tex_state::node::Node::Lig { font, .. } => {
                fonts.push(*font);
            }
            tex_state::node::Node::Whatsit(tex_state::node::Whatsit::Language {
                language, ..
            }) => languages.push(*language),
            tex_state::node::Node::HList(node) => {
                if child(node.children, languages, fonts) {
                    return true;
                }
            }
            // `\vadjust` payloads migrate out before line construction. Their
            // already-built graph is retained with the finished result, but
            // it contributes no horizontal break facts.
            tex_state::node::Node::Adjust(_) => {}
            // A built vertical box contributes only its fixed dimensions to
            // horizontal line breaking. Its child graph is retained verbatim
            // and was already covered by front-end or external-box reads.
            tex_state::node::Node::VList(_) => {}
            tex_state::node::Node::Ins { .. } | tex_state::node::Node::Unset(_) => return true,
            tex_state::node::Node::Glue {
                leader: Some(_), ..
            } => return true,
            tex_state::node::Node::Disc {
                pre, post, replace, ..
            } => {
                for list in [pre, post, replace] {
                    if child(*list, languages, fonts) {
                        return true;
                    }
                }
            }
            _ => {}
        }
    }
    false
}

fn collect_frozen_break_graph_facts(
    stores: &Universe,
    list: tex_state::ids::NodeListId,
    languages: &mut Vec<u8>,
    fonts: &mut Vec<tex_state::ids::FontId>,
) -> bool {
    for node in stores.nodes(list) {
        match node {
            tex_state::node_arena::NodeRef::Char { font, .. } => {
                fonts.push(font);
            }
            tex_state::node_arena::NodeRef::Lig { font, .. } => {
                fonts.push(font);
            }
            tex_state::node_arena::NodeRef::Whatsit(tex_state::node::Whatsit::Language {
                language,
                ..
            }) => languages.push(*language),
            tex_state::node_arena::NodeRef::HList(node) => {
                if collect_frozen_break_graph_facts(stores, node.children, languages, fonts) {
                    return true;
                }
            }
            // As above, an already-frozen vertical box is an opaque fixed-size
            // break item; its descendants cannot affect line construction.
            tex_state::node_arena::NodeRef::VList(_) => {}
            tex_state::node_arena::NodeRef::Ins { .. }
            | tex_state::node_arena::NodeRef::Adjust(_)
            | tex_state::node_arena::NodeRef::Unset(_) => return true,
            tex_state::node_arena::NodeRef::Glue {
                leader: Some(_), ..
            } => return true,
            tex_state::node_arena::NodeRef::Disc {
                pre, post, replace, ..
            } => {
                for child in [pre, post, replace] {
                    if collect_frozen_break_graph_facts(stores, child, languages, fonts) {
                        return true;
                    }
                }
            }
            _ => {}
        }
    }
    false
}

fn paragraph_validation_value(
    stores: &Universe,
    execution: &mut ExecutionContext<'_>,
    key: tex_state::DependencyKey,
) -> tex_state::DependencyValue {
    let changed_at = stores.dependency_changed_at(key);
    if let Some(cached) = execution.paragraph_dependency_cache.get(&key)
        && cached.observation.changed_at == changed_at
    {
        return cached.observation.value.clone();
    }
    let value = paragraph_semantic_dependency_value(stores, key)
        .unwrap_or(tex_state::DependencyValue::Absent);
    if execution.paragraph_dependency_cache.len() < MAX_PARAGRAPH_DEPENDENCY_CACHE_ENTRIES {
        execution.paragraph_dependency_cache.insert(
            key,
            crate::executor::CachedParagraphDependency {
                observation: tex_state::ObservedDependency {
                    key,
                    changed_at,
                    value: value.clone(),
                },
                recorded_ordinal: None,
            },
        );
    }
    value
}

fn paragraph_observed_dependency(
    stores: &mut Universe,
    execution: &mut ExecutionContext<'_>,
    key: tex_state::DependencyKey,
) -> Option<u32> {
    let changed_at = stores.track_dependency(key);
    paragraph_observation_for_stamp(stores, execution, key, changed_at)
}

fn paragraph_observation_for_stamp(
    stores: &mut Universe,
    execution: &mut ExecutionContext<'_>,
    key: tex_state::DependencyKey,
    changed_at: tex_state::ChangedAt,
) -> Option<u32> {
    if let Some(cached) = execution.paragraph_dependency_cache.get(&key)
        && cached.observation.changed_at == changed_at
    {
        if let Some(ordinal) = cached.recorded_ordinal {
            return Some(ordinal);
        }
        return Some(intern_paragraph_observation(
            stores,
            execution,
            cached.observation.clone(),
        ));
    }
    let observed = tex_state::ObservedDependency {
        key,
        changed_at,
        value: paragraph_semantic_dependency_value(stores, key)?,
    };
    Some(intern_paragraph_observation(stores, execution, observed))
}

fn intern_paragraph_observation(
    stores: &mut Universe,
    execution: &mut ExecutionContext<'_>,
    observed: tex_state::ObservedDependency,
) -> u32 {
    if let Some(cached) = execution.paragraph_dependency_cache.get(&observed.key)
        && cached.observation.changed_at == observed.changed_at
        && let Some(ordinal) = cached.recorded_ordinal
    {
        return ordinal;
    }
    let ordinal = stores.record_paragraph_observation(observed.clone());
    if let Some(cached) = execution.paragraph_dependency_cache.get_mut(&observed.key)
        && cached.observation.changed_at == observed.changed_at
    {
        cached.recorded_ordinal = Some(ordinal);
    } else if execution.paragraph_dependency_cache.len() < MAX_PARAGRAPH_DEPENDENCY_CACHE_ENTRIES {
        execution.paragraph_dependency_cache.insert(
            observed.key,
            crate::executor::CachedParagraphDependency {
                observation: observed,
                recorded_ordinal: Some(ordinal),
            },
        );
    }
    ordinal
}

fn paragraph_semantic_dependency_value(
    stores: &Universe,
    key: tex_state::DependencyKey,
) -> Option<tex_state::DependencyValue> {
    if key == tex_state::DependencyKey::Engine(tex_state::DependencyEngineField::Mode) {
        // Paragraph candidates are recorded and validated only at the outer
        // vertical boundary. Mode changes within the retained body are then
        // reproduced deterministically by `start_paragraph`.
        return Some(tex_state::DependencyValue::Integer(0));
    }
    stores.semantic_dependency_value(key)
}

fn detach_effects(records: &[EffectRecord]) -> Option<Vec<DetachedVirtualEffect>> {
    records
        .iter()
        .map(|record| match record {
            EffectRecord::StreamWrite { sink, text } => {
                let (operation, stream) = match sink {
                    PrintSink::Terminal => ("terminal", None),
                    PrintSink::Log => ("log", None),
                    PrintSink::TerminalAndLog => ("terminal-and-log", None),
                    PrintSink::Stream(stream) => ("stream", Some(stream.raw())),
                };
                Some(DetachedVirtualEffect {
                    operation: operation.to_owned(),
                    stream,
                    payload: text.as_bytes().to_vec(),
                })
            }
            EffectRecord::StreamOpen { .. }
            | EffectRecord::StreamClose { .. }
            | EffectRecord::DeferredWrite { .. }
            | EffectRecord::Special { .. }
            | EffectRecord::PdfObjectPlaceholder { .. }
            | EffectRecord::ShellEscape(_) => None,
        })
        .collect()
}
