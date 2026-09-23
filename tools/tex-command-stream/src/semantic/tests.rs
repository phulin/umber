//! Unit tests for the minifixture minimality contract: the byte ceiling, the
//! line ceiling, and the no-format-loading rule. Each rule gets an accept and
//! a reject case so a rule that cannot fail is not actually enforced.

use super::{
    Case, CaseManifestV2, ChannelContract, CommandDeliveryBoundary, CommandObservation,
    MAX_SOURCE_BYTES, MAX_SOURCE_LINES, Projection, SemanticRun, StreamChannel, StreamDisposition,
    channels::compare, evaluate_expectation, input_targets, project,
    validate_completion_observations, validate_no_format_loading, validate_source_dimensions,
};

fn empty_run() -> SemanticRun {
    SemanticRun {
        observations: Vec::new(),
        diagnostic_root_name: "probe.tex".into(),
        diagnostic_root_bytes: std::sync::Arc::from(&b""[..]),
        counts: [0; super::COUNT_SLOTS],
        box_outlines: Default::default(),
        mode_transitions: Vec::new(),
        artifacts: Vec::new(),
        dvi: Vec::new(),
        fatal: None,
        terminal: Vec::new(),
        log: Vec::new(),
        pending_effects: Vec::new(),
        effect_artifacts: Vec::new(),
        complete_job_channels: None,
    }
}

#[test]
fn page_count_is_semantic_and_equal_counts_do_not_hide_changed_dvi() {
    let projection: Projection =
        serde_json::from_str(r#"{"kind":"execution-boundaries","include_page_count":true}"#)
            .expect("page-count projection");
    let mut run = empty_run();
    assert_eq!(project(&run, &projection), ["page-count:0"]);
    run.artifacts
        .push(tex_state::ContentHash::from_bytes(b"first page"));
    assert_eq!(project(&run, &projection), ["page-count:1"]);
    assert!(
        evaluate_expectation(
            &["page-count:0".into()],
            &Ok(project(&run, &projection)),
            &super::Expectation::Pass,
        )
        .is_err()
    );

    let reference = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/corpus/command-semantic/page-output/single-glyph/expected.dvi"
    ));
    let parsed = tex_out::dvi::disasm::DviFile::parse(reference).expect("reference DVI");
    assert_eq!(parsed.pages.len(), 1);
    let mut changed = reference.to_vec();
    changed[parsed.post_offset + 24] ^= 1; // postamble maximum page width
    assert_eq!(
        tex_out::dvi::disasm::DviFile::parse(&changed)
            .expect("mutated DVI retains page framing")
            .pages
            .len(),
        1
    );
    let captured = super::CapturedChannels {
        events: 0,
        status: "clean".into(),
        streams: [Vec::new(), Vec::new(), changed, Vec::new(), Vec::new()],
    };
    let contract = ChannelContract {
        status: "clean".into(),
        terminal: StreamDisposition::Empty,
        log: StreamDisposition::Empty,
        dvi: StreamDisposition::File,
        effects: StreamDisposition::Empty,
        diagnostics: StreamDisposition::Empty,
    };
    let failures = compare(&captured, &contract, &|channel| {
        (channel == StreamChannel::Dvi).then(|| reference.to_vec())
    });
    assert!(failures.iter().any(|failure| matches!(
        failure,
        super::ChannelFailure::Content { channel: "dvi", .. }
    )));
}

fn command(name: &str, operand: Option<i64>, expanded: bool) -> CommandObservation {
    CommandObservation::Command(tex_command::CommandDeliveryRecord {
        boundary: if expanded {
            CommandDeliveryBoundary::Expanded
        } else {
            CommandDeliveryBoundary::Raw
        },
        spelling: tex_command::ObservedToken::ControlSequence(name.into()),
        command: if name == "probe" { "call" } else { "relax" }.into(),
        command_operand: operand,
        semantic_operand: None,
        provenance: tex_command::CommandProvenance {
            input_level: 0,
            position: 0,
            delivery_sequence: 0,
            has_origin: false,
            origin: tex_state::token::OriginId::UNKNOWN,
            source_range: None,
            source_location: None,
        },
    })
}

