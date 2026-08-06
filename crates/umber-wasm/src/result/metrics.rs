use wasm_bindgen::{JsCast as _, JsValue};

use crate::{
    JsAcceptedInputObservationLedger, JsRenderedSourceResult, JsRetentionMetrics, JsReuseMetrics,
    wire,
};

pub(crate) fn reuse_metrics(
    metrics: Option<umber::ReuseMetrics>,
) -> Result<Option<JsReuseMetrics>, JsValue> {
    metrics
        .map(reuse_metrics_dto)
        .transpose()?
        .map(|dto| Ok(wire::to_js_value(&dto)?.unchecked_into()))
        .transpose()
}

fn reuse_metrics_dto(metrics: umber::ReuseMetrics) -> Result<wire::ReuseMetricsDto, JsValue> {
    Ok(wire::ReuseMetricsDto {
        pages_reused: to_u32(metrics.pages_reused, "pagesReused")?,
        pages_retyped: to_u32(metrics.pages_retyped, "pagesRetyped")?,
        reexecuted_bytes: safe(metrics.reexecuted_bytes, "reexecutedBytes")?,
        reexecuted_tokens: safe(metrics.reexecuted_tokens, "reexecutedTokens")?,
        reexecuted_commands: safe(metrics.reexecuted_commands, "reexecutedCommands")?,
        reexecuted_paragraphs: safe(metrics.reexecuted_paragraphs, "reexecutedParagraphs")?,
        same_history_attempts: to_u32(metrics.same_history_attempts, "sameHistoryAttempts")?,
        same_history_hash_mismatches: to_u32(
            metrics.same_history_hash_mismatches,
            "sameHistoryHashMismatches",
        )?,
        same_history_stop: match metrics.same_history_stop {
            umber::SameHistoryStop::Matched => wire::SameHistoryStopDto::Matched,
            umber::SameHistoryStop::ScheduleDiverged => wire::SameHistoryStopDto::ScheduleDiverged,
            umber::SameHistoryStop::HashesDiverged => wire::SameHistoryStopDto::HashesDiverged,
            umber::SameHistoryStop::NoComparableBoundary => {
                wire::SameHistoryStopDto::NoComparableBoundary
            }
            umber::SameHistoryStop::NotAttempted => wire::SameHistoryStopDto::NotAttempted,
        },
        restart_fork_microseconds: wire::SafeInteger::new(
            metrics
                .restart_fork_latency
                .as_micros()
                .try_into()
                .map_err(|_| {
                    crate::js_error(
                        "restartForkMicroseconds exceeds JavaScript's safe integer range",
                    )
                })?,
        )
        .map_err(crate::boundary_error)?,
        reexecution_microseconds: wire::SafeInteger::new(
            metrics
                .reexecution_latency
                .as_micros()
                .try_into()
                .map_err(|_| {
                    crate::js_error(
                        "reexecutionMicroseconds exceeds JavaScript's safe integer range",
                    )
                })?,
        )
        .map_err(crate::boundary_error)?,
        splice_microseconds: wire::SafeInteger::new(
            metrics.splice_latency.as_micros().try_into().map_err(|_| {
                crate::js_error("spliceMicroseconds exceeds JavaScript's safe integer range")
            })?,
        )
        .map_err(crate::boundary_error)?,
    })
}

pub(crate) fn retention_metrics(
    metrics: Option<umber::RetentionMetrics>,
) -> Result<Option<JsRetentionMetrics>, JsValue> {
    metrics
        .map(|metrics| {
            Ok::<_, JsValue>(wire::RetentionMetricsDto {
                checkpoint_root_bytes: safe(metrics.checkpoint_root_bytes, "checkpointRootBytes")?,
                diagnostic_bytes: safe(metrics.diagnostic_bytes, "diagnosticBytes")?,
                output_bytes: safe(metrics.output_bytes, "outputBytes")?,
                resource_bytes: safe(metrics.resource_bytes, "resourceBytes")?,
                protected_overage_bytes: safe(
                    metrics.protected_overage_bytes,
                    "protectedOverageBytes",
                )?,
            })
        })
        .transpose()?
        .map(|dto| Ok(wire::to_js_value(&dto)?.unchecked_into()))
        .transpose()
}

