//! Recorder-driven paragraph front-end eligibility and transitional detached reuse.

use tex_lex::InputStack;
use tex_state::env::banks::{DimenParam, GlueParam, IntParam, TokParam};
use tex_state::meaning::{Meaning, UnexpandablePrimitive};
use tex_state::token::{Catcode, Token, TracedTokenWord};
use tex_state::{
    ContentHash, DetachedVirtualEffect, EffectRecord, ExpansionState, MemoValueLimits, PrintSink,
    PureMemoKey, Universe,
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
    let every_par = stores.tok_param(TokParam::EVERY_PAR);
    if !stores.tokens(every_par).is_empty() {
        // Cold execution records `everypar` like any other token-list input.
        // Until the prior-generation lookup path lands, do not speculate past
        // it and do not classify it as a barrier.
        return Ok(false);
    }

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
            } => {
                semantic.push(token);
            }
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
        let boundary_only = traced.last().is_some_and(|last| {
            matches!(
                tex_expand::semantic_token(*last),
                Token::Char {
                    cat: Catcode::BeginGroup | Catcode::EndGroup,
                    ..
                }
            ) && traced[..traced.len() - 1].iter().all(|token| {
                matches!(
                    tex_expand::semantic_token(*token),
                    Token::Char {
                        cat: Catcode::Space,
                        ..
                    }
                )
            })
        });
        crate::push_traced_tokens(input, stores, traced);
        let _ = boundary_only;
        return Ok(false);
    }

    let key = paragraph_key(stores, &semantic);
    let Some(entry) = stores.lookup_pure_paragraph(key) else {
        let trace_origins = traced.iter().map(|token| token.origin()).collect();
        crate::push_traced_tokens(input, stores, traced);
        execution.pending_paragraph_memo = Some(crate::executor::PendingParagraphMemo {
            key,
            effect_start: stores.world().effect_records().len(),
            trace_origins,
        });
        stores.begin_pure_paragraph_recording();
        return Ok(false);
    };

    if !validate_mutations(stores, &entry.mutations) {
        stores.record_pure_paragraph_validation_miss();
        let trace_origins = traced.iter().map(|token| token.origin()).collect();
        crate::push_traced_tokens(input, stores, traced);
        execution.pending_paragraph_memo = Some(crate::executor::PendingParagraphMemo {
            key,
            effect_start: stores.world().effect_records().len(),
            trace_origins,
        });
        stores.begin_pure_paragraph_recording();
        return Ok(false);
    }
    let imported_bytes = entry.hlist.retained_bytes();
    let list = match stores.import_memo_node_list(&entry.hlist, MemoValueLimits::default()) {
        Ok(list) => list,
        Err(_) => {
            stores.record_pure_paragraph_import_failure();
            stores.reject_pure_memo(key);
            let trace_origins = traced.iter().map(|token| token.origin()).collect();
            crate::push_traced_tokens(input, stores, traced);
            execution.pending_paragraph_memo = Some(crate::executor::PendingParagraphMemo {
                key,
                effect_start: stores.world().effect_records().len(),
                trace_origins,
            });
            stores.begin_pure_paragraph_recording();
            return Ok(false);
        }
    };
    replay_mutations(stores, &entry.mutations);
    replay_effects(stores, &entry.effects);
    let mut nodes: Vec<_> = stores
        .nodes(list)
        .into_iter()
        .map(|node| node.to_owned())
        .collect();
    rebind_literal_origins(&mut nodes, &traced, &entry.origin_ordinals);
    crate::assignments::install_reused_paragraph_hlist(nest, input, stores, nodes)?;
    stats.delivered_tokens = stats.delivered_tokens.saturating_add(traced.len());
    stores.record_pure_paragraph_hit(traced.len(), entry.mutations.len(), imported_bytes);
    Ok(true)
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
            Token::Param(_) | Token::Frozen(_) => {
                unreachable!("literal paragraph preflight rejects non-literal tokens")
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

pub(crate) fn publish_prepared_hlist(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut ExecutionContext<'_>,
    nodes: &[tex_state::node::Node],
) {
    let (recorded_mutations, recorded_eligible) = publish_recorded_region(input, stores, execution);
    let Some(pending) = execution.pending_paragraph_memo.take() else {
        return;
    };
    if !recorded_eligible {
        return;
    }
    let mutations = recorded_mutations
        .unwrap_or_else(|| stores.finish_pure_paragraph_recording().unwrap_or_default());
    let Some(effects) = detach_effects(&stores.world().effect_records()[pending.effect_start..])
    else {
        return;
    };
    let origin_ordinals = paragraph_origin_ordinals(nodes, &pending.trace_origins);
    let list = stores.freeze_node_list(nodes);
    if let Ok(detached) = stores.detach_node_list(list) {
        stores.insert_pure_paragraph(
            pending.key,
            tex_state::PureParagraphEntry {
                hlist: detached,
                mutations,
                effects,
                origin_ordinals,
            },
        );
    }
}

fn publish_recorded_region(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut ExecutionContext<'_>,
) -> (Option<Vec<tex_state::PureParagraphMutation>>, bool) {
    let Some(mut recording) = execution.cold_paragraph_recording.take() else {
        return (None, true);
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
    stores.record_paragraph_region(tex_state::RecordedParagraphRegion {
        consumed_spans,
        dependencies,
        mutations: mutations.clone(),
        effects,
        ending_input,
        barriers: recording.barriers.into_iter().collect(),
    });
    (Some(mutations), eligible)
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
