//! Recorder-driven paragraph front-end eligibility and transitional detached reuse.

use tex_lex::InputStack;
use tex_state::env::banks::{DimenParam, GlueParam, IntParam, TokParam};
use tex_state::token::Token;
use tex_state::{
    DetachedVirtualEffect, EffectRecord, MemoTimingPhase, ParagraphRecordingPhase,
    ParagraphValidationFailure, PrintSink, PureMemoLayer, Universe,
};

use crate::{ExecError, ExecutionContext, ExecutionStats, ModeNest};

const MAX_PARAGRAPH_DEPENDENCY_CACHE_ENTRIES: usize = 4_096;

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
    let dependency_failure = stores
        .validate_dependencies_with_failure(&mut entry.dependencies, |key| {
            paragraph_validation_value(stores, execution, key)
        });
    let prepared_input = entry
        .starting_span
        .zip(entry.ending_span)
        .and_then(|(start, end)| {
            input.prepare_paragraph_transition(
                start,
                &entry.consumed_spans,
                end,
                &entry.ending_input,
            )
        });
    let validation_failure = dependency_failure
        .map(ParagraphValidationFailure::from_dependency)
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
    stores.record_pure_memo_timing(
        PureMemoLayer::Paragraph,
        MemoTimingPhase::Validation,
        validation_started.elapsed(),
    );
    if let Some(failure) = validation_failure {
        stores.record_pure_paragraph_validation_failure(failure);
        return Ok(false);
    }
    let Some(retained) = entry.hlist else {
        stores.record_pure_paragraph_validation_failure(ParagraphValidationFailure::RetainedResult);
        return Ok(false);
    };
    #[allow(clippy::disallowed_methods)]
    let import_started = std::time::Instant::now();
    let list = match stores.import_retained_paragraph_result(retained) {
        Some(list) => list,
        None => {
            stores.record_pure_memo_timing(
                PureMemoLayer::Paragraph,
                MemoTimingPhase::Import,
                import_started.elapsed(),
            );
            stores.record_pure_paragraph_import_failure();
            return Ok(false);
        }
    };
    let imported_bytes = stores
        .nodes(list)
        .len()
        .saturating_mul(std::mem::size_of::<tex_state::node::Node>());
    let imported_lines = entry
        .lines
        .and_then(|lines| stores.import_retained_paragraph_result(lines));
    stores.record_pure_memo_timing(
        PureMemoLayer::Paragraph,
        MemoTimingPhase::Import,
        import_started.elapsed(),
    );
    let transition_applied = input.apply_paragraph_transition(
        stores,
        prepared_input.expect("validated paragraph transition"),
    )?;
    assert!(
        transition_applied,
        "validated paragraph source transition must apply"
    );
    let current_input = input.publication_summary(stores);
    let current_trace_origins = entry
        .trace_spans
        .iter()
        .map(|span| {
            span.and_then(|span| stores.origin_for_root_span(span))
                .unwrap_or(tex_state::token::OriginId::UNKNOWN)
        })
        .collect::<Vec<_>>();
    let _ = stores.finish_pure_paragraph_recording();
    replay_mutations(stores, &entry.mutations);
    replay_effects(stores, &entry.effects);
    let mut nodes: Vec<_> = stores
        .nodes(list)
        .into_iter()
        .map(|node| node.to_owned())
        .collect();
    rebind_literal_origins(&mut nodes, &current_trace_origins, &entry.origin_ordinals);
    #[allow(clippy::disallowed_methods)]
    let line_validation_started = std::time::Instant::now();
    let line_dependency_failure = stores
        .validate_dependencies_with_failure(&mut entry.break_dependencies, |key| {
            paragraph_validation_value(stores, execution, key)
        });
    let lines_valid = imported_lines.is_some() && line_dependency_failure.is_none();
    stores.record_pure_memo_timing(
        PureMemoLayer::Paragraph,
        MemoTimingPhase::Validation,
        line_validation_started.elapsed(),
    );
    if line_dependency_failure.is_some() {
        stores
            .record_pure_paragraph_validation_failure(ParagraphValidationFailure::BreakDependency);
    }
    let mut lines = imported_lines.map(|list| {
        stores
            .nodes(list)
            .into_iter()
            .map(|node| node.to_owned())
            .collect::<Vec<_>>()
    });
    if lines_valid {
        rebind_graph_origins(
            stores,
            lines.as_mut().expect("validated imported lines"),
            &current_trace_origins,
            &entry.line_origin_ordinals,
        );
    }
    execution.abandon_cold_paragraph_recording();
    entry.ending_input = current_input;
    entry.hlist = Some(stores.retain_paragraph_result(list));
    entry.lines = if lines_valid {
        let current_lines = stores.freeze_node_list(lines.as_deref().unwrap_or_default());
        Some(stores.retain_paragraph_result(current_lines))
    } else {
        None
    };
    let line_count = entry.line_count;
    let mutation_count = entry.mutations.len();
    let trace_len = entry.trace_spans.len();
    stores.record_carried_paragraph(&entry);
    stores.record_paragraph_region(entry);
    execution.pending_paragraph_memo =
        (!lines_valid).then_some(crate::executor::PendingParagraphMemo {
            trace_origins: current_trace_origins,
        });
    crate::assignments::install_reused_paragraph_hlist(
        nest,
        input,
        stores,
        execution,
        nodes,
        lines_valid.then_some((lines.expect("validated imported lines"), line_count)),
    )?;
    stores.record_pure_paragraph_hit(trace_len, mutation_count, imported_bytes);
    stores.record_pure_paragraph_line_hit(!lines_valid);
    Ok(true)
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
    let mut current = std::collections::BTreeMap::<(u8, u16), i32>::new();
    for mutation in mutations {
        let (key, expected, value, initial) = match *mutation {
            tex_state::PureParagraphMutation::Count {
                index,
                expected,
                value,
                ..
            } => ((0, index), expected, value, stores.count(index)),
            tex_state::PureParagraphMutation::IntParam {
                param,
                expected,
                value,
                ..
            } => ((1, param.raw()), expected, value, stores.int_param(param)),
        };
        if current.get(&key).copied().unwrap_or(initial) != expected {
            return false;
        }
        current.insert(key, value);
    }
    true
}