pub(crate) fn rendered_source_result(
    result: umber::RenderedSourceResult,
) -> Result<JsRenderedSourceResult, JsValue> {
    let dto = match result {
        umber::RenderedSourceResult::Current(location) => wire::RenderedSourceResultDto::Current {
            path: location.path,
            start: to_u32(location.start, "start")?,
            end: to_u32(location.end, "end")?,
            line: location.line,
            column: location.column,
        },
        umber::RenderedSourceResult::Deleted { minted_revision } => {
            wire::RenderedSourceResultDto::Deleted {
                minted_revision: to_u32(minted_revision, "mintedRevision")?,
            }
        }
        umber::RenderedSourceResult::StaleRevision { accepted } => {
            wire::RenderedSourceResultDto::StaleRevision {
                accepted: to_u32(accepted.raw(), "accepted")?,
            }
        }
        umber::RenderedSourceResult::OutputMismatch { accepted } => {
            wire::RenderedSourceResultDto::OutputMismatch {
                accepted_output: accepted.to_string(),
            }
        }
    };
    Ok(wire::to_js_value(&dto)?.unchecked_into())
}

pub(crate) fn accepted_input_observations(
    ledger: Option<&umber::AcceptedInputObservationLedger>,
) -> Result<Option<JsAcceptedInputObservationLedger>, JsValue> {
    let Some(ledger) = ledger else {
        return Ok(None);
    };
    let observations = ledger
        .observations()
        .iter()
        .map(|observation| {
            Ok(wire::AcceptedInputObservationDto {
                path: observation.path().as_str().to_owned(),
                namespace: match observation.namespace() {
                    umber::InputObservationNamespace::Authored => {
                        wire::ObservationNamespaceDto::Authored
                    }
                    umber::InputObservationNamespace::Generated => {
                        wire::ObservationNamespaceDto::Generated
                    }
                    umber::InputObservationNamespace::Distribution => {
                        wire::ObservationNamespaceDto::Distribution
                    }
                },
                outcome: match observation.outcome() {
                    umber::InputObservationOutcome::Present(hash) => {
                        wire::ObservationOutcomeDto::Present {
                            content_hash: hash.hex(),
                        }
                    }
                    umber::InputObservationOutcome::Missing => wire::ObservationOutcomeDto::Missing,
                },
                access: match observation.access() {
                    umber::InputDependencyAccess::RequiredRead => {
                        wire::ObservationAccessDto::RequiredRead
                    }
                    umber::InputDependencyAccess::AuthoritativeProbe => {
                        wire::ObservationAccessDto::AuthoritativeProbe
                    }
                },
                resource_kind: super::file_kind(observation.resource_kind()),
                phase: match observation.phase() {
                    umber::InputObservationPhase::Tex => wire::ObservationPhaseDto::Tex,
                    umber::InputObservationPhase::BibliographyDetection => {
                        wire::ObservationPhaseDto::BibliographyDetection
                    }
                    umber::InputObservationPhase::Bibliography => {
                        wire::ObservationPhaseDto::Bibliography
                    }
                },
                revision: to_u32(observation.revision().raw(), "revision")?,
                project_pass: observation.project_pass(),
                requesting_source: observation
                    .requesting_source()
                    .map(|source| source.as_str().to_owned()),
                owner: match observation.owner() {
                    umber::InputObservationOwner::TexEngine => wire::ObservationOwnerDto::TexEngine,
                    umber::InputObservationOwner::BibliographyDetector => {
                        wire::ObservationOwnerDto::BibliographyDetector
                    }
                    umber::InputObservationOwner::Biblatex => wire::ObservationOwnerDto::Biblatex,
                    umber::InputObservationOwner::ClassicBibtex => {
                        wire::ObservationOwnerDto::ClassicBibtex
                    }
                },
            })
        })
        .collect::<Result<Vec<_>, JsValue>>()?;
    let dto = wire::AcceptedInputObservationLedgerDto {
        schema_version: ledger.schema_version(),
        revision: to_u32(ledger.revision().raw(), "revision")?,
        observations,
    };
    Ok(Some(wire::to_js_value(&dto)?.unchecked_into()))
}

fn safe(value: usize, name: &str) -> Result<wire::SafeInteger, JsValue> {
    wire::SafeInteger::new(value as u64)
        .map_err(|_| crate::js_error(&format!("{name} exceeds JavaScript's safe integer range")))
}

fn to_u32<T>(value: T, name: &str) -> Result<u32, JsValue>
where
    u32: TryFrom<T>,
{
    u32::try_from(value).map_err(|_| crate::js_error(&format!("{name} is out of range")))
}
