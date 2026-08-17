//! Production output seam for the first bounded native batch episode.

use std::fmt;
use std::sync::Arc;

use tex_arith::Scaled;
use tex_command::{
    CharacterCode, CommandProfile, NativeBatchBarrier, NativeBatchNode, NativeBatchProgram,
    NativeBatchRequiredBarrier,
};
use tex_fonts::LoadedFont;
use tex_out::{
    BoxNode, ContentHash, FontResource, FontResourceConstruction, GlueOrder, GlueSetRatio,
    GlueSign, JobInfo, KernKind, PageArtifact, PageNode, UnvalidatedPageArtifact,
};
use tex_state::{EffectRecord, Universe};

use crate::{
    EpisodeCommit, EpisodeCommitBoundary, EpisodeCoverageFallback, EpisodeCoverageFamily,
    EpisodeTelemetry, SemanticEpisodeBarrier,
};

const DVI_ONE_INCH: i32 = 4_736_286;

/// Complete immutable input to the bounded production batch runner.
#[derive(Clone, Debug)]
pub struct NativeBatchRequest {
    pub source: Arc<[u8]>,
    pub expected_calls: usize,
    pub profile: CommandProfile,
    pub font_id: u32,
    pub font: LoadedFont,
}

/// Exact implementation reason for a temporary native-episode coverage gap.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NativeBatchFallbackReason {
    Command(NativeBatchBarrier),
}

/// Typed and counted fallback from the native episode to canonical stepping.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeBatchFallback {
    pub reason: NativeBatchFallbackReason,
    pub protocol: EpisodeCoverageFallback,
    pub telemetry: EpisodeTelemetry,
}

/// Result of attempting the bounded production batch path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NativeBatchAttempt {
    Completed(Box<NativeBatchResult>),
    Fallback(NativeBatchFallback),
    Barrier {
        barrier: SemanticEpisodeBarrier,
        telemetry: EpisodeTelemetry,
    },
}

/// Complete state, artifact, byte, DVI, effect, and diagnostic projection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeBatchResult {
    pub counts: [i32; 3],
    pub artifact: PageArtifact,
    pub artifact_bytes: Vec<u8>,
    pub dvi: Vec<u8>,
    pub effects: Vec<EffectRecord>,
    pub terminal: Vec<u8>,
    pub log: Vec<u8>,
    pub calls: usize,
    pub commit: EpisodeCommit,
    pub telemetry: EpisodeTelemetry,
}

/// Failure after a program has completed its typed admission boundary.
#[derive(Debug)]
pub enum NativeBatchRunError {
    DimensionOverflow,
    Artifact(tex_out::ArtifactValidationError),
    Serialize(tex_out::SerializeError),
    Parse(tex_out::ParseError),
    Dvi(tex_out::dvi::DviError),
}

impl fmt::Display for NativeBatchRunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "native batch episode failed: {self:?}")
    }
}

impl std::error::Error for NativeBatchRunError {}