#[test]
fn macro_projection_ignores_definition_address_but_requires_expansion_order() {
    let projection: Projection = serde_json::from_str(
        r#"{"kind":"observations","kinds":["command","macro"],"commands":["call","relax"]}"#,
    )
    .expect("macro invocation projection");
    let activation = CommandObservation::Macro(tex_command::MacroRecord::Activation {
        control_sequence: "probe".into(),
        argument_count: 0,
        token_count: 0,
    });
    let mut run = empty_run();
    run.observations = vec![
        command("probe", Some(249_984), false),
        activation.clone(),
        command("relax", Some(256), true),
    ];
    let expected = [
        "command:raw:cs:probe:call".to_owned(),
        "macro:activate:probe:0:".to_owned(),
        "command:expanded:cs:relax:relax:256".to_owned(),
    ];
    assert_eq!(project(&run, &projection), expected);
    run.observations[0] = command("probe", Some(9_999_999), false);
    assert_eq!(project(&run, &projection), expected);
    run.observations.swap(1, 2);
    assert!(
        evaluate_expectation(
            &expected,
            &Ok(project(&run, &projection)),
            &super::Expectation::Pass
        )
        .is_err()
    );
    run.observations.swap(1, 2);
    run.observations.pop();
    assert!(
        evaluate_expectation(
            &expected,
            &Ok(project(&run, &projection)),
            &super::Expectation::Pass
        )
        .is_err()
    );
}

