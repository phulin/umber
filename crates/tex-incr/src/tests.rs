use super::*;

mod long_session;

const CMR10: &[u8] = include_bytes!("../../tex-fonts/tests/fixtures/cm/cmr10.tfm");

fn page_source(width: usize) -> String {
    format!("\\shipout\\vbox{{\\hrule height1pt width{width}pt}}\\end")
}

fn session(revision: RevisionId, source: &str) -> Session<'static> {
    let store = Box::leak(Box::new(new_reachability_store()));
    Session::start(store, "incremental-test", revision, source, usize::MAX).expect("session starts")
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
            .all(|record| record.key().boundary != EngineBoundary::ShipoutComplete),
        "shipout is completion evidence, not restart eligibility"
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
fn too_many_errors_reports_the_fatal_instead_of_terminal_completion_failure() {
    let mut source = "\\nonstopmode\n".to_owned();
    for _ in 0..100 {
        source.push_str("\\badness ");
    }
    source.push_str("\\end");
    let mut session = session(RevisionId::new(1), &source);

    let error = session
        .cold()
        .expect_err("the hundredth recoverable error must reject the candidate");
    let SessionError::Execute(error) = error else {
        panic!("too many errors returned the wrong error family: {error:?}");
    };
    assert_eq!(
        error.as_fatal(),
        Some(tex_command::FatalError::TooManyErrors)
    );
}

