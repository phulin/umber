use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use parity_harness::run_named_fixture_document;
use parity_harness::{
    ManifestBoundSource, TripTriageChannels, TripTriageInput, TripTriageSource, compare_dvi_files,
    write_trip_triage_artifact,
};
use sha2::{Digest, Sha256};
use test_support::dvi::normalized_dvi_for_comparison;
use tex_command::RegisteredSourceKind;
use tex_observe::{
    CapturedObservations, GeometryEvidenceProfile, LiveSessionOutcome, LiveSessionStreams,
    LiveSessionTranslator, LiveSource, SemanticEvidenceProfile,
};
use tex_oracle::{ObservationStream, SchemaVersion};
use tex_state::{EffectRecord, PrintSink, ProvenanceDemand};
use tex_state::{JobClock, World};

use umber::FormatCacheStore;
use umber::{
    EngineMode, FormatGenerationGuards, FormatRecipe, FormatResource, LoadedFormatProjectionDemand,
    LoadedFormatResource, OutputCapability, PreparedFormatJob, PreparedFormatProvider,
    dvi_from_page_plans,
};

#[path = "e2e_conformance/assets.rs"]
mod assets;
#[path = "e2e_conformance/etrip_official.rs"]
mod etrip_official;

use assets::GateAssets;

use super::umber_format_worker_launcher;
#[path = "e2e_conformance/canonical_dvi.rs"]
mod canonical_dvi;
#[path = "e2e_conformance/loaded_diagnostics.rs"]
mod loaded_diagnostics;
#[path = "e2e_conformance/loaded_math.rs"]
mod loaded_math;
#[path = "e2e_conformance/phase_channels.rs"]
mod phase_channels;
#[path = "e2e_conformance/plain_format.rs"]
mod plain_format;
#[path = "e2e_conformance/trip_recipe.rs"]
mod trip_recipe;

#[path = "e2e_conformance/trip_support.rs"]
mod trip_support;
use trip_support::*;
#[path = "e2e_conformance/plain_support.rs"]
mod plain_support;
use plain_support::*;
#[path = "e2e_conformance/phase_support.rs"]
mod phase_support;
use phase_support::*;
#[path = "e2e_conformance/loaded_support.rs"]
mod loaded_support;
use loaded_support::*;

#[test]
#[allow(clippy::disallowed_methods)] // Verifies repository-pinned fixture bytes.
fn plain_recipe_has_exact_pinned_ordered_closure_and_stable_identity() {
    let repo_root = test_support::repository_root();
    let first = plain_format_recipe(&repo_root).expect("complete Plain recipe");
    let second = plain_format_recipe(&repo_root).expect("repeat complete Plain recipe");
    assert_eq!(first.engine, EngineMode::Tex82);
    assert_eq!(first.format_name, "repository-plain-tex82");
    assert_eq!(first.construction_source_name, "repository-plain-tex82.ini");
    assert_eq!(
        first.construction_source.as_slice(),
        b"\\input plain.tex\n\\dump\n"
    );
    assert!(
        !fs::read(repo_root.join("third_party/corpus/plain.tex"))
            .expect("pinned plain.tex")
            .windows(b"\\dump".len())
            .any(|window| window == b"\\dump"),
        "the recipe-owned dump must be the only Plain construction dump"
    );
    assert_eq!(first.clock, PLAIN_CLOCK);
    assert_eq!(
        first.construction_interaction,
        tex_state::InteractionMode::Nonstop
    );
    assert_eq!(first.construction_error_context_widths.error_line(), 64);
    assert_eq!(
        first.construction_error_context_widths.half_error_line(),
        32
    );
    assert_eq!(first.guards, plain_guards());
    assert_eq!(
        first.resources.len(),
        2 + parity_harness::PLAIN_PRELOAD_FONTS.len()
    );
    assert!(matches!(
        &first.resources[0],
        FormatResource::Input { logical_name, bytes, .. }
            if logical_name == "plain.tex"
                && bytes.as_ref() == fs::read(repo_root.join("third_party/corpus/plain.tex"))
                    .expect("pinned plain.tex")
    ));
    assert!(matches!(
        &first.resources[1],
        FormatResource::Input { logical_name, bytes, .. }
            if logical_name == "hyphen.tex"
                && bytes.as_ref() == fs::read(repo_root.join("third_party/hyphen/hyphen.tex"))
                    .expect("pinned hyphen.tex")
    ));
    for (resource, name) in first.resources[2..]
        .iter()
        .zip(parity_harness::PLAIN_PRELOAD_FONTS)
    {
        assert!(matches!(
            resource,
            FormatResource::Tfm { logical_name, bytes }
                if logical_name == &format!("{name}.tfm")
                    && bytes.as_ref() == fs::read(repo_root.join(format!("third_party/fonts/{name}.tfm")))
                        .expect("pinned Plain preload TFM")
        ));
    }
    assert_eq!(
        first.identity().expect("Plain identity").key(),
        second.identity().expect("stable Plain identity").key()
    );
}

