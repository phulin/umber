//! Recorder-driven paragraph front-end eligibility and transitional detached reuse.

use tex_lex::InputStack;
use tex_state::env::banks::{DimenParam, GlueParam, IntParam, TokParam};
use tex_state::meaning::{Meaning, UnexpandablePrimitive};
use tex_state::token::{Catcode, Token, TracedTokenWord};
use tex_state::{
    ContentHash, DetachedVirtualEffect, EffectRecord, ExpansionState, PrintSink, PureMemoKey,
    Universe,
};

use crate::{ExecError, ExecutionContext, ExecutionStats, ModeNest};

const PARAGRAPH_FRONT_END_DOMAIN: u32 = 2;
const PARAGRAPH_FRONT_END_SCHEMA: u32 = 1;
const PARAGRAPH_ENV_HASH_DOMAIN: u64 = 0x7061_7261_656e_7601;
const MAX_PREFLIGHT_TOKENS: usize = 1 << 16;

pub(crate) fn try_reuse_literal_paragraph(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut ExecutionContext<'_>,
    stats: &mut ExecutionStats,
) -> Result<bool, ExecError> {
    let mut traced = Vec::new();
    let mut semantic = Vec::new();
    let mut terminated = false;
    let mut effect_argument_depth = 0usize;
    let mut expects_effect_argument = false;
    for _ in 0..MAX_PREFLIGHT_TOKENS {
        let next = tex_expand::next_semantic_raw_token(
            input,
            &mut tex_state::ExpansionContext::new(stores),
        )?;
        let Some(next) = next else {
            break;
        };
        let token = tex_expand::semantic_token(next);
        traced.push(next);
        match token {
            Token::Char {
                cat: Catcode::Letter | Catcode::Other | Catcode::Space,
                ..
            } => semantic.push(token),
            Token::Cs(symbol) => {
                let meaning = stores.meaning(symbol);
                semantic.push(token);
                if effect_argument_depth == 0
                    && matches!(
                        meaning,
                        Meaning::UnexpandablePrimitive(
                            UnexpandablePrimitive::Par | UnexpandablePrimitive::EndGraf
                        )
                    )
                {
                    terminated = true;
                    break;
                }
                if matches!(
                    meaning,
                    Meaning::UnexpandablePrimitive(
                        UnexpandablePrimitive::Message | UnexpandablePrimitive::ErrMessage
                    )
                ) {
                    expects_effect_argument = true;
                    continue;
                }
                if !matches!(
                    meaning,
                    Meaning::CountRegister(_)
                        | Meaning::IntParam(_)
                        | Meaning::UnexpandablePrimitive(
                            UnexpandablePrimitive::Count | UnexpandablePrimitive::Global
                        )
                ) {
                    break;
                }
            }
            Token::Char {
                cat: Catcode::BeginGroup,
                ..
            } if expects_effect_argument || effect_argument_depth > 0 => {
                semantic.push(token);
                effect_argument_depth = effect_argument_depth.saturating_add(1);
                expects_effect_argument = false;
            }
            Token::Char {
                cat: Catcode::EndGroup,
                ..
            } if effect_argument_depth > 0 => {
                semantic.push(token);
                effect_argument_depth -= 1;
            }
            _ => break,
        }
    }

    let eligible = terminated
        && semantic
            .iter()
            .any(|token| matches!(token, Token::Char { cat, .. } if *cat != Catcode::Space));
    if !eligible {
        crate::push_traced_tokens(input, stores, traced);
        return Ok(false);
    }

    let key = paragraph_key(stores, &semantic);
    let Some(mut entry) = stores.lookup_recorded_paragraph(key) else {
        let trace_origins = traced.iter().map(|token| token.origin()).collect();
        crate::push_traced_tokens(input, stores, traced);
        execution.pending_paragraph_memo =
            Some(crate::executor::PendingParagraphMemo { key, trace_origins });
        stores.begin_pure_paragraph_recording();
        return Ok(false);
    };

    let current_spans = traced
        .iter()
        .filter_map(|token| stores.root_span_for_origin(token.origin()))
        .fold(Vec::new(), |mut spans, span| {
            if !spans.contains(&span) {
                spans.push(span);
            }
            spans
        });
    let dependencies_valid = stores.validate_dependencies(&mut entry.dependencies, |key| {
        stores
            .semantic_dependency_value(key)
            .unwrap_or(tex_state::DependencyValue::Absent)
    });
    let current_input = input.publication_summary(stores);
    let input_valid = current_input.paragraph_transition_matches(&entry.ending_input);
    if current_spans != entry.consumed_spans
        || !dependencies_valid
        || !validate_mutations(stores, &entry.mutations)
        || !validate_effects(&entry.effects)
        || !input_valid
    {
        stores.record_pure_paragraph_validation_miss();
        let trace_origins = traced.iter().map(|token| token.origin()).collect();
        crate::push_traced_tokens(input, stores, traced);
        execution.pending_paragraph_memo =
            Some(crate::executor::PendingParagraphMemo { key, trace_origins });
        stores.begin_pure_paragraph_recording();
        return Ok(false);
    }
    let Some(retained) = entry.hlist else {
        stores.record_pure_paragraph_validation_miss();
        crate::push_traced_tokens(input, stores, traced);
        return Ok(false);
    };
    let list = match stores.import_retained_paragraph_result(retained) {
        Some(list) => list,
        None => {
            stores.record_pure_paragraph_import_failure();
            let trace_origins = traced.iter().map(|token| token.origin()).collect();
            crate::push_traced_tokens(input, stores, traced);
            execution.pending_paragraph_memo =
                Some(crate::executor::PendingParagraphMemo { key, trace_origins });
            stores.begin_pure_paragraph_recording();
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
    let _ = stores.finish_pure_paragraph_recording();
    replay_mutations(stores, &entry.mutations);
    replay_effects(stores, &entry.effects);
    let mut nodes: Vec<_> = stores
        .nodes(list)
        .into_iter()
        .map(|node| node.to_owned())
        .collect();
    rebind_literal_origins(&mut nodes, &traced, &entry.origin_ordinals);
    let lines_valid = imported_lines.is_some()
        && stores.validate_dependencies(&mut entry.break_dependencies, |key| {
            stores
                .semantic_dependency_value(key)
                .unwrap_or(tex_state::DependencyValue::Absent)
        });
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
            &traced,
            &entry.line_origin_ordinals,
        );
    }
    execution.abandon_cold_paragraph_recording();
    entry.consumed_spans = current_spans;
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
    stores.record_paragraph_region(entry);
    execution.pending_paragraph_memo =
        (!lines_valid).then(|| crate::executor::PendingParagraphMemo {
            key,
            trace_origins: traced.iter().map(|token| token.origin()).collect(),
        });
    crate::assignments::install_reused_paragraph_hlist(
        nest,
        input,
        stores,
        execution,
        nodes,
        lines_valid.then_some((lines.expect("validated imported lines"), line_count)),
    )?;
    stats.delivered_tokens = stats.delivered_tokens.saturating_add(traced.len());
    stores.record_pure_paragraph_hit(traced.len(), mutation_count, imported_bytes);
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

fn paragraph_key(stores: &Universe, tokens: &[Token]) -> PureMemoKey {
    let mut chars: Vec<char> = tokens
        .iter()
        .filter_map(|token| match token {
            Token::Char { ch, cat } if *cat != Catcode::Space => Some(*ch),
            _ => None,
        })
        .collect();
    chars.sort_unstable();
    chars.dedup();
    let environment = stores.engine_boundary_hash(PARAGRAPH_ENV_HASH_DOMAIN, |hash| {
        hash.font(stores.current_font());
        hash.glue(stores.glue_param(GlueParam::SPACE_SKIP));
        hash.glue(stores.glue_param(GlueParam::XSPACE_SKIP));
        hash.i32(stores.dimen_param(DimenParam::PAR_INDENT).raw());
        hash.i32(stores.int_param(IntParam::LANGUAGE));
        hash.i32(stores.int_param(IntParam::LEFT_HYPHEN_MIN));
        hash.i32(stores.int_param(IntParam::RIGHT_HYPHEN_MIN));
        hash.u32(stores.execution_group_depth());
        hash.i32(
            stores
                .innermost_group_kind()
                .map_or(0, |kind| kind.etex_code()),
        );
        for ch in &chars {
            hash.u32(*ch as u32);
            hash.u16(stores.sfcode(*ch));
        }
    });
    let mut bytes = Vec::with_capacity(tokens.len().saturating_mul(8).saturating_add(24));
    bytes.extend_from_slice(&PARAGRAPH_FRONT_END_SCHEMA.to_le_bytes());
    bytes.extend_from_slice(&environment.to_le_bytes());
    for token in tokens {
        match token {
            Token::Char { ch, cat } => {
                bytes.push(0);
                bytes.extend_from_slice(&(*ch as u32).to_le_bytes());
                bytes.push(*cat as u8);
            }
            Token::Cs(symbol) => {
                bytes.push(1);
                let name = stores.resolve(*symbol);
                bytes.extend_from_slice(&(name.len() as u64).to_le_bytes());
                bytes.extend_from_slice(name.as_bytes());
            }
            Token::Param(slot) => {
                bytes.push(2);
                bytes.push(*slot);
            }
            Token::Frozen(kind) => {
                bytes.push(3);
                bytes.extend_from_slice(format!("{kind:?}").as_bytes());
            }
        }
    }
    PureMemoKey::new(
        PARAGRAPH_FRONT_END_DOMAIN,
        environment,
        ContentHash::from_bytes(&bytes),
    )
}

fn rebind_literal_origins(
    nodes: &mut [tex_state::node::Node],
    traced: &[TracedTokenWord],
    ordinals: &[u32],
) {
    let mut ordinals = ordinals.iter().copied();
    let origin_at = |ordinal: u32| {
        usize::try_from(ordinal)
            .ok()
            .and_then(|ordinal| traced.get(ordinal))
            .map_or(tex_state::token::OriginId::UNKNOWN, |token| token.origin())
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
    traced: &[TracedTokenWord],
    ordinals: &[u32],
) {
    let mut ordinals = ordinals.iter().copied();
    rebind_graph_origins_inner(stores, nodes, traced, &mut ordinals);
}

fn rebind_graph_origins_inner(
    stores: &mut Universe,
    nodes: &mut [tex_state::node::Node],
    traced: &[TracedTokenWord],
    ordinals: &mut impl Iterator<Item = u32>,
) {
    let origin_at = |ordinal: u32| {
        usize::try_from(ordinal)
            .ok()
            .and_then(|ordinal| traced.get(ordinal))
            .map_or(tex_state::token::OriginId::UNKNOWN, |token| token.origin())
    };
    let rebuild = |stores: &mut Universe,
                   id: tex_state::ids::NodeListId,
                   traced: &[TracedTokenWord],
                   ordinals: &mut _| {
        let mut children = stores.nodes(id).to_vec();
        rebind_graph_origins_inner(stores, &mut children, traced, ordinals);
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
                box_node.children = rebuild(stores, box_node.children, traced, ordinals);
            }
            tex_state::node::Node::Glue {
                leader:
                    Some(
                        tex_state::node::LeaderPayload::HList(box_node)
                        | tex_state::node::LeaderPayload::VList(box_node),
                    ),
                ..
            } => {
                box_node.children = rebuild(stores, box_node.children, traced, ordinals);
            }
            tex_state::node::Node::Unset(unset) => {
                unset.children = rebuild(stores, unset.children, traced, ordinals);
            }
            tex_state::node::Node::Disc {
                pre, post, replace, ..
            } => {
                *pre = rebuild(stores, *pre, traced, ordinals);
                *post = rebuild(stores, *post, traced, ordinals);
                *replace = rebuild(stores, *replace, traced, ordinals);
            }
            tex_state::node::Node::Ins { content, .. } | tex_state::node::Node::Adjust(content) => {
                *content = rebuild(stores, *content, traced, ordinals);
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
    let dependencies = match dependencies {
        Some(dependencies) => dependencies,
        None => {
            recording
                .barriers
                .insert(tex_state::ParagraphBarrierReason::UnsupportedInputTransition);
            Vec::new()
        }
    };
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
    let mut consumed_spans = Vec::new();
    for token in &recording.trace {
        if let Some(span) = stores.root_span_for_origin(token.origin())
            && !consumed_spans.contains(&span)
        {
            consumed_spans.push(span);
        }
    }
    let ending_input = input.publication_summary(stores);
    let eligible = recording.barriers.is_empty();
    let pending = execution.pending_paragraph_memo.as_ref();
    let key = pending.map_or_else(
        || {
            let semantic = recording
                .trace
                .iter()
                .map(|token| tex_expand::semantic_token(*token))
                .collect::<Vec<_>>();
            paragraph_key(stores, &semantic)
        },
        |pending| pending.key,
    );
    let trace_origins = pending.map_or_else(
        || recording.trace.iter().map(|token| token.origin()).collect(),
        |pending| pending.trace_origins.clone(),
    );
    let (hlist, origin_ordinals) = if eligible {
        let list = stores.freeze_node_list(nodes);
        (
            Some(stores.retain_paragraph_result(list)),
            paragraph_origin_ordinals(nodes, &trace_origins),
        )
    } else {
        (None, Vec::new())
    };
    stores.record_paragraph_region(tex_state::RecordedParagraphRegion {
        key,
        consumed_spans,
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
    if eligible && execution.pending_paragraph_memo.is_none() {
        execution.pending_paragraph_memo =
            Some(crate::executor::PendingParagraphMemo { key, trace_origins });
    }
}

fn paragraph_origin_ordinals(
    nodes: &[tex_state::node::Node],
    trace_origins: &[tex_state::token::OriginId],
) -> Vec<u32> {
    let ordinal = |origin| {
        trace_origins
            .iter()
            .position(|candidate| *candidate == origin)
            .and_then(|index| u32::try_from(index).ok())
            .unwrap_or(u32::MAX)
    };
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
    let Some(dependencies) = paragraph_break_dependencies(stores, nodes) else {
        return;
    };
    let list = stores.freeze_node_list(nodes);
    let retained = stores.retain_paragraph_result(list);
    let ordinals = paragraph_graph_origin_ordinals(stores, nodes, &pending.trace_origins);
    stores.finish_recorded_paragraph_lines(dependencies, retained, line_count, ordinals);
}

fn paragraph_graph_origin_ordinals(
    stores: &Universe,
    nodes: &[tex_state::node::Node],
    trace_origins: &[tex_state::token::OriginId],
) -> Vec<u32> {
    fn visit(
        stores: &Universe,
        nodes: &[tex_state::node::Node],
        trace_origins: &[tex_state::token::OriginId],
        out: &mut Vec<u32>,
    ) {
        let ordinal = |origin| {
            trace_origins
                .iter()
                .position(|candidate| *candidate == origin)
                .and_then(|index| u32::try_from(index).ok())
                .unwrap_or(u32::MAX)
        };
        let child = |id, out: &mut Vec<u32>| {
            let nodes = stores.nodes(id).to_vec();
            visit(stores, &nodes, trace_origins, out);
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
    visit(stores, nodes, trace_origins, &mut out);
    out
}

fn paragraph_break_dependencies(
    stores: &mut Universe,
    nodes: &[tex_state::node::Node],
) -> Option<Vec<tex_state::ObservedDependency>> {
    use tex_state::{
        DependencyBank as Bank, DependencyCodeTable as Code, DependencyEngineField as Engine,
        DependencyFontField as Font, DependencyKey as Key,
    };

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
    keys.into_iter()
        .map(|key| {
            Some(tex_state::ObservedDependency {
                key,
                changed_at: stores.track_dependency(key),
                value: stores.semantic_dependency_value(key)?,
            })
        })
        .collect()
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
