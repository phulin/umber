//! Two-phase channel parity policy, mismatch controls, and image contract.

use super::*;

#[test]
fn dump_construction_excludes_only_textual_diagnostics() {
    let expected = b"reference allocator diagnostics";
    let actual = b"implementation-owned serialization diagnostics";
    assert_eq!(
        PhaseParityContract::DumpConstruction.text_channel(expected, actual),
        (&[][..], &[][..])
    );
}

#[test]
fn loaded_and_ordinary_phases_retain_exact_text_channels() {
    let expected = b"reference output";
    let actual = b"mutated output";
    assert_eq!(
        PhaseParityContract::OutputProducing.text_channel(expected, actual),
        (&expected[..], &actual[..])
    );
    assert_ne!(
        expected.as_slice(),
        actual.as_slice(),
        "negative control must remain divergent"
    );
}

#[test]
fn trip_channel_mismatch_controls_fail_at_the_caller_boundary() {
    fn mismatch_panics(
        root: &Path,
        expected: TripTriageChannels<'_>,
        actual: TripTriageChannels<'_>,
    ) -> String {
        let verdict = write_trip_triage_artifact(
            root,
            TripTriageInput {
                label: "negative-control",
                phase: "bounded",
                expected_source: TripTriageSource {
                    name: "expected",
                    identity: "expected-id",
                },
                actual_source: TripTriageSource {
                    name: "actual",
                    identity: "actual-id",
                },
                expected,
                actual,
            },
        )
        .expect("write negative-control report");
        let panic = std::panic::catch_unwind(|| assert_trip_channels_match(&verdict))
            .expect_err("channel mismatch must fail");
        panic
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| {
                panic
                    .downcast_ref::<&str>()
                    .map(|message| (*message).to_owned())
            })
            .expect("panic carries a string")
    }

    let temp = tempfile::tempdir().expect("negative-control directory");
    let base = TripTriageChannels {
        initialization_events: None,
        command_events: None,
        geometry_events: None,
        transcript: b"transcript",
        log: b"log",
        dvi: None,
    };
    for (channel, expected, actual) in [
        (
            "command_events",
            TripTriageChannels {
                command_events: Some(b""),
                ..base
            },
            base,
        ),
        (
            "transcript",
            base,
            TripTriageChannels {
                transcript: b"mutated transcript",
                ..base
            },
        ),
        (
            "log",
            base,
            TripTriageChannels {
                log: b"mutated log",
                ..base
            },
        ),
    ] {
        let message = mismatch_panics(temp.path(), expected, actual);
        assert!(
            message.contains("TRIP compared-channel mismatch"),
            "{message}"
        );
        assert!(message.contains("report:"), "{message}");
        assert!(
            message.contains(&format!("earliest.channel: {channel}")),
            "{message}"
        );
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // Reads the bounded host-side triage artifact.
fn trip_geometry_only_mismatch_is_reported_but_non_gating() {
    let temp = tempfile::tempdir().expect("geometry control directory");
    let base = TripTriageChannels {
        initialization_events: None,
        command_events: None,
        geometry_events: None,
        transcript: b"transcript",
        log: b"log",
        dvi: None,
    };
    let verdict = write_trip_triage_artifact(
        temp.path(),
        TripTriageInput {
            label: "geometry-advisory-control",
            phase: "bounded",
            expected_source: TripTriageSource {
                name: "expected",
                identity: "expected-id",
            },
            actual_source: TripTriageSource {
                name: "actual",
                identity: "actual-id",
            },
            expected: TripTriageChannels {
                geometry_events: Some(b""),
                ..base
            },
            actual: base,
        },
    )
    .expect("write advisory report");
    assert!(!verdict.gating_mismatch);
    assert!(verdict.advisory_geometry_mismatch);
    let report = fs::read_to_string(verdict.artifact.as_ref().expect("advisory report path"))
        .expect("read advisory report");
    assert!(
        report.contains("status: advisory-geometry-mismatch"),
        "{report}"
    );
    assert!(
        report.contains("geometry.policy: advisory-non-gating"),
        "{report}"
    );
    assert_trip_channels_match(&verdict);
}

#[test]
fn format_image_contract_excludes_runtime_state_and_rebuilds_registry() {
    let format = umber::with_engine_universe(|source| {
        EngineMode::Tex82.prepare_initex(source);
        source
            .world_mut()
            .write_text(PrintSink::TerminalAndLog, "host effect excluded");
        let mut session =
            umber::EngineSession::prepared_initex(source, tex_command::CommandProfile::TEX82);
        session
            .register_authored_job("format.tex", Arc::from(&b"\\dump"[..]))
            .expect("format contract root registers");
        let mut host =
            umber::FileSessionResolvers::new(Path::new("format.tex"), Vec::new(), Vec::new());
        session
            .run(&mut host, &mut Vec::new())
            .expect("format contract construction")
            .format_dump
            .expect("bounded format image")
            .image
            .into_bytes()
    })
    .expect("fresh format-contract universe");

    assert_format_image_contract(&format, EngineMode::Tex82);
}