fn replay_mutations(stores: &mut Universe, mutations: &[tex_state::PureParagraphMutation]) {
    for mutation in mutations {
        match *mutation {
            tex_state::PureParagraphMutation::Count {
                index,
                expected: _,
                value,
                global,
            } => {
                if global {
                    stores.set_count_global(index, value);
                } else {
                    stores.set_count(index, value);
                }
            }
            tex_state::PureParagraphMutation::IntParam {
                param,
                expected: _,
                value,
                global,
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

fn rebind_literal_origins(
    nodes: &mut [tex_state::node::Node],
    trace_origins: &[tex_state::token::OriginId],
    ordinals: &[u32],
) {
    let mut ordinals = ordinals.iter().copied();
    let origin_at = |ordinal: u32| {
        usize::try_from(ordinal)
            .ok()
            .and_then(|ordinal| trace_origins.get(ordinal))
            .copied()
            .unwrap_or(tex_state::token::OriginId::UNKNOWN)
    };
    for node in nodes {
        match node {
            tex_state::node::Node::Char { origin, .. } => {
                *origin = origin_at(ordinals.next().unwrap_or(u32::MAX));
            }
            tex_state::node::Node::Lig {
                orig,
                origins: node_origins,
                ..
            } => {
                node_origins.clear();
                node_origins.extend(
                    (0..orig.len()).map(|_| origin_at(ordinals.next().unwrap_or(u32::MAX))),
                );
            }
            _ => {}
        }
    }
}

fn rebind_graph_origins(
    stores: &mut Universe,
    nodes: &mut [tex_state::node::Node],
    trace_origins: &[tex_state::token::OriginId],
    ordinals: &[u32],
) {
    let mut ordinals = ordinals.iter().copied();
    rebind_graph_origins_inner(stores, nodes, trace_origins, &mut ordinals);
}

fn rebind_graph_origins_inner(
    stores: &mut Universe,
    nodes: &mut [tex_state::node::Node],
    trace_origins: &[tex_state::token::OriginId],
    ordinals: &mut impl Iterator<Item = u32>,
) {
    let origin_at = |ordinal: u32| {
        usize::try_from(ordinal)
            .ok()
            .and_then(|ordinal| trace_origins.get(ordinal))
            .copied()
            .unwrap_or(tex_state::token::OriginId::UNKNOWN)
    };
    let rebuild = |stores: &mut Universe,
                   id: tex_state::ids::NodeListId,
                   trace_origins: &[tex_state::token::OriginId],
                   ordinals: &mut _| {
        let mut children = stores.nodes(id).to_vec();
        rebind_graph_origins_inner(stores, &mut children, trace_origins, ordinals);
        stores.freeze_node_list(&children)
    };
    for node in nodes {
        match node {
            tex_state::node::Node::Char { origin, .. } => {
                *origin = origin_at(ordinals.next().unwrap_or(u32::MAX));
            }
            tex_state::node::Node::Lig { orig, origins, .. } => {
                origins.clear();
                origins.extend(
                    (0..orig.len()).map(|_| origin_at(ordinals.next().unwrap_or(u32::MAX))),
                );
            }
            tex_state::node::Node::HList(box_node) | tex_state::node::Node::VList(box_node) => {
                box_node.children = rebuild(stores, box_node.children, trace_origins, ordinals);
            }
            tex_state::node::Node::Glue {
                leader:
                    Some(
                        tex_state::node::LeaderPayload::HList(box_node)
                        | tex_state::node::LeaderPayload::VList(box_node),
                    ),
                ..
            } => {
                box_node.children = rebuild(stores, box_node.children, trace_origins, ordinals);
            }
            tex_state::node::Node::Unset(unset) => {
                unset.children = rebuild(stores, unset.children, trace_origins, ordinals);
            }
            tex_state::node::Node::Disc {
                pre, post, replace, ..
            } => {
                *pre = rebuild(stores, *pre, trace_origins, ordinals);
                *post = rebuild(stores, *post, trace_origins, ordinals);
                *replace = rebuild(stores, *replace, trace_origins, ordinals);
            }
            tex_state::node::Node::Ins { content, .. } | tex_state::node::Node::Adjust(content) => {
                *content = rebuild(stores, *content, trace_origins, ordinals);
            }
            _ => {}
        }
    }
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
    #[cfg(feature = "profiling-stats")]
    stores.record_pure_paragraph_phase_samples(
        ParagraphRecordingPhase::TraceCapture,
        std::time::Duration::from_nanos(recording.trace_capture_nanos),
        recording.trace_capture_samples,
    );
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
    let mutations = stores.finish_pure_paragraph_recording().unwrap_or_default();
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
    if recording.starting_span.is_some()
        && (recording.starting_group_depth
            != tex_state::ExpansionState::execution_group_depth(stores)
            || recording.starting_group_changed_at != stores.track_dependency(group_key))
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
    ]);
    for token in &recording.trace {
        if let Token::Char { ch, .. } = tex_expand::semantic_token(*token) {
            keys.push(tex_state::DependencyKey::Code {
                table: tex_state::DependencyCodeTable::Sfcode,
                scalar: ch as u32,
            });
        }
    }
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
    finish_phase(
        stores,
        ParagraphRecordingPhase::FrontEndDependencies,
        dependency_started,
    );
    let provenance_started = start_phase();
    let trace_spans = recording
        .trace
        .iter()
        .map(|token| stores.root_span_for_origin(token.origin()))
        .collect::<Vec<_>>();
    finish_phase(
        stores,
        ParagraphRecordingPhase::FrontEndProvenance,
        provenance_started,
    );
    let pending = execution.pending_paragraph_memo.as_ref();
    let trace_origins = pending.map_or_else(
        || recording.trace.iter().map(|token| token.origin()).collect(),
        |pending| pending.trace_origins.clone(),
    );
    let retention_started = start_phase();
    let list = stores.freeze_node_list(nodes);
    let hlist = Some(stores.retain_paragraph_result(list));
    let origin_ordinals = paragraph_origin_ordinals(nodes, &trace_origins);
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
        trace_spans,
        dependencies,
        mutations: mutations.clone(),
        effects,
        ending_input,
        barriers: recording.barriers.into_iter().collect(),
        hlist,
        origin_ordinals,
        break_dependencies: Vec::new(),
        lines: None,
        line_count: 0,
        line_origin_ordinals: Vec::new(),
    });
    finish_phase(
        stores,
        ParagraphRecordingPhase::RegionPublication,
        publication_started,
    );
    if execution.pending_paragraph_memo.is_none() {
        execution.pending_paragraph_memo =
            Some(crate::executor::PendingParagraphMemo { trace_origins });
    }
}

