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
        streams: [String::new(), String::new(), String::new(), String::new()],
    }
}

fn no_files(_: StreamChannel) -> Option<String> {
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
    run.streams[1] = "! Undefined control sequence.\n".into();
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
    run.streams[0] = "terminal".into();
    run.streams[1] = "log".into();
    run.streams[2] = "page:0:abc".into();
    run.streams[3] = "special:dvi:x".into();
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
    declared.log = StreamDisposition::File {
        authority: ChannelAuthority::UmberBaseline,
    };
    let mut run = captured();
    run.streams[1] = "anything".into();
    assert_eq!(
        compare(&run, &declared, &no_files),
        vec![ChannelFailure::MissingFile {
            channel: "log",
            path: "expected/<case>.log".into()
        }]
    );
}

#[test]
fn a_file_disposition_names_the_first_differing_line() {
    let mut declared = contract();
    declared.log = StreamDisposition::File {
        authority: ChannelAuthority::Oracle,
    };
    let mut run = captured();
    run.streams[1] = "same\nmoved\n".into();
    let committed = |channel: StreamChannel| match channel {
        StreamChannel::Log => Some("same\noriginal\n".to_owned()),
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
    declared.terminal = StreamDisposition::File {
        authority: ChannelAuthority::Oracle,
    };
    let mut run = captured();
    run.streams[0] = "one\n".into();
    let committed = |channel: StreamChannel| match channel {
        StreamChannel::Terminal => Some("one\ntwo\n".to_owned()),
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

/// An `xfail` channel pins the known-wrong bytes, so byte-identity passes.
/// Nothing here can call a change a fix; that is the bug's own gate's job.
#[test]
fn an_xfail_channel_pins_its_observed_bytes() {
    let mut declared = contract();
    declared.effects = StreamDisposition::Xfail {
        bug: "umber2-johp.246".into(),
        authority: ChannelAuthority::UmberBaseline,
    };
    let mut run = captured();
    run.streams[3] = "special:dvi:wrong\n".into();
    let committed = |channel: StreamChannel| match channel {
        StreamChannel::Effects => Some("special:dvi:wrong\n".to_owned()),
        _ => None,
    };
    assert_eq!(compare(&run, &declared, &committed), Vec::new());

    run.streams[3] = "special:dvi:changed\n".into();
    assert_eq!(compare(&run, &declared, &committed).len(), 1);
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
}
