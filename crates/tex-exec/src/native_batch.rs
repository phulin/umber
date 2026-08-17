//! Production output seam for the first bounded native batch episode.

use std::fmt;

use tex_arith::Scaled;
use tex_command::{
    NativeBatchBarrier, NativeBatchNode, NativeBatchProgram, NativeBatchRequiredBarrier,
};
use tex_fonts::LoadedFont;
use tex_out::{
    BoxNode, ContentHash, FontResource, FontResourceConstruction, GlueOrder, GlueSetRatio,
    GlueSign, JobInfo, KernKind, PageArtifact, PageNode, UnvalidatedPageArtifact,
};
use tex_state::Universe;

use crate::{EpisodeCoverageFallback, EpisodeCoverageFamily, SemanticEpisodeBarrier};

const DVI_ONE_INCH: i32 = 4_736_286;

/// Result of executing one already-admitted packed program.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PackedEpisodeAttempt {
    Completed(Box<PackedEpisodeOutput>),
    Coverage(EpisodeCoverageFallback),
    Barrier(SemanticEpisodeBarrier),
}

/// Page output produced inside MainControl's aggregate transaction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PackedEpisodeOutput {
    pub counts: [i32; 3],
    pub artifact: PageArtifact,
    pub artifact_bytes: Vec<u8>,
    pub fuel_charges: u64,
}

/// Failure after a program has completed its typed admission boundary.
#[derive(Debug)]
pub enum NativeBatchRunError {
    DimensionOverflow,
    Artifact(tex_out::ArtifactValidationError),
    Serialize(tex_out::SerializeError),
}

impl fmt::Display for NativeBatchRunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DimensionOverflow => formatter.write_str("packed episode dimension overflow"),
            Self::Artifact(error) => write!(formatter, "invalid packed episode artifact: {error}"),
            Self::Serialize(error) => {
                write!(
                    formatter,
                    "unable to serialize packed episode artifact: {error}"
                )
            }
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
    font_id: u32,
    font: &LoadedFont,
) -> Result<PackedEpisodeAttempt, NativeBatchRunError> {
    let Some(metrics) = font.character_metrics('A') else {
        return Ok(PackedEpisodeAttempt::Barrier(
            SemanticEpisodeBarrier::Diagnostic,
        ));
    };
    (|| {
        let outcome = match program.execute(stores) {
            Ok(outcome) => outcome,
            Err(barrier) => {
                if let Some(required) = semantic_barrier(&barrier) {
                    return Ok(PackedEpisodeAttempt::Barrier(required));
                }
                let protocol = EpisodeCoverageFallback::rolled_back(coverage_family(&barrier));
                return Ok(PackedEpisodeAttempt::Coverage(protocol));
            }
        };

        let mut width = 0_i32;
        let mut nodes = Vec::with_capacity(outcome.nodes.len());
        for node in outcome.nodes {
            let (page_node, contribution) = match node {
                NativeBatchNode::Character(ch) => (
                    PageNode::Char {
                        font_id,
                        ch: u32::from(ch),
                        width: metrics.width,
                    },
                    metrics.width.raw(),
                ),
                NativeBatchNode::Kern(amount) => (
                    PageNode::Kern {
                        amount: Scaled::from_raw(amount),
                        kind: KernKind::Explicit,
                    },
                    amount,
                ),
            };
            width = width
                .checked_add(contribution)
                .ok_or(NativeBatchRunError::DimensionOverflow)?;
            nodes.push(page_node);
        }
        let root = PageNode::HList(BoxNode {
            width: Scaled::from_raw(width),
            height: metrics.height,
            depth: metrics.depth,
            shift: Scaled::from_raw(0),
            glue_set: GlueSetRatio::ZERO,
            glue_sign: GlueSign::Normal,
            glue_order: GlueOrder::Normal,
            children: nodes,
        });
        let mut page_counts = [0; 10];
        page_counts[..3].copy_from_slice(&outcome.counts);
        let artifact = UnvalidatedPageArtifact {
            job: JobInfo {
                mag: 1000,
                banner: tex_out::DEFAULT_BANNER.to_owned(),
                h_offset: Scaled::from_raw(0),
                v_offset: Scaled::from_raw(0),
                page_origin_x: Scaled::from_raw(DVI_ONE_INCH),
                page_origin_y: Scaled::from_raw(DVI_ONE_INCH),
                page_width: Scaled::from_raw(0),
                page_height: Scaled::from_raw(0),
            },
            fonts: vec![FontResource {
                font_id,
                name: font.name().to_owned(),
                tfm_content_hash: ContentHash::new(font.content_hash()),
                tfm_checksum: font.checksum(),
                design_size: font.design_size(),
                at_size: font.size(),
                layout_policy: font.layout_policy(),
                mapping_fallback: font.mapping_fallback(),
                opentype: None,
                semantic_identity: font.source_identity(),
                construction: FontResourceConstruction::Loaded,
            }],
            counts: page_counts,
            root,
            effects: Vec::new(),
            math_events: Vec::new(),
        }
        .validate()
        .map_err(NativeBatchRunError::Artifact)?;
        let artifact_bytes = artifact
            .to_bytes()
            .map_err(NativeBatchRunError::Serialize)?;
        Ok(PackedEpisodeAttempt::Completed(Box::new(
            PackedEpisodeOutput {
                counts: outcome.counts,
                artifact,
                artifact_bytes,
                fuel_charges: outcome.fuel_charges,
            },
        )))
    })()
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
