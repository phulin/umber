use super::*;

mod long_session;

const CMR10: &[u8] = include_bytes!("../../tex-fonts/tests/fixtures/cm/cmr10.tfm");

fn page_source(width: usize) -> String {
    format!("\\shipout\\vbox{{\\hrule height1pt width{width}pt}}\\end")
}

fn session(revision: RevisionId, source: &str) -> Session {
    Session::start((), "incremental-test", revision, source, 4096).expect("session starts")
}

fn edit(session: &Session, range: std::ops::Range<usize>, text: &str) -> Edit {
    Edit {
        base_revision: session.revision(),
        expected_hash: session.content_hash(),
        range,
        replacement: text.to_owned(),
    }
}

fn artifacts(output: &AcceptedOutput) -> Vec<&CommittedArtifact> {
    output
        .pages()
        .iter()
        .map(DetachedPreparedPage::artifact)
        .collect()
}

fn assert_detached_output_eq(actual: &AcceptedOutput, expected: &AcceptedOutput) {
    assert_eq!(
        actual.completion().effects(),
        expected.completion().effects()
    );
    assert_eq!(artifacts(actual), artifacts(expected));
    assert_eq!(
        actual.dvi_bytes().expect("actual DVI serializes"),
        expected.dvi_bytes().expect("expected DVI serializes")
    );
    assert_eq!(actual.pdf(), expected.pdf());
}

