use super::*;

fn contract() -> ChannelContract {
    ChannelContract {
        events: 3,
        status: "clean".into(),
        terminal: StreamDisposition::Empty,
        log: StreamDisposition::Empty,
        dvi: StreamDisposition::Empty,
        effects: StreamDisposition::Empty,
    }
}

fn captured() -> CapturedChannels {
    CapturedChannels {
        events: 3,
        status: "clean".into(),
        streams: [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
    }
}

fn run_with_printable_sink_writes(committed: bool) -> SemanticRun {
    let writes = vec![
        EffectRecord::StreamWrite {
            sink: PrintSink::Terminal,
            text: "terminal|".into(),
        },
        EffectRecord::StreamWrite {
            sink: PrintSink::Log,
            text: "log-only|".into(),
        },
        EffectRecord::StreamWrite {
            sink: PrintSink::TerminalAndLog,
            text: "both|".into(),
        },
    ];
    SemanticRun {
        observations: Vec::new(),
        counts: [0; super::super::COUNT_SLOTS],
        box_outlines: std::collections::BTreeMap::new(),
        mode_transitions: Vec::new(),
        artifacts: Vec::new(),
        dvi: Vec::new(),
        fatal: None,
        terminal: if committed {
            b"terminal|both|".to_vec()
        } else {
            Vec::new()
        },
        log: if committed {
            b"log-only|both|".to_vec()
        } else {
            Vec::new()
        },
        pending_effects: if committed { Vec::new() } else { writes },
        effect_artifacts: Vec::new(),
        complete_job_channel_streams: None,
    }
}

#[test]
fn terminal_projection_excludes_log_only_writes_before_and_after_commit() {
    for committed in [false, true] {
        let run = run_with_printable_sink_writes(committed);
        assert_eq!(super::super::captured_terminal_text(&run), "terminal|both|");
    }
}

#[test]
fn channel_capture_preserves_terminal_log_and_shared_sink_routing() {
    for committed in [false, true] {
        let captured = CapturedChannels::capture(&run_with_printable_sink_writes(committed));
        assert_eq!(captured.stream(StreamChannel::Terminal), b"terminal|both|");
        assert_eq!(captured.stream(StreamChannel::Log), b"log-only|both|");
    }
}

fn no_files(_: StreamChannel) -> Option<Vec<u8>> {
    None
}

#[test]
fn a_matching_run_reports_nothing() {
    assert_eq!(compare(&captured(), &contract(), &no_files), Vec::new());
}

#[test]
fn an_event_count_change_is_reported() {
    let mut run = captured();
    run.events = 4;
    assert_eq!(
        compare(&run, &contract(), &no_files),
        vec![ChannelFailure::EventCount {
            declared: 3,
            observed: 4
        }]
    );
}

#[test]
fn a_fatal_termination_is_reported_against_a_clean_declaration() {
    let mut run = captured();
    run.status = "fatal:confusion(vpack)".into();
    assert_eq!(
        compare(&run, &contract(), &no_files),
        vec![ChannelFailure::Status {
            declared: "clean".into(),
            observed: "fatal:confusion(vpack)".into()
        }]
    );
}

/// The defect this module exists to prevent: output on a channel no
/// projection reads. Declaring `empty` is what turns that into a gate.
#[test]
fn output_on_a_channel_declared_empty_fails() {
    let mut run = captured();
    run.streams[1] = b"! Undefined control sequence.\n".to_vec();
    assert_eq!(
        compare(&run, &contract(), &no_files),
        vec![ChannelFailure::NotEmpty {
            channel: "log",
            bytes: 30
        }]
    );
}

#[test]
fn portable_effects_are_typed_ordered_and_artifact_sorted() {
    let effects = [
        EffectEvent {
            kind: EffectKind::Message,
            channel: "terminal".into(),
            value: tex_oracle::CanonicalValue::Bytes(b"not duplicated".to_vec()),
        },
        EffectEvent {
            kind: EffectKind::Open,
            channel: "stream:3".into(),
            value: tex_oracle::CanonicalValue::Name("alpha.out".into()),
        },
        EffectEvent {
            kind: EffectKind::Write,
            channel: "stream:3".into(),
            value: tex_oracle::CanonicalValue::Bytes(b"alpha".to_vec()),
        },
        EffectEvent {
            kind: EffectKind::Close,
            channel: "stream:3".into(),
            value: tex_oracle::CanonicalValue::None,
        },
        EffectEvent {
            kind: EffectKind::Terminate,
            channel: "engine".into(),
            value: tex_oracle::CanonicalValue::None,
        },
    ];
    let artifacts = [
        EffectArtifact {
            path: "z.out".into(),
            bytes: vec![],
        },
        EffectArtifact {
            path: "alpha.out".into(),
            bytes: b"alpha\n".to_vec(),
        },
    ];
    assert_eq!(
        String::from_utf8(portable_effect_channel(effects, artifacts)).expect("utf8 JSON"),
        concat!(
            "{\"record\":\"effect\",\"effect\":{\"kind\":\"open\",\"channel\":\"stream:3\",\"value\":{\"type\":\"name\",\"value\":\"alpha.out\"}}}\n",
            "{\"record\":\"effect\",\"effect\":{\"kind\":\"write\",\"channel\":\"stream:3\",\"value\":{\"type\":\"bytes\",\"value\":[97,108,112,104,97]}}}\n",
            "{\"record\":\"effect\",\"effect\":{\"kind\":\"close\",\"channel\":\"stream:3\",\"value\":{\"type\":\"none\"}}}\n",
            "{\"record\":\"artifact\",\"path\":\"alpha.out\",\"bytes\":[97,108,112,104,97,10]}\n",
            "{\"record\":\"artifact\",\"path\":\"z.out\",\"bytes\":[]}\n",
        )
    );
}

#[test]
fn unsupported_is_an_explicit_non_authoritative_effect_disposition() {
    let mut declared = contract();
    declared.effects = StreamDisposition::Unsupported {
        reason: "reference profile disables shell escape".into(),
    };
    let mut run = captured();
    run.streams[3] = b"implementation-private record".to_vec();
    assert_eq!(compare(&run, &declared, &no_files), Vec::new());
}

#[test]
fn every_diverging_channel_is_reported_not_just_the_first() {
    let mut run = captured();
    run.streams[0] = b"terminal".to_vec();
    run.streams[1] = b"log".to_vec();
    run.streams[2] = b"page:0:abc".to_vec();
    run.streams[3] = b"special:dvi:x".to_vec();
    let failures = compare(&run, &contract(), &no_files);
    assert_eq!(failures.len(), 4, "{failures:?}");
    assert!(
        failures
            .iter()
            .all(|failure| matches!(failure, ChannelFailure::NotEmpty { .. })),
        "{failures:?}"
    );
}

#[test]
fn a_file_disposition_without_a_committed_file_fails() {
    let mut declared = contract();
    declared.log = StreamDisposition::File;
    let mut run = captured();
    run.streams[1] = b"anything".to_vec();
    assert_eq!(
        compare(&run, &declared, &no_files),
        vec![ChannelFailure::MissingFile {
            channel: "log",
            path: "expected.log".into()
        }]
    );
}

#[test]
fn a_file_disposition_names_the_first_differing_line() {
    let mut declared = contract();
    declared.log = StreamDisposition::File;
    let mut run = captured();
    run.streams[1] = b"same\nmoved\n".to_vec();
    let committed = |channel: StreamChannel| match channel {
        StreamChannel::Log => Some(b"same\noriginal\n".to_vec()),
        _ => None,
    };
    assert_eq!(
        compare(&run, &declared, &committed),
        vec![ChannelFailure::Content {
            channel: "log",
            line: 2,
            declared: "original".into(),
            observed: "moved".into()
        }]
    );
}

#[test]
fn a_truncated_channel_reports_the_end_rather_than_matching() {
    let mut declared = contract();
    declared.terminal = StreamDisposition::File;
    let mut run = captured();
    run.streams[0] = b"one\n".to_vec();
    let committed = |channel: StreamChannel| match channel {
        StreamChannel::Terminal => Some(b"one\ntwo\n".to_vec()),
        _ => None,
    };
    assert_eq!(
        compare(&run, &declared, &committed),
        vec![ChannelFailure::Content {
            channel: "terminal",
            line: 2,
            declared: "two".into(),
            observed: "<end of channel>".into()
        }]
    );
}

/// The committed file under an `xfail` disposition always holds the
/// *reference engine's* bytes. `mismatch` pins exactly where Umber's own
/// output first diverges from them, and matching that pin exactly is what
/// passes -- unlike the old contract, byte-identity to the committed file is
/// not what passes here; it is what triggers an xpass instead (below).
fn xfail_effects(mismatch: ChannelMismatch) -> StreamDisposition {
    StreamDisposition::Xfail {
        bug: "umber2-johp.246".into(),
        mismatch,
    }
}

#[test]
fn an_xfail_channel_matching_its_pinned_divergence_passes() {
    let mut declared = contract();
    declared.effects = xfail_effects(ChannelMismatch {
        line: 1,
        expected: "special:dvi:reference".into(),
        actual: "special:dvi:wrong".into(),
    });
    let mut run = captured();
    run.streams[3] = b"special:dvi:wrong\n".to_vec();
    let committed = |channel: StreamChannel| match channel {
        StreamChannel::Effects => Some(b"special:dvi:reference\n".to_vec()),
        _ => None,
    };
    assert_eq!(compare(&run, &declared, &committed), Vec::new());
}

/// Umber now produces exactly the reference bytes: the pin no longer
/// describes anything, so this is a failure (an xpass) rather than a quiet
/// improvement, and it names the bug the author must close.
#[test]
fn an_xfail_channel_that_now_matches_the_reference_is_an_xpass() {
    let mut declared = contract();
    declared.effects = xfail_effects(ChannelMismatch {
        line: 1,
        expected: "special:dvi:reference".into(),
        actual: "special:dvi:wrong".into(),
    });
    let mut run = captured();
    run.streams[3] = b"special:dvi:reference\n".to_vec();
    let committed = |channel: StreamChannel| match channel {
        StreamChannel::Effects => Some(b"special:dvi:reference\n".to_vec()),
        _ => None,
    };
    assert_eq!(
        compare(&run, &declared, &committed),
        vec![ChannelFailure::Xpass {
            channel: "effects",
            bug: "umber2-johp.246".into(),
        }]
    );
}

/// Umber diverges from the reference, but not the way the pin says: this is
/// a changed failure, reporting the pinned divergence alongside the one now
/// observed, so a shift in bug behavior cannot be mistaken for the pinned one.
#[test]
fn an_xfail_channel_diverging_differently_is_a_changed_failure() {
    let mut declared = contract();
    let pinned = ChannelMismatch {
        line: 1,
        expected: "special:dvi:reference".into(),
        actual: "special:dvi:wrong".into(),
    };
    declared.effects = xfail_effects(pinned.clone());
    let mut run = captured();
    run.streams[3] = b"special:dvi:different\n".to_vec();
    let committed = |channel: StreamChannel| match channel {
        StreamChannel::Effects => Some(b"special:dvi:reference\n".to_vec()),
        _ => None,
    };
    assert_eq!(
        compare(&run, &declared, &committed),
        vec![ChannelFailure::ChangedFailure {
            channel: "effects",
            bug: "umber2-johp.246".into(),
            pinned,
            observed: ChannelMismatch {
                line: 1,
                expected: "special:dvi:reference".into(),
                actual: "special:dvi:different".into(),
            },
        }]
    );
}

/// A changed failure also fires when the divergence moves to a different
/// line rather than just changing its text on the same line.
#[test]
fn an_xfail_channel_diverging_at_a_different_line_is_a_changed_failure() {
    let mut declared = contract();
    let pinned = ChannelMismatch {
        line: 1,
        expected: "reference-one".into(),
        actual: "wrong-one".into(),
    };
    declared.effects = xfail_effects(pinned.clone());
    let mut run = captured();
    run.streams[3] = b"reference-one\nwrong-two\n".to_vec();
    let committed = |channel: StreamChannel| match channel {
        StreamChannel::Effects => Some(b"reference-one\nreference-two\n".to_vec()),
        _ => None,
    };
    assert_eq!(
        compare(&run, &declared, &committed),
        vec![ChannelFailure::ChangedFailure {
            channel: "effects",
            bug: "umber2-johp.246".into(),
            pinned,
            observed: ChannelMismatch {
                line: 2,
                expected: "reference-two".into(),
                actual: "wrong-two".into(),
            },
        }]
    );
}

/// An `xfail` channel with no committed reference file fails the same way a
/// `file` channel does: the reference bytes are still mandatory to commit.
#[test]
fn an_xfail_channel_without_a_committed_file_fails() {
    let mut declared = contract();
    declared.effects = xfail_effects(ChannelMismatch {
        line: 1,
        expected: "a".into(),
        actual: "b".into(),
    });
    let mut run = captured();
    run.streams[3] = b"anything".to_vec();
    assert_eq!(
        compare(&run, &declared, &no_files),
        vec![ChannelFailure::MissingFile {
            channel: "effects",
            path: "expected.effects".into()
        }]
    );
}

#[test]
fn validate_xfail_disposition_rejects_a_malformed_bug_id() {
    let mismatch = ChannelMismatch {
        line: 1,
        expected: "a".into(),
        actual: "b".into(),
    };
    assert!(
        validate_xfail_disposition(StreamChannel::Effects, "not-a-bead", &mismatch)
            .expect_err("malformed bug id must be rejected")
            .contains("malformed bug")
    );
}

#[test]
fn validate_xfail_disposition_accepts_a_well_formed_bug_id() {
    let mismatch = ChannelMismatch {
        line: 1,
        expected: "a".into(),
        actual: "b".into(),
    };
    assert!(
        validate_xfail_disposition(StreamChannel::Effects, "umber2-johp.246", &mismatch).is_ok()
    );
}

/// A mismatch whose `expected` and `actual` are equal pins nothing: it does
/// not describe any divergence at all, so it must be rejected rather than
/// silently accepted as a no-op pin.
#[test]
fn validate_xfail_disposition_rejects_a_mismatch_that_pins_nothing() {
    let mismatch = ChannelMismatch {
        line: 1,
        expected: "same".into(),
        actual: "same".into(),
    };
    assert!(
        validate_xfail_disposition(StreamChannel::Effects, "umber2-johp.246", &mismatch)
            .expect_err("an equal expected/actual pair pins nothing")
            .contains("pins nothing")
    );
}

#[test]
fn stream_channels_covers_every_channel_and_names_are_unique() {
    let mut names: Vec<&str> = STREAM_CHANNELS.iter().map(|c| c.name()).collect();
    let count = names.len();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), count);
    assert_eq!(names, ["dvi", "effects", "log", "terminal"]);
}