/// Attempts one atomic, effect-free batch episode.
///
/// Canonical tokenization and the complete supported-vocabulary check happen
/// before direct execution. Every command-side refusal is returned as a typed
/// fallback while `stores` and all host-visible output remain unchanged.
pub fn run_native_batch_episode(
    stores: &mut Universe,
    request: NativeBatchRequest,
) -> Result<NativeBatchAttempt, NativeBatchRunError> {
    let mut telemetry = EpisodeTelemetry::default();
    telemetry.record_attempt();
    let program = match NativeBatchProgram::compile(
        Arc::clone(&request.source),
        request.profile,
        stores.endlinechar(),
        |code: CharacterCode| {
            let Ok(byte) = code.to_byte() else {
                unreachable!("exact-byte admission checked first");
            };
            stores.catcode(char::from(byte))
        },
        request.expected_calls,
    ) {
        Ok(program) => program,
        Err(barrier) => {
            if let Some(required) = semantic_barrier(&barrier) {
                telemetry.record_semantic_barrier(required);
                return Ok(NativeBatchAttempt::Barrier {
                    barrier: required,
                    telemetry,
                });
            }
            let protocol = EpisodeCoverageFallback::mutation_free(coverage_family(&barrier));
            telemetry.record_fallback(protocol);
            return Ok(NativeBatchAttempt::Fallback(NativeBatchFallback {
                reason: NativeBatchFallbackReason::Command(barrier),
                protocol,
                telemetry,
            }));
        }
    };
    let Some(metrics) = request.font.character_metrics('A') else {
        telemetry.record_semantic_barrier(SemanticEpisodeBarrier::Diagnostic);
        return Ok(NativeBatchAttempt::Barrier {
            barrier: SemanticEpisodeBarrier::Diagnostic,
            telemetry,
        });
    };

    let rollback = stores.snapshot_for_local_retry();
    let attempt = (|| {
        let outcome = match program.execute(stores) {
            Ok(outcome) => outcome,
            Err(barrier) => {
                if let Some(required) = semantic_barrier(&barrier) {
                    return Ok(NativeBatchAttempt::Barrier {
                        barrier: required,
                        telemetry,
                    });
                }
                let protocol = EpisodeCoverageFallback::rolled_back(coverage_family(&barrier));
                return Ok(NativeBatchAttempt::Fallback(NativeBatchFallback {
                    reason: NativeBatchFallbackReason::Command(barrier),
                    protocol,
                    telemetry,
                }));
            }
        };

        let mut width = 0_i32;
        let mut nodes = Vec::with_capacity(outcome.nodes.len());
        for node in outcome.nodes {
            let (page_node, contribution) = match node {
                NativeBatchNode::Character(ch) => (
                    PageNode::Char {
                        font_id: request.font_id,
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
                font_id: request.font_id,
                name: request.font.name().to_owned(),
                tfm_content_hash: ContentHash::new(request.font.content_hash()),
                tfm_checksum: request.font.checksum(),
                design_size: request.font.design_size(),
                at_size: request.font.size(),
                layout_policy: request.font.layout_policy(),
                mapping_fallback: request.font.mapping_fallback(),
                opentype: None,
                semantic_identity: request.font.source_identity(),
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
        let artifact =
            PageArtifact::from_bytes(&artifact_bytes).map_err(NativeBatchRunError::Parse)?;
        let plan =
            tex_out::dvi::DviPagePlan::compile(&artifact).map_err(NativeBatchRunError::Dvi)?;
        let mut writer = tex_out::dvi::DviStreamWriter::new(Vec::new());
        writer
            .write_page_plan(&plan)
            .map_err(NativeBatchRunError::Dvi)?;
        let dvi = writer.finish().map_err(NativeBatchRunError::Dvi)?;
        let terminal = format!(
            "[{}.{}.{}]",
            outcome.counts[0], outcome.counts[1], outcome.counts[2]
        )
        .into_bytes();
        let log = terminal.clone();
        let commit = EpisodeCommit::new(
            1,
            EpisodeCommitBoundary::Semantic(SemanticEpisodeBarrier::Output),
        );
        Ok(NativeBatchAttempt::Completed(Box::new(NativeBatchResult {
            counts: outcome.counts,
            artifact,
            artifact_bytes,
            dvi,
            effects: Vec::new(),
            terminal,
            log,
            calls: outcome.calls,
            commit,
            telemetry,
        })))
    })();
    if matches!(&attempt, Ok(NativeBatchAttempt::Completed(_))) {
        stores.commit_local_retry_snapshot(rollback);
    } else {
        stores.rollback_local_retry_snapshot(rollback);
    }
    match attempt {
        Ok(NativeBatchAttempt::Completed(mut result)) => {
            result.telemetry.record_commit(result.commit);
            Ok(NativeBatchAttempt::Completed(result))
        }
        Ok(NativeBatchAttempt::Fallback(mut fallback)) => {
            fallback.telemetry.record_fallback(fallback.protocol);
            fallback.telemetry.record_coverage_rollback();
            Ok(NativeBatchAttempt::Fallback(fallback))
        }
        Ok(NativeBatchAttempt::Barrier {
            barrier,
            mut telemetry,
        }) => {
            telemetry.record_rollback(barrier);
            Ok(NativeBatchAttempt::Barrier { barrier, telemetry })
        }
        Err(error) => Err(error),
    }
}

fn semantic_barrier(barrier: &NativeBatchBarrier) -> Option<SemanticEpisodeBarrier> {
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

fn coverage_family(barrier: &NativeBatchBarrier) -> EpisodeCoverageFamily {
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