#[test]
fn terminal_budget_failure_retains_attempted_fuel_telemetry() {
    let mut session = session(RevisionId::new(1), "\\end");
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
fn exhausted_fuel_is_not_reported_as_an_invalid_terminal_revision() {
    let error = super::map_terminal_completion_error(
        tex_exec::EngineCompletionError::TerminalRevisionUnavailable,
        17,
        17,
    );
    assert!(matches!(
        error,
        SessionError::Execute(tex_exec::ExecError::CumulativeFuelExceeded {
            limit: 17,
            attempted: 18,
        })
    ));
}

#[test]
fn root_framing_alias_is_used_for_startup_while_provenance_keeps_the_source_path() {
    let mut session = Session::start_with_source_path(
        Box::leak(Box::new(new_reachability_store())),
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
    loaded.set_job_clock(clock);
    loaded
        .set_format_image(image)
        .expect("loaded format checkpoint admission");
    let loaded_text = terminal_effect_text(&loaded.cold().expect("loaded LaTeX candidate"));

    for text in [fresh_text, loaded_text] {
        assert!(text.contains("created=D:20260809123423Z"), "{text}");
        assert!(!text.contains("Undefined control sequence"));
    }
}

#[test]
fn loaded_format_checkpoint_survives_rejection_and_seeds_later_revisions() {
    let mut format = session(RevisionId::new(1), r"\def\fmtvalue{41}\dump");
    format.set_utf8_input_as_bytes(true);
    let format = format.cold().expect("format construction");
    let image = DetachedFormatImage::try_from_bytes(
        format
            .format_dump()
            .expect("format dump")
            .image
            .as_bytes()
            .to_vec(),
    )
    .expect("detached format image");
    let image_bytes = image.as_bytes().to_vec();

    let mut loaded = session(RevisionId::new(1), r"\message{first=\fmtvalue}\end");
    loaded.set_utf8_input_as_bytes(true);
    loaded
        .set_format_image(image)
        .expect("initial format checkpoint admission");
    assert_eq!(
        loaded
            .job_start_anchor
            .as_ref()
            .expect("loaded anchor")
            .image
            .as_ref(),
        image_bytes,
        "format admission retains the validated bytes exactly"
    );
    assert!(loaded.prior_generation.is_none());
    assert_eq!(loaded.retained_generation_count(), 0);
    assert_eq!(loaded.current_retained_checkpoint_count(), 0);
    let anchor = loaded
        .job_start_anchor_metrics()
        .expect("loaded format owns an immutable anchor");
    assert!(anchor.bytes > 0);
    assert_eq!(anchor.image_bytes, image_bytes.len());
    assert_eq!(anchor.session_metadata_bytes, 0);
    assert_eq!(anchor.restore_count, 0);

    let rejected = loaded.start_cold_candidate().expect("first candidate");
    assert!(
        rejected.generation.is_none(),
        "cold materialization is lazy"
    );
    assert_eq!(loaded.retained_generation_count(), 0);
    assert_eq!(
        loaded
            .job_start_anchor_metrics()
            .expect("anchor metrics")
            .restore_count,
        0
    );
    drop(rejected);
    assert_eq!(loaded.retained_generation_count(), 0);

    let first = loaded.cold().expect("replacement first candidate");
    assert!(terminal_effect_text(&first).contains("first=41"));
    assert_eq!(
        loaded
            .job_start_anchor_metrics()
            .expect("anchor metrics")
            .restore_count,
        1
    );
    assert_eq!(
        loaded
            .job_start_anchor_metrics()
            .expect("anchor metrics")
            .session_metadata_bytes,
        size_of::<JobStartSessionMetadata>()
    );
    let first_generation = loaded
        .prior_generation
        .as_ref()
        .expect("first document is the accepted prior")
        .generation
        .witness();

    let second = loaded
        .advance(
            RevisionId::new(2),
            edit(&loaded, 0..0, r"\message{second=\fmtvalue}"),
        )
        .expect("later candidate forks the accepted JobStart checkpoint");
    let terminal = terminal_effect_text(&second);
    assert!(terminal.contains("second=41"), "{terminal}");
    assert!(terminal.contains("first=41"), "{terminal}");
    assert!(!first_generation.is_live());
    assert_eq!(loaded.retained_generation_count(), 1);
    assert_eq!(loaded.occupied_generation_slot_count(), 1);
}

#[test]
fn loaded_format_node_rows_publish_with_job_start_identity_and_survive_the_session() {
    let mut format = session(RevisionId::new(1), r"\setbox0=\hbox{\penalty123}\dump");
    format.set_utf8_input_as_bytes(true);
    let format = format.cold().expect("node-bearing format construction");
    let image = DetachedFormatImage::try_from_bytes(
        format
            .format_dump()
            .expect("node-bearing format dump")
            .image
            .as_bytes()
            .to_vec(),
    )
    .expect("detached node-bearing format image");

    let source = r"\setbox1=\copy0 \message{first}\end";
    let mut loaded = session(RevisionId::new(1), source);
    loaded.set_utf8_input_as_bytes(true);
    loaded
        .set_format_image(image)
        .expect("node-bearing format checkpoint admission");
    let first = loaded.cold().expect("first loaded-format document");
    assert!(terminal_effect_text(&first).contains("first"));
    assert_eq!(loaded.history()[0].key().boundary, EngineBoundary::JobStart);

    let insertion = source.find("\\end").expect("end command");
    let second = loaded
        .advance(
            RevisionId::new(2),
            edit(
                &loaded,
                insertion..insertion,
                r"\setbox2=\copy0 \message{second}",
            ),
        )
        .expect("later revision reuses the retained loaded-format JobStart");
    let terminal = terminal_effect_text(&second);
    assert!(terminal.contains("first"), "{terminal}");
    assert!(terminal.contains("second"), "{terminal}");
    assert_eq!(
        second.reuse.restart_boundary,
        Some(BoundaryKey {
            position: 0,
            boundary: EngineBoundary::JobStart,
            ordinal: 0,
        })
    );
}

#[test]
fn loaded_job_start_round_trip_preserves_dense_hyphenation_and_pdf_identity() {
    let mut format = session(
        RevisionId::new(1),
        r"\count23=314
          \toks4={anchor}
          \skip5=2pt plus 1fil
          \def\fmtvalue{41}
          \patterns{a1b}
          \hyphenation{hy-phen}
          \pdfoutput=1
          \immediate\pdfobj{format-object}
          \dump",
    );
    format.set_command_profile(CommandProfile::PDFTEX14029, true);
    format.set_utf8_input_as_bytes(true);
    let format = format.cold().expect("PDF format construction");
    let image = DetachedFormatImage::try_from_bytes(
        format
            .format_dump()
            .expect("PDF format dump")
            .image
            .as_bytes()
            .to_vec(),
    )
    .expect("detached PDF format image");

    let mut loaded = session(
        RevisionId::new(1),
        r"\message{value=\fmtvalue,count=\the\count23,toks=\the\toks4}\shipout\vbox{}\end",
    );
    loaded.set_command_profile(CommandProfile::PDFTEX14029, false);
    loaded.set_utf8_input_as_bytes(true);
    loaded
        .set_format_image(image)
        .expect("install frozen anchor");
    let first = loaded.cold().expect("first loaded-format job");
    assert!(terminal_effect_text(&first).contains("value=41,count=314,toks=anchor"));
    let first_job_start = loaded.history()[0]
        .reachable_state_identity()
        .expect("loaded dense/hyphenation/PDF owners publish a complete identity");

    let mut fallback = loaded
        .start_advance_candidate_from_job_start(RevisionId::new(2), edit(&loaded, 0..0, ""))
        .expect("forced JobStart fallback");
    drive_synchronous_candidate(&mut fallback, &mut DirectResourceHost)
        .expect("drive frozen fallback");
    let transaction = loaded
        .prepare_revision_candidate(fallback)
        .expect("prepare fallback acceptance");
    let accepted = loaded
        .accept_revision(transaction)
        .expect("accept frozen fallback");

    assert_detached_output_eq(&accepted, &first);
    assert_eq!(
        accepted.reuse.restart_boundary,
        Some(BoundaryKey {
            position: 0,
            boundary: EngineBoundary::JobStart,
            ordinal: 0,
        })
    );
    assert_eq!(
        loaded.history()[0].reachable_state_identity(),
        Some(first_job_start),
        "byte-identical anchor materialization reproduces every aggregate owner root"
    );
    assert_eq!(
        loaded
            .job_start_anchor_metrics()
            .expect("anchor metrics")
            .restore_count,
        2
    );
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
fn complete_identity_matches_no_op_convergence() {
    let source = page_source(12);
    let mut session = session(RevisionId::new(1), &source);
    session.cold().expect("baseline");
    let output = session
        .advance(RevisionId::new(2), edit(&session, 0..0, ""))
        .expect("no-op revision");
    assert_eq!(output.reuse.same_history_stop, SameHistoryStop::Matched);
    assert!(output.reuse.convergence_boundary.is_some());
    assert!(output.reuse.same_history_attempts > 0);
    assert!(
        session
            .history()
            .iter()
            .all(|record| record.reachable_state_identity().is_some()),
        "incremental history demands a complete owner-composed identity"
    );
}

#[test]
fn root_file_checkpoint_filter_keeps_history_and_convergence_deterministic() {
    let source = r"\font\tenrm=cmr10 \tenrm
\begingroup A\par\endgroup
\input child
\finish
\shipout\vbox{}
\end";
    let child = br"\def\finish{B\par}C\par\shipout\vbox{}\endinput";
    let mut session = session(RevisionId::new(1), source);
    session
        .register_input_file(Path::new("cmr10.tfm"), CMR10.to_vec())
        .expect("font registers");
    session
        .register_input_file(Path::new("child.tex"), child.to_vec())
        .expect("nested input registers");
    session.cold().expect("baseline");

    let before = session
        .history()
        .iter()
        .map(BoundaryRecord::key)
        .collect::<Vec<_>>();
    assert_eq!(
        before.iter().map(|key| key.boundary).collect::<Vec<_>>(),
        [EngineBoundary::JobStart, EngineBoundary::OuterParagraphEnd,],
        "only the root-main-file, group-zero outer paragraph is restart eligible"
    );

    let output = session
        .advance(RevisionId::new(2), edit(&session, 0..0, ""))
        .expect("no-op revision");
    assert_eq!(output.reuse.same_history_stop, SameHistoryStop::Matched);
    assert!(output.reuse.convergence_boundary.is_some());
    assert_eq!(
        session
            .history()
            .iter()
            .map(BoundaryRecord::key)
            .collect::<Vec<_>>(),
        before,
        "filtered nested occurrences do not perturb accepted schedule keys"
    );
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
    assert!(
        candidate.runtime_key.is_some(),
        "terminal command/mode owners remain attached until aggregate settlement"
    );
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

#[test]
fn non_job_start_command_fork_cancels_suspension_and_reuses_accepted_siblings() {
    let source = "A\\par\n\\font\\tenrm=cmr10 \\tenrm B\\par\nC\\par\\end";
    let mut incremental = session(RevisionId::new(1), source);
    incremental
        .register_input_file(Path::new("cmr10.tfm"), CMR10.to_vec())
        .expect("accepted font registers");
    incremental.cold().expect("accepted prior");
    let edit_position = source.find("\\font").expect("font follows first paragraph");
    let candidate_edit = edit(&incremental, edit_position..edit_position, "\\relax ");

    let mut suspended = incremental
        .start_advance_candidate(RevisionId::new(2), candidate_edit.clone())
        .expect("non-JobStart font candidate");
    assert!(matches!(
        suspended
            .drive_with_resource_resolvers(&mut DeclineResource, &Cancellation::new())
            .expect("candidate suspends"),
        RevisionCandidateResult::AwaitingResources(ResourceNeed::Font { .. })
    ));
    suspended.reject();

    let mut accepted = incremental
        .start_advance_candidate(RevisionId::new(2), candidate_edit)
        .expect("accepted sibling survives suspended rejection");
    drive_synchronous_candidate(&mut accepted, &mut DirectResourceHost)
        .expect("registered resource drives replacement candidate");
    let accepted = incremental
        .prepare_revision_candidate(accepted)
        .expect("resumed candidate prepares");
    assert_eq!(
        accepted
            .reuse()
            .restart_boundary
            .expect("resource candidate retained restart")
            .boundary,
        EngineBoundary::OuterParagraphEnd
    );
    let output = incremental
        .accept_revision(accepted)
        .expect("resumed candidate accepts");

    let edited = format!(
        "{}\\relax {}",
        &source[..edit_position],
        &source[edit_position..]
    );
    let mut cold = session(RevisionId::new(2), &edited);
    cold.register_input_file(Path::new("cmr10.tfm"), CMR10.to_vec())
        .expect("cold font registers");
    let expected = cold.cold().expect("resource cold comparison");
    assert_detached_output_eq(&output, &expected);
}

struct CountingResourceHost(usize);

impl ResourceHost for CountingResourceHost {
    fn fulfill(&mut self, _world: &mut ResourceWorld<'_>, _need: &ResourceNeed) -> ResourceOutcome {
        self.0 += 1;
        ResourceOutcome::Unavailable
    }
}

struct DeclineResource;

impl ResourceHost for DeclineResource {
    fn fulfill(&mut self, _world: &mut ResourceWorld<'_>, _need: &ResourceNeed) -> ResourceOutcome {
        ResourceOutcome::Declined
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
fn prepared_transaction_blocks_newer_candidate_until_rejected() {
    let original = page_source(10);
    let mut session = session(RevisionId::new(1), &original);
    session.cold().expect("baseline");
    let rejected = session
        .prepare_revision_with_resolvers(
            RevisionId::new(2),
            edit(&session, 0..original.len(), &page_source(20)),
            &mut DirectResourceHost,
        )
        .expect("first transaction");
    assert_eq!(session.occupied_generation_slot_count(), 2);
    assert!(matches!(
        session.advance(
            RevisionId::new(2),
            edit(&session, 0..original.len(), &page_source(30)),
        ),
        Err(SessionError::CandidateAlreadyLive)
    ));
    rejected.reject();
    assert_eq!(session.occupied_generation_slot_count(), 1);

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
        let start = source
            .find(declaration)
            .unwrap_or_else(|| panic!("declaration exists: {declaration}"));
        let body = &source[start + declaration.len()..];
        body.split_once("\n}").expect("field block closes").0
    }

    let source = include_str!("lib.rs");
    for declaration in [
        "pub struct AcceptedOutput {",
        "pub struct RevisionTransaction<'store> {",
        "struct CandidateCompletion {",
        "pub struct RevisionCandidate<'store> {",
        "pub struct Session<'store> {",
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
        "pub struct RevisionTransaction<'store> {",
        "pub struct RevisionCandidate<'store> {",
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
    for _ in 1..=8 {
        source.push_str("\\indent\\par");
    }
    source.push_str("\\end");
    let unbounded_source = source.clone();
    let mut session = Session::start(
        Box::leak(Box::new(new_reachability_store())),
        "budget",
        RevisionId::new(1),
        source,
        0,
    )
    .expect("session");
    session.cold().expect("cold run");
    assert_eq!(session.history().len(), 2);
    assert_eq!(
        session.history()[0].key().boundary,
        EngineBoundary::JobStart
    );
    assert_eq!(
        session.history()[1].key().boundary,
        EngineBoundary::OuterParagraphEnd
    );
    assert_eq!(
        session.current_retained_checkpoint_count(),
        0,
        "JobStart is frozen session-owned data; newest evidence is detached"
    );
    let bounded = session.retention_metrics().expect("bounded retention");
    assert!(
        bounded.job_start_anchor_bytes > 0
            && bounded.checkpoint_root_bytes >= size_of::<BoundaryRecord>() * 2
    );

    let mut unbounded = Session::start(
        Box::leak(Box::new(new_reachability_store())),
        "unbounded-owner-reference",
        RevisionId::new(1),
        unbounded_source,
        usize::MAX,
    )
    .expect("unbounded session");
    unbounded.cold().expect("unbounded cold run");
    assert!(
        bounded.checkpoint_shared_owner_bytes
            < unbounded
                .retention_metrics()
                .expect("unbounded retention")
                .checkpoint_shared_owner_bytes,
        "releasing distinct checkpoint banks must release their owner charges"
    );
}

#[test]
fn retention_charges_one_shared_owner_and_distinguishes_detached_evidence() {
    let mut source = String::new();
    for _ in 1..=8 {
        source.push_str("\\indent\\par");
    }
    source.push_str("\\end");
    let mut session = Session::start(
        Box::leak(Box::new(new_reachability_store())),
        "retention-dedup",
        RevisionId::new(1),
        source,
        usize::MAX,
    )
    .expect("session");
    session.cold().expect("cold run");
    let roots = session.current_retained_checkpoint_count();
    assert!(roots > 4, "fixture must retain several restart roots");
    let retention = session.retention_metrics().expect("accepted retention");
    assert_eq!(
        retention.checkpoint_root_bytes,
        retention
            .checkpoint_shared_owner_bytes
            .saturating_add(retention.checkpoint_metadata_bytes)
            .saturating_add(retention.detached_boundary_bytes)
    );
    assert!(retention.checkpoint_shared_owner_bytes > 0);
    assert!(retention.detached_boundary_bytes > 0);
    assert_eq!(retention.checkpoint_metadata_bytes % roots, 0);
    assert_ne!(
        retention.checkpoint_root_bytes,
        retention
            .checkpoint_shared_owner_bytes
            .saturating_mul(roots)
            .saturating_add(retention.checkpoint_metadata_bytes)
            .saturating_add(retention.detached_boundary_bytes),
        "the coarse generation owner must not be charged once per restart root"
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
fn late_edit_restarts_from_a_retained_non_job_start_boundary() {
    let source = "A\\par\nB\\par\nC\\par\\end";
    let mut incremental = session(RevisionId::new(1), source);
    incremental.cold().expect("baseline");
    let edit_position = source.find('C').expect("third paragraph exists");
    let output = incremental
        .advance(
            RevisionId::new(2),
            edit(&incremental, edit_position..edit_position, "\\relax "),
        )
        .expect("late edit");
    let restart = output
        .reuse
        .restart_boundary
        .expect("accepted history supplies a restart boundary");
    assert_ne!(restart.boundary, EngineBoundary::JobStart);

    let edited = format!(
        "{}\\relax {}",
        &source[..edit_position],
        &source[edit_position..]
    );
    let mut cold = session(RevisionId::new(2), &edited);
    let expected = cold.cold().expect("cold comparison");
    assert_detached_output_eq(&output, &expected);
}

#[test]
fn non_job_start_mode_candidate_reject_accept_and_sibling_reuse_are_explicit() {
    let source = "A\\par\nB\\par\nC\\par\\end";
    let mut incremental = session(RevisionId::new(1), source);
    incremental.cold().expect("baseline");
    let edit_position = source.find('C').expect("third paragraph exists");
    let candidate_edit = edit(&incremental, edit_position..edit_position, "\\relax ");

    let mut rejected = incremental
        .start_advance_candidate(RevisionId::new(2), candidate_edit.clone())
        .expect("non-JobStart rejection candidate");
    drive_synchronous_candidate(&mut rejected, &mut DirectResourceHost)
        .expect("drive rejection candidate");
    let rejected = incremental
        .prepare_revision_candidate(rejected)
        .expect("prepare explicit rejection");
    let selected = rejected
        .reuse()
        .restart_boundary
        .expect("candidate selects a retained boundary");
    assert_eq!(selected.boundary, EngineBoundary::OuterParagraphEnd);
    rejected.reject();

    let mut accepted = incremental
        .start_advance_candidate(RevisionId::new(2), candidate_edit)
        .expect("same sibling mark remains seedable after rejection");
    drive_synchronous_candidate(&mut accepted, &mut DirectResourceHost)
        .expect("drive accepted candidate");
    let accepted = incremental
        .prepare_revision_candidate(accepted)
        .expect("prepare explicit acceptance");
    assert_eq!(accepted.reuse().restart_boundary, Some(selected));
    let output = incremental
        .accept_revision(accepted)
        .expect("accept candidate exactly once");

    let edited = format!(
        "{}\\relax {}",
        &source[..edit_position],
        &source[edit_position..]
    );
    let mut cold = session(RevisionId::new(2), &edited);
    let expected = cold.cold().expect("cold comparison");
    assert_detached_output_eq(&output, &expected);

    let next_position = incremental
        .source()
        .find('C')
        .expect("accepted third paragraph remains");
    let mut sibling = incremental
        .start_advance_candidate(
            RevisionId::new(3),
            edit(&incremental, next_position..next_position, "\\relax "),
        )
        .expect("post-accept sibling candidate");
    drive_synchronous_candidate(&mut sibling, &mut DirectResourceHost)
        .expect("drive post-accept sibling");
    let sibling = incremental
        .prepare_revision_candidate(sibling)
        .expect("prepare post-accept sibling rejection");
    assert_eq!(
        sibling
            .reuse()
            .restart_boundary
            .expect("post-accept restart")
            .boundary,
        EngineBoundary::OuterParagraphEnd
    );
    sibling.reject();
}

#[test]
fn far_command_checkpoint_settles_exact_deltas_and_preserves_production_siblings() {
    let source = concat!(
        "A\\par\n",
        "\\def\\emit#1{[#1]}\n",
        "\\iftrue\\count0=17\\else\\count0=99\\fi\n",
        "\\halign{#\\cr X\\cr Y\\cr}\\par\n",
        "\\emit{B}\\par\n",
        "C\\par\n",
        "D\\par\\end",
    );
    let mut incremental = session(RevisionId::new(1), source);
    incremental.cold().expect("accepted command-rich prior");
    let edit_position = source
        .find("\\def")
        .expect("suffix follows first paragraph");
    let candidate_edit = edit(&incremental, edit_position..edit_position, "\\relax ");
    let before_reject = incremental
        .command_timeline_counters()
        .expect("accepted command counters")
        .expect("accepted generation");

    let mut rejected = incremental
        .start_advance_candidate(RevisionId::new(2), candidate_edit.clone())
        .expect("far non-JobStart rejection candidate");
    drive_synchronous_candidate(&mut rejected, &mut DirectResourceHost)
        .expect("drive command-rich rejection candidate");
    let mut rejected = incremental
        .prepare_revision_candidate(rejected)
        .expect("prepare command-rich rejection");
    assert_eq!(
        rejected
            .reuse()
            .restart_boundary
            .expect("retained command restart")
            .boundary,
        EngineBoundary::OuterParagraphEnd
    );
    let before_settle = rejected
        .command_timeline_counters()
        .expect("candidate command counters");
    let selected_delta = before_settle
        .selected_rewind_records
        .saturating_sub(before_reject.selected_rewind_records);
    assert!(
        selected_delta != 0,
        "the far accepted command delta rewinds"
    );
    rejected.reject();

    let after_reject = incremental
        .command_timeline_counters()
        .expect("returned command counters")
        .expect("returned accepted generation");
    assert_eq!(
        after_reject.accepted_redo_records - before_reject.accepted_redo_records,
        selected_delta,
        "rejection redoes exactly the selected accepted command delta"
    );
    assert!(
        after_reject.candidate_reject_records > before_reject.candidate_reject_records,
        "rejection undoes a private candidate command suffix"
    );
    assert!(
        after_reject.candidate_chunks_released > before_reject.candidate_chunks_released,
        "rejection returns candidate command chunks"
    );
    assert_eq!(after_reject.frame_index_searches, 0);
    assert_eq!(after_reject.frame_keys_copied, 0);

    let before_accept = after_reject;
    let mut accepted = incremental
        .start_advance_candidate(RevisionId::new(2), candidate_edit)
        .expect("rejected command sibling remains seedable");
    drive_synchronous_candidate(&mut accepted, &mut DirectResourceHost)
        .expect("drive accepted command-rich candidate");
    let accepted = incremental
        .prepare_revision_candidate(accepted)
        .expect("prepare command-rich acceptance");
    let output = incremental
        .accept_revision(accepted)
        .expect("accept command-rich candidate");
    let after_accept = incremental
        .command_timeline_counters()
        .expect("accepted command counters")
        .expect("accepted generation");
    assert!(
        after_accept.accepted_chunks_released > before_accept.accepted_chunks_released,
        "acceptance releases obsolete accepted chunks"
    );
    assert_eq!(
        after_accept.candidate_reject_records, before_accept.candidate_reject_records,
        "acceptance does not undo candidate history"
    );
    assert_eq!(
        after_accept.accepted_redo_records, before_accept.accepted_redo_records,
        "acceptance does not replay accepted history"
    );
    assert_eq!(after_accept.frame_index_searches, 0);
    assert_eq!(after_accept.frame_keys_copied, 0);

    let edited = format!(
        "{}\\relax {}",
        &source[..edit_position],
        &source[edit_position..]
    );
    let mut cold = session(RevisionId::new(2), &edited);
    let expected = cold.cold().expect("command-rich cold comparison");
    assert_detached_output_eq(&output, &expected);

    let next_position = incremental
        .source()
        .find('D')
        .expect("accepted fourth paragraph remains");
    let mut sibling = incremental
        .start_advance_candidate(
            RevisionId::new(3),
            edit(&incremental, next_position..next_position, "\\relax "),
        )
        .expect("post-accept command sibling candidate");
    drive_synchronous_candidate(&mut sibling, &mut DirectResourceHost)
        .expect("drive post-accept command sibling");
    let sibling = incremental
        .prepare_revision_candidate(sibling)
        .expect("prepare post-accept command sibling");
    assert_eq!(
        sibling
            .reuse()
            .restart_boundary
            .expect("post-accept command restart")
            .boundary,
        EngineBoundary::OuterParagraphEnd
    );
    sibling.reject();
}

#[test]
fn non_job_start_page_owner_settles_insertions_marks_and_output_closure() {
    let source = concat!(
        "A\\par\n",
        "\\relax\n",
        "\\hsize=40pt\\vsize=12pt\\maxdepth=0pt",
        "\\count0=1000\\dimen0=5pt\\skip0=0pt",
        "\\output={\\shipout\\box255}",
        "B\\mark{top}\\insert0{\\hrule height10pt}\\par",
        "\\vskip12pt\\penalty-10000",
        "C\\marks7{class-seven}\\par",
        "D\\par\\end",
    );
    let mut incremental = session(RevisionId::new(1), source);
    incremental.cold().expect("accepted page-rich prior");
    let edit_position = source
        .find("\\hsize")
        .expect("page-rich suffix follows first paragraph");
    let candidate_edit = edit(&incremental, edit_position..edit_position, "\\relax ");

    let mut rejected = incremental
        .start_advance_candidate(RevisionId::new(2), candidate_edit.clone())
        .expect("page-rich non-JobStart rejection candidate");
    drive_synchronous_candidate(&mut rejected, &mut DirectResourceHost)
        .expect("drive page-rich rejection candidate");
    let mut rejected = incremental
        .prepare_revision_candidate(rejected)
        .expect("prepare page-rich rejection");
    assert_eq!(
        rejected
            .reuse()
            .restart_boundary
            .expect("page-rich retained restart")
            .boundary,
        EngineBoundary::OuterParagraphEnd
    );
    let before_reject = rejected
        .page_candidate_settlement_counters()
        .expect("candidate settlement counters");
    rejected.reject();
    let after_reject = incremental
        .page_candidate_settlement_counters()
        .expect("accepted settlement counters")
        .expect("accepted generation");
    assert_eq!(
        after_reject.candidate_rejections,
        before_reject.candidate_rejections + 1
    );
    assert_eq!(after_reject.canonical_lane_records_scanned, 0);
    assert_eq!(after_reject.canonical_values_copied, 0);

    let mut accepted = incremental
        .start_advance_candidate(RevisionId::new(2), candidate_edit)
        .expect("rejected sibling page mark remains seedable");
    drive_synchronous_candidate(&mut accepted, &mut DirectResourceHost)
        .expect("drive page-rich accepted candidate");
    let mut accepted = incremental
        .prepare_revision_candidate(accepted)
        .expect("prepare page-rich acceptance");
    let before_accept = accepted
        .page_candidate_settlement_counters()
        .expect("candidate settlement counters");
    let output = incremental
        .accept_revision(accepted)
        .expect("accept page-rich candidate");
    let after_accept = incremental
        .page_candidate_settlement_counters()
        .expect("accepted settlement counters")
        .expect("accepted generation");
    assert_eq!(
        after_accept.candidate_acceptances,
        before_accept.candidate_acceptances + 1
    );
    assert_eq!(after_accept.acceptance_payload_records_scanned, 0);
    assert_eq!(after_accept.canonical_lane_records_scanned, 0);
    assert_eq!(after_accept.canonical_values_copied, 0);
    let region_counters = incremental
        .page_region_counters()
        .expect("accepted page-region counters")
        .expect("accepted generation");
    assert!(
        region_counters.held_over_nodes_copied != 0
            || region_counters.held_over_envelopes_moved != 0,
        "the page-rich production candidate settles a held-over closure"
    );

    let edited = format!(
        "{}\\relax {}",
        &source[..edit_position],
        &source[edit_position..]
    );
    let mut cold = session(RevisionId::new(2), &edited);
    let expected = cold.cold().expect("page-rich cold comparison");
    assert_detached_output_eq(&output, &expected);

    let next_position = incremental
        .source()
        .find("\\hsize")
        .expect("accepted page-rich suffix remains");
    let mut sibling = incremental
        .start_advance_candidate(
            RevisionId::new(3),
            edit(&incremental, next_position..next_position, "\\relax "),
        )
        .expect("post-accept page-rich sibling candidate");
    drive_synchronous_candidate(&mut sibling, &mut DirectResourceHost)
        .expect("drive post-accept page-rich sibling");
    let sibling = incremental
        .prepare_revision_candidate(sibling)
        .expect("prepare post-accept page-rich sibling rejection");
    assert_eq!(
        sibling
            .reuse()
            .restart_boundary
            .expect("post-accept page-rich restart")
            .boundary,
        EngineBoundary::OuterParagraphEnd
    );
    sibling.reject();
}

#[test]
fn non_job_start_page_owner_accepts_the_default_output_path() {
    let source = concat!(
        "A\\par\n",
        "\\relax\n",
        "\\vsize=8pt\\maxdepth=0pt",
        "B\\par\\vskip8pt\\penalty-10000",
        "C\\par\\end",
    );
    let mut incremental = session(RevisionId::new(1), source);
    incremental.cold().expect("accepted default-output prior");
    let edit_position = source.find("\\vsize").expect("default-output suffix");
    let mut candidate = incremental
        .start_advance_candidate(
            RevisionId::new(2),
            edit(&incremental, edit_position..edit_position, "\\relax "),
        )
        .expect("default-output non-JobStart candidate");
    drive_synchronous_candidate(&mut candidate, &mut DirectResourceHost)
        .expect("drive default-output candidate");
    let candidate = incremental
        .prepare_revision_candidate(candidate)
        .expect("prepare default-output candidate");
    assert_eq!(
        candidate
            .reuse()
            .restart_boundary
            .expect("default-output retained restart")
            .boundary,
        EngineBoundary::OuterParagraphEnd
    );
    let output = incremental
        .accept_revision(candidate)
        .expect("accept default-output candidate");

    let edited = format!(
        "{}\\relax {}",
        &source[..edit_position],
        &source[edit_position..]
    );
    let mut cold = session(RevisionId::new(2), &edited);
    let expected = cold.cold().expect("default-output cold comparison");
    assert_detached_output_eq(&output, &expected);
    let counters = incremental
        .page_candidate_settlement_counters()
        .expect("accepted settlement counters")
        .expect("accepted generation");
    assert_eq!(counters.acceptance_payload_records_scanned, 0);
    assert_eq!(counters.canonical_lane_records_scanned, 0);
    assert_eq!(counters.canonical_values_copied, 0);
}

#[test]
fn caught_candidate_run_panic_returns_every_owner_and_keeps_prior_reusable() {
    let source = "A\\par\nB\\par\nC\\par\\end";
    let mut incremental = session(RevisionId::new(1), source);
    incremental.cold().expect("accepted prior");
    let prior = incremental
        .prior_generation
        .as_ref()
        .expect("prior generation")
        .generation
        .witness();
    let prior_checkpoints = incremental.current_retained_checkpoint_count();
    let edit_position = source.find('C').expect("third paragraph exists");
    let mut panicking = incremental
        .start_advance_candidate(
            RevisionId::new(2),
            edit(&incremental, edit_position..edit_position, "\\relax "),
        )
        .expect("rooted candidate");
    let current = panicking
        .generation
        .as_ref()
        .expect("rooted candidate owns current generation")
        .witness();

    super::arm_candidate_owner_unwind_for_test();
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        panicking.drive_with_resource_resolvers(&mut DirectResourceHost, &Cancellation::new())
    }));
    assert!(
        panic.is_err(),
        "host panic reaches the caller, got {panic:?}"
    );
    assert!(
        prior.is_live(),
        "aggregate unwind preserves the prior owner"
    );
    assert!(
        !current.is_live(),
        "aggregate unwind rejects the complete current owner exactly once"
    );
    assert_eq!(incremental.reachability_store.live_generation_count(), 1);
    assert_eq!(
        incremental.current_retained_checkpoint_count(),
        prior_checkpoints,
        "command, boundary, ledger, mode, state, page, and PDF roots returned to the prior lane"
    );
    drop(panicking);
    assert_eq!(incremental.current_candidate_generation_count(), 0);

    let mut sibling = incremental
        .start_advance_candidate(
            RevisionId::new(2),
            edit(&incremental, edit_position..edit_position, "\\relax "),
        )
        .expect("returned prior owners seed a sibling candidate");
    drive_synchronous_candidate(&mut sibling, &mut DirectResourceHost)
        .expect("sibling drive uses the returned command/state owners");
    let sibling = incremental
        .prepare_revision_candidate(sibling)
        .expect("sibling prepares after unwind");
    assert_eq!(
        sibling
            .reuse()
            .restart_boundary
            .expect("sibling reuses the prior boundary")
            .boundary,
        EngineBoundary::OuterParagraphEnd
    );
    let output = incremental
        .accept_revision(sibling)
        .expect("sibling acceptance consumes every returned owner");
    let edited = format!(
        "{}\\relax {}",
        &source[..edit_position],
        &source[edit_position..]
    );
    let mut cold = session(RevisionId::new(2), &edited);
    let expected = cold.cold().expect("cold comparison");
    assert_detached_output_eq(&output, &expected);
}

#[test]
fn zero_history_budget_retires_only_complete_old_generations() {
    let mut source = page_source(1);
    let mut incremental = Session::start(
        Box::leak(Box::new(new_reachability_store())),
        "retirement",
        RevisionId::new(1),
        &source,
        0,
    )
    .expect("session");
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
    assert!(
        incremental
            .prior_generation
            .as_ref()
            .expect("prior generation")
            .generation
            .same_store(rejected.generation.as_ref().expect("current generation")),
        "prior and current are leases in one external store"
    );
    assert!(prior.is_live());
    assert!(rejected_generation.is_live());
    assert_eq!(incremental.retained_generation_count(), 1);
    assert_eq!(incremental.current_candidate_generation_count(), 1);
    assert_eq!(incremental.occupied_generation_slot_count(), 2);
    assert_eq!(incremental.reachability_store.live_generation_count(), 2);
    assert!(matches!(
        incremental.start_cold_candidate(),
        Err(SessionError::CandidateAlreadyLive)
    ));
    drop(rejected);
    assert!(prior.is_live(), "rejection preserves prior wholesale");
    assert!(
        !rejected_generation.is_live(),
        "rejection drops current wholesale"
    );
    assert_eq!(incremental.current_candidate_generation_count(), 0);
    assert_eq!(incremental.occupied_generation_slot_count(), 1);
    assert_eq!(incremental.reachability_store.live_generation_count(), 1);

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
    assert_eq!(incremental.current_candidate_generation_count(), 0);
    assert_eq!(incremental.occupied_generation_slot_count(), 1);
    assert_eq!(incremental.reachability_store.live_generation_count(), 1);
    assert_eq!(incremental.retired_generation_count(), 1);
}

#[test]
fn explicit_candidate_rejection_releases_generation_and_lease() {
    let mut incremental = session(RevisionId::new(1), "\\relax\\end");
    incremental.cold().expect("accepted prior");
    let mut candidate = incremental
        .start_advance_candidate(RevisionId::new(2), edit(&incremental, 0..0, "\\relax "))
        .expect("candidate");
    drive_synchronous_candidate(&mut candidate, &mut DirectResourceHost).expect("drive candidate");
    let current = candidate
        .generation
        .as_ref()
        .expect("current generation")
        .witness();

    candidate.reject();

    assert!(!current.is_live());
    assert_eq!(incremental.occupied_generation_slot_count(), 1);
    incremental
        .start_advance_candidate(RevisionId::new(2), edit(&incremental, 0..0, "\\relax "))
        .expect("lease is immediately reusable")
        .reject();
}

#[test]
fn repeated_candidate_drop_keeps_generation_high_water_at_prior_plus_current() {
    let mut incremental = session(RevisionId::new(1), "\\relax\\end");
    incremental.cold().expect("accepted prior");
    let prior = incremental
        .prior_generation
        .as_ref()
        .expect("prior generation")
        .generation
        .witness();
    let mut high_water = incremental.occupied_generation_slot_count();

    for _ in 0..64 {
        let mut candidate = incremental.start_cold_candidate().expect("candidate");
        drive_synchronous_candidate(&mut candidate, &mut DirectResourceHost)
            .expect("drive candidate");
        let current = candidate
            .generation
            .as_ref()
            .expect("current generation")
            .witness();
        high_water = high_water.max(incremental.occupied_generation_slot_count());
        assert_eq!(incremental.occupied_generation_slot_count(), 2);
        drop(candidate);
        assert!(!current.is_live());
        assert!(prior.is_live());
        assert_eq!(incremental.occupied_generation_slot_count(), 1);
    }

    assert_eq!(high_water, 2);
    assert_eq!(incremental.retired_generation_count(), 0);
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
        Box::leak(Box::new(new_reachability_store())),
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
