//! Gate for the declarative command-semantic minifixture corpus.
//!
//! The corpus contract itself lives in `tex_command_stream::semantic`; this
//! test only asserts it.

#![allow(
    clippy::disallowed_methods,
    reason = "this host-only fixture test reads its committed corpus"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::process::Command;

use sha2::{Digest, Sha256};
use tex_command::FatalError;
use umber::FormatCacheStore;
use umber::{FormatWorkerLauncher, PreparedFormatProvider};

use tex_command_stream::semantic::channels::compare;
use tex_command_stream::semantic::*;

struct HermeticFormats {
    _root: tempfile::TempDir,
    provider: PreparedFormatProvider,
}

impl HermeticFormats {
    fn new() -> Self {
        let root = tempfile::TempDir::new().expect("hermetic format cache");
        let provider = PreparedFormatProvider::with_store(
            FormatCacheStore::new(root.path()),
            FormatWorkerLauncher::registered_libtest("umber_format_worker_bootstrap"),
        );
        Self {
            _root: root,
            provider,
        }
    }

    fn execute(&self, source: &[u8], case: &Case) -> Result<SemanticRun, String> {
        execute_with_provider(source, case, &self.provider)
    }
}

fn geometry_signatures(run: &SemanticRun) -> Vec<(&'static str, i64, i64, i64, u32)> {
    run.observations
        .iter()
        .filter_map(|observation| match observation {
            tex_command::CommandObservation::Geometry(tex_command::GeometryRecord::Hpack {
                width_sp,
                height_sp,
                depth_sp,
                line,
                ..
            }) => Some(("hpack", *width_sp, *height_sp, *depth_sp, *line)),
            tex_command::CommandObservation::Geometry(tex_command::GeometryRecord::Vpack {
                width_sp,
                height_sp,
                depth_sp,
                line,
                ..
            }) => Some(("vpack", *width_sp, *height_sp, *depth_sp, *line)),
            tex_command::CommandObservation::Geometry(tex_command::GeometryRecord::Shipout {
                page_width_sp,
                page_height_sp,
                line,
                ..
            }) => Some(("shipout", *page_width_sp, *page_height_sp, 0, *line)),
            _ => None,
        })
        .collect()
}

fn rust_function_body<'a>(source: &'a str, name: &str) -> &'a str {
    let signature = format!("fn {name}(");
    let start = source
        .find(&signature)
        .unwrap_or_else(|| panic!("function definition exists: {name}"));
    let open = source[start..]
        .find('{')
        .map(|offset| start + offset)
        .unwrap_or_else(|| panic!("function body opens: {name}"));
    let mut depth = 0usize;
    for (offset, byte) in source.as_bytes()[open..].iter().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return &source[open + 1..open + offset];
                }
            }
            _ => {}
        }
    }
    panic!("function body closes: {name}")
}

#[test]
fn every_loaded_command_route_has_only_the_generic_provider_owner() {
    let source = include_str!("../../src/semantic.rs");
    let dispatch = rust_function_body(source, "execute_with_provider_completion");
    for owner in [
        "execute_raw_tex82_loaded",
        "execute_raw_etex26_loaded",
        "execute_production_pdftex14029_loaded",
    ] {
        assert_eq!(dispatch.matches(owner).count(), 1, "dispatch owns {owner}");
        let body = rust_function_body(source, owner);
        assert_eq!(
            body.matches("execute_loaded_format").count(),
            1,
            "{owner} delegates exactly once"
        );
    }

    let generic = rust_function_body(source, "execute_loaded_format");
    for required in [
        "provider",
        ".prepare(&recipe)",
        "PreparedFormatJob {",
        ".run(",
        ".run_fragment(",
    ] {
        assert!(
            generic.contains(required),
            "generic owner requires {required}"
        );
    }
    for forbidden in [
        concat!("Once", "Lock"),
        concat!("Temp", "Dir"),
        concat!("ensure_", "format("),
        concat!("Universe::from_", "format"),
        concat!("dump_", "format"),
        concat!("run_format_", "worker"),
    ] {
        assert!(
            !generic.contains(forbidden),
            "generic loaded owner forbids {forbidden}"
        );
    }
}