/// The committed JSON schema is documentation: `load_suite` skips it, and the
/// Rust types are what actually reject a malformed manifest. Pin the two
/// together so the document cannot drift away from the contract it describes.
#[test]
fn the_committed_schema_requires_exactly_the_contract_fields() {
    let path =
        super::super::repository_root().join("tests/corpus/command-semantic/manifest.schema.json");
    let text = std::fs::read_to_string(&path).expect("committed schema is readable");
    let committed: serde_json::Value =
        serde_json::from_str(&text).expect("committed schema is valid JSON");
    assert_eq!(committed, super::super::manifest_schema());
}

/// A minimal DVI preamble: `pre`, version, num/den/mag, comment length, then
/// `comment` -- followed by `body` so a test can prove the normalization
/// leaves everything past the comment alone.
fn dvi_with_comment(comment: &[u8], body: &[u8]) -> Vec<u8> {
    let mut bytes = vec![247, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 232];
    bytes.push(u8::try_from(comment.len()).expect("test comment fits a byte"));
    bytes.extend_from_slice(comment);
    bytes.extend_from_slice(body);
    bytes
}

/// The whole point of the change this test guards: two DVI files that differ
/// only in the preamble comment -- pdfTeX's `␣TeX output <clock>` against
/// `tex_out::model::DEFAULT_BANNER` -- compare equal, while a single byte
/// past the comment still diverges. Both banners are 27 bytes, which is the
/// documented precondition of `test_support::dvi`'s normalization: it rewrites
/// the payload in place and the length byte itself must already match.
#[test]
fn dvi_normalization_neutralizes_the_preamble_banner_only() {
    let oracle = dvi_with_comment(b" TeX output 2026.07.30:1421", &[139, 65, 66, 140]);
    let umber = dvi_with_comment(b"  Umber DVI 1970.01.01:0000", &[139, 65, 66, 140]);
    assert_eq!(
        normalize_channel(StreamChannel::Dvi, &oracle),
        normalize_channel(StreamChannel::Dvi, &umber),
    );

    let reordered = dvi_with_comment(b"  Umber DVI 1970.01.01:0000", &[139, 66, 65, 140]);
    assert_ne!(
        normalize_channel(StreamChannel::Dvi, &oracle),
        normalize_channel(StreamChannel::Dvi, &reordered),
        "a body byte must still diverge",
    );
}