fn paragraph_origin_ordinals(
    nodes: &[tex_state::node::Node],
    trace_origins: &[tex_state::token::OriginId],
) -> Vec<u32> {
    let origin_ordinals = origin_ordinal_map(trace_origins);
    let ordinal = |origin| origin_ordinals.get(&origin).copied().unwrap_or(u32::MAX);
    let mut ordinals = Vec::new();
    for node in nodes {
        match node {
            tex_state::node::Node::Char { origin, .. } => ordinals.push(ordinal(*origin)),
            tex_state::node::Node::Lig { origins, .. } => {
                ordinals.extend(origins.iter().map(|origin| ordinal(*origin)));
            }
            _ => {}
        }
    }
    ordinals
}

pub(crate) fn publish_finished_lines(
    stores: &mut Universe,
    execution: &mut ExecutionContext<'_>,
    nodes: &[tex_state::node::Node],
    line_count: i32,
) {
    let Some(pending) = execution.pending_paragraph_memo.take() else {
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
    let ordinals = paragraph_graph_origin_ordinals(stores, nodes, &pending.trace_origins);
    finish_phase(
        stores,
        ParagraphRecordingPhase::LineProvenance,
        provenance_started,
    );
    let publication_started = start_phase();
    stores.finish_recorded_paragraph_lines(dependencies, retained, line_count, ordinals);
    finish_phase(
        stores,
        ParagraphRecordingPhase::RegionPublication,
        publication_started,
    );
}

fn paragraph_graph_origin_ordinals(
    stores: &Universe,
    nodes: &[tex_state::node::Node],
    trace_origins: &[tex_state::token::OriginId],
) -> Vec<u32> {
    fn visit(
        stores: &Universe,
        nodes: &[tex_state::node::Node],
        origin_ordinals: &ahash::AHashMap<tex_state::token::OriginId, u32>,
        out: &mut Vec<u32>,
    ) {
        let ordinal = |origin| origin_ordinals.get(&origin).copied().unwrap_or(u32::MAX);
        let child = |id, out: &mut Vec<u32>| {
            let nodes = stores.nodes(id).to_vec();
            visit(stores, &nodes, origin_ordinals, out);
        };
        for node in nodes {
            match node {
                tex_state::node::Node::Char { origin, .. } => out.push(ordinal(*origin)),
                tex_state::node::Node::Lig { origins, .. } => {
                    out.extend(origins.iter().map(|origin| ordinal(*origin)));
                }
                tex_state::node::Node::HList(box_node) | tex_state::node::Node::VList(box_node) => {
                    child(box_node.children, out)
                }
                tex_state::node::Node::Glue {
                    leader:
                        Some(
                            tex_state::node::LeaderPayload::HList(box_node)
                            | tex_state::node::LeaderPayload::VList(box_node),
                        ),
                    ..
                } => child(box_node.children, out),
                tex_state::node::Node::Unset(unset) => child(unset.children, out),
                tex_state::node::Node::Disc {
                    pre, post, replace, ..
                } => {
                    child(*pre, out);
                    child(*post, out);
                    child(*replace, out);
                }
                tex_state::node::Node::Ins { content, .. }
                | tex_state::node::Node::Adjust(content) => child(*content, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    let origin_ordinals = origin_ordinal_map(trace_origins);
    visit(stores, nodes, &origin_ordinals, &mut out);
    out
}

fn origin_ordinal_map(
    trace_origins: &[tex_state::token::OriginId],
) -> ahash::AHashMap<tex_state::token::OriginId, u32> {
    let mut ordinals = ahash::AHashMap::with_capacity(trace_origins.len());
    for (index, &origin) in trace_origins.iter().enumerate() {
        let Ok(index) = u32::try_from(index) else {
            break;
        };
        ordinals.entry(origin).or_insert(index);
    }
    ordinals
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