fn terminal_effect_text(output: &AcceptedOutput) -> String {
    output
        .completion()
        .effects()
        .iter()
        .filter_map(|effect| match effect {
            EffectRecord::StreamWrite { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn cold_execution_publishes_detached_artifact_and_boundary_history() {
    let source = page_source(10);
    let mut session = session(RevisionId::new(1), &source);
    let output = session.cold().expect("cold execution");
    assert_eq!(output.pages().len(), 1);
    assert!(output.pages()[0].dvi().is_some());
    assert_eq!(
        session.history()[0].key().boundary,
        EngineBoundary::JobStart
    );
    assert!(
        session
            .history()
            .iter()
            .any(|record| record.key().boundary == EngineBoundary::ShipoutComplete)
    );
}

#[test]
fn cold_candidate_runs_canonical_job_start_before_root_input() {
    let mut session = session(
        RevisionId::new(1),
        r"\immediate\write16{\the\time/\the\day/\the\month/\the\year}\end",
    );
    session.set_job_clock(JobClock {
        time: 754,
        second: 23,
        day: 9,
        month: 8,
        year: 2026,
    });

    let output = session.cold().expect("cold execution");
    let text = terminal_effect_text(&output);

    assert!(
        text.contains("This is TeX"),
        "startup banner missing: {text:?}"
    );
    assert!(
        text.contains("**<editor>"),
        "startup line missing: {text:?}"
    );
    assert!(
        text.contains("754/9/8/2026"),
        "§241 clock cells were not refreshed: {text:?}"
    );
}

#[test]
fn unavailable_input_fatal_does_not_accept_the_revision() {
    let mut session = session(RevisionId::new(1), r"\input absent \end");

    let error = session
        .cold()
        .expect_err("an unavailable required input must reject the candidate");
    let SessionError::Execute(error) = error else {
        panic!("unavailable input returned the wrong error family: {error:?}");
    };
    assert_eq!(
        error.as_fatal(),
        Some(tex_command::FatalError::emergency_stop(
            "job aborted, file error in nonstop mode"
        ))
    );
    assert_eq!(session.revision(), RevisionId::new(1));
}

#[test]
fn terminal_budget_failure_retains_attempted_fuel_telemetry() {
    let session = session(RevisionId::new(1), "\\end");
    let mut candidate = session.start_cold_candidate().expect("candidate");
    candidate.set_execution_budgets(tex_exec::ExecutionBudgets {
        steps: 0,
        ..tex_exec::ExecutionBudgets::default()
    });

    assert!(matches!(
        candidate.drive_with_resource_resolvers(&mut DirectResourceHost, &Cancellation::new()),
        Err(SessionError::Execute(
            tex_exec::ExecError::ResourceBudgetExceeded {
                resource: "steps",
                ..
            }
        ))
    ));
    assert_eq!(candidate.execution_telemetry().cumulative_fuel, 1);
}

#[test]
fn root_framing_alias_is_used_for_startup_while_provenance_keeps_the_source_path() {
    let mut session = Session::start_with_source_path(
        (),
        "job",
        "/job/main.tex",
        RevisionId::new(1),
        "\\end",
        4096,
    )
    .expect("session starts");
    session.set_root_source_framing_name("texput");

    let output = session.cold().expect("cold execution");
    let text = output
        .completion()
        .effects()
        .iter()
        .filter_map(|effect| match effect {
            EffectRecord::StreamWrite { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();

    assert!(text.contains("(texput"), "startup framing: {text:?}");
    assert!(
        !text.contains("(/job/main.tex"),
        "startup framing: {text:?}"
    );
}

#[test]
fn latex_compatibility_is_distinct_from_etex() {
    let source = r"\catcode123=1 \catcode125=2 \immediate\write16{created=\creationdate}\end";
    let clock = JobClock {
        time: 12 * 60 + 34,
        second: 23,
        day: 9,
        month: 8,
        year: 2026,
    };

    let mut etex = session(RevisionId::new(1), source);
    etex.set_command_profile(CommandProfile::ETEX26, true);
    etex.set_job_clock(clock);
    let etex_text = terminal_effect_text(&etex.cold().expect("e-TeX execution"));
    assert!(
        etex_text.contains("Undefined control sequence"),
        "{etex_text}"
    );
    assert!(!etex_text.contains("created=D:20260809123423Z"));

    let mut latex = session(RevisionId::new(1), source);
    latex.set_command_profile(CommandProfile::ETEX26, true);
    latex.set_command_compatibility(CommandCompatibility::Latex);
    latex.set_job_clock(clock);
    let latex_text = terminal_effect_text(&latex.cold().expect("LaTeX execution"));
    assert!(
        latex_text.contains("created=D:20260809123423Z"),
        "{latex_text}"
    );
    assert!(!latex_text.contains("Undefined control sequence"));
}

#[test]
fn latex_compatibility_matches_fresh_and_loaded_candidates() {
    let clock = JobClock {
        time: 12 * 60 + 34,
        second: 23,
        day: 9,
        month: 8,
        year: 2026,
    };
    let source = r"\catcode123=1 \catcode125=2 \immediate\write16{created=\creationdate}\end";

    let mut format = session(RevisionId::new(1), r"\dump");
    format.set_command_profile(CommandProfile::ETEX26, true);
    format.set_command_compatibility(CommandCompatibility::Latex);
    let format = format.cold().expect("LaTeX format construction");
    let image = DetachedFormatImage::try_from_bytes(
        format
            .format_dump()
            .expect("LaTeX format dump")
            .image
            .as_bytes()
            .to_vec(),
    )
    .expect("detached LaTeX format image");

    let mut fresh = session(RevisionId::new(1), source);
    fresh.set_command_profile(CommandProfile::ETEX26, true);
    fresh.set_command_compatibility(CommandCompatibility::Latex);
    fresh.set_job_clock(clock);
    let fresh_text = terminal_effect_text(&fresh.cold().expect("fresh LaTeX candidate"));

    let mut loaded = session(RevisionId::new(1), source);
    loaded.set_command_profile(CommandProfile::ETEX26, false);
    loaded.set_command_compatibility(CommandCompatibility::Latex);
    loaded.set_format_image(image);
    loaded.set_job_clock(clock);
    let loaded_text = terminal_effect_text(&loaded.cold().expect("loaded LaTeX candidate"));

    for text in [fresh_text, loaded_text] {
        assert!(text.contains("created=D:20260809123423Z"), "{text}");
        assert!(!text.contains("Undefined control sequence"));
    }
}

#[test]
fn semantic_edit_matches_a_fresh_cold_execution() {
    let original = page_source(10);
    let replacement = page_source(24);
    let mut incremental = session(RevisionId::new(1), &original);
    incremental.cold().expect("baseline");
    let accepted = incremental
        .advance(
            RevisionId::new(2),
            edit(&incremental, 0..original.len(), &replacement),
        )
        .expect("edited revision");
    let mut cold = session(RevisionId::new(2), &replacement);
    let expected = cold.cold().expect("fresh comparison");
    assert_detached_output_eq(&accepted, &expected);
    assert_eq!(
        accepted.reuse.execution_path,
        RevisionExecutionPath::SlowEdit
    );
    assert_eq!(accepted.reuse.pages_reused, 0);
}

#[test]
fn no_op_edit_reports_detached_history_convergence() {
    let source = page_source(12);
    let mut session = session(RevisionId::new(1), &source);
    session.cold().expect("baseline");
    let output = session
        .advance(RevisionId::new(2), edit(&session, 0..0, ""))
        .expect("no-op revision");
    assert_eq!(output.reuse.same_history_stop, SameHistoryStop::Matched);
    assert!(output.reuse.convergence_boundary.is_some());
    assert!(output.reuse.same_history_attempts > 0);
}

#[test]
fn semantic_change_does_not_claim_suffix_adoption() {
    let original = page_source(10);
    let mut session = session(RevisionId::new(1), &original);
    session.cold().expect("baseline");
    let output = session
        .advance(
            RevisionId::new(2),
            edit(&session, 0..original.len(), &page_source(11)),
        )
        .expect("changed revision");
    assert_eq!(output.reuse.pages_reused, 0);
    assert_eq!(output.reuse.suffixes_adopted, 0);
    assert_eq!(output.reuse.convergence_boundary, None);
}

#[test]
fn dropping_prepared_revision_keeps_accepted_session_unchanged() {
    let original = page_source(10);
    let mut session = session(RevisionId::new(1), &original);
    let before = session.cold().expect("baseline");
    let transaction = session
        .prepare_revision_with_resolvers(
            RevisionId::new(2),
            edit(&session, 0..original.len(), &page_source(20)),
            &mut DirectResourceHost,
        )
        .expect("prepared revision");
    assert_ne!(
        transaction.pages()[0].artifact(),
        before.pages()[0].artifact()
    );
    drop(transaction);
    assert_eq!(session.revision(), RevisionId::new(1));
    assert_eq!(session.source(), original);
    assert_eq!(session.content_hash(), before.content_hash);
}

#[test]
fn stale_revision_and_hash_rejections_do_not_mutate_state() {
    let source = page_source(10);
    let mut session = session(RevisionId::new(2), &source);
    session.cold().expect("baseline");
    let before = session.content_hash();
    let stale = Edit {
        base_revision: RevisionId::new(1),
        expected_hash: before,
        range: 0..0,
        replacement: String::new(),
    };
    assert!(matches!(
        session.advance(RevisionId::new(3), stale),
        Err(SessionError::StaleRevision { .. })
    ));
    let wrong_hash = Edit {
        base_revision: RevisionId::new(2),
        expected_hash: ContentHash::from_bytes(b"foreign"),
        range: 0..0,
        replacement: String::new(),
    };
    assert!(matches!(
        session.advance(RevisionId::new(3), wrong_hash),
        Err(SessionError::ContentHashMismatch)
    ));
    assert_eq!(session.content_hash(), before);
}

struct DeclineOnceInput(bool);

impl ResourceHost for DeclineOnceInput {
    fn fulfill(&mut self, _world: &mut ResourceWorld<'_>, need: &ResourceNeed) -> ResourceOutcome {
        let ResourceNeed::Input { name, .. } = need else {
            return ResourceOutcome::Unavailable;
        };
        if !self.0 {
            self.0 = true;
            return ResourceOutcome::Declined;
        }
        ResourceOutcome::Fulfilled(ResourceFulfillment::input(
            name,
            RegisteredSourceKind::Generated,
            Arc::from(b"\\relax".as_slice()),
        ))
    }
}

#[test]
fn resource_suspension_replays_from_detached_plan_and_accepts_once() {
    let mut session = session(RevisionId::new(1), "\\input child \\end");
    let mut candidate = session.start_cold_candidate().expect("candidate");
    let mut host = DeclineOnceInput(false);
    assert!(candidate.completion_resource_discovery().is_none());
    assert!(matches!(
        candidate
            .drive_with_resource_resolvers(&mut host, &Cancellation::new())
            .expect("first drive"),
        RevisionCandidateResult::AwaitingResources(ResourceNeed::Input { .. })
    ));
    assert!(candidate.generation.is_some());
    assert!(candidate.runtime_key.is_some());
    assert!(candidate.completion_resource_discovery().is_none());
    assert!(matches!(
        candidate
            .drive_with_resource_resolvers(&mut host, &Cancellation::new())
            .expect("replay"),
        RevisionCandidateResult::Complete
    ));
    assert!(candidate.generation.is_some());
    assert!(candidate.runtime_key.is_none());
    let discovery = candidate
        .completion_resource_discovery()
        .expect("terminal candidate exposes detached completion");
    assert_eq!(discovery.output_id(), session.output_id());
    assert_eq!(discovery.revision(), session.revision());
    assert_eq!(discovery.content_hash(), session.content_hash());
    let output = session
        .accept_cold_candidate(candidate)
        .expect("accept replay");
    assert_eq!(output.revision, RevisionId::new(1));
}

struct CountingResourceHost(usize);

impl ResourceHost for CountingResourceHost {
    fn fulfill(&mut self, _world: &mut ResourceWorld<'_>, _need: &ResourceNeed) -> ResourceOutcome {
        self.0 += 1;
        ResourceOutcome::Unavailable
    }
}

#[test]
fn terminal_pdf_discovery_moves_exact_completion_through_acceptance() {
    let source = r"\font\tenrm=cmr10\relax
        \tenrm\pdfoutput=1
        \pdfmapline{}
        \immediate\pdfobj stream file{payload.bin}
        \shipout\hbox{A}\end";
    let mut session = session(RevisionId::new(7), source);
    session.set_command_profile(CommandProfile::PDFTEX14029, true);
    session
        .register_input_file(Path::new("cmr10.tfm"), CMR10.to_vec())
        .expect("register font");
    let mut candidate = session.start_cold_candidate().expect("candidate");
    assert!(candidate.completion_resource_discovery().is_none());
    let mut host = CountingResourceHost(0);
    assert!(matches!(
        candidate
            .drive_with_resource_resolvers(&mut host, &Cancellation::new())
            .expect("drive"),
        RevisionCandidateResult::Complete
    ));

    let discovery = candidate
        .completion_resource_discovery()
        .expect("terminal resource projection");
    assert_eq!(discovery.output_id(), session.output_id());
    assert_eq!(discovery.revision(), RevisionId::new(7));
    assert_eq!(discovery.content_hash(), session.content_hash());
    assert_eq!(discovery.completion().pages().len(), 1);
    assert_eq!(discovery.pdf_raw_object_file_needs().count(), 1);
    assert!(discovery.pdf_font_operations().next().is_some());
    let font_recipes = discovery
        .pdf_fonts()
        .map(|font| font.recipe.clone())
        .collect::<Vec<_>>();
    let raw_needs = discovery
        .pdf_raw_object_file_needs()
        .cloned()
        .collect::<Vec<_>>();
    let expected_pdf = discovery.pdf().expect("PDF completion").clone();
    let expected_effects = discovery.completion().effects().to_vec();
    let expected_artifacts = discovery
        .completion()
        .pages()
        .iter()
        .map(|page| page.artifact().clone())
        .collect::<Vec<_>>();
    let resource_calls_at_completion = host.0;

    // Resource acquisition and response ownership stay outside tex-incr. The
    // already-complete candidate is accepted without another engine drive.
    let host_responses = raw_needs
        .iter()
        .map(|need| (need.object, b"external payload".to_vec()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(host_responses.len(), 1);
    let transaction = session
        .prepare_revision_candidate(candidate)
        .expect("prepare terminal candidate");
    assert_eq!(transaction.completion().effects(), expected_effects);
    assert_eq!(
        transaction
            .pages()
            .iter()
            .map(DetachedPreparedPage::artifact)
            .collect::<Vec<_>>(),
        expected_artifacts.iter().collect::<Vec<_>>()
    );
    assert_eq!(transaction.completion().pdf(), Some(&expected_pdf));
    let output = session.accept_revision(transaction).expect("atomic accept");
    assert_eq!(output.output_id(), session.output_id());
    assert_eq!(output.completion().effects(), expected_effects);
    assert_eq!(
        artifacts(&output),
        expected_artifacts.iter().collect::<Vec<_>>()
    );
    assert_eq!(output.pdf(), Some(&expected_pdf));
    assert_eq!(
        output.pdf().expect("accepted PDF").raw_object_file_needs(),
        raw_needs
    );
    assert_eq!(
        output
            .pdf()
            .expect("accepted PDF")
            .fonts()
            .iter()
            .map(|font| &font.recipe)
            .collect::<Vec<_>>(),
        font_recipes.iter().collect::<Vec<_>>()
    );
    assert_eq!(
        host.0, resource_calls_at_completion,
        "terminal discovery and acceptance did not rerun resource IO"
    );
}

#[test]
fn stale_transaction_does_not_change_newer_accepted_completion() {
    let original = page_source(10);
    let mut session = session(RevisionId::new(1), &original);
    session.cold().expect("baseline");
    let stale = session
        .prepare_revision_with_resolvers(
            RevisionId::new(2),
            edit(&session, 0..original.len(), &page_source(20)),
            &mut DirectResourceHost,
        )
        .expect("first transaction");
    let accepted = session
        .advance(
            RevisionId::new(2),
            edit(&session, 0..original.len(), &page_source(30)),
        )
        .expect("newer acceptance");
    let accepted_effects = accepted.completion().effects().to_vec();
    let accepted_artifacts = accepted
        .pages()
        .iter()
        .map(|page| page.artifact().clone())
        .collect::<Vec<_>>();
    let accepted_hash = session.content_hash();

    assert!(matches!(
        session.accept_revision(stale),
        Err(SessionError::StaleRevision {
            expected,
            actual
        }) if expected == RevisionId::new(2) && actual == RevisionId::new(1)
    ));
    assert_eq!(session.revision(), RevisionId::new(2));
    assert_eq!(session.content_hash(), accepted_hash);
    assert_eq!(accepted.completion().effects(), accepted_effects);
    assert_eq!(
        accepted
            .pages()
            .iter()
            .map(DetachedPreparedPage::artifact)
            .collect::<Vec<_>>(),
        accepted_artifacts.iter().collect::<Vec<_>>()
    );
}

#[test]
fn incremental_terminal_values_forbid_live_and_parallel_output_owners() {
    fn declaration_fields<'a>(source: &'a str, declaration: &str) -> &'a str {
        let start = source.find(declaration).expect("declaration exists");
        let body = &source[start + declaration.len()..];
        body.split_once("\n}").expect("field block closes").0
    }

    let source = include_str!("lib.rs");
    for declaration in [
        "pub struct AcceptedOutput {",
        "pub struct RevisionTransaction {",
        "struct CandidateCompletion {",
        "pub struct RevisionCandidate {",
        "pub struct Session {",
    ] {
        let fields = declaration_fields(source, declaration);
        for forbidden in [
            "Universe",
            "World",
            "PdfState",
            "RevisionOutputPatch",
            "GenerationSubstrate",
            "effects: Vec",
            "artifacts: Vec",
            "dvi_pages: Vec",
        ] {
            assert!(
                !fields.contains(forbidden),
                "incremental DTO field leaked {forbidden} in {declaration}: {fields}"
            );
        }
    }

    for declaration in [
        "pub struct RevisionTransaction {",
        "pub struct RevisionCandidate {",
    ] {
        let fields = declaration_fields(source, declaration);
        assert!(
            !fields.contains("prior_generation"),
            "current typestate retained the prior generation: {fields}"
        );
    }

    let history_source = include_str!("history.rs");
    let fields = declaration_fields(history_source, "pub struct BoundaryRecord {");
    for forbidden in [
        "RetainedEngineGeneration",
        "RetainedCheckpointKey",
        "GenerationOwner",
        "Universe",
    ] {
        assert!(
            !fields.contains(forbidden),
            "detached history retained {forbidden}: {fields}"
        );
    }
}

#[test]
fn registered_font_resource_survives_each_fresh_generation() {
    let source = "\\font\\tenrm=cmr10\\relax\\tenrm\\shipout\\hbox{A}\\end";
    let mut session = session(RevisionId::new(1), source);
    session
        .register_input_file(Path::new("cmr10.tfm"), CMR10.to_vec())
        .expect("register font");
    let first = session.cold().expect("font run");
    let second = session
        .advance(RevisionId::new(2), edit(&session, 0..0, ""))
        .expect("font rerun");
    assert_eq!(artifacts(&first), artifacts(&second));
}

#[test]
fn history_budget_keeps_job_start_and_newest_observation() {
    let mut source = String::new();
    for width in 1..=8 {
        source.push_str(&format!(
            "\\shipout\\vbox{{\\hrule height1pt width{width}pt}}"
        ));
    }
    source.push_str("\\end");
    let mut session = Session::start((), "budget", RevisionId::new(1), source, 0).expect("session");
    session.cold().expect("cold run");
    assert_eq!(session.history().len(), 2);
    assert_eq!(
        session.history()[0].key().boundary,
        EngineBoundary::JobStart
    );
    assert_eq!(
        session.history()[1].key().boundary,
        EngineBoundary::ShipoutComplete
    );
    assert_eq!(
        session.current_retained_checkpoint_count(),
        session.history().len(),
        "pruning releases every unnamed checkpoint root"
    );
}

#[test]
fn repeated_revisions_match_fresh_cold_output() {
    let mut source = page_source(1);
    let mut incremental = session(RevisionId::new(1), &source);
    incremental.cold().expect("baseline");
    for revision in 2..=12 {
        let next = page_source(revision as usize);
        let output = incremental
            .advance(
                RevisionId::new(revision),
                edit(&incremental, 0..source.len(), &next),
            )
            .expect("accepted edit");
        let mut cold = session(RevisionId::new(revision), &next);
        let expected = cold.cold().expect("cold comparison");
        assert_detached_output_eq(&output, &expected);
        source = next;
    }
    assert_eq!(
        incremental
            .retired_generation_count()
            .saturating_add(incremental.retained_generation_count()),
        12,
        "every accepted revision is either retained whole or retired once"
    );
}

#[test]
fn zero_history_budget_retires_only_complete_old_generations() {
    let mut source = page_source(1);
    let mut incremental =
        Session::start((), "retirement", RevisionId::new(1), &source, 0).expect("session");
    incremental.cold().expect("baseline");
    for revision in 2_u64..=4 {
        let next = page_source(revision as usize);
        incremental
            .advance(
                RevisionId::new(revision),
                edit(&incremental, 0..source.len(), &next),
            )
            .expect("accepted revision");
        source = next;
    }
    assert_eq!(incremental.retained_generation_count(), 1);
    assert_eq!(incremental.retired_generation_count(), 3);
    assert_eq!(
        incremental.retained_revision_ids().collect::<Vec<_>>(),
        vec![RevisionId::new(4)]
    );
}

#[test]
fn rejection_drops_only_current_and_acceptance_drops_whole_prior() {
    let mut incremental = session(RevisionId::new(1), "\\relax\\end");
    incremental.cold().expect("accepted prior");
    let prior = incremental
        .prior_generation
        .as_ref()
        .expect("prior generation")
        .generation
        .witness();

    let mut rejected = incremental
        .start_advance_candidate(RevisionId::new(2), edit(&incremental, 0..0, "\\relax "))
        .expect("candidate");
    drive_synchronous_candidate(&mut rejected, &mut DirectResourceHost).expect("drive candidate");
    let rejected_generation = rejected
        .generation
        .as_ref()
        .expect("current generation")
        .witness();
    assert!(prior.is_live());
    assert!(rejected_generation.is_live());
    assert_eq!(incremental.retained_generation_count(), 1);
    drop(rejected);
    assert!(prior.is_live(), "rejection preserves prior wholesale");
    assert!(
        !rejected_generation.is_live(),
        "rejection drops current wholesale"
    );

    let mut accepted = incremental
        .start_advance_candidate(RevisionId::new(2), edit(&incremental, 0..0, "\\relax "))
        .expect("replacement candidate");
    drive_synchronous_candidate(&mut accepted, &mut DirectResourceHost).expect("drive replacement");
    let current = accepted
        .generation
        .as_ref()
        .expect("current generation")
        .witness();
    let transaction = incremental
        .prepare_revision_candidate(accepted)
        .expect("prepare acceptance");
    incremental
        .accept_revision(transaction)
        .expect("accept current");
    assert!(!prior.is_live(), "acceptance drops the former prior");
    assert!(current.is_live(), "current becomes the sole accepted prior");
    assert_eq!(incremental.retained_generation_count(), 1);
    assert_eq!(incremental.retired_generation_count(), 1);
}

/// Deterministic edit fuzzing is deliberately an explicit tier: it executes a
/// fresh oracle for every accepted revision and is too expensive for the
/// routine workspace suite.
#[test]
#[ignore = "explicit 1,000-edit incremental/cold semantic tier"]
fn thousand_edit_scripted_fuzz_matches_cold_every_revision() {
    let mut source = page_source(1);
    let mut incremental = session(RevisionId::new(1), &source);
    incremental.cold().expect("initial revision");

    // A fixed full-period recurrence makes failures reproducible while still
    // exercising growing and shrinking decimal replacements.
    let mut state = 0x4d59_5df4_d0f3_3173_u64;
    for revision in 2_u64..=1_001 {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let width = usize::try_from((state >> 24) % 10_000 + 1).expect("bounded width");
        let next = page_source(width);
        let accepted = incremental
            .advance(
                RevisionId::new(revision),
                edit(&incremental, 0..source.len(), &next),
            )
            .expect("scripted edit accepts");
        let mut cold = session(RevisionId::new(revision), &next);
        let expected = cold.cold().expect("fresh comparison");
        assert_detached_output_eq(&accepted, &expected);
        source = next;
    }
}

#[test]
fn byte_projection_round_trips_invalid_utf8_source() {
    let bytes = vec![b'\\', b'e', b'n', b'd', 0xff];
    let session = Session::start_with_source_bytes(
        (),
        "bytes",
        "bytes.tex",
        RevisionId::new(1),
        bytes.clone(),
        1024,
    )
    .expect("byte session");
    assert_eq!(session.source_file_bytes(session.source()), bytes);
}

#[test]
fn rendered_query_rejects_foreign_output_before_artifact_lookup() {
    let source = page_source(10);
    let mut first = session(RevisionId::new(1), &source);
    let accepted = first.cold().expect("first");
    let second = session(RevisionId::new(1), &source);
    assert_eq!(
        first
            .rendered_source_location(&accepted, 1, 0, None, second.output_id(), first.revision(),)
            .expect("query"),
        Some(RenderedSourceResult::OutputMismatch {
            accepted: first.output_id()
        })
    );
}