/// Idempotence is what lets one call serve a committed file (normalized when
/// it was written) and a fresh capture (never normalized) alike.
#[test]
fn dvi_normalization_is_idempotent() {
    let once = normalize_channel(
        StreamChannel::Dvi,
        &dvi_with_comment(b" TeX output 2026.07.30:1421", &[139, 140]),
    )
    .expect("valid preamble");
    let twice = normalize_channel(StreamChannel::Dvi, &once).expect("valid preamble");
    assert_eq!(once, twice);
}

/// A case that ships no page is an ordinary observation, not corruption, so
/// it normalizes to empty and stays comparable against a committed reference.
#[test]
fn empty_dvi_normalizes_rather_than_failing() {
    assert_eq!(normalize_channel(StreamChannel::Dvi, &[]), Ok(Vec::new()));
}

/// Non-empty bytes with no locatable preamble comment are corrupt, and a raw
/// comparison would dress that up as an ordinary content divergence.
#[test]
fn malformed_dvi_refuses_to_normalize() {
    assert!(normalize_channel(StreamChannel::Dvi, &[1, 2, 3]).is_err());
    // `pre` present, but the declared comment length runs past the end.
    assert!(normalize_channel(StreamChannel::Dvi, &dvi_with_comment(b"abc", &[])[..16]).is_err());
}

