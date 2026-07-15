//! Effect-free literal paragraph-front-end memoization.

use tex_lex::InputStack;
use tex_state::env::banks::{DimenParam, GlueParam, IntParam, TokParam};
use tex_state::meaning::{Meaning, UnexpandablePrimitive};
use tex_state::token::{Catcode, Token, TracedTokenWord};
use tex_state::{ContentHash, ExpansionState, MemoValueLimits, PureMemoKey, Universe};

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
        execution.bypass_paragraph_memo_once = true;
        return Ok(false);
    }

    let mut traced = Vec::new();
    let mut semantic = Vec::new();
    let mut terminated = false;
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
            Token::Cs(symbol)
                if matches!(
                    stores.meaning(symbol),
                    Meaning::UnexpandablePrimitive(
                        UnexpandablePrimitive::Par | UnexpandablePrimitive::EndGraf
                    )
                ) =>
            {
                semantic.push(token);
                terminated = true;
                break;
            }
            _ => break,
        }
    }

    let eligible = terminated
        && matches!(semantic.first(), Some(Token::Char { cat, .. }) if *cat != Catcode::Space);
    if !eligible {
        crate::push_traced_tokens(input, stores, traced);
        execution.bypass_paragraph_memo_once = true;
        return Ok(false);
    }

    let key = paragraph_key(stores, &semantic);
    let Some(detached) = stores.lookup_pure_paragraph(key) else {
        crate::push_traced_tokens(input, stores, traced);
        execution.pending_paragraph_memo = Some(key);
        return Ok(false);
    };

    let imported_bytes = detached.retained_bytes();
    let list = match stores.import_memo_node_list(&detached, MemoValueLimits::default()) {
        Ok(list) => list,
        Err(_) => {
            stores.reject_pure_memo(key);
            crate::push_traced_tokens(input, stores, traced);
            execution.pending_paragraph_memo = Some(key);
            return Ok(false);
        }
    };
    let mut nodes: Vec<_> = stores
        .nodes(list)
        .into_iter()
        .map(|node| node.to_owned())
        .collect();
    rebind_literal_origins(&mut nodes, &traced);
    crate::assignments::install_reused_paragraph_hlist(nest, input, stores, nodes)?;
    stats.delivered_tokens = stats.delivered_tokens.saturating_add(traced.len());
    stores.record_pure_paragraph_hit(traced.len(), imported_bytes);
    Ok(true)
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

fn rebind_literal_origins(nodes: &mut [tex_state::node::Node], traced: &[TracedTokenWord]) {
    let mut origins =
        traced
            .iter()
            .filter_map(|traced| match tex_expand::semantic_token(*traced) {
                Token::Char { cat, .. } if cat != Catcode::Space => Some(traced.origin()),
                _ => None,
            });
    for node in nodes {
        match node {
            tex_state::node::Node::Char { origin, .. } => {
                *origin = origins
                    .next()
                    .unwrap_or(tex_state::token::OriginId::UNKNOWN);
            }
            tex_state::node::Node::Lig {
                orig,
                origins: node_origins,
                ..
            } => {
                node_origins.clear();
                node_origins.extend((0..orig.len()).map(|_| {
                    origins
                        .next()
                        .unwrap_or(tex_state::token::OriginId::UNKNOWN)
                }));
            }
            _ => {}
        }
    }
}

pub(crate) fn publish_prepared_hlist(
    stores: &mut Universe,
    execution: &mut ExecutionContext<'_>,
    nodes: &[tex_state::node::Node],
) {
    let Some(key) = execution.pending_paragraph_memo.take() else {
        return;
    };
    let list = stores.freeze_node_list(nodes);
    if let Ok(detached) = stores.detach_node_list(list) {
        stores.insert_pure_paragraph(key, detached);
    }
}
