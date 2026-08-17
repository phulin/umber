//! Production output seam for the first bounded native batch episode.

use std::fmt;

use tex_command::{
    CommandHostCapabilities, CommandHostContext, CommandProcessor, CommandState,
    NativeBatchBarrier, NativeBatchNodeSink, NativeBatchProgram, NativeBatchRequiredBarrier,
};
use tex_fonts::LoadedFont;
use tex_state::Universe;
use tex_state::glue::Order;
use tex_state::ids::FontId;
use tex_state::node::{BoxLr, BoxNode, BoxNodeFields, KernKind, Node, Sign};
use tex_state::node_arena::NodeListBuilder;

use crate::{EpisodeCoverageFallback, EpisodeCoverageFamily, SemanticEpisodeBarrier};

/// Result of executing one canonical-input episode attempt.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PackedEpisodeAttempt {
    Completed(Box<PackedEpisodeOutput>),
    Coverage(EpisodeCoverageFallback),
    Barrier(SemanticEpisodeBarrier),
    RootCompletion,
}

/// Page output produced inside MainControl's aggregate transaction.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PackedEpisodeOutput {
    pub counts: [i32; 3],
    pub root: Node,
    pub fuel_charges: u64,
}

/// Failure while lowering the completed episode into output.
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

/// Executes one canonical-input episode inside its caller's aggregate
/// transaction. Main control owns that transaction so command input, state,
/// output, and the typed barrier all cross one rollback boundary.
pub(crate) fn execute_packed_episode(
    stores: &mut Universe,
    command: &mut CommandState,
    capabilities: &mut CommandHostCapabilities,
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
        let outcome = {
            let mut processor = CommandProcessor::new(
                command,
                stores.command_context(),
                CommandHostContext::new(capabilities),
            );
            program.execute(&mut processor, &mut nodes)
        };
        let outcome = match outcome {
            Ok(outcome) => outcome,
            Err(NativeBatchBarrier::RootCompletion) => {
                return Ok(PackedEpisodeAttempt::RootCompletion);
            }
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
            .compact_width(metrics.width)
            .ok_or(NativeBatchRunError::DimensionOverflow)?;
        let children = stores.freeze_node_list_ref(nodes.builder);
        let root = Node::HList(BoxNode::new(BoxNodeFields {
            width,
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
        self.builder
            .push_unknown_character(self.font, char::from(ch));
    }

    fn kern(&mut self, amount: i32) {
        self.builder
            .push_kern(tex_arith::Scaled::from_raw(amount), KernKind::Explicit);
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
        NativeBatchBarrier::Command(error) => Some(match error {
            tex_command::CommandError::MissingInput { .. }
            | tex_command::CommandError::MissingInputProbe(_) => SemanticEpisodeBarrier::Resource,
            tex_command::CommandError::FuelExhausted { .. } => SemanticEpisodeBarrier::Fuel,
            tex_command::CommandError::InputInvariant(_)
            | tex_command::CommandError::StaleDelivery
            | tex_command::CommandError::MacroPrefixMismatch
            | tex_command::CommandError::ParagraphInMacroArgument
            | tex_command::CommandError::OuterInMacroArgument
            | tex_command::CommandError::AtOrigin { .. }
            | tex_command::CommandError::UnsupportedExpandablePrimitive(_)
            | tex_command::CommandError::PdfNavigation(_)
            | tex_command::CommandError::Fatal(_) => SemanticEpisodeBarrier::Diagnostic,
        }),
        NativeBatchBarrier::UnsupportedCommand(_)
        | NativeBatchBarrier::RootCompletion
        | NativeBatchBarrier::Malformed(_)
        | NativeBatchBarrier::ArithmeticOverflow => None,
    }
}

pub(crate) fn coverage_family(barrier: &NativeBatchBarrier) -> EpisodeCoverageFamily {
    match barrier {
        NativeBatchBarrier::UnsupportedCommand(_) => EpisodeCoverageFamily::CommandVocabulary,
        NativeBatchBarrier::Required(_) => {
            unreachable!("required barriers are never coverage fallback")
        }
        NativeBatchBarrier::Command(_) => {
            unreachable!("command failures are semantic barriers, never coverage fallback")
        }
        NativeBatchBarrier::RootCompletion => {
            unreachable!("root completion returns to canonical main control")
        }
        NativeBatchBarrier::Malformed(_) | NativeBatchBarrier::ArithmeticOverflow => {
            EpisodeCoverageFamily::ScannerOrExpansion
        }
        NativeBatchBarrier::State(_) => EpisodeCoverageFamily::RollbackLineage,
    }
}

#[cfg(test)]
mod tests;
