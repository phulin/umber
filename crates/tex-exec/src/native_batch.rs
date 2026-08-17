//! Production output seam for the first bounded native batch episode.

use std::fmt;

use tex_command::{
    NativeBatchBarrier, NativeBatchNodeSink, NativeBatchProgram, NativeBatchRequiredBarrier,
};
use tex_fonts::LoadedFont;
use tex_state::Universe;
use tex_state::glue::Order;
use tex_state::ids::FontId;
use tex_state::node::{BoxLr, BoxNode, BoxNodeFields, KernKind, Node, Sign};
use tex_state::node_arena::NodeListBuilder;
use tex_state::provenance::OriginRef;

use crate::{EpisodeCoverageFallback, EpisodeCoverageFamily, SemanticEpisodeBarrier};

/// Result of executing one already-admitted packed program.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PackedEpisodeAttempt {
    Completed(Box<PackedEpisodeOutput>),
    Coverage(EpisodeCoverageFallback),
    Barrier(SemanticEpisodeBarrier),
}

/// Page output produced inside MainControl's aggregate transaction.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PackedEpisodeOutput {
    pub counts: [i32; 3],
    pub root: Node,
    pub fuel_charges: u64,
}

/// Failure after a program has completed its typed admission boundary.
#[derive(Debug)]
pub enum NativeBatchRunError {
    DimensionOverflow,
}

impl fmt::Display for NativeBatchRunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DimensionOverflow => formatter.write_str("packed episode dimension overflow"),
        }
    }
}

impl std::error::Error for NativeBatchRunError {}

/// Executes one already-admitted packed program inside its caller's aggregate
/// transaction. Main control owns that transaction so state, output, and the
/// typed barrier all cross one rollback boundary.
pub(crate) fn execute_packed_episode(
    stores: &mut Universe,
    program: &NativeBatchProgram,
    font_id: FontId,
    font: &LoadedFont,
) -> Result<PackedEpisodeAttempt, NativeBatchRunError> {
    let Some(metrics) = font.character_metrics('A') else {
        return Ok(PackedEpisodeAttempt::Barrier(
            SemanticEpisodeBarrier::Diagnostic,
        ));
    };
    (|| {
        let mut nodes = CanonicalNodeSink {
            builder: stores.node_list_builder(),
            font: font_id,
        };
        let outcome = match program.execute(stores, &mut nodes) {
            Ok(outcome) => outcome,
            Err(barrier) => {
                if let Some(required) = semantic_barrier(&barrier) {
                    return Ok(PackedEpisodeAttempt::Barrier(required));
                }
                let protocol = EpisodeCoverageFallback::rolled_back(coverage_family(&barrier));
                return Ok(PackedEpisodeAttempt::Coverage(protocol));
            }
        };

        let width = nodes
            .builder
            .as_slice()
            .iter()
            .try_fold(0_i32, |width, node| {
                let contribution = match node {
                    Node::Char { .. } => metrics.width.raw(),
                    Node::Kern { amount, .. } => amount.raw(),
                    _ => unreachable!("packed sink emits only migrated native families"),
                };
                width.checked_add(contribution)
            })
            .ok_or(NativeBatchRunError::DimensionOverflow)?;
        let children = stores.freeze_node_list_ref(nodes.builder);
        let root = Node::HList(BoxNode::new(BoxNodeFields {
            width: tex_arith::Scaled::from_raw(width),
            height: metrics.height,
            depth: metrics.depth,
            shift: tex_arith::Scaled::from_raw(0),
            box_lr: BoxLr::Normal,
            glue_set: tex_arith::GlueSetRatio::ZERO,
            glue_sign: Sign::Normal,
            glue_order: Order::Normal,
            children,
        }));
        Ok(PackedEpisodeAttempt::Completed(Box::new(
            PackedEpisodeOutput {
                counts: outcome.counts,
                root,
                fuel_charges: outcome.fuel_charges,
            },
        )))
    })()
}

struct CanonicalNodeSink {
    builder: NodeListBuilder,
    font: FontId,
}

impl NativeBatchNodeSink for CanonicalNodeSink {
    fn reserve(&mut self, additional: usize) {
        self.builder.reserve(additional);
    }

    fn character(&mut self, ch: u8) {
        self.builder.push(Node::Char {
            font: self.font,
            ch: char::from(ch),
            origin: OriginRef::unknown(),
        });
    }

    fn kern(&mut self, amount: i32) {
        self.builder.push(Node::Kern {
            amount: tex_arith::Scaled::from_raw(amount),
            kind: KernKind::Explicit,
        });
    }
}

pub(crate) fn semantic_barrier(barrier: &NativeBatchBarrier) -> Option<SemanticEpisodeBarrier> {
    match barrier {
        NativeBatchBarrier::State(
            tex_state::CountGroupEpisodeBarrier::ActiveTrackedRegion
            | tex_state::CountGroupEpisodeBarrier::ObservableGroupTracing,
        ) => Some(SemanticEpisodeBarrier::Observer),
        NativeBatchBarrier::Required(required) => Some(match required {
            NativeBatchRequiredBarrier::Resource => SemanticEpisodeBarrier::Resource,
            NativeBatchRequiredBarrier::Effect => SemanticEpisodeBarrier::Effect,
            NativeBatchRequiredBarrier::Diagnostic => SemanticEpisodeBarrier::Diagnostic,
            NativeBatchRequiredBarrier::Format => SemanticEpisodeBarrier::Format,
        }),
        NativeBatchBarrier::CharacterMode
        | NativeBatchBarrier::SourceRegistration(_)
        | NativeBatchBarrier::InvalidCharacter
        | NativeBatchBarrier::UnsupportedCharacter
        | NativeBatchBarrier::UnsupportedCatcode(_)
        | NativeBatchBarrier::UnsupportedControlSequence(_)
        | NativeBatchBarrier::MaterialAfterEnd
        | NativeBatchBarrier::MissingEnd
        | NativeBatchBarrier::Malformed(_)
        | NativeBatchBarrier::ArithmeticOverflow => None,
    }
}

pub(crate) fn coverage_family(barrier: &NativeBatchBarrier) -> EpisodeCoverageFamily {
    match barrier {
        NativeBatchBarrier::CharacterMode => EpisodeCoverageFamily::CharacterProfile,
        NativeBatchBarrier::SourceRegistration(_)
        | NativeBatchBarrier::InvalidCharacter
        | NativeBatchBarrier::UnsupportedCharacter
        | NativeBatchBarrier::UnsupportedCatcode(_)
        | NativeBatchBarrier::MaterialAfterEnd
        | NativeBatchBarrier::MissingEnd => EpisodeCoverageFamily::SourceTokenization,
        NativeBatchBarrier::UnsupportedControlSequence(_) => {
            EpisodeCoverageFamily::CommandVocabulary
        }
        NativeBatchBarrier::Required(_) => {
            unreachable!("required barriers are never coverage fallback")
        }
        NativeBatchBarrier::Malformed(_) | NativeBatchBarrier::ArithmeticOverflow => {
            EpisodeCoverageFamily::ScannerOrExpansion
        }
        NativeBatchBarrier::State(_) => EpisodeCoverageFamily::RollbackLineage,
    }
}

#[cfg(test)]
mod tests;