fn run_plain_fixture_case(document: &str, gate: &GateAssets) {
    run_named_fixture_document(&gate.repo_root, document, &gate.oracle, |path| {
        let run = run_file_with_plain_format(path)?;
        run.dvi
            .ok_or_else(|| "Umber did not produce DVI".to_owned())
    })
    .unwrap_or_else(|error| panic!("{error:#}"));
}

#[test]
fn e2e_conformance_story() {
    assets::with_gate("story", |gate| run_plain_fixture_case("story.tex", gate));
}

#[test]
#[ignore = "manual full-document Gentle parity and provenance tier"]
fn e2e_conformance_gentle() {
    assets::with_gate("gentle", |gate| run_plain_fixture_case("gentle.tex", gate));
}

/// Runs one self-contained staged fixture as a fresh job loaded from the
/// shared persistent raw-TeX82 format and returns its assembled DVI bytes.
fn run_file_in_process_canonical(path: &Path) -> Result<Vec<u8>, String> {
    run_file_with_raw_tex82_format(path)?
        .dvi
        .ok_or_else(|| "canonical Umber run did not produce DVI".to_owned())
}

fn run_file_in_process_plain_canonical(path: &Path) -> Result<Vec<u8>, String> {
    run_file_with_plain_format(path)?
        .dvi
        .ok_or_else(|| "canonical Umber run did not produce DVI".to_owned())
}

fn run_plain_fixture_case_canonical(document: &str, gate: &GateAssets) {
    run_named_fixture_document(
        &gate.repo_root,
        document,
        &gate.oracle,
        run_file_in_process_plain_canonical,
    )
    .unwrap_or_else(|error| panic!("{error:#}"));
}

/// Compares Story DVI with real pdfTeX after preamble-comment normalization.
/// Both Story tests use the same loaded-format engine; the other test also
/// checks the macro-invocation provenance budget.
#[test]
fn e2e_conformance_story_canonical() {
    assets::with_gate("story", |gate| {
        run_plain_fixture_case_canonical("story.tex", gate);
    });
}

/// Pins the canonical engine's Gentle DVI to the real-pdfTeX oracle. The
/// shared conformance comparator permits only the variable preamble comment;
/// every remaining byte, including list-setting geometry, must match.
#[test]
#[ignore = "manual full-document Gentle DVI parity tier"]
fn e2e_conformance_gentle_canonical() {
    assets::with_gate("gentle", |gate| {
        run_plain_fixture_case_canonical("gentle.tex", gate);
    });
}

#[test]
#[ignore = "manual full-document TRIP parity tier"]
fn e2e_conformance_trip_canonical() {
    assets::with_gate("trip", |gate| {
        run_two_phase_fixture(TripEngineProfile::Tex82, "trip.tex", "trip.tex", gate);
    });
}

#[test]
#[ignore = "manual full-document e-TRIP parity tier"]
fn e2e_conformance_etrip() {
    assets::with_gate("etrip", |gate| {
        run_two_phase_fixture(
            TripEngineProfile::ETex,
            "etrip.tex",
            "etrip-local.tex",
            gate,
        );
    });
}