/// A malformed artifact is reported as its own failure rather than compared.
#[test]
fn compare_reports_an_unnormalizable_dvi_channel() {
    let mut contract = contract();
    contract.dvi = StreamDisposition::File;
    let mut captured = captured();
    captured.streams[StreamChannel::Dvi as usize] = vec![1, 2, 3];
    let committed = |_channel: StreamChannel| Some(dvi_with_comment(b"x", &[139, 140]));

    assert_eq!(
        compare(&captured, &contract, &committed),
        vec![ChannelFailure::Unnormalizable {
            channel: "dvi",
            side: "observed",
            detail: "DVI is missing a valid preamble".into(),
        }]
    );
}

/// tex.web §82 frames every report the same way, so one cut recognizes all
/// of them: §306's runaway heading and its one line of partial token list,
/// `print_err`'s `!␣` line, `show_context`'s levels, §90's help lines, and
/// `error`'s own closing `print_ln`. Everything else survives.
#[test]
fn strip_diagnostic_reports_cuts_exactly_section_82s_frame() {
    let channel = b"(./x.tex\n\
                    Runaway definition?\n\
                    ->abc \n\
                    ! Forbidden control sequence found while scanning definition of \\a.\n\
                    <inserted text> \n\
                    \x20               }\n\
                    l.3 \\def\\a{\n\
                    I suspect you have forgotten a `}'.\n\
                    \n\
                    [1] )\n\
                    No pages of output.\n";
    assert_eq!(
        strip_diagnostic_reports(&split_channel_lines(channel)),
        vec![&b"(./x.tex"[..], &b"[1] )"[..], &b"No pages of output."[..],]
    );
}

