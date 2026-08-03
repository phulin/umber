use js_sys::{Array, Object};
use wasm_bindgen::{JsCast, JsValue};

use crate::{JsAcceptedInputObservationLedger, JsRenderedSourceResult};

use super::set;

pub(crate) fn reuse_metrics(metrics: Option<umber::ReuseMetrics>) -> Result<JsValue, JsValue> {
    let Some(metrics) = metrics else {
        return Ok(JsValue::UNDEFINED);
    };
    let object = Object::new();
    set(&object, "pagesReused", &usize_value(metrics.pages_reused))?;
    set(&object, "pagesRetyped", &usize_value(metrics.pages_retyped))?;
    set(
        &object,
        "reexecutedBytes",
        &usize_value(metrics.reexecuted_bytes),
    )?;
    set(
        &object,
        "reexecutedTokens",
        &usize_value(metrics.reexecuted_tokens),
    )?;
    set(
        &object,
        "reexecutedCommands",
        &usize_value(metrics.reexecuted_commands),
    )?;
    set(
        &object,
        "reexecutedParagraphs",
        &usize_value(metrics.reexecuted_paragraphs),
    )?;
    set(
        &object,
        "sameHistoryAttempts",
        &usize_value(metrics.same_history_attempts),
    )?;
    set(
        &object,
        "sameHistoryHashMismatches",
        &usize_value(metrics.same_history_hash_mismatches),
    )?;
    let stop = match metrics.same_history_stop {
        umber::SameHistoryStop::Matched => "matched",
        umber::SameHistoryStop::ScheduleDiverged => "schedule-diverged",
        umber::SameHistoryStop::HashesDiverged => "hashes-diverged",
        umber::SameHistoryStop::NoComparableBoundary => "no-comparable-boundary",
        umber::SameHistoryStop::NotAttempted => "not-attempted",
    };
    set(&object, "sameHistoryStop", &JsValue::from_str(stop))?;
    set(
        &object,
        "restartForkMicroseconds",
        &JsValue::from_f64(metrics.restart_fork_latency.as_micros() as f64),
    )?;
    set(
        &object,
        "reexecutionMicroseconds",
        &JsValue::from_f64(metrics.reexecution_latency.as_micros() as f64),
    )?;
    set(
        &object,
        "spliceMicroseconds",
        &JsValue::from_f64(metrics.splice_latency.as_micros() as f64),
    )?;
    Ok(object.into())
}

pub(crate) fn retention_metrics(
    metrics: Option<umber::RetentionMetrics>,
) -> Result<JsValue, JsValue> {
    let Some(metrics) = metrics else {
        return Ok(JsValue::UNDEFINED);
    };
    let object = Object::new();
    set(
        &object,
        "checkpointRootBytes",
        &usize_value(metrics.checkpoint_root_bytes),
    )?;
    set(
        &object,
        "diagnosticBytes",
        &usize_value(metrics.diagnostic_bytes),
    )?;
    set(&object, "outputBytes", &usize_value(metrics.output_bytes))?;
    set(
        &object,
        "resourceBytes",
        &usize_value(metrics.resource_bytes),
    )?;
    set(
        &object,
        "protectedOverageBytes",
        &usize_value(metrics.protected_overage_bytes),
    )?;
    Ok(object.into())
}