#[ignore = "manual exact-parity tier: tracked by umber2-alfh.11"]
#[test]
fn declared_command_semantic_cases_match() {
    let root = tempfile::TempDir::new().expect("hermetic persistent format cache");
    let launcher = FormatWorkerLauncher::registered_libtest("umber_format_worker_bootstrap");
    let cases =
        load_suite().unwrap_or_else(|error| panic!("invalid command-semantic corpus: {error}"));
    let mut failures = Vec::new();
    for declared in &cases {
        let label = format!("{}/{}", declared.domain, declared.case.id);
        let run = fs::read(declared.fixture_dir.join(&declared.case.source))
            .map_err(|error| format!("source read: {error}"))
            .and_then(|source| {
                // Independent provider instances exercise persistent reuse;
                // the shared authority is the store and complete identity.
                let provider = PreparedFormatProvider::with_store(
                    FormatCacheStore::new(root.path()),
                    launcher.clone(),
                );
                execute_with_provider(&source, &declared.case, &provider)
            });
        let actual = run
            .as_ref()
            .map(|run| project(run, &declared.case.projection))
            .map_err(Clone::clone);
        if let Err(error) =
            evaluate_expectation(&declared.case.expected, &actual, &declared.case.expectation)
        {
            failures.push(format!("{label}: {error:?}"));
        }
        // The projection is a focused property claim about one observable.
        // The channel contract is the completeness claim about the rest of
        // the same run, and both have to hold.
        if let Ok(run) = &run {
            for failure in compare_declared_channels(declared, run) {
                failures.push(format!("{label}: {failure:?}"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} declared cases failed:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
    let published_entries = fs::read_dir(root.path().join("blobs-v1"))
        .expect("persistent format namespace")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("sha256-"))
        .count();
    assert_eq!(published_entries, 3);
    let routes = [
        ExecutionRoute::RawTex82Loaded,
        ExecutionRoute::RawEtex26Loaded,
        ExecutionRoute::ProductionPdftex14029Loaded,
    ];
    let identities: Vec<_> = routes
        .into_iter()
        .map(|route| {
            let recipe = loaded_format_recipe(route).expect("loaded recipe");
            assert_eq!(recipe, loaded_format_recipe(route).expect("stable recipe"));
            let first = PreparedFormatProvider::with_store(
                FormatCacheStore::new(root.path()),
                launcher.clone(),
            )
            .prepare(&recipe)
            .expect("first independent provider reuses entry");
            let second = PreparedFormatProvider::with_store(
                FormatCacheStore::new(root.path()),
                launcher.clone(),
            )
            .prepare(&recipe)
            .expect("second independent provider reuses entry");
            assert_eq!(first.image(), second.image());
            assert_eq!(
                first.construction_evidence(),
                second.construction_evidence()
            );
            recipe.identity().expect("bounded complete identity")
        })
        .collect();
    assert!(identities.iter().enumerate().all(|(index, identity)| {
        identities
            .iter()
            .enumerate()
            .all(|(other_index, other)| index == other_index || identity != other)
    }));
}

#[test]
fn count_write_fixture_keeps_direct_the_internal_to_scan_toks() {
    let formats = HermeticFormats::new();
    let cases = load_suite().expect("valid command-semantic corpus");
    let declared = cases
        .iter()
        .find(|declared| declared.case.id == "count-write-and-text")
        .expect("count-write-and-text fixture");
    let source = fs::read(declared.fixture_dir.join(&declared.case.source))
        .expect("count-write-and-text source");
    let run = formats
        .execute(&source, &declared.case)
        .expect("count-write-and-text executes");

    assert_eq!(run.observations.len(), 260);
    assert_eq!(
        geometry_signatures(&run),
        [
            ("hpack", 327_681, 422_343, 0, 6),
            ("shipout", 327_681, 422_343, 0, 8),
        ],
        "TeX82's explicit box pack precedes the matching shipout geometry"
    );
    assert!(matches!(
        run.observations.as_slice(),
        [.., tex_command::CommandObservation::Effect(effect), tex_command::CommandObservation::DiagnosticLifecycle(tex_command::DiagnosticLifecycleRecord::Outcome { .. })]
            if effect.kind == tex_command::ObservationEffectKind::Terminate
                && effect.channel == "engine"
    ));
    assert_eq!(
        run.observations
            .iter()
            .filter(|observation| matches!(
                observation,
                tex_command::CommandObservation::Command(command)
                    if command.command == "the"
                        && command.boundary
                            == tex_command::CommandDeliveryBoundary::Raw
                        && !command.provenance.has_origin
            ))
            .count(),
        1,
        "TeX82 §478 delivers the replayed direct `the` raw exactly once"
    );
    assert!(!run.observations.iter().any(|observation| matches!(
        observation,
        tex_command::CommandObservation::Command(command)
            if command.command == "the"
                && command.boundary == tex_command::CommandDeliveryBoundary::Expanded
    )));
    assert_eq!(
        run.observations
            .iter()
            .filter(|observation| matches!(
                observation,
                tex_command::CommandObservation::TokenList(record)
                    if record.transition == "splice" && record.purpose == "the_toks"
            ))
            .count(),
        1,
        "TeX82 §478 publishes the direct `the_toks` splice exactly once"
    );
}

#[test]
fn immediate_open_close_materializes_empty_artifacts_without_shipout() {
    let formats = HermeticFormats::new();
    let cases = load_suite().expect("valid command-semantic corpus");
    let declared = cases
        .iter()
        .find(|declared| declared.case.id == "closeout-stream-selectors")
        .expect("closeout-stream-selectors fixture");
    let source = fs::read(declared.fixture_dir.join(&declared.case.source))
        .expect("closeout-stream-selectors source");
    let run = formats
        .execute(&source, &declared.case)
        .expect("closeout-stream-selectors executes");

    assert!(
        run.artifacts.is_empty(),
        "the zero-page job shipped no pages"
    );
    assert!(
        compare_declared_channels(declared, &run).is_empty(),
        "TeX82 §§1373--1375 immediate effects and empty artifacts match the oracle"
    );
}

#[test]
fn paragraph_line_shape_matches_canonical_projection_and_channels() {
    let formats = HermeticFormats::new();
    let cases = load_suite().expect("valid command-semantic corpus");
    let declared = cases
        .iter()
        .find(|declared| declared.case.id == "paragraph-line-shape")
        .expect("paragraph-line-shape fixture");
    assert_eq!(declared.domain, "line-breaking");
    assert_eq!(
        declared.case.property_id,
        "tex82.linebreak.post-line-materialization"
    );
    let source = fs::read(declared.fixture_dir.join(&declared.case.source))
        .expect("paragraph-line-shape source");
    let run = formats
        .execute(&source, &declared.case)
        .expect("bounded paragraph-line-shape fixture executes");

    assert_eq!(
        geometry_signatures(&run),
        [
            ("hpack", 1_310_720, 282_168, 0, 15),
            ("hpack", 1_638_400, 282_168, 0, 15),
            ("hpack", 1_179_648, 282_168, 0, 15),
            ("hpack", 1_179_648, 282_168, 0, 15),
            ("hpack", 1_179_648, 282_168, 0, 15),
            ("vpack", 1_638_400, 1_410_840, 0, 15),
            ("hpack", 2_293_760, 282_168, 0, 16),
            ("hpack", 1_638_400, 282_168, 0, 16),
            ("hpack", 1_638_400, 282_168, 0, 16),
            ("hpack", 1_638_400, 282_168, 0, 16),
            ("vpack", 2_293_760, 1_128_672, 0, 16),
        ],
        "line materialization commits each hpack before its enclosing vpack"
    );

    assert_eq!(
        project(&run, &declared.case.projection),
        declared.case.expected
    );
    assert_eq!(
        compare_declared_channels(declared, &run),
        [],
        "TeX82 §§847–849 and §§877–890 channel bytes match strictly"
    );
}

#[test]
fn raw_tex82_loaded_uses_pdftex_invalid_unit_help() {
    let formats = HermeticFormats::new();
    let cases = load_suite().expect("valid command-semantic corpus");
    let declared = cases
        .iter()
        .find(|declared| declared.case.id == "vacuous-dimension-units")
        .expect("vacuous-dimension-units fixture");
    assert_eq!(declared.case.profile, SessionProfile::RawTex82Loaded);
    let source = fs::read(declared.fixture_dir.join(&declared.case.source))
        .expect("vacuous-dimension-units source");
    let run = formats
        .execute(&source, &declared.case)
        .expect("raw TeX82 loaded by pdfTeX executes");

    assert_eq!(
        geometry_signatures(&run),
        [
            ("hpack", 0, 0, 0, 4),
            ("hpack", 0, 0, 0, 4),
            ("vpack", 0, 0, 0, 4),
            ("shipout", 0, 0, 0, 4),
        ],
        "the empty output page commits its packs before shipout"
    );

    assert!(matches!(
        run.observations.as_slice(),
        [.., tex_command::CommandObservation::Effect(effect), tex_command::CommandObservation::DiagnosticLifecycle(tex_command::DiagnosticLifecycleRecord::Outcome { .. })]
            if effect.kind == tex_command::ObservationEffectKind::Terminate
                && effect.channel == "engine"
    ));
    assert_eq!(
        compare_declared_channels(declared, &run),
        [],
        "pdfTeX 1.40.29 §459 channel bytes match strictly"
    );
}

#[ignore = "manual exact-parity tier: tracked by umber2-alfh.11"]
#[test]
fn loaded_projection_distinguishes_explicit_end_from_nested_source_exhaustion() {
    let formats = HermeticFormats::new();
    let cases = load_suite().expect("valid command-semantic corpus");
    let run = |name: &str| {
        let declared = cases
            .iter()
            .find(|declared| declared.case.id == name)
            .unwrap_or_else(|| panic!("missing focused case {name}"));
        let source = fs::read(declared.fixture_dir.join(&declared.case.source))
            .unwrap_or_else(|error| panic!("{name} source: {error}"));
        formats
            .execute(&source, &declared.case)
            .unwrap_or_else(|error| panic!("{name}: {error}"))
    };
    let explicit_end = run("etex-loaded-ifcsname");
    assert_eq!(
        explicit_end.mode_transitions.last(),
        Some(&tex_exec::Mode::Vertical)
    );
    assert!(explicit_end.observations.iter().any(|observation| {
        matches!(
            observation,
            tex_command::CommandObservation::Effect(effect)
                if effect.kind == tex_command::ObservationEffectKind::Terminate
                    && effect.channel == "engine"
        )
    }));

    let exhausted = run("etex-loaded-glue-component-enquiries");
    assert_ne!(
        exhausted.mode_transitions.last(),
        Some(&tex_exec::Mode::Vertical)
    );
    assert!(matches!(
        exhausted.observations.as_slice(),
        [.., tex_command::CommandObservation::Input(input), tex_command::CommandObservation::Effect(effect)]
            if input.transition == tex_command::InputTransition::Stop
                && input.reason == tex_command::InputReason::Source
                && effect.kind == tex_command::ObservationEffectKind::Terminate
                && effect.channel == "engine"
    ));
}

#[test]
fn v2_identity_capture_policy_and_resolved_channels_match_the_migrated_corpus() {
    let cases = load_suite().expect("valid command-semantic corpus");
    assert_eq!(cases.len(), 210);
    assert_eq!(
        cases
            .iter()
            .map(|declared| declared.case.expected.len())
            .sum::<usize>(),
        1_323
    );

    let selected_raw: Vec<_> = cases
        .iter()
        .filter(|declared| {
            declared.case.profile == SessionProfile::RawTex82Loaded
                && declared.case.capture.selected()
        })
        .collect();
    assert_eq!(selected_raw.len(), 176);
    let mut selected_by_domain = BTreeMap::new();
    for declared in &selected_raw {
        *selected_by_domain
            .entry(declared.domain.as_str())
            .or_insert(0) += 1;
    }
    assert_eq!(
        selected_by_domain,
        BTreeMap::from([
            ("alignments", 18),
            ("conditionals", 9),
            ("input-expansion", 13),
            ("line-breaking", 1),
            ("main-control", 55),
            ("math", 34),
            ("page-output", 33),
            ("scanners-internal-quantities", 13),
        ])
    );
    let page_output_dvi_dispositions = selected_raw
        .iter()
        .filter(|declared| declared.domain == "page-output")
        .fold([0usize; 5], |mut counts, declared| {
            let index = match &declared
                .case
                .channels
                .as_ref()
                .expect("resolved channels")
                .dvi
            {
                StreamDisposition::Empty => 0,
                StreamDisposition::File => 1,
                StreamDisposition::Unsupported { .. } => 2,
                StreamDisposition::Xfail { .. } => 3,
                StreamDisposition::XfailDiagnostics { .. } => 4,
            };
            counts[index] += 1;
            counts
        });
    assert_eq!(
        page_output_dvi_dispositions,
        [4, 29, 0, 0, 0],
        "page-output DVI dispositions: empty, file, unsupported, xfail, xfail-diagnostics"
    );
    assert_eq!(
        cases
            .iter()
            .filter(|declared| !declared.case.terminal_lines.is_empty())
            .count(),
        11
    );
    assert_eq!(
        cases
            .iter()
            .filter(|declared| {
                fs::read(declared.fixture_dir.join(&declared.case.source))
                    .expect("fixture source")
                    .windows(b"\\openout".len())
                    .any(|window| window == b"\\openout")
            })
            .count(),
        5
    );
    let excluded: Vec<_> = cases
        .iter()
        .filter(|declared| !declared.case.capture.selected())
        .map(|declared| format!("{}/{}", declared.domain, declared.case.id))
        .collect();
    assert_eq!(excluded, ["main-control/hyphenation-data"]);

    // One compact identity replaces the former 467-line census while pinning
    // every resolved field, including routes, projections, xfails, channels,
    // statuses, host inputs, and interaction policy for all 210 cases.
    let mut digest = Sha256::new();
    for declared in &cases {
        assert_eq!(
            declared
                .fixture_dir
                .file_name()
                .and_then(|name| name.to_str()),
            Some(declared.case.id.as_str())
        );
        assert_eq!(declared.case.source, format!("{}.tex", declared.case.id));
        assert!(!matches!(
            declared
                .case
                .channels
                .as_ref()
                .expect("resolved channels")
                .effects,
            StreamDisposition::Unsupported { .. }
        ));
        digest.update(format!("{}:{:?}\n", declared.domain, declared.case).as_bytes());
    }
    assert_eq!(
        format!("{:x}", digest.finalize()),
        "32927da0621bb6206593179b81c75f1567dc3de19801e129739e6a321c77732b"
    );
}

#[test]
fn raw_tex82_loaded_supplies_the_oracle_default_terminal_line() {
    let formats = HermeticFormats::new();
    let case: Case = serde_json::from_value(serde_json::json!({
        "id": "raw-loaded-empty-terminal-read",
        "property_id": "tex82.assignment.read-to-definition",
        "profile": "raw-tex82-loaded",
        "source": "raw-loaded-empty-terminal-read.tex",
        "provenance": {
            "authority": "tex.web",
            "manifest": "tests/tex82-oracle-manifest.txt",
            "sections": [360, 484, 1225]
        },
        "projection": {
            "kind": "observations",
            "kinds": ["input", "recovery"]
        },
        "expected": [],
        "expectation": {"kind": "pass"}
    }))
    .expect("bounded regression case is valid");
    let run = formats
        .execute(br"\read-1 to\line\end", &case)
        .expect("the oracle's implicit empty terminal line satisfies the terminal read");
    let channels = CapturedChannels::capture(&run);
    assert_eq!(channels.events, 25);
    assert_eq!(channels.status, "clean");
    assert_eq!(
        channels.stream(StreamChannel::Terminal),
        concat!(
            "This is pdfTeX, Version 3.141592653-2.6-1.40.29 (TeX Live 2026) ",
            "(preloaded format=production)\n",
            "(./raw-loaded-empty-terminal-read.tex )\n",
            "No pages of output.\n",
            "Transcript written on raw-loaded-empty-terminal-read.log.\n"
        )
        .as_bytes()
    );
    assert_eq!(
        channels.stream(StreamChannel::Log),
        concat!(
            "This is pdfTeX, Version 3.141592653-2.6-1.40.29 (TeX Live 2026) ",
            "(preloaded format=production 2026.3.1)  1 JAN 1970 00:00\n",
            "**raw-loaded-empty-terminal-read.tex\n",
            "(./raw-loaded-empty-terminal-read.tex\n",
            " )\n",
            "No pages of output.\n"
        )
        .as_bytes()
    );
    assert_eq!(
        project(&run, &case.projection),
        [
            "input:push:terminal",
            "input:retire:terminal",
            "input:retire:file",
            "input:stop:terminal",
        ]
    );
}

#[test]
fn raw_tex82_loaded_reapplies_declared_job_input_with_resolved_name() {
    let formats = HermeticFormats::new();
    let case: Case = serde_json::from_value(serde_json::json!({
        "id": "raw-loaded-declared-input",
        "property_id": "tex82.input.loaded-job-resource",
        "profile": "raw-tex82-loaded",
        "source": "raw-loaded-declared-input.tex",
        "provenance": {
            "authority": "tex.web",
            "manifest": "tests/tex82-oracle-manifest.txt",
            "sections": [24, 534, 536, 537]
        },
        "projection": {
            "kind": "observations",
            "count_registers": [0],
            "kinds": ["input"]
        },
        "expected": [],
        "expectation": {"kind": "pass"},
        "inputs": {"child.tex": "\\count0=37 "}
    }))
    .expect("bounded loaded-input regression case is valid");
    let run = formats
        .execute(br"\input child\end", &case)
        .expect("declared loaded-job input is available");
    let channels = CapturedChannels::capture(&run);

    assert_eq!(run.counts[0], 37);
    assert_eq!(channels.events, 48);
    assert_eq!(channels.status, "clean");
    assert_eq!(
        channels.stream(StreamChannel::Terminal),
        concat!(
            "This is pdfTeX, Version 3.141592653-2.6-1.40.29 (TeX Live 2026) ",
            "(preloaded format=production)\n",
            "(./raw-loaded-declared-input.tex (./child.tex) )\n",
            "No pages of output.\n",
            "Transcript written on raw-loaded-declared-input.log.\n"
        )
        .as_bytes()
    );
    assert_eq!(
        channels.stream(StreamChannel::Log),
        concat!(
            "This is pdfTeX, Version 3.141592653-2.6-1.40.29 (TeX Live 2026) ",
            "(preloaded format=production 2026.3.1)  1 JAN 1970 00:00\n",
            "**raw-loaded-declared-input.tex\n",
            "(./raw-loaded-declared-input.tex (./child.tex) )\n",
            "No pages of output.\n"
        )
        .as_bytes()
    );
    assert!(channels.stream(StreamChannel::Dvi).is_empty());
    assert!(channels.stream(StreamChannel::Effects).is_empty());
}

#[test]
fn raw_tex82_loaded_reapplies_declared_job_tfm() {
    let formats = HermeticFormats::new();
    let case: Case = serde_json::from_value(serde_json::json!({
        "id": "raw-loaded-declared-tfm",
        "property_id": "tex82.font.loaded-job-resource",
        "profile": "raw-tex82-loaded",
        "source": "raw-loaded-declared-tfm.tex",
        "provenance": {
            "authority": "tex.web",
            "manifest": "tests/tex82-oracle-manifest.txt",
            "sections": [560, 561, 565, 618]
        },
        "projection": {
            "kind": "execution-boundaries",
            "command_names": ["leader_ship"],
            "include_artifact_hashes": true
        },
        "expected": [],
        "expectation": {"kind": "pass"},
        "font_inputs": {
            "cmr10.tfm": "crates/tex-fonts/tests/fixtures/cm/cmr10.tfm"
        }
    }))
    .expect("bounded loaded-font regression case is valid");
    let run = formats
        .execute(br"\font\ten=cmr10 \ten\shipout\hbox{A}\end", &case)
        .expect("declared loaded-job TFM is available");
    let channels = CapturedChannels::capture(&run);

    assert_eq!(channels.events, 77);
    assert_eq!(
        geometry_signatures(&run),
        [
            ("hpack", 1_146_883, 455_111, 0, 1),
            ("shipout", 1_146_883, 455_111, 0, 1),
        ],
        "the font-backed hbox commits before its matching shipout"
    );
    assert!(matches!(
        run.observations.as_slice(),
        [.., tex_command::CommandObservation::Effect(effect), tex_command::CommandObservation::DiagnosticLifecycle(tex_command::DiagnosticLifecycleRecord::Outcome { .. })]
            if effect.kind == tex_command::ObservationEffectKind::Terminate
                && effect.channel == "engine"
    ));
    assert_eq!(channels.status, "clean");
    assert_eq!(
        (
            format!(
                "{:x}",
                Sha256::digest(channels.stream(StreamChannel::Terminal))
            ),
            format!("{:x}", Sha256::digest(channels.stream(StreamChannel::Log))),
            format!("{:x}", Sha256::digest(channels.stream(StreamChannel::Dvi))),
        ),
        (
            "e8a5805201a08281aa19be9ef2c78066d76bb0f3dd06b7eaebbfe25991aa54a6".to_owned(),
            "4f7fbc043fb18c924056de974a555218f57692d9fe1e24d68a88efd7efd340a3".to_owned(),
            "07c3e696d0a55c9e9beec4c55efb22417ecffa8d3381d696608d87f41b3cf7bc".to_owned(),
        )
    );
    assert!(channels.stream(StreamChannel::Effects).is_empty());
}

#[test]
fn raw_tex82_loaded_preserves_nontrivial_mode_transitions() {
    let formats = HermeticFormats::new();
    let case: Case = serde_json::from_value(serde_json::json!({
        "id": "raw-loaded-mode-transitions",
        "property_id": "tex82.main-control.loaded-job-outcomes",
        "profile": "raw-tex82-loaded",
        "source": "raw-loaded-mode-transitions.tex",
        "provenance": {
            "authority": "tex.web",
            "manifest": "tests/tex82-oracle-manifest.txt",
            "sections": [1027, 1090, 1138]
        },
        "projection": {
            "kind": "execution-boundaries",
            "count_registers": [0],
            "include_mode_transitions": true
        },
        "expected": [],
        "expectation": {"kind": "pass"}
    }))
    .expect("bounded loaded-mode regression case is valid");

    let mutated = formats
        .execute(br"\count0=123\end", &case)
        .expect("first loaded job mutates its world");
    let isolated = formats
        .execute(br"\end", &case)
        .expect("second loaded job starts fresh");
    assert_eq!(mutated.counts[0], 123);
    assert_eq!(isolated.counts[0], 0);

    let run = formats
        .execute(br"a\par b\par\end", &case)
        .expect("loaded mode sequence completes");

    assert_eq!(
        run.mode_transitions,
        [
            tex_exec::Mode::Vertical,
            tex_exec::Mode::Horizontal,
            tex_exec::Mode::Vertical,
            tex_exec::Mode::Horizontal,
            tex_exec::Mode::Vertical,
        ]
    );
}

#[test]
fn raw_tex82_loaded_preserves_fatal_completion_and_channel_status() {
    let formats = HermeticFormats::new();
    let case: Case = serde_json::from_value(serde_json::json!({
        "id": "raw-loaded-fatal",
        "property_id": "tex82.main-control.loaded-job-outcomes",
        "profile": "raw-tex82-loaded",
        "source": "raw-loaded-fatal.tex",
        "interaction_mode": "nonstopmode",
        "provenance": {
            "authority": "tex.web",
            "manifest": "tests/tex82-oracle-manifest.txt",
            "sections": [81, 93, 360]
        },
        "projection": {"kind": "state", "count_registers": [0]},
        "expected": [],
        "expectation": {"kind": "pass"}
    }))
    .expect("bounded loaded-fatal regression case is valid");

    let run = formats
        .execute(br"\input unavailable", &case)
        .expect("TeX fatal completion remains a completed loaded run");

    assert!(run.fatal.is_some());
    assert_eq!(
        CapturedChannels::capture(&run).status,
        format!("fatal:{}", run.fatal.expect("fatal state").label())
    );
}

fn compare_declared_channels(declared: &DeclaredCase, run: &SemanticRun) -> Vec<ChannelFailure> {
    let contract = declared
        .case
        .channels
        .as_ref()
        .expect("load_suite requires every case to declare a channel contract");
    let committed =
        |channel: StreamChannel| fs::read(channel_file(&declared.fixture_dir, channel)).ok();
    compare(&CapturedChannels::capture(run), contract, &committed)
}

/// The set of cases exempt from the channel contract is exactly the set whose
/// engine run does not complete -- and it is empty.
///
/// This used to also pin the corpus size, from when the exempt set was
/// non-empty and a count was the only thing stopping someone from growing the
/// exemptions instead of fixing a case. With the set asserted empty the count
/// guarded nothing: a new case either declares a channel contract or lands in
/// `exempt` and fails here. It only ever obstructed legitimate additions, so
/// it is gone.
#[test]
fn only_unrunnable_xfail_cases_are_exempt_from_the_channel_contract() {
    let formats = HermeticFormats::new();
    let cases =
        load_suite().unwrap_or_else(|error| panic!("invalid command-semantic corpus: {error}"));
    let mut exempt = Vec::new();
    for declared in &cases {
        if declared.case.channels.is_some() {
            continue;
        }
        let source = fs::read(declared.fixture_dir.join(&declared.case.source))
            .expect("an exempt case still has a readable source");
        assert!(
            formats.execute(&source, &declared.case).is_err(),
            "{}/{} runs and must therefore declare a channel contract",
            declared.domain,
            declared.case.id
        );
        exempt.push(format!("{}/{}", declared.domain, declared.case.id));
    }
    exempt.sort();
    // Empty, and that is the point of the ledger: the three cases that used to
    // sit here -- `input-expansion/expansion-conversions`,
    // `input-expansion/input-start-file`, and `main-control/read-to-definition`
    // -- all reach the end of their run now, so every case in the corpus
    // declares a channel contract. Growing this list again is a regression to
    // argue for, not a convenience.
    assert_eq!(exempt, [] as [String; 0], "the exempt set moved");
}

#[test]
fn every_minifixture_file_is_local_and_tracked() {
    let root = repository_root();
    let cases =
        load_suite().unwrap_or_else(|error| panic!("invalid command-semantic corpus: {error}"));
    let mut fixture_dirs = BTreeSet::new();
    for declared in &cases {
        assert!(
            fixture_dirs.insert(declared.fixture_dir.clone()),
            "duplicate fixture directory {}",
            declared.fixture_dir.display()
        );
        let relative = declared
            .fixture_dir
            .strip_prefix(&root)
            .expect("fixture is beneath the repository");
        let output = Command::new("git")
            .args(["ls-files", "--error-unmatch", "--"])
            .arg(relative)
            .current_dir(&root)
            .output()
            .expect("git is available for the repository fixture gate");
        assert!(
            output.status.success(),
            "{} contains an untracked fixture file:\n{}",
            relative.display(),
            String::from_utf8_lossy(&output.stderr)
        );
        let tracked = String::from_utf8(output.stdout).expect("Git paths are UTF-8");
        assert_eq!(
            tracked.lines().count(),
            fs::read_dir(&declared.fixture_dir)
                .expect("fixture directory is readable")
                .count(),
            "{} has a file not represented in Git",
            relative.display()
        );
        for channel in STREAM_CHANNELS {
            assert_eq!(
                channel_file(&declared.fixture_dir, channel),
                declared
                    .fixture_dir
                    .join(format!("expected.{}", channel.name())),
                "the generator must emit channels inside their fixture directory"
            );
        }
    }
}

#[test]
fn validator_rejects_duplicate_and_unowned_cases() {
    let mut case_ids = BTreeSet::new();
    let mut sources = BTreeSet::new();
    let mut declared_sources = BTreeSet::new();
    assert!(
        claim_case_identity(
            &mut case_ids,
            &mut sources,
            &mut declared_sources,
            "conditionals",
            "case-a",
            "case-a.tex",
        )
        .is_ok()
    );
    assert!(
        claim_case_identity(
            &mut case_ids,
            &mut sources,
            &mut declared_sources,
            "conditionals",
            "case-a",
            "case-b.tex",
        )
        .expect_err("duplicate case identity must be rejected")
        .contains("duplicate case")
    );

    let unowned: Case = serde_json::from_slice(
        br#"{
            "id":"case-b",
            "property_id":"tex82.conditionals.not-owned",
            "source":"case-b.tex",
            "provenance":{"authority":"tex.web","manifest":"tests/tex82-oracle-manifest.txt","sections":[505]},
            "projection":{"kind":"predicate-outcomes"},
            "expected":["predicate:iftrue:-:true"],
            "expectation":{"kind":"pass"}
        }"#,
    )
    .expect("negative case has a valid manifest shape");
    assert!(
        validate_case(
            &unowned,
            "conditionals",
            Path::new("."),
            Path::new("."),
            &BTreeMap::new(),
            ChannelPolicy::Deriving,
        )
        .expect_err("unowned property must be rejected")
        .contains("unowned property")
    );
}
#[test]
fn xfail_manifest_validation_rejects_malformed_and_missing_bug_links() {
    let missing_bug = br#"{
        "kind":"xfail",
        "mismatch":{"index":0,"kind":"observation","expected":"a","actual":"b"}
    }"#;
    assert!(serde_json::from_slice::<Expectation>(missing_bug).is_err());

    let malformed: Expectation = serde_json::from_slice(
        br#"{
        "kind":"xfail",
        "bug":"not-a-bead",
        "mismatch":{"index":0,"kind":"observation","expected":"a","actual":"b"}
    }"#,
    )
    .expect("shape is parseable before semantic validation");
    assert!(validate_expectation(&malformed).is_err());

    let opaque: Expectation = serde_json::from_slice(
        br#"{
        "kind":"xfail",
        "bug":"umber2-o96f",
        "mismatch":{"index":0,"kind":"observation","expected":"a","actual":"b"}
    }"#,
    )
    .expect("opaque Beads id has the manifest shape");
    assert!(validate_expectation(&opaque).is_ok());
}