/// A context level's second line is padding spaces rather than nothing, so it
/// cannot be mistaken for `error`'s closing `print_ln` and end the cut early.
#[test]
fn strip_diagnostic_reports_does_not_end_a_report_on_a_padded_context_line() {
    let channel = b"! Emergency stop.\n<*> \n    \nEnd of file on the terminal!\n\nkept\n";
    assert_eq!(
        strip_diagnostic_reports(&split_channel_lines(channel)),
        vec![&b"kept"[..]]
    );
}

/// A report the channel ends inside -- §93's `fatal_error` reaches
/// `close_files_and_terminate` without `error`'s closing `print_ln` on some
/// channels -- is cut to the end rather than left half-present.
#[test]
fn strip_diagnostic_reports_cuts_an_unterminated_report_to_the_end() {
    let channel = b"kept\n! Emergency stop.\n<*> \n";
    assert_eq!(
        strip_diagnostic_reports(&split_channel_lines(channel)),
        vec![&b"kept"[..]]
    );
}

fn diagnostics_contract(bug: &str) -> ChannelContract {
    let mut declared = contract();
    declared.terminal = StreamDisposition::XfailDiagnostics { bug: bug.into() };
    declared
}

/// The whole point of the disposition: a divergence confined to a §82 report
/// passes while the rest of the channel stays held to the reference bytes.
#[test]
fn an_xfail_diagnostics_channel_ignores_a_divergence_inside_a_report() {
    let declared = diagnostics_contract("umber2-johp.246");
    let mut run = captured();
    run.streams[0] = b"(./x.tex\n! You can't do that.\nl.1 \\x\n\nNo pages of output.\n".to_vec();
    let committed = |channel: StreamChannel| match channel {
        StreamChannel::Terminal => {
            Some(b"(./x.tex\n! Missing } inserted.\n<*> x.tex\n\nNo pages of output.\n".to_vec())
        }
        _ => None,
    };
    assert_eq!(compare(&run, &declared, &committed), Vec::new());
}