#[test]
fn terminal_checks_use_complete_job_stream_and_detect_changed_result() {
    let projection: Projection =
        serde_json::from_str(r#"{"kind":"terminal-checks","terminal_checks":["final cleanup"]}"#)
            .expect("terminal phrase projection");
    let mut run = empty_run();
    run.terminal = b"fragment without the phrase".to_vec();
    run.complete_job_channels = Some(super::CapturedChannels {
        events: 0,
        status: "clean".into(),
        streams: [
            b"root closed; final cleanup".to_vec(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ],
    });
    let expected = ["terminal-check:final cleanup=true".to_owned()];
    assert_eq!(project(&run, &projection), expected);
    run.complete_job_channels
        .as_mut()
        .expect("complete terminal stream")
        .streams[0] = b"root closed".to_vec();
    assert!(
        evaluate_expectation(
            &expected,
            &Ok(project(&run, &projection)),
            &super::Expectation::Pass,
        )
        .is_err()
    );
    run.complete_job_channels = None;
    assert_eq!(
        project(&run, &projection),
        ["terminal-check:final cleanup=false"]
    );
}

fn effect(kind: tex_command::ObservationEffectKind) -> tex_command::CommandObservation {
    tex_command::CommandObservation::Effect(tex_command::EffectRecord {
        kind,
        channel: "engine".into(),
        value: tex_command::ObservationValue::None,
        source: None,
    })
}

fn outcome(
    history: tex_state::print::ErrorHistory,
    aborted: bool,
) -> tex_command::CommandObservation {
    tex_command::CommandObservation::DiagnosticLifecycle(
        tex_command::DiagnosticLifecycleRecord::terminal(history, aborted),
    )
}

#[test]
fn completion_pair_allows_its_first_difference_at_fragment_termination() {
    let shared = effect(tex_command::ObservationEffectKind::Message);
    let fragment = [
        shared.clone(),
        effect(tex_command::ObservationEffectKind::Terminate),
    ];
    let complete = [shared, effect(tex_command::ObservationEffectKind::Input)];

    assert!(validate_completion_observations(&fragment, &complete).is_ok());
}

#[test]
fn completion_pair_rejects_semantic_drift_before_fragment_termination() {
    let fragment = [effect(tex_command::ObservationEffectKind::Message)];
    let complete = [effect(tex_command::ObservationEffectKind::Input)];

    let error = validate_completion_observations(&fragment, &complete)
        .expect_err("early semantic drift must fail");
    assert!(error.contains("before the fragment root-EOF boundary at index 0"));
    assert!(error.contains("fragment=Effect("));
    assert!(error.contains("complete=Some(Effect("));
}

#[test]
fn completion_pair_allows_only_the_final_fragment_outcome_to_differ() {
    let shared = effect(tex_command::ObservationEffectKind::Message);
    let fragment = [
        shared.clone(),
        outcome(tex_state::print::ErrorHistory::Spotless, false),
    ];
    let complete = [
        shared,
        outcome(tex_state::print::ErrorHistory::FatalErrorStop, true),
    ];
    assert!(validate_completion_observations(&fragment, &complete).is_ok());
}

#[test]
fn completion_pair_rejects_early_drift_even_with_a_final_outcome() {
    let fragment = [
        effect(tex_command::ObservationEffectKind::Message),
        outcome(tex_state::print::ErrorHistory::Spotless, false),
    ];
    let complete = [
        effect(tex_command::ObservationEffectKind::Input),
        outcome(tex_state::print::ErrorHistory::Spotless, false),
    ];
    let error = validate_completion_observations(&fragment, &complete)
        .expect_err("different commands before the terminal outcome must fail");
    assert!(error.contains("at index 0"));

    let fragment = [
        outcome(tex_state::print::ErrorHistory::Spotless, false),
        effect(tex_command::ObservationEffectKind::Message),
        outcome(tex_state::print::ErrorHistory::Spotless, false),
    ];
    let complete = [
        outcome(tex_state::print::ErrorHistory::FatalErrorStop, true),
        effect(tex_command::ObservationEffectKind::Message),
        outcome(tex_state::print::ErrorHistory::Spotless, false),
    ];
    let error = validate_completion_observations(&fragment, &complete)
        .expect_err("an earlier diagnostic outcome is still shared execution");
    assert!(error.contains("at index 0"));
}

#[test]
fn completion_pair_rejects_a_complete_run_shorter_than_the_shared_prefix() {
    let fragment = [
        effect(tex_command::ObservationEffectKind::Message),
        outcome(tex_state::print::ErrorHistory::Spotless, false),
    ];
    let error = validate_completion_observations(&fragment, &[])
        .expect_err("a truncated complete run has no shared observation");
    assert!(error.contains("at index 0"));
    assert!(error.contains("complete=None"));
}

#[test]
fn completion_pair_rejects_semantic_events_after_a_fragment_termination() {
    let fragment = [
        effect(tex_command::ObservationEffectKind::Terminate),
        effect(tex_command::ObservationEffectKind::Message),
        outcome(tex_state::print::ErrorHistory::Spotless, false),
    ];
    let complete = [
        effect(tex_command::ObservationEffectKind::Input),
        effect(tex_command::ObservationEffectKind::Message),
        outcome(tex_state::print::ErrorHistory::Spotless, false),
    ];
    let error = validate_completion_observations(&fragment, &complete)
        .expect_err("a command after fragment termination is not a terminal marker");
    assert!(error.contains("nonterminal observation after termination at index 1"));
}

#[test]
fn completion_pair_rejects_a_complete_run_with_no_terminal_continuation() {
    let fragment = [outcome(tex_state::print::ErrorHistory::Spotless, false)];
    let error = validate_completion_observations(&fragment, &[])
        .expect_err("a complete job must reach its own terminal observation");
    assert!(error.contains("complete job ended before the fragment root-EOF boundary"));
}

#[test]
fn v2_manifest_admits_omitted_channels_only_for_derivation_validation() {
    let manifest: CaseManifestV2 = serde_json::from_str(
        r#"{
            "schema": 2,
            "property_id": "tex82.probe.case",
            "provenance": {
                "authority": "tex.web",
                "manifest": "tests/tex82-oracle-manifest.txt",
                "sections": [1]
            },
            "projection": {"kind": "predicate-outcomes"},
            "expected": []
        }"#,
    )
    .expect("a fresh V2 candidate is parseable before derivation");
    let resolved = manifest.resolve(std::path::Path::new("."), "probe".to_owned());
    assert!(resolved.channels.is_none());
    assert!(resolved.expected.is_empty());
}

fn case_with_inputs(inputs: &[(&str, &str)]) -> Case {
    let inputs_json = inputs
        .iter()
        .map(|(name, content)| format!("{name:?}:{content:?}"))
        .collect::<Vec<_>>()
        .join(",");
    let json = format!(
        r#"{{
            "id": "probe",
            "property_id": "tex82.probe.case",
            "source": "probe.tex",
            "provenance": {{
                "authority": "tex.web",
                "manifest": "tests/tex82-oracle-manifest.txt",
                "sections": [1]
            }},
            "projection": {{ "kind": "predicate-outcomes" }},
            "expected": ["predicate:probe:-:true"],
            "expectation": {{ "kind": "pass" }},
            "inputs": {{ {inputs_json} }}
        }}"#
    );
    serde_json::from_str(&json).expect("probe case JSON is well-formed")
}

fn case() -> Case {
    case_with_inputs(&[])
}

#[test]
fn fresh_runner_opens_registered_root_in_both_reference_text_channels() {
    let run = super::execute_fresh(br"\end", &case()).expect("complete fresh job");
    let channels = super::CapturedChannels::capture(&run);
    for channel in [StreamChannel::Terminal, StreamChannel::Log] {
        assert!(
            channels
                .stream(channel)
                .windows(b"(./probe.tex".len())
                .any(|window| window == b"(./probe.tex"),
            "{} must include TeX82 §537's opened root name",
            channel.name()
        );
    }
}