#[test]
fn state_projection_emits_only_requested_final_counts() {
    let mut counts = [0; COUNT_SLOTS];
    counts[2] = 7;
    let run = SemanticRun {
        observations: Vec::new(),
        diagnostic_root_name: "./test.tex".into(),
        diagnostic_root_bytes: std::sync::Arc::from(&b""[..]),
        counts,
        box_outlines: BTreeMap::new(),
        mode_transitions: Vec::new(),
        artifacts: Vec::new(),
        dvi: Vec::new(),
        fatal: None,
        terminal: Vec::new(),
        log: Vec::new(),
        pending_effects: Vec::new(),
        effect_artifacts: Vec::new(),
        complete_job_channel_streams: None,
    };
    let projection = Projection {
        kind: ProjectionKind::State,
        count_registers: vec![2],
        include_count_mutations: false,
        kinds: Vec::new(),
        commands: Vec::new(),
        command_names: Vec::new(),
        alignment_transitions: Vec::new(),
        box_registers: Vec::new(),
        node_depth: None,
        include_mode_transitions: false,
        include_artifact_hashes: false,
        terminal_checks: Vec::new(),
    };

    assert_eq!(project(&run, &projection), ["count:2=7"]);
}

#[test]
fn fatal_termination_precedes_every_projection_kinds_own_output() {
    let mut counts = [0; COUNT_SLOTS];
    counts[2] = 7;
    let run = SemanticRun {
        observations: Vec::new(),
        diagnostic_root_name: "./test.tex".into(),
        diagnostic_root_bytes: std::sync::Arc::from(&b""[..]),
        counts,
        box_outlines: BTreeMap::new(),
        mode_transitions: Vec::new(),
        artifacts: Vec::new(),
        dvi: Vec::new(),
        fatal: Some(FatalError::confusion("256 spans")),
        terminal: Vec::new(),
        log: Vec::new(),
        pending_effects: Vec::new(),
        effect_artifacts: Vec::new(),
        complete_job_channel_streams: None,
    };
    let projection = Projection {
        kind: ProjectionKind::State,
        count_registers: vec![2],
        include_count_mutations: false,
        kinds: Vec::new(),
        commands: Vec::new(),
        command_names: Vec::new(),
        alignment_transitions: Vec::new(),
        box_registers: Vec::new(),
        node_depth: None,
        include_mode_transitions: false,
        include_artifact_hashes: false,
        terminal_checks: Vec::new(),
    };

    assert_eq!(
        project(&run, &projection),
        ["execution:error:confusion(256 spans)", "count:2=7"]
    );
}

#[test]
fn terminal_checks_report_presence_and_absence_in_declaration_order() {
    let checks = vec!["alpha beta".into(), "gamma".into()];

    assert_eq!(
        terminal_check_results("alpha beta", &checks),
        [
            "terminal-check:alpha beta=true",
            "terminal-check:gamma=false"
        ]
    );
}

#[test]
fn strict_xfail_accepts_only_the_pinned_failure_and_rejects_xpass() {
    let expectation = Expectation::Xfail {
        bug: "umber2-o96f".into(),
        mismatch: MismatchFingerprint {
            index: 0,
            kind: "observation".into(),
            expected: "expected".into(),
            actual: "known-bug".into(),
        },
    };
    let expected = vec!["expected".into()];
    assert_eq!(
        evaluate_expectation(&expected, &Ok(vec!["expected".into()]), &expectation),
        Err(ExpectationError::Xpass)
    );
    assert!(matches!(
        evaluate_expectation(&expected, &Ok(vec!["new-failure".into()]), &expectation),
        Err(ExpectationError::ChangedFailure { .. })
    ));
    assert_eq!(
        evaluate_expectation(&expected, &Ok(vec!["known-bug".into()]), &expectation),
        Ok(())
    );
}