pub(crate) fn rendered_source_result(
    result: umber::RenderedSourceResult,
) -> Result<JsRenderedSourceResult, JsValue> {
    let object = Object::new();
    match result {
        umber::RenderedSourceResult::Current(location) => {
            set(&object, "kind", &JsValue::from_str("current"))?;
            set(&object, "path", &JsValue::from_str(&location.path))?;
            set(&object, "start", &JsValue::from_f64(location.start as f64))?;
            set(&object, "end", &JsValue::from_f64(location.end as f64))?;
            set(
                &object,
                "line",
                &JsValue::from_f64(f64::from(location.line)),
            )?;
            set(
                &object,
                "column",
                &JsValue::from_f64(f64::from(location.column)),
            )?;
        }
        umber::RenderedSourceResult::Deleted { minted_revision } => {
            set(&object, "kind", &JsValue::from_str("deleted"))?;
            set(
                &object,
                "mintedRevision",
                &JsValue::from_f64(minted_revision as f64),
            )?;
        }
        umber::RenderedSourceResult::StaleRevision { accepted } => {
            set(&object, "kind", &JsValue::from_str("stale-revision"))?;
            set(
                &object,
                "accepted",
                &JsValue::from_f64(accepted.raw() as f64),
            )?;
        }
        umber::RenderedSourceResult::OutputMismatch { accepted } => {
            set(&object, "kind", &JsValue::from_str("output-mismatch"))?;
            set(
                &object,
                "acceptedOutput",
                &JsValue::from_str(&accepted.to_string()),
            )?;
        }
    }
    Ok(object.unchecked_into())
}

fn usize_value(value: usize) -> JsValue {
    JsValue::from_f64(value as f64)
}

pub(crate) fn accepted_input_observations(
    ledger: Option<&umber::AcceptedInputObservationLedger>,
) -> Result<Option<JsAcceptedInputObservationLedger>, JsValue> {
    let Some(ledger) = ledger else {
        return Ok(None);
    };
    let object = Object::new();
    set(
        &object,
        "schemaVersion",
        &JsValue::from_f64(f64::from(ledger.schema_version())),
    )?;
    set(
        &object,
        "revision",
        &JsValue::from_f64(ledger.revision().raw() as f64),
    )?;
    let observations = Array::new();
    for observation in ledger.observations() {
        let value = Object::new();
        set(
            &value,
            "path",
            &JsValue::from_str(observation.path().as_str()),
        )?;
        set(
            &value,
            "namespace",
            &JsValue::from_str(match observation.namespace() {
                umber::InputObservationNamespace::Authored => "authored",
                umber::InputObservationNamespace::Generated => "generated",
                umber::InputObservationNamespace::Distribution => "distribution",
            }),
        )?;
        let outcome = Object::new();
        match observation.outcome() {
            umber::InputObservationOutcome::Present(hash) => {
                set(&outcome, "kind", &JsValue::from_str("present"))?;
                set(&outcome, "contentHash", &JsValue::from_str(&hash.hex()))?;
            }
            umber::InputObservationOutcome::Missing => {
                set(&outcome, "kind", &JsValue::from_str("missing"))?;
            }
        }
        set(&value, "outcome", &outcome)?;
        set(
            &value,
            "access",
            &JsValue::from_str(match observation.access() {
                umber::InputDependencyAccess::RequiredRead => "required-read",
                umber::InputDependencyAccess::AuthoritativeProbe => "authoritative-probe",
            }),
        )?;
        set(
            &value,
            "resourceKind",
            &JsValue::from_str(observation.resource_kind().wire_name()),
        )?;
        set(
            &value,
            "phase",
            &JsValue::from_str(match observation.phase() {
                umber::InputObservationPhase::Tex => "tex",
                umber::InputObservationPhase::BibliographyDetection => "bibliography-detection",
                umber::InputObservationPhase::Bibliography => "bibliography",
            }),
        )?;
        set(
            &value,
            "revision",
            &JsValue::from_f64(observation.revision().raw() as f64),
        )?;
        if let Some(pass) = observation.project_pass() {
            set(&value, "projectPass", &JsValue::from_f64(f64::from(pass)))?;
        }
        if let Some(source) = observation.requesting_source() {
            set(
                &value,
                "requestingSource",
                &JsValue::from_str(source.as_str()),
            )?;
        }
        set(
            &value,
            "owner",
            &JsValue::from_str(match observation.owner() {
                umber::InputObservationOwner::TexEngine => "tex-engine",
                umber::InputObservationOwner::BibliographyDetector => "bibliography-detector",
                umber::InputObservationOwner::Biblatex => "biblatex",
                umber::InputObservationOwner::ClassicBibtex => "classic-bibtex",
            }),
        )?;
        observations.push(&value);
    }
    set(&object, "observations", &observations)?;
    Ok(Some(object.unchecked_into()))
}