// --- MAX_SOURCE_BYTES -------------------------------------------------

#[test]
fn source_dimensions_accepts_a_source_within_the_byte_ceiling() {
    assert!(validate_source_dimensions("probe", 1, 1).is_ok());
    assert!(validate_source_dimensions("probe", MAX_SOURCE_BYTES as usize, 1).is_ok());
}

#[test]
fn source_dimensions_rejects_a_source_over_the_byte_ceiling() {
    let error = validate_source_dimensions("probe", MAX_SOURCE_BYTES as usize + 1, 1)
        .expect_err("a source over the byte ceiling must be rejected");
    assert_eq!(
        error,
        format!("case probe source must be 1..={MAX_SOURCE_BYTES} bytes")
    );
}

#[test]
fn source_dimensions_rejects_an_empty_source() {
    let error =
        validate_source_dimensions("probe", 0, 0).expect_err("an empty source must be rejected");
    assert_eq!(
        error,
        format!("case probe source must be 1..={MAX_SOURCE_BYTES} bytes")
    );
}

// --- MAX_SOURCE_LINES ---------------------------------------------------

#[test]
fn source_dimensions_accepts_a_source_within_the_line_ceiling() {
    assert!(validate_source_dimensions("probe", 1, MAX_SOURCE_LINES).is_ok());
}

#[test]
fn source_dimensions_rejects_a_source_over_the_line_ceiling() {
    let error = validate_source_dimensions("probe", 1, MAX_SOURCE_LINES + 1)
        .expect_err("a source over the line ceiling must be rejected");
    assert_eq!(
        error,
        format!("case probe source must be at most {MAX_SOURCE_LINES} lines")
    );
}

// --- no format or package loading ---------------------------------------

#[test]
fn format_loading_accepts_a_self_contained_source() {
    assert!(validate_no_format_loading(&case(), "\\count0=1\\end").is_ok());
}

#[test]
fn format_loading_rejects_plain_tex_reference() {
    let error = validate_no_format_loading(&case(), "\\input plain.tex\\end")
        .expect_err("a reference to plain.tex must be rejected");
    assert_eq!(
        error,
        "case probe source references plain.tex, which loads a format or package"
    );
}

#[test]
fn format_loading_rejects_input_plain() {
    let error = validate_no_format_loading(&case(), "\\input plain\\end")
        .expect_err("\\input plain must be rejected");
    assert_eq!(
        error,
        "case probe source uses \\input plain, which loads a format"
    );
}

/// `\dump` writes a format rather than loading one, so it does not bear on
/// minimality and is not forbidden. `main-control/final-cleanup-end-or-dump`
/// exists to exercise tex.web §1335's rejection of it. What actually stops a
/// fixture assembling a format is the undeclared-`\input` rule, which applies
/// to every case without exception.
#[test]
fn format_loading_permits_dump_which_writes_rather_than_loads_a_format() {
    assert!(validate_no_format_loading(&case(), "\\dump").is_ok());
    assert!(validate_no_format_loading(&case(), "\\count0=1\\dump").is_ok());
}

#[test]
fn format_loading_rejects_undeclared_input_target() {
    let error = validate_no_format_loading(&case(), "\\input nested\\end")
        .expect_err("an \\input target absent from the inputs map must be rejected");
    assert_eq!(
        error,
        "case probe uses \\input \"nested.tex\", which is not declared in this case's inputs map"
    );
}

#[test]
fn format_loading_accepts_input_target_declared_in_inputs_map() {
    let declared = case_with_inputs(&[("nested.tex", "N")]);
    assert!(validate_no_format_loading(&declared, "\\input nested\\end").is_ok());

    let declared = case_with_inputs(&[("child.tex", "C")]);
    assert!(validate_no_format_loading(&declared, "\\input child.tex\\end").is_ok());
}

// --- input_targets: TeX file-name scanning ------------------------------

#[test]
fn input_targets_appends_tex_when_the_name_has_no_extension() {
    assert_eq!(input_targets("\\input nested"), ["nested.tex"]);
}

#[test]
fn input_targets_keeps_an_explicit_extension() {
    assert_eq!(input_targets("\\input child.tex"), ["child.tex"]);
}

#[test]
fn input_targets_skips_a_longer_control_word() {
    let empty: Vec<String> = Vec::new();
    assert_eq!(input_targets("\\inputlineno"), empty);
}

#[test]
fn input_targets_finds_every_occurrence() {
    assert_eq!(input_targets("\\input a\\input b.tex"), ["a.tex", "b.tex"]);
}