/// A divergence the reports do not cover is exactly what this disposition
/// still checks, so it fails and names the line that escaped.
#[test]
fn an_xfail_diagnostics_channel_reports_a_divergence_outside_a_report() {
    let declared = diagnostics_contract("umber2-johp.246");
    let mut run = captured();
    run.streams[0] = b"(./x.tex\n! You can't do that.\nl.1 \\x\n\nNo pages of output.\n".to_vec();
    let committed = |channel: StreamChannel| match channel {
        StreamChannel::Terminal => {
            Some(b"(./x.tex\n! Missing } inserted.\n<*> x.tex\n\n[1] )\n".to_vec())
        }
        _ => None,
    };
    assert_eq!(
        compare(&run, &declared, &committed),
        vec![ChannelFailure::DiagnosticsAside {
            channel: "terminal",
            bug: "umber2-johp.246".into(),
            line: 2,
            declared: "[1] )".into(),
            observed: "No pages of output.".into(),
        }]
    );
}

/// The reports are what `bug` describes, so a channel that matches the
/// reference raw has nothing left to describe and owes a promotion to `file`.
#[test]
fn an_xfail_diagnostics_channel_that_now_matches_the_reference_is_an_xpass() {
    let declared = diagnostics_contract("umber2-johp.246");
    let bytes = b"(./x.tex\n! Missing } inserted.\n<*> x.tex\n\nNo pages of output.\n".to_vec();
    let mut run = captured();
    run.streams[0] = bytes.clone();
    let committed = |channel: StreamChannel| match channel {
        StreamChannel::Terminal => Some(bytes.clone()),
        _ => None,
    };
    assert_eq!(
        compare(&run, &declared, &committed),
        vec![ChannelFailure::Xpass {
            channel: "terminal",
            bug: "umber2-johp.246".into(),
        }]
    );
}

/// `dvi` and `effects` hold no §82 reports, so the disposition would quietly
/// mean "compare normally" there and hide a real divergence behind a bug id.
#[test]
fn xfail_diagnostics_is_rejected_on_a_channel_that_carries_no_diagnostics() {
    assert!(validate_xfail_diagnostics_disposition(StreamChannel::Log, "umber2-johp.246").is_ok(),);
    assert!(validate_xfail_diagnostics_disposition(StreamChannel::Dvi, "umber2-johp.246").is_err(),);
    assert!(validate_xfail_diagnostics_disposition(StreamChannel::Terminal, "nope").is_err());
}
