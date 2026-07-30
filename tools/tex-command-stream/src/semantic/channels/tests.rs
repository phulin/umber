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
    let schema: serde_json::Value =
        serde_json::from_str(&text).expect("committed schema is valid JSON");
    let required = schema["$defs"]["channels"]["required"]
        .as_array()
        .expect("the schema declares required channel keys");
    let mut declared: Vec<&str> = required
        .iter()
        .map(|value| value.as_str().expect("required keys are strings"))
        .collect();
    declared.sort_unstable();

    let mut expected = vec!["events", "status"];
    expected.extend(STREAM_CHANNELS.iter().map(|channel| channel.name()));
    expected.sort_unstable();

    assert_eq!(declared, expected);

    // The `xfail` branch of `streamDisposition` must require exactly the
    // fields `StreamDisposition::Xfail` carries: a bug id and a mismatch pin.
    // Missing `mismatch` here is exactly the drift this test exists to catch
    // -- a schema that still allowed an `xfail` with no pin would document a
    // contract Rust no longer accepts.
    let one_of = schema["$defs"]["streamDisposition"]["oneOf"]
        .as_array()
        .expect("streamDisposition is a oneOf");
    let xfail_required = one_of
        .iter()
        .find(|branch| branch["properties"]["kind"]["const"] == "xfail")
        .expect("streamDisposition declares an xfail branch")["required"]
        .as_array()
        .expect("the xfail branch declares required keys");
    let mut xfail_declared: Vec<&str> = xfail_required
        .iter()
        .map(|value| value.as_str().expect("required keys are strings"))
        .collect();
    xfail_declared.sort_unstable();
    let mut xfail_expected = vec!["kind", "bug", "mismatch"];
    xfail_expected.sort_unstable();
    assert_eq!(xfail_declared, xfail_expected);

    // `channelMismatch` must require exactly `ChannelMismatch`'s own fields.
    let mismatch_required = schema["$defs"]["channelMismatch"]["required"]
        .as_array()
        .expect("channelMismatch declares required keys");
    let mut mismatch_declared: Vec<&str> = mismatch_required
        .iter()
        .map(|value| value.as_str().expect("required keys are strings"))
        .collect();
    mismatch_declared.sort_unstable();
    let mut mismatch_expected = vec!["line", "expected", "actual"];
    mismatch_expected.sort_unstable();
    assert_eq!(mismatch_declared, mismatch_expected);
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
