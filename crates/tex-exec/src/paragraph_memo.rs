//! Recorder-driven paragraph front-end eligibility and transitional detached reuse.

use tex_lex::InputStack;
use tex_state::env::banks::{DimenParam, GlueParam, IntParam, TokParam};
use tex_state::{
    DetachedVirtualEffect, EffectRecord, MemoTimingPhase, ParagraphRecordingPhase,
    ParagraphValidationFailure, PrintSink, PureMemoLayer, Universe,
};

use crate::{ExecError, ExecutionContext, ExecutionStats, ModeNest};

const MAX_PARAGRAPH_DEPENDENCY_CACHE_ENTRIES: usize = 4_096;

struct ValidatedParagraphEntry {
    input: tex_lex::PreparedParagraphTransition,
    retained: tex_state::survivor::RetainedNodeList,
    relaxed_state: bool,
}

#[cfg(feature = "profiling-stats")]
type PhaseStart = std::time::Instant;
#[cfg(not(feature = "profiling-stats"))]
struct PhaseStart;

#[inline]
fn start_phase() -> PhaseStart {
    #[cfg(feature = "profiling-stats")]
    {
        std::time::Instant::now()
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

pub(crate) fn try_reuse_aligned_paragraph(
    starting_span: Option<tex_state::RootSpanId>,
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut ExecutionContext<'_>,
    _stats: &mut ExecutionStats,
) -> Result<bool, ExecError> {
    let Some(mut entry) =
        starting_span.and_then(|start| stores.align_recorded_paragraph_start(start))
    else {
        return Ok(false);
    };
    debug_assert!(entry.barriers.is_empty());
    #[allow(clippy::disallowed_methods)]
    let validation_started = std::time::Instant::now();
    let validated = validate_paragraph_entry(&mut entry, input, stores, execution);
    stores.record_pure_memo_timing(
        PureMemoLayer::Paragraph,
        MemoTimingPhase::Validation,
        validation_started.elapsed(),
    );
    let Some(ValidatedParagraphEntry {
        input: prepared_input,
        retained,
        relaxed_state,
    }) = validated
    else {
        return Ok(false);
    };
    let mountable_lines = entry
        .lines
        .as_ref()
        .filter(|lines| stores.can_mount_retained_paragraph_result(lines))
        .cloned();
    let transition_applied = input.apply_paragraph_transition(stores, prepared_input)?;
    assert!(
        transition_applied,
        "validated paragraph source transition must apply"
    );
    let current_input = input.publication_summary(stores);
    let _ = stores.finish_pure_paragraph_recording();
    replay_mutations(stores, &entry.mutations);
    if !relaxed_state {
        debug_assert_eq!(
            stores.count_int_fingerprint(),
            entry.mutation_exit_fingerprint,
            "paragraph survivor redo must reproduce the recorded count/int state"
        );
    }
    replay_effects(stores, &entry.effects);
    let lines_valid =
        validate_finished_lines(&mut entry, mountable_lines.is_some(), stores, execution);
    #[allow(clippy::disallowed_methods)]
    let mount_started = std::time::Instant::now();
    let mounted_lines = lines_valid.then(|| {
        let origins = resolve_paragraph_provenance(stores, &entry.line_provenance);
        stores
            .mount_retained_paragraph_result(
                mountable_lines.as_ref().expect("validated mounted lines"),
                &origins,
                &entry.line_provenance.origin_slots,
            )
            .expect("prevalidated paragraph mount must remain valid")
    });
    let (nodes, retained_hlist) = if lines_valid {
        (Vec::new(), retained.clone())
    } else {
        let origins = resolve_paragraph_provenance(stores, &entry.hlist_provenance);
        let mounted = stores
            .mount_retained_paragraph_result(
                &retained,
                &origins,
                &entry.hlist_provenance.origin_slots,
            )
            .expect("prevalidated paragraph hlist mount must remain valid");
        let nodes = stores.nodes(mounted).to_vec();
        (nodes, retained.clone())
    };
    let lines = mounted_lines.map(|list| stores.nodes(list).to_vec());
    stores.record_pure_memo_timing(
        PureMemoLayer::Paragraph,
        MemoTimingPhase::Import,
        mount_started.elapsed(),
    );
    execution.abandon_cold_paragraph_recording();
    entry.ending_input = current_input;
    entry.hlist = Some(retained_hlist);
    entry.lines = lines_valid.then_some(mountable_lines).flatten();
    let line_count = entry.line_count;
    let mutation_count = entry.mutations.len();
    let delivered_tokens = entry.delivered_tokens;
    stores.record_carried_paragraph(&entry);
    stores.record_paragraph_region(entry);
    execution.pending_paragraph_memo =
        (!lines_valid).then_some(crate::executor::PendingParagraphMemo);
    crate::assignments::install_reused_paragraph_hlist(
        nest,
        input,
        stores,
        execution,
        nodes,
        lines.map(|lines| (lines, line_count)),
    )?;
    stores.record_pure_paragraph_hit(delivered_tokens, mutation_count, relaxed_state);
    stores.record_pure_paragraph_line_hit(!lines_valid);
    Ok(true)
}

/// Central fail-before-mutation validation boundary for an accepted paragraph.
/// Source/input alignment, exact entry identity, typed semantic fallback,
/// state-delta preconditions, effects, and retained hlist liveness are all
/// established before the caller advances input or replays state.
fn validate_paragraph_entry(
    entry: &mut tex_state::RecordedParagraphRegion,
    input: &InputStack,
    stores: &mut Universe,
    execution: &mut ExecutionContext<'_>,
) -> Option<ValidatedParagraphEntry> {
    let prepared_input = entry
        .starting_span
        .zip(entry.ending_span)
        .and_then(|(start, end)| {
            input.prepare_paragraph_transition(
                stores,
                start,
                &entry.consumed_spans,
                end,
                &entry.ending_input,
            )
        });
    let current_count_int_fingerprint = stores.count_int_fingerprint();
    let relaxed_state = current_count_int_fingerprint != entry.mutation_entry_fingerprint;
    let exact_entry =
        entry
            .entry_identity
            .matches(&entry.dependencies, current_count_int_fingerprint, |key| {
                stores.dependency_changed_at(key)
            });
    let dependency_failure = (!exact_entry)
        .then(|| {
            stores.validate_dependencies_with_failure(&mut entry.dependencies, |key| {
                paragraph_validation_value(stores, execution, key)
            })
        })
        .flatten();
    let validation_failure = dependency_failure
        .map(ParagraphValidationFailure::from_dependency)
        .or_else(|| {
            (!exact_entry && relaxed_state && !validate_mutations(stores, &entry.mutations))
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
    if !exact_entry {
        entry
            .entry_identity
            .refresh(&entry.dependencies, current_count_int_fingerprint);
    }
    let Some(retained) = entry.hlist.clone() else {
        stores.record_pure_paragraph_validation_failure(ParagraphValidationFailure::RetainedResult);
        return None;
    };
    if !stores.can_mount_retained_paragraph_result(&retained) {
        stores.record_pure_paragraph_validation_failure(ParagraphValidationFailure::RetainedResult);
        return None;
    }
    Some(ValidatedParagraphEntry {
        input: prepared_input?,
        retained,
        relaxed_state,
    })
}

fn validate_finished_lines(
    entry: &mut tex_state::RecordedParagraphRegion,
    mountable: bool,
    stores: &mut Universe,
    execution: &mut ExecutionContext<'_>,
) -> bool {
    #[allow(clippy::disallowed_methods)]
    let line_validation_started = std::time::Instant::now();
    let exact = entry
        .break_identity
        .matches(&entry.break_dependencies, |key| {
            stores.dependency_changed_at(key)
        });
    let line_dependency_failure = (!exact)
        .then(|| {
            stores.validate_dependencies_with_failure(&mut entry.break_dependencies, |key| {
                paragraph_validation_value(stores, execution, key)
            })
        })
        .flatten();
    if !exact && line_dependency_failure.is_none() {
        entry.break_identity.refresh(&entry.break_dependencies);
    }
    let lines_valid = mountable && line_dependency_failure.is_none();
    stores.record_pure_memo_timing(
        PureMemoLayer::Paragraph,
        MemoTimingPhase::Validation,
        line_validation_started.elapsed(),
    );
    if line_dependency_failure.is_some() {
        stores
            .record_pure_paragraph_validation_failure(ParagraphValidationFailure::BreakDependency);
    }
    lines_valid
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
    mutations.iter().all(|mutation| match *mutation {
        tex_state::PureParagraphMutation::Count {
            index, expected, ..
        } => stores.count(index) == expected,
        tex_state::PureParagraphMutation::IntParam {
            param, expected, ..
        } => stores.int_param(param) == expected,
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
        }
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

pub(crate) fn publish_prepared_hlist(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut ExecutionContext<'_>,
    nodes: &[tex_state::node::Node],
) {
    publish_recorded_region(input, stores, execution, nodes);
}

fn publish_recorded_region(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut ExecutionContext<'_>,
    nodes: &[tex_state::node::Node],
) {
    let Some(mut recording) = execution.cold_paragraph_recording.take() else {
        let _ = stores.finish_pure_paragraph_recording();
        return;
    };
    let (mut keys, expansion_barriers) = execution.finish_paragraph_expansion_recording();
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
        .starting_span
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
    let group_key = tex_state::DependencyKey::Engine(tex_state::DependencyEngineField::GroupLevel);
    let ending_group_depth = tex_state::ExpansionState::execution_group_depth(stores);
    let group_transition_changed =
        recording.starting_group_changed_at != stores.track_dependency(group_key);
    // At depth zero the dedicated setter recorder retains only root/global
    // transitions. Inside a live entry group, count/int writes cannot reproduce
    // assignment ownership, so any such write remains a conservative barrier.
    if recording.starting_span.is_some()
        && (recording.starting_group_depth != ending_group_depth
            || (recording.starting_group_depth != 0
                && (group_transition_changed || mutation_summary.unsupported_group_ownership)))
    {
        recording
            .barriers
            .insert(tex_state::ParagraphBarrierReason::UnsupportedGroupTransition);
    }
    if ending_input.frames().len() != 1 {
        recording
            .barriers
            .insert(tex_state::ParagraphBarrierReason::UnsupportedInputTransition);
    }
    finish_phase(
        stores,
        ParagraphRecordingPhase::InputTransition,
        input_started,
    );
    if !recording.barriers.is_empty() {
        let barriers = recording.barriers.into_iter().collect::<Vec<_>>();
        stores.record_pure_paragraph_barriers(&barriers);
        return;
    }
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
    append_paragraph_hlist_dependencies(stores, nodes, &mut keys);
    keys.sort_unstable();
    keys.dedup();
    let dependencies = keys
        .into_iter()
        .map(|key| {
            Some(tex_state::ObservedDependency {
                key,
                changed_at: stores.track_dependency(key),
                value: stores.semantic_dependency_value(key)?,
            })
        })
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
    let entry_identity =
        tex_state::ParagraphEntryIdentity::new(&dependencies, mutation_summary.entry_fingerprint);
    finish_phase(
        stores,
        ParagraphRecordingPhase::FrontEndDependencies,
        dependency_started,
    );
    let provenance_started = start_phase();
    let hlist_provenance = paragraph_graph_provenance(stores, nodes);
    finish_phase(
        stores,
        ParagraphRecordingPhase::FrontEndProvenance,
        provenance_started,
    );
    let retention_started = start_phase();
    let list = stores.freeze_node_list(nodes);
    let hlist = Some(stores.retain_paragraph_result(list));
    finish_phase(
        stores,
        ParagraphRecordingPhase::HlistRetention,
        retention_started,
    );
    let publication_started = start_phase();
    stores.record_paragraph_region(tex_state::RecordedParagraphRegion {
        starting_span: recording.starting_span,
        ending_span,
        consumed_spans,
        delivered_tokens: recording.delivered_tokens,
        entry_identity,
        dependencies,
        mutation_entry_fingerprint: mutation_summary.entry_fingerprint,
        mutation_exit_fingerprint: mutation_summary.exit_fingerprint,
        mutations: mutation_summary.mutations,
        effects,
        ending_input,
        barriers: recording.barriers.into_iter().collect(),
        hlist,
        hlist_provenance,
        break_dependencies: Vec::new(),
        break_identity: tex_state::ParagraphReadIdentity::default(),
        lines: None,
        line_count: 0,
        line_provenance: tex_state::ParagraphProvenanceRecipe::default(),
    });
    finish_phase(
        stores,
        ParagraphRecordingPhase::RegionPublication,
        publication_started,
    );
    if execution.pending_paragraph_memo.is_none() {
        execution.pending_paragraph_memo = Some(crate::executor::PendingParagraphMemo);
    }
}

pub(crate) fn publish_finished_lines(
    stores: &mut Universe,
    execution: &mut ExecutionContext<'_>,
    nodes: &[tex_state::node::Node],
    line_count: i32,
) {
    let Some(_) = execution.pending_paragraph_memo.take() else {
        return;
    };
    let dependencies_started = start_phase();
    let Some(dependencies) = paragraph_break_dependencies(stores, execution, nodes) else {
        return;
    };
    finish_phase(
        stores,
        ParagraphRecordingPhase::BreakDependencies,
        dependencies_started,
    );
    let retention_started = start_phase();
    let list = stores.freeze_node_list(nodes);
    let retained = stores.retain_paragraph_result(list);
    finish_phase(
        stores,
        ParagraphRecordingPhase::LineRetention,
        retention_started,
    );
    let provenance_started = start_phase();
    let provenance = paragraph_graph_provenance(stores, nodes);
    finish_phase(
        stores,
        ParagraphRecordingPhase::LineProvenance,
        provenance_started,
    );
    let publication_started = start_phase();
    stores.finish_recorded_paragraph_lines(dependencies, retained, line_count, provenance);
    finish_phase(
        stores,
        ParagraphRecordingPhase::RegionPublication,
        publication_started,
    );
}

fn paragraph_graph_provenance(
    stores: &Universe,
    nodes: &[tex_state::node::Node],
) -> tex_state::ParagraphProvenanceRecipe {
    fn visit(
        stores: &Universe,
        nodes: &[tex_state::node::Node],
        root_ordinals: &mut ahash::AHashMap<tex_state::RootSpanId, u32>,
        piece_ordinals: &mut ahash::AHashMap<tex_state::PieceId, u32>,
        recipe: &mut tex_state::ParagraphProvenanceRecipe,
    ) {
        fn push_origin(
            stores: &Universe,
            origin: tex_state::token::OriginId,
            root_ordinals: &mut ahash::AHashMap<tex_state::RootSpanId, u32>,
            piece_ordinals: &mut ahash::AHashMap<tex_state::PieceId, u32>,
            recipe: &mut tex_state::ParagraphProvenanceRecipe,
        ) {
            let Some(span) = stores.root_span_for_origin(origin) else {
                recipe.origin_slots.push(u32::MAX);
                return;
            };
            let ordinal = if let Some(&ordinal) = root_ordinals.get(&span) {
                ordinal
            } else {
                let Ok(ordinal) = u32::try_from(recipe.root_spans.len()) else {
                    recipe.origin_slots.push(u32::MAX);
                    return;
                };
                let piece = span.piece();
                let piece_ordinal = if let Some(&piece_ordinal) = piece_ordinals.get(&piece) {
                    piece_ordinal
                } else {
                    let Ok(piece_ordinal) = u32::try_from(recipe.piece_anchors.len()) else {
                        recipe.origin_slots.push(u32::MAX);
                        return;
                    };
                    recipe.piece_anchors.push(span.start_anchor());
                    piece_ordinals.insert(piece, piece_ordinal);
                    piece_ordinal
                };
                recipe.root_spans.push(tex_state::ParagraphProvenanceSpan {
                    piece: piece_ordinal,
                    start: span.start(),
                    end: span.end(),
                });
                root_ordinals.insert(span, ordinal);
                ordinal
            };
            recipe.origin_slots.push(ordinal);
        }
        let child = |id,
                     root_ordinals: &mut ahash::AHashMap<tex_state::RootSpanId, u32>,
                     piece_ordinals: &mut ahash::AHashMap<tex_state::PieceId, u32>,
                     recipe: &mut tex_state::ParagraphProvenanceRecipe| {
            let nodes = stores.nodes(id).to_vec();
            visit(stores, &nodes, root_ordinals, piece_ordinals, recipe);
        };
        for node in nodes {
            match node {
                tex_state::node::Node::Char { origin, .. } => {
                    push_origin(stores, *origin, root_ordinals, piece_ordinals, recipe);
                }
                tex_state::node::Node::Lig { origins, .. } => {
                    for &origin in origins {
                        push_origin(stores, origin, root_ordinals, piece_ordinals, recipe);
                    }
                }
                tex_state::node::Node::HList(box_node) | tex_state::node::Node::VList(box_node) => {
                    child(box_node.children, root_ordinals, piece_ordinals, recipe)
                }
                tex_state::node::Node::Glue {
                    leader:
                        Some(
                            tex_state::node::LeaderPayload::HList(box_node)
                            | tex_state::node::LeaderPayload::VList(box_node),
                        ),
                    ..
                } => child(box_node.children, root_ordinals, piece_ordinals, recipe),
                tex_state::node::Node::Unset(unset) => {
                    child(unset.children, root_ordinals, piece_ordinals, recipe)
                }
                tex_state::node::Node::Disc {
                    pre, post, replace, ..
                } => {
                    child(*pre, root_ordinals, piece_ordinals, recipe);
                    child(*post, root_ordinals, piece_ordinals, recipe);
                    child(*replace, root_ordinals, piece_ordinals, recipe);
                }
                tex_state::node::Node::Ins { content, .. }
                | tex_state::node::Node::Adjust(content) => {
                    child(*content, root_ordinals, piece_ordinals, recipe)
                }
                _ => {}
            }
        }
    }
    let mut recipe = tex_state::ParagraphProvenanceRecipe::default();
    let mut root_ordinals = ahash::AHashMap::new();
    let mut piece_ordinals = ahash::AHashMap::new();
    visit(
        stores,
        nodes,
        &mut root_ordinals,
        &mut piece_ordinals,
        &mut recipe,
    );
    recipe
}

fn append_paragraph_hlist_dependencies(
    stores: &Universe,
    nodes: &[tex_state::node::Node],
    keys: &mut Vec<tex_state::DependencyKey>,
) {
    let child = |id, keys: &mut Vec<tex_state::DependencyKey>| {
        let nodes = stores.nodes(id).to_vec();
        append_paragraph_hlist_dependencies(stores, &nodes, keys);
    };
    for node in nodes {
        match node {
            tex_state::node::Node::Char { font, ch, .. } => {
                push_sfcode_dependency(*ch, keys);
                push_hlist_font_dependencies(*font, keys);
            }
            tex_state::node::Node::Lig { font, orig, .. } => {
                push_hlist_font_dependencies(*font, keys);
                for &ch in orig {
                    push_sfcode_dependency(ch, keys);
                }
            }
            tex_state::node::Node::HList(box_node) | tex_state::node::Node::VList(box_node) => {
                child(box_node.children, keys);
            }
            tex_state::node::Node::Glue {
                leader:
                    Some(
                        tex_state::node::LeaderPayload::HList(box_node)
                        | tex_state::node::LeaderPayload::VList(box_node),
                    ),
                ..
            } => child(box_node.children, keys),
            tex_state::node::Node::Unset(unset) => child(unset.children, keys),
            tex_state::node::Node::Disc {
                pre, post, replace, ..
            } => {
                child(*pre, keys);
                child(*post, keys);
                child(*replace, keys);
            }
            tex_state::node::Node::Ins { content, .. } | tex_state::node::Node::Adjust(content) => {
                child(*content, keys)
            }
            _ => {}
        }
    }
}

fn push_sfcode_dependency(ch: char, keys: &mut Vec<tex_state::DependencyKey>) {
    keys.push(tex_state::DependencyKey::Code {
        table: tex_state::DependencyCodeTable::Sfcode,
        scalar: ch as u32,
    });
}

fn push_hlist_font_dependencies(
    font: tex_state::ids::FontId,
    keys: &mut Vec<tex_state::DependencyKey>,
) {
    for field in [
        tex_state::DependencyFontField::Metrics,
        tex_state::DependencyFontField::HyphenChar,
    ] {
        keys.push(tex_state::DependencyKey::Font {
            field,
            font: font.raw(),
            index: 0,
        });
    }
}

fn paragraph_break_dependencies(
    stores: &mut Universe,
    execution: &mut ExecutionContext<'_>,
    nodes: &[tex_state::node::Node],
) -> Option<Vec<tex_state::ObservedDependency>> {
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
        IntParam::LANGUAGE,
        IntParam::LEFT_HYPHEN_MIN,
        IntParam::RIGHT_HYPHEN_MIN,
        IntParam::UC_HYPH,
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
    let mut chars = Vec::new();
    for node in nodes {
        match node {
            tex_state::node::Node::Char { font, ch, .. } => {
                fonts.push(*font);
                chars.push(*ch);
            }
            tex_state::node::Node::Lig { font, orig, .. } => {
                fonts.push(*font);
                chars.extend(orig.iter().copied());
            }
            tex_state::node::Node::Whatsit(tex_state::node::Whatsit::Language {
                language, ..
            }) => languages.push(*language),
            _ => {}
        }
    }
    fonts.sort_unstable_by_key(|font| font.raw());
    fonts.dedup();
    for font in fonts {
        keys.push(Key::Font {
            field: Font::Metrics,
            font: font.raw(),
            index: 0,
        });
        keys.push(Key::Font {
            field: Font::HyphenChar,
            font: font.raw(),
            index: 0,
        });
    }
    chars.sort_unstable();
    chars.dedup();
    for ch in chars {
        keys.push(Key::Code {
            table: Code::Lccode,
            scalar: ch as u32,
        });
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
            if let Some(cached) = execution.paragraph_dependency_cache.get(&key)
                && cached.changed_at == changed_at
            {
                return Some(cached.clone());
            }
            let observed = tex_state::ObservedDependency {
                key,
                changed_at,
                value: stores.semantic_dependency_value(key)?,
            };
            if execution.paragraph_dependency_cache.len() < MAX_PARAGRAPH_DEPENDENCY_CACHE_ENTRIES {
                execution
                    .paragraph_dependency_cache
                    .insert(key, observed.clone());
            }
            Some(observed)
        })
        .collect();
    finish_phase(
        stores,
        ParagraphRecordingPhase::BreakValueProjection,
        projection_started,
    );
    dependencies
}

fn paragraph_validation_value(
    stores: &Universe,
    execution: &mut ExecutionContext<'_>,
    key: tex_state::DependencyKey,
) -> tex_state::DependencyValue {
    let changed_at = stores.dependency_changed_at(key);
    if let Some(cached) = execution.paragraph_dependency_cache.get(&key)
        && cached.changed_at == changed_at
    {
        return cached.value.clone();
    }
    let value = stores
        .semantic_dependency_value(key)
        .unwrap_or(tex_state::DependencyValue::Absent);
    if execution.paragraph_dependency_cache.len() < MAX_PARAGRAPH_DEPENDENCY_CACHE_ENTRIES {
        execution.paragraph_dependency_cache.insert(
            key,
            tex_state::ObservedDependency {
                key,
                changed_at,
                value: value.clone(),
            },
        );
    }
    value
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
