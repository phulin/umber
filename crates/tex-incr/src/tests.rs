use super::*;

fn all_memo_layers() -> tex_state::PureMemoConfig {
    tex_state::PureMemoConfig {
        recording: tex_state::PureMemoRecordingPolicy::all(),
        ..tex_state::PureMemoConfig::default()
    }
}

use tex_state::RootSpanId;

const CMR10: &[u8] = include_bytes!("../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
const CMTT10: &[u8] = include_bytes!("../../tex-fonts/tests/fixtures/cm/cmtt10.tfm");
const CMSY10: &[u8] = include_bytes!("../../tex-fonts/tests/fixtures/cm/cmsy10.tfm");
const CMEX10: &[u8] = include_bytes!("../../tex-fonts/tests/fixtures/cm/cmex10.tfm");

fn template() -> Universe {
    let mut universe = Universe::with_world(tex_state::World::memory());
    tex_exec::install_unexpandable_primitives(&mut universe);
    tex_expand::install_expandable_primitives(&mut universe);
    universe
}

fn install_pdf_paragraph_test_parameters(universe: &mut Universe) {
    for (name, meaning) in [
        (
            "pdfadjustspacing",
            tex_state::meaning::Meaning::IntParam(
                tex_state::env::banks::IntParam::PDF_ADJUST_SPACING.raw(),
            ),
        ),
        (
            "pdfeachlineheight",
            tex_state::meaning::Meaning::DimenParam(
                tex_state::env::banks::DimenParam::PDF_EACH_LINE_HEIGHT.raw(),
            ),
        ),
    ] {
        let symbol = universe.intern(name);
        universe.set_meaning_global(symbol, meaning);
    }
}

fn source(label: &str) -> String {
    format!(
        "\\shipout\\vbox{{\\hrule height 1pt width {}pt}}\\shipout\\vbox{{\\hrule height 2pt}}\\end",
        label.len() + 1
    )
}

fn persistent_source(value: usize) -> String {
    format!("\\shipout\\vbox{{\\hrule height 1pt width {value}pt}}\\count0={value}\\end")
}

fn multi_page_source(pages: usize) -> String {
    let mut source = String::new();
    for page in 0..pages {
        source.push_str(&format!(
            "% page {page}\n\\shipout\\vbox{{\\hrule height1pt width {}pt}}\n",
            page + 10
        ));
    }
    source.push_str("\\end");
    source
}

fn root_span_at(session: &Session, range: std::ops::Range<usize>) -> RootSpanId {
    session
        .layout
        .pieces()
        .iter()
        .enumerate()
        .find_map(|(index, piece)| {
            let doc_start = session.layout.doc_starts()[index] as usize;
            let doc_end = doc_start + (piece.end() - piece.start()) as usize;
            (doc_start <= range.start && range.end <= doc_end).then(|| {
                session.fragments.root_span_id(
                    piece,
                    u32::try_from(range.start - doc_start).expect("local start")
                        ..u32::try_from(range.end - doc_start).expect("local end"),
                )
            })?
        })
        .expect("range belongs to one retained piece")
}

#[test]
fn pure_memo_runtime_survives_accepted_revisions() {
    let mut universe = template();
    universe.enable_pure_memo(all_memo_layers());
    let paragraph = "\\vrule width1pt height1pt \\vrule width1pt height1pt";
    let source = format!(
        "\\hsize=20pt\\pretolerance=10000 {paragraph}\\par\n\\prevgraf=0 {paragraph}\\par\n\\vfill\\eject\\end"
    );
    let mut session = Session::start(
        universe,
        "pure-memo-lifetime",
        RevisionId::new(1),
        source.clone(),
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("cold revision");
    let after_cold = session.pure_memo_stats();
    assert!(after_cold.retained_entries > 0);
    assert_eq!(
        session
            .retention_metrics()
            .expect("cold retention")
            .memo_result_bytes,
        after_cold.retained_bytes
    );

    let digit = source.find("width1pt").expect("first width") + "width".len();
    session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(source.as_bytes()),
                range: digit..digit + 1,
                replacement: "2".to_owned(),
            },
        )
        .expect("accepted edit");
    let after_edit = session.pure_memo_stats();
    assert!(after_edit.lookups > after_cold.lookups);
    assert!(after_edit.hits > after_cold.hits);
    assert_eq!(
        session
            .retention_metrics()
            .expect("edited retention")
            .memo_result_bytes,
        after_edit.retained_bytes
    );
}

#[test]
fn cold_paragraph_recording_preserves_source_batching() {
    fn cold(memo: bool) -> ReuseMetrics {
        let mut universe = template();
        if memo {
            universe.enable_pure_memo(tex_state::PureMemoConfig::default());
        }
        Session::start(
            universe,
            "paragraph-command-accounting",
            RevisionId::new(1),
            "abcdef\\par\\end".to_owned(),
            usize::MAX,
        )
        .expect("session starts")
        .cold()
        .expect("cold revision")
        .reuse
    }

    let ordinary = cold(false);
    let memo_miss = cold(true);
    assert_eq!(ordinary.reexecuted_tokens, memo_miss.reexecuted_tokens);
    assert_eq!(
        ordinary.reexecuted_source_text_span_tokens,
        memo_miss.reexecuted_source_text_span_tokens,
    );
    assert_eq!(
        ordinary.reexecuted_commands, memo_miss.reexecuted_commands,
        "recording must not introduce a token-preflight execution seam"
    );
}

#[test]
fn paragraph_recording_keeps_line_provenance_opaque() {
    let mut universe = template();
    universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let repetitions = 4_096;
    let source = format!(
        "\\font\\tenrm=cmr10\\relax\\tenrm\\def\\r{{\\relax}}\nx{}y\\par\\vfill\\eject\\end",
        "\\r".repeat(repetitions),
    );
    let mut session = Session::start(
        universe,
        "paragraph-output-provenance",
        RevisionId::new(1),
        source,
        usize::MAX,
    )
    .expect("session starts");
    session
        .register_input_file(Path::new("cmr10.tfm"), CMR10.to_vec())
        .expect("font fixture");
    session.cold().expect("cold revision");

    let region = session
        .pure_memo
        .accepted_paragraphs()
        .iter()
        .find(|region| region.lines.is_some())
        .expect("literal paragraph is retained");
    assert!(region.delivered_tokens > repetitions);
    assert!(matches!(
        region.line_provenance,
        tex_state::ParagraphLineProvenance::Accepted(_)
    ));
}

#[test]
fn paragraph_history_interns_changed_observations_per_generation() {
    let mut universe = template();
    universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let source = concat!(
        "\\font\\tenrm=cmr10\\relax\\tenrm\\vsize=1000pt\n",
        "\\count0=1 \\ifnum\\count0=1 first paragraph\\fi\\par\n",
        "\\count0=2 \\ifnum\\count0=2 second paragraph\\fi\\par\n",
        "\\vfill\\eject\\end",
    );
    let mut session = Session::start(
        universe,
        "paragraph-observation-interning",
        RevisionId::new(1),
        source,
        usize::MAX,
    )
    .expect("session starts");
    session
        .register_input_file(Path::new("cmr10.tfm"), CMR10.to_vec())
        .expect("font fixture");
    session.cold().expect("cold revision");

    let regions = session
        .pure_memo
        .accepted_paragraphs()
        .iter()
        .filter(|region| region.lines.is_some())
        .collect::<Vec<_>>();
    assert!(regions.len() >= 2, "both paragraphs should be retained");
    let first_table = regions[0]
        .dependency_observations
        .as_ref()
        .expect("accepted region has an observation table");
    assert!(regions.iter().all(|region| {
        std::sync::Arc::ptr_eq(
            first_table,
            region
                .dependency_observations
                .as_ref()
                .expect("accepted region has an observation table"),
        )
    }));

    let count_key = tex_state::DependencyKey::Cell {
        bank: tex_state::DependencyBank::Count,
        index: 0,
    };
    let stamps = regions
        .iter()
        .filter_map(|region| {
            region
                .dependencies()
                .find(|dependency| dependency.key == count_key)
                .map(|dependency| dependency.changed_at)
        })
        .collect::<Vec<_>>();
    assert!(stamps.len() >= 2, "both count reads should be observed");
    assert_ne!(
        stamps[0], stamps[1],
        "a real intervening write needs a distinct generation-table observation"
    );
}

#[test]
fn replayed_paragraph_provenance_tracks_current_then_deleted_layout() {
    let mut universe = template();
    universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let source = concat!(
        "\\font\\tenrm=cmr10\\relax\\tenrm\\hsize=45pt\\vsize=40pt\n",
        "changed prefix paragraph text\\par\n",
        "stable replay paragraph text\\par\n",
        "another stable paragraph text\\par\n",
        "\\vfill\\eject\\end",
    );
    let stable_start = source.find("stable replay").expect("stable paragraph");
    let stable_end = stable_start + "stable replay paragraph text".len();
    let changed = source.find("changed").expect("changed paragraph");
    let mut session = Session::start(
        universe,
        "paragraph-mounted-provenance",
        RevisionId::new(1),
        source,
        usize::MAX,
    )
    .expect("session starts");
    session
        .register_input_file(Path::new("cmr10.tfm"), CMR10.to_vec())
        .expect("font fixture");
    session.cold().expect("cold revision");
    assert!(
        session
            .pure_memo
            .accepted_paragraphs()
            .iter()
            .any(|region| matches!(
                region.line_provenance,
                tex_state::ParagraphLineProvenance::Accepted(_)
            )),
        "cold retained lines should carry an opaque accepted-generation resolver"
    );
    let before = session.pure_memo_stats();

    session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(source.as_bytes()),
                range: changed..changed + "changed".len(),
                replacement: "altered".to_owned(),
            },
        )
        .expect("prefix edit");
    assert!(
        session.pure_memo_stats().paragraph_line_hits > before.paragraph_line_hits,
        "fixture must mount at least one finished-line graph"
    );
    assert!(
        session.artifacts.iter().any(|artifact| {
            (0..artifact.render_node_count()).any(|node| {
                (0..16).any(|source| {
                    matches!(
                        artifact.render_origin(node, source),
                        tex_state::ArtifactOrigin::Stable(_)
                    )
                })
            })
        }),
        "replayed paragraph provenance should remain a stable lazy recipe (deferred artifacts: {})",
        session
            .artifacts
            .iter()
            .filter(|artifact| artifact.has_deferred_render_origins())
            .count()
    );
    let stable_start_revision_two = stable_start;
    let (page, event) = (1..=session.artifacts.len() as u32)
        .flat_map(|page| (0..256).map(move |event| (page, event)))
        .find(|&(page, event)| {
            matches!(
                session.rendered_source_origin(page, event, None),
                Ok(Some(LayoutResolvedOrigin::Current {
                    doc_offset_lo,
                    doc_offset_hi,
                    ..
                })) if doc_offset_lo < stable_end as u64 && doc_offset_hi > stable_start_revision_two as u64
            )
        })
        .expect("stable paragraph output has mounted provenance");
    let replayed_span = match session
        .rendered_artifact_origin(page, event, None)
        .expect("render lookup")
    {
        Some(tex_state::ArtifactOrigin::Stable(span)) => span,
        origin => panic!("replayed paragraph should retain a stable origin: {origin:?}"),
    };
    let revision_two = session.source.clone();
    let stable_text = revision_two[stable_start..stable_end].to_owned();
    session
        .advance(
            RevisionId::new(3),
            Edit {
                base_revision: RevisionId::new(2),
                expected_hash: ContentHash::from_bytes(revision_two.as_bytes()),
                range: stable_start..stable_end,
                replacement: stable_text,
            },
        )
        .expect("identity replacement converges");
    assert_eq!(
        session
            .substrate
            .as_ref()
            .expect("accepted substrate")
            .resolve_stable_layout_origin(replayed_span, &session.fragments, &session.layout),
        LayoutResolvedOrigin::Deleted { minted_revision: 1 }
    );
}

#[test]
fn paragraph_front_end_hit_survives_prefix_shift_and_unrelated_register_write() {
    let mut universe = template();
    universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let paragraph = "stable literal paragraph text";
    let source =
        format!("{paragraph}\\par\n{paragraph}\\par\n{paragraph}\\par\n\\vfill\\eject\\end");
    let mut session = Session::start(
        universe,
        "paragraph-prefix-shift",
        RevisionId::new(1),
        source.clone(),
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("cold revision");
    let before = session.pure_memo_stats();
    assert_eq!(
        before.paragraph_hits, 0,
        "cold generation cannot hit itself"
    );

    let inserted = "\\count77=123 ";
    let incremental = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(source.as_bytes()),
                range: 0..0,
                replacement: inserted.to_owned(),
            },
        )
        .expect("prefix edit");
    let after = session.pure_memo_stats();
    assert!(after.paragraph_hits > before.paragraph_hits);

    let edited = format!("{inserted}{source}");
    let mut cold_universe = template();
    cold_universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let mut cold = Session::start(
        cold_universe,
        "paragraph-prefix-shift",
        RevisionId::new(2),
        edited,
        usize::MAX,
    )
    .expect("cold comparison");
    let cold_output = cold.cold().expect("cold edited revision");
    assert_eq!(
        incremental.dvi_bytes().expect("incremental DVI"),
        cold_output.dvi_bytes().expect("cold DVI")
    );
    let schedule = |output: &AcceptedOutput| {
        output
            .history
            .iter()
            .map(|record| {
                (
                    record.key(),
                    record.effect_prefix(),
                    record.artifact_prefix(),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        schedule(&incremental),
        schedule(&cold_output),
        "paragraph replay must publish the same named-boundary schedule as cold execution"
    );
}

struct UnavailableImageResolver;

impl tex_exec::PdfImageResolver for UnavailableImageResolver {
    fn open_image(
        &mut self,
        _input: &mut dyn InputReadState,
        _request: &tex_exec::PdfImageRequest,
        _request_index: u64,
    ) -> ResourceResult<tex_state::PdfExternalImageSource> {
        Ok(ResourceLookup::Unavailable)
    }
}

#[test]
fn external_input_delta_replays_paragraphs_from_job_start_without_new_revision() {
    let mut universe = template();
    universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let source = concat!(
        "\\font\\tenrm=cmr10\\relax\\tenrm \\input refs \\hsize=45pt ",
        "stable first paragraph text\\par ",
        "\\hskip\\refwidth reference paragraph text\\par ",
        "stable third paragraph text\\par ",
        "\\vfill\\eject\\end",
    );
    let mut session = Session::start(
        universe,
        "external-input-paragraph-replay",
        RevisionId::new(1),
        source,
        usize::MAX,
    )
    .expect("session starts");
    session
        .register_input_file(Path::new("cmr10.tfm"), CMR10.to_vec())
        .expect("font fixture");
    let mut old_inputs = StagedInputResolver::default();
    old_inputs
        .files
        .insert("refs".to_owned(), "\\def\\refwidth{1pt}".to_owned());
    let mut fonts = DirectFontResolver;
    let initial = session
        .cold_with_resolvers(&mut old_inputs, &mut fonts)
        .expect("initial external-input run");
    let initial_artifacts = initial
        .artifacts
        .iter()
        .map(CommittedArtifact::hash)
        .collect::<Vec<_>>();
    let before = session.pure_memo_stats();
    let meaning_misses_before =
        before.paragraph_validation_failure_count(tex_state::ParagraphValidationFailure::Meaning);

    let mut candidate = session
        .start_external_input_delta_candidate()
        .expect("JobStart delta candidate");
    assert_eq!(
        candidate.retention_metrics().memo_result_bytes,
        before.retained_bytes,
        "candidate telemetry must carry the accepted memo-runtime charge",
    );
    assert_eq!(
        session
            .artifacts
            .iter()
            .map(CommittedArtifact::hash)
            .collect::<Vec<_>>(),
        initial_artifacts,
        "constructing the candidate must not mutate accepted output",
    );
    let mut new_inputs = StagedInputResolver::default();
    new_inputs
        .files
        .insert("refs".to_owned(), "\\def\\refwidth{2pt}".to_owned());
    let mut images = UnavailableImageResolver;
    assert!(matches!(
        candidate
            .drive_with_resource_resolvers(
                &mut new_inputs,
                &mut fonts,
                &mut images,
                &Cancellation::new(),
            )
            .expect("delta execution"),
        RevisionCandidateResult::Complete
    ));
    let pending = session
        .finish_advance_candidate(candidate)
        .expect("finish unchanged-root delta");
    assert_eq!(pending.revision(), RevisionId::new(1));
    assert_eq!(
        pending.reuse().restart_boundary.map(|key| key.boundary),
        Some(EngineBoundary::JobStart),
    );
    assert_eq!(pending.reuse().suffixes_adopted, 0);
    assert_eq!(
        pending.reuse().same_history_stop,
        SameHistoryStop::NotAttempted
    );
    let accepted = session.accept_pending(pending).expect("accept delta rerun");
    assert_eq!(accepted.revision, RevisionId::new(1));
    assert_eq!(
        accepted.content_hash,
        ContentHash::from_bytes(source.as_bytes())
    );
    assert_eq!(
        accepted.reuse.execution_path,
        RevisionExecutionPath::ExternalInputDelta,
    );
    assert!(
        accepted.reuse.paragraph_replay_lookups >= 3,
        "{:#?}",
        accepted.reuse
    );
    assert!(
        accepted.reuse.paragraph_replay_hits >= 1,
        "{:#?}",
        accepted.reuse
    );
    let stable_start = source
        .find("stable first")
        .expect("stable paragraph source");
    let stable_end = stable_start + "stable first paragraph text".len();
    assert!(
        accepted.reuse.paragraph_replay_validation_misses >= 1,
        "{:#?}",
        accepted.reuse,
    );
    let after = session.pure_memo_stats();
    assert!(
        after.paragraph_line_hits > before.paragraph_line_hits,
        "unrelated paragraphs should mount retained finished lines",
    );
    assert!(
        after.paragraph_validation_failure_count(tex_state::ParagraphValidationFailure::Meaning,)
            > meaning_misses_before,
        "the paragraph that reads the changed reference meaning must miss: {after:?}",
    );

    let mut cold_universe = template();
    cold_universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let mut cold = Session::start(
        cold_universe,
        "external-input-paragraph-replay",
        RevisionId::new(1),
        source,
        usize::MAX,
    )
    .expect("cold comparison starts");
    cold.register_input_file(Path::new("cmr10.tfm"), CMR10.to_vec())
        .expect("cold font fixture");
    let cold = cold
        .cold_with_resolvers(&mut new_inputs, &mut fonts)
        .expect("cold comparison runs");
    assert_eq!(
        accepted.dvi_bytes().expect("delta DVI"),
        cold.dvi_bytes().expect("cold DVI"),
    );
    assert_eq!(accepted.effects, cold.effects, "detached effects differ");
    assert_eq!(
        accepted
            .artifacts
            .iter()
            .map(CommittedArtifact::hash)
            .collect::<Vec<_>>(),
        cold.artifacts
            .iter()
            .map(CommittedArtifact::hash)
            .collect::<Vec<_>>(),
        "committed artifacts differ",
    );
    assert_eq!(
        accepted
            .history
            .iter()
            .map(|record| {
                (
                    record.key(),
                    record.effect_prefix(),
                    record.artifact_prefix(),
                    record.state_hash(),
                )
            })
            .collect::<Vec<_>>(),
        cold.history
            .iter()
            .map(|record| {
                (
                    record.key(),
                    record.effect_prefix(),
                    record.artifact_prefix(),
                    record.state_hash(),
                )
            })
            .collect::<Vec<_>>(),
        "JobStart replay must publish the cold named-boundary schedule and final state",
    );

    assert!(
        (1..=accepted.artifacts.len() as u32)
            .flat_map(|page| (0..256).map(move |event| (page, event)))
            .any(|(page, event)| {
                matches!(
                    session.rendered_source_origin(page, event, None),
                    Ok(Some(LayoutResolvedOrigin::Current {
                        doc_offset_lo,
                        doc_offset_hi,
                        ..
                    })) if doc_offset_lo < stable_end as u64
                        && doc_offset_hi > stable_start as u64
                )
            }),
        "a mounted paragraph must resolve through current-revision provenance",
    );

    let before_second_delta = session.pure_memo_stats();
    let mut second_candidate = session
        .start_external_input_delta_candidate()
        .expect("second JobStart delta candidate");
    let mut newest_inputs = StagedInputResolver::default();
    newest_inputs
        .files
        .insert("refs".to_owned(), "\\def\\refwidth{3pt}".to_owned());
    assert!(matches!(
        second_candidate
            .drive_with_resource_resolvers(
                &mut newest_inputs,
                &mut fonts,
                &mut images,
                &Cancellation::new(),
            )
            .expect("second delta execution"),
        RevisionCandidateResult::Complete
    ));
    let second_pending = session
        .finish_advance_candidate(second_candidate)
        .expect("finish second unchanged-root delta");
    assert_eq!(
        second_pending.reuse().execution_path,
        RevisionExecutionPath::ExternalInputDelta,
    );
    assert!(second_pending.reuse().paragraph_replay_hits >= 1);
    assert!(second_pending.reuse().paragraph_replay_validation_misses >= 1);
    let second = session
        .accept_pending(second_pending)
        .expect("accept second delta rerun");
    assert!(
        session.pure_memo_stats().paragraph_line_hits > before_second_delta.paragraph_line_hits,
        "accepted records carried across generations must remain live",
    );

    let mut newest_cold_universe = template();
    newest_cold_universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let mut newest_cold = Session::start(
        newest_cold_universe,
        "external-input-paragraph-replay",
        RevisionId::new(1),
        source,
        usize::MAX,
    )
    .expect("second cold comparison starts");
    newest_cold
        .register_input_file(Path::new("cmr10.tfm"), CMR10.to_vec())
        .expect("second cold font fixture");
    let newest_cold = newest_cold
        .cold_with_resolvers(&mut newest_inputs, &mut fonts)
        .expect("second cold comparison runs");
    assert_eq!(second.dvi_bytes(), newest_cold.dvi_bytes());
    assert_eq!(second.effects, newest_cold.effects);
    assert_eq!(
        second
            .history
            .iter()
            .map(|record| (record.key(), record.state_hash()))
            .collect::<Vec<_>>(),
        newest_cold
            .history
            .iter()
            .map(|record| (record.key(), record.state_hash()))
            .collect::<Vec<_>>(),
    );
}

#[test]
fn forced_job_start_fallback_is_private_and_attributed() {
    let source = "stable paragraph text\\par\\vfill\\eject\\end";
    let mut universe = template();
    universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let mut session = Session::start(
        universe,
        "forced-job-start-fallback",
        RevisionId::new(1),
        source,
        usize::MAX,
    )
    .expect("session starts");
    let initial = session.cold().expect("cold revision");
    let initial_artifacts = initial
        .artifacts
        .iter()
        .map(CommittedArtifact::hash)
        .collect::<Vec<_>>();
    let initial_memo = session.pure_memo_stats();
    let edit = Edit {
        base_revision: RevisionId::new(1),
        expected_hash: ContentHash::from_bytes(source.as_bytes()),
        range: 0..0,
        replacement: "% external dependency mismatch\n".to_owned(),
    };

    let mut failed = session
        .start_advance_candidate_from_job_start(RevisionId::new(2), edit.clone())
        .expect("private fallback candidate");
    failed.set_cumulative_fuel_limit(0);
    assert!(
        failed
            .drive_with_resource_resolvers(
                &mut DirectInputResolver,
                &mut DirectFontResolver,
                &mut UnavailableImageResolver,
                &Cancellation::new(),
            )
            .is_err(),
        "forced fallback fixture must fail before acceptance",
    );
    assert_eq!(session.revision, RevisionId::new(1));
    assert_eq!(session.source, source);
    assert_eq!(session.pure_memo_stats(), initial_memo);
    assert_eq!(
        session
            .artifacts
            .iter()
            .map(CommittedArtifact::hash)
            .collect::<Vec<_>>(),
        initial_artifacts,
        "failed fallback must not mutate accepted output",
    );

    let mut completed = session
        .start_advance_candidate_from_job_start(RevisionId::new(2), edit)
        .expect("retry fallback candidate");
    assert!(matches!(
        completed
            .drive_with_resource_resolvers(
                &mut DirectInputResolver,
                &mut DirectFontResolver,
                &mut UnavailableImageResolver,
                &Cancellation::new(),
            )
            .expect("fallback retry"),
        RevisionCandidateResult::Complete,
    ));
    let pending = session
        .finish_advance_candidate(completed)
        .expect("finish fallback retry");
    assert_eq!(
        pending.reuse().execution_path,
        RevisionExecutionPath::ForcedJobStartFallback,
    );
    assert_eq!(pending.reuse().paragraph_replay_lookups, 0);
    drop(pending);
    assert_eq!(session.revision, RevisionId::new(1));
    assert_eq!(session.source, source);
}

#[test]
fn cold_middle_paragraph_and_carried_suffix_keep_generation_observation_tables() {
    let mut universe = template();
    universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let source = concat!(
        "\\font\\tenrm=cmr10\\relax\\tenrm\\vsize=1000pt\n",
        "first literal paragraph text\\par\n",
        "changed middle paragraph text\\par\n",
        "stable suffix paragraph text\\par\n",
        "\\vfill\\eject\\end",
    );
    let mut session = Session::start(
        universe,
        "paragraph-observation-generations",
        RevisionId::new(1),
        source.to_owned(),
        usize::MAX,
    )
    .expect("session starts");
    session
        .register_input_file(Path::new("cmr10.tfm"), CMR10.to_vec())
        .expect("font fixture");
    session.cold().expect("cold revision");
    let original_table = std::sync::Arc::as_ptr(
        session.pure_memo.accepted_paragraphs()[0]
            .dependency_observations
            .as_ref()
            .expect("cold observation table"),
    ) as *const tex_state::ObservedDependency as usize;

    let changed = source.find("changed").expect("changed paragraph");
    let before = session.pure_memo_stats();
    session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(source.as_bytes()),
                range: changed..changed + "changed".len(),
                replacement: "altered".to_owned(),
            },
        )
        .expect("middle paragraph edit");
    assert!(
        session.pure_memo_stats().paragraph_hits > before.paragraph_hits,
        "the unchanged suffix must be carried: before={before:?}, after={:?}",
        session.pure_memo_stats(),
    );
    let accepted = session
        .pure_memo
        .accepted_paragraphs()
        .iter()
        .filter(|region| region.lines.is_some())
        .collect::<Vec<_>>();
    let tables = accepted
        .iter()
        .map(|region| {
            std::sync::Arc::as_ptr(
                region
                    .dependency_observations
                    .as_ref()
                    .expect("accepted region has an observation table"),
            ) as *const tex_state::ObservedDependency as usize
        })
        .collect::<Vec<_>>();
    assert!(
        accepted.windows(2).any(|pair| {
            !std::sync::Arc::ptr_eq(
                pair[0]
                    .dependency_observations
                    .as_ref()
                    .expect("accepted region has an observation table"),
                pair[1]
                    .dependency_observations
                    .as_ref()
                    .expect("accepted region has an observation table"),
            )
        }),
        "a newly cold paragraph and carried suffix must retain distinct generation tables: original={original_table} accepted={tables:?}"
    );
    assert!(accepted.iter().all(|region| {
        region.dependencies().count() == region.dependency_ordinals.len()
            && region.break_dependencies().count() == region.break_dependency_ordinals.len()
    }));
}

#[test]
fn paragraph_list_local_assignments_do_not_block_replay() {
    let mut universe = template();
    universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let paragraph = "list local \\spacefactor=2000 state \\prevdepth=5pt paragraph text";
    let source =
        format!("{paragraph}\\par\n{paragraph}\\par\n{paragraph}\\par\n\\vfill\\eject\\end");
    let mut session = Session::start(
        universe,
        "paragraph-list-local-state",
        RevisionId::new(1),
        source.clone(),
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("cold revision");
    let before = session.pure_memo_stats();
    assert_eq!(
        before.paragraph_unsupported_write_barriers, 0,
        "horizontal-list state must not escape the retained paragraph: {before:?}"
    );

    let prefix = "% retain all paragraph source spans\n";
    let incremental = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(source.as_bytes()),
                range: 0..0,
                replacement: prefix.to_owned(),
            },
        )
        .expect("prefix edit");
    let after = session.pure_memo_stats();
    assert!(
        after.paragraph_hits >= before.paragraph_hits + 2,
        "unchanged list-local paragraphs should replay: {after:?}"
    );

    let mut cold_universe = template();
    cold_universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let mut cold = Session::start(
        cold_universe,
        "paragraph-list-local-state",
        RevisionId::new(2),
        format!("{prefix}{source}"),
        usize::MAX,
    )
    .expect("cold comparison starts");
    let cold_output = cold.cold().expect("cold edited revision");
    assert_eq!(
        incremental.dvi_bytes().expect("incremental DVI"),
        cold_output.dvi_bytes().expect("cold DVI")
    );
}

#[test]
fn paragraph_hit_preserves_outer_paragraph_and_shipout_boundaries() {
    let paragraph = "stable paragraph words stable paragraph words stable paragraph words stable paragraph words stable paragraph words stable paragraph words";
    let source = format!(
        "\\font\\tenrm=cmr10\\relax \\tenrm \\hsize=35pt \\vsize=24pt\n{paragraph}\\par\n{paragraph}\\par\n{paragraph}\\par\n\\end"
    );
    let inserted = "\\count77=123 ";
    let edited = format!("{inserted}{source}");

    let mut universe = template();
    universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let mut session = Session::start(
        universe,
        "paragraph-boundary-replay",
        RevisionId::new(1),
        source.clone(),
        usize::MAX,
    )
    .expect("session starts");
    session
        .register_input_file(Path::new("cmr10.tfm"), CMR10.to_vec())
        .expect("font fixture");
    session.cold().expect("cold revision");
    let before = session.pure_memo_stats();
    let incremental = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(source.as_bytes()),
                range: 0..0,
                replacement: inserted.to_owned(),
            },
        )
        .expect("prefix edit");
    assert!(
        session.pure_memo_stats().paragraph_line_hits > before.paragraph_line_hits,
        "the schedule comparison must exercise finished-line paragraph replay"
    );

    let mut cold_universe = template();
    cold_universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let mut cold = Session::start(
        cold_universe,
        "paragraph-boundary-replay",
        RevisionId::new(2),
        edited,
        usize::MAX,
    )
    .expect("cold comparison starts");
    cold.register_input_file(Path::new("cmr10.tfm"), CMR10.to_vec())
        .expect("cold font fixture");
    let expected = cold.cold().expect("cold comparison");
    let schedule = |output: &AcceptedOutput| {
        output
            .history
            .iter()
            .map(|record| {
                (
                    record.key(),
                    record.effect_prefix(),
                    record.artifact_prefix(),
                )
            })
            .collect::<Vec<_>>()
    };
    assert!(
        expected
            .history
            .iter()
            .filter(|record| record.key().boundary == EngineBoundary::ShipoutComplete)
            .count()
            > 1,
        "fixture must ship a page before the final end cleanup"
    );
    assert_eq!(incremental.effects, expected.effects);
    assert_eq!(incremental.artifacts, expected.artifacts);
    assert_eq!(
        incremental.dvi_bytes().expect("incremental DVI"),
        expected.dvi_bytes().expect("cold DVI")
    );
    assert_eq!(
        schedule(&incremental),
        schedule(&expected),
        "paragraph replay must preserve outer-paragraph and shipout checkpoints"
    );
}

#[test]
fn paragraph_replay_deopts_before_a_new_paragraph_start_output_fire() {
    let first = "A\\vrule width40pt height7pt depth2pt";
    let inserted = " \\penalty-10000 B";
    let source = format!(
        concat!(
            "\\font\\tenrm=cmr10\\relax\\tenrm ",
            "\\hsize=60pt\\vsize=12pt ",
            "\\output={{\\immediate\\write16{{OUT count=\\the\\count0 line=\\the\\inputlineno}}",
            "\\shipout\\box255}}\n",
            "{}\\par\n",
            "X\\global\\count0=7 stable suffix text\\par\n",
            "\\vfill\\eject\\end",
        ),
        first
    );
    let insertion = source.find("\\par\nX").expect("first paragraph end");

    let mut universe = template();
    universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let mut session = Session::start(
        universe,
        "paragraph-start-output-deopt",
        RevisionId::new(1),
        source.clone(),
        usize::MAX,
    )
    .expect("session starts");
    session
        .register_input_file(Path::new("cmr10.tfm"), CMR10.to_vec())
        .expect("font fixture");
    session.cold().expect("cold revision");
    let before = session.pure_memo_stats();
    let incremental = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(source.as_bytes()),
                range: insertion..insertion,
                replacement: inserted.to_owned(),
            },
        )
        .expect("pagination-changing edit");
    let after = session.pure_memo_stats();
    assert!(
        after.paragraph_validation_failure_count(
            tex_state::ParagraphValidationFailure::ParagraphStart,
        ) > before.paragraph_validation_failure_count(
            tex_state::ParagraphValidationFailure::ParagraphStart,
        ),
        "the stable paragraph must deopt when its new start fires output: {after:?}"
    );

    let edited = format!("{}{inserted}{}", &source[..insertion], &source[insertion..]);
    let mut cold_universe = template();
    cold_universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let mut cold = Session::start(
        cold_universe,
        "paragraph-start-output-deopt",
        RevisionId::new(2),
        edited,
        usize::MAX,
    )
    .expect("cold comparison starts");
    cold.register_input_file(Path::new("cmr10.tfm"), CMR10.to_vec())
        .expect("cold font fixture");
    let expected = cold.cold().expect("cold comparison");

    assert_eq!(incremental.effects, expected.effects);
    assert_eq!(incremental.artifacts, expected.artifacts);
    assert_eq!(
        incremental.dvi_bytes().expect("incremental DVI"),
        expected.dvi_bytes().expect("cold DVI")
    );
    assert!(incremental.effects.iter().any(|effect| matches!(
        effect,
        tex_state::EffectRecord::StreamWrite { text, .. }
            if text.contains("OUT count=0")
    )));
}

#[test]
fn paragraph_front_end_hit_replays_nonempty_everypar_across_revision() {
    let mut universe = template();
    universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let source = "\\everypar{\\penalty10000}first paragraph text\\par\nsecond paragraph text\\par\nthird paragraph text\\par\n\\vfill\\eject\\end";
    let mut session = Session::start(
        universe,
        "paragraph-everypar-reuse",
        RevisionId::new(1),
        source.to_owned(),
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("cold revision");
    let before = session.pure_memo_stats();

    let inserted = "% shift unchanged everypar paragraphs\n";
    let incremental = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(source.as_bytes()),
                range: 0..0,
                replacement: inserted.to_owned(),
            },
        )
        .expect("prefix edit");
    let after = session.pure_memo_stats();
    assert!(
        after.paragraph_lookups > before.paragraph_lookups,
        "{after:?}"
    );
    assert!(after.paragraph_hits > before.paragraph_hits, "{after:?}");

    let mut cold_universe = template();
    cold_universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let mut cold = Session::start(
        cold_universe,
        "paragraph-everypar-reuse",
        RevisionId::new(2),
        format!("{inserted}{source}"),
        usize::MAX,
    )
    .expect("cold comparison");
    let cold_output = cold.cold().expect("cold edited revision");
    assert_eq!(
        incremental.dvi_bytes().expect("incremental DVI"),
        cold_output.dvi_bytes().expect("cold DVI")
    );
}

#[test]
fn paragraph_after_entry_group_replacement_is_replayed() {
    let mut universe = template();
    universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let source = concat!(
        "\\def\\replacegroup{\\endgroup\\begingroup grouped paragraph text\\par}\n",
        "% header a\n",
        "\\begingroup\\replacegroup\\endgroup\n",
        "stable literal paragraph text\\par\n",
        "\\vfill\\eject\\end",
    );
    let mut session = Session::start(
        universe,
        "paragraph-group-transition-barrier",
        RevisionId::new(1),
        source,
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("cold revision");
    let after_cold = session.pure_memo_stats();
    assert_eq!(
        after_cold.paragraph_unsupported_group_transition_barriers, 0,
        "vertical group replacement finishes before the paragraph region starts"
    );

    let header = source.find("header a").expect("header marker") + "header ".len();
    let before = session.pure_memo_stats();
    let first = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(source.as_bytes()),
                range: header..header + 1,
                replacement: "b".to_owned(),
            },
        )
        .expect("header edit");
    assert!(first.reuse.restart_boundary.is_some());
    assert!(
        session.pure_memo_stats().paragraph_line_hits > before.paragraph_line_hits,
        "the paragraph inside the replacement group should replay"
    );
    let edited = format!("{}b{}", &source[..header], &source[header + 1..]);

    let second = session
        .advance(
            RevisionId::new(3),
            Edit {
                base_revision: RevisionId::new(2),
                expected_hash: ContentHash::from_bytes(edited.as_bytes()),
                range: header..header + 1,
                replacement: "a".to_owned(),
            },
        )
        .expect("inverse header edit");
    assert!(second.reuse.restart_boundary.is_some());

    let mut cold_universe = template();
    cold_universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let mut cold = Session::start(
        cold_universe,
        "paragraph-group-transition-barrier",
        RevisionId::new(3),
        source,
        usize::MAX,
    )
    .expect("cold comparison starts");
    let expected = cold.cold().expect("cold comparison");
    assert_eq!(
        second.dvi_bytes().expect("incremental DVI"),
        expected.dvi_bytes().expect("cold DVI")
    );
    assert_eq!(second.effects, expected.effects);
}

#[test]
fn paragraph_with_balanced_root_level_group_is_replayed() {
    let mut universe = template();
    universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let source = concat!(
        "changed prefix paragraph text\\par\n",
        "\\vskip 12pt\n",
        "root paragraph with {locally grouped words} after the group\\par\n",
        "stable suffix paragraph text\\par\n",
        "\\vfill\\eject\\end",
    );
    let mut session = Session::start(
        universe,
        "paragraph-balanced-root-group",
        RevisionId::new(1),
        source,
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("cold revision");
    let after_cold = session.pure_memo_stats();
    assert_eq!(
        after_cold.paragraph_unsupported_group_transition_barriers, 0,
        "a fully closed root-level group does not replace an entry group"
    );

    let changed = source.find("changed").expect("changed word");
    let before = session.pure_memo_stats();
    let incremental = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(source.as_bytes()),
                range: changed..changed + "changed".len(),
                replacement: "altered".to_owned(),
            },
        )
        .expect("first paragraph edit");
    let after = session.pure_memo_stats();
    assert!(
        after.paragraph_line_hits > before.paragraph_line_hits,
        "{after:?}"
    );

    let edited = format!(
        "{}altered{}",
        &source[..changed],
        &source[changed + "changed".len()..]
    );
    let mut cold_universe = template();
    cold_universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let mut cold = Session::start(
        cold_universe,
        "paragraph-balanced-root-group",
        RevisionId::new(2),
        edited,
        usize::MAX,
    )
    .expect("cold comparison starts");
    let expected = cold.cold().expect("cold comparison");
    assert_eq!(
        incremental.dvi_bytes().expect("incremental DVI"),
        expected.dvi_bytes().expect("cold DVI")
    );
    assert_eq!(incremental.effects, expected.effects);
}

#[test]
fn paragraph_replay_excludes_preceding_recoverable_vertical_error() {
    let mut universe = template();
    universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let source = concat!(
        "changed prefix paragraph text\\par\n",
        "\\notdefined\n",
        "stable paragraph after the error\\par\n",
        "\\vfill\\eject\\end",
    );
    let mut session = Session::start(
        universe,
        "paragraph-after-recoverable-error",
        RevisionId::new(1),
        source,
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("cold revision");

    let changed = source.find("changed").expect("changed word");
    let before = session.pure_memo_stats();
    let incremental = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(source.as_bytes()),
                range: changed..changed + "changed".len(),
                replacement: "altered".to_owned(),
            },
        )
        .expect("prefix edit");
    assert!(
        session.pure_memo_stats().paragraph_line_hits > before.paragraph_line_hits,
        "the paragraph after the diagnostic should still replay"
    );

    let edited = format!(
        "{}altered{}",
        &source[..changed],
        &source[changed + "changed".len()..]
    );
    let mut cold_universe = template();
    cold_universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let mut cold = Session::start(
        cold_universe,
        "paragraph-after-recoverable-error",
        RevisionId::new(2),
        edited,
        usize::MAX,
    )
    .expect("cold comparison starts");
    let expected = cold.cold().expect("cold comparison");
    assert_eq!(
        incremental.dvi_bytes().expect("incremental DVI"),
        expected.dvi_bytes().expect("cold DVI")
    );
    assert_eq!(
        incremental.effects, expected.effects,
        "the vertical diagnostic must execute once rather than replay with the paragraph"
    );
}

#[test]
fn paragraph_with_group_local_mutation_replays_without_root_write() {
    let mut universe = template();
    universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let source = concat!(
        "changed prefix paragraph text\\par\n",
        "root paragraph with {\\count255=123 locally assigned count} and root value \\the\\count255\\par\n",
        "stable suffix paragraph text\\par\n",
        "\\vfill\\eject\\end",
    );
    let mut session = Session::start(
        universe,
        "paragraph-balanced-root-group-mutation",
        RevisionId::new(1),
        source,
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("cold revision");
    let after_cold = session.pure_memo_stats();
    assert_eq!(
        after_cold.paragraph_unsupported_group_transition_barriers, 0,
        "balanced local writes disappear from the compacted root journal"
    );

    let changed = source.find("changed").expect("changed word");
    let before = session.pure_memo_stats();
    let incremental = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(source.as_bytes()),
                range: changed..changed + "changed".len(),
                replacement: "altered".to_owned(),
            },
        )
        .expect("prefix edit");
    let after = session.pure_memo_stats();
    assert!(
        after.paragraph_line_hits > before.paragraph_line_hits,
        "balanced local mutation paragraph should replay: {after:?}"
    );

    let edited = format!(
        "{}altered{}",
        &source[..changed],
        &source[changed + "changed".len()..]
    );
    let mut cold_universe = template();
    cold_universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let mut cold = Session::start(
        cold_universe,
        "paragraph-balanced-root-group-mutation",
        RevisionId::new(2),
        edited,
        usize::MAX,
    )
    .expect("cold comparison starts");
    let expected = cold.cold().expect("cold comparison");
    assert_eq!(
        incremental.dvi_bytes().expect("incremental DVI"),
        expected.dvi_bytes().expect("cold DVI")
    );
    assert_eq!(incremental.effects, expected.effects);
}

#[test]
fn paragraph_with_discharged_nested_assignments_replays() {
    let mut universe = template();
    universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let source = concat!(
        "changed prefix paragraph text\\par\n",
        "nested assignments {\\count0=8 \\multiply\\count0 by 2 \\divide\\count0 by 4 ",
        "\\dimen0=5pt \\advance\\dimen0 by 2pt ",
        "\\skip0=1pt plus 1fil \\advance\\skip0 by 2pt ",
        "\\muskip0=1mu \\advance\\muskip0 by 2mu \\def\\local{local words} ",
        "\\catcode`\\@=11 \\setbox0=\\hbox{temporary box} \\local} after group\\par\n",
        "\\ifvoid0 stable suffix paragraph\\else leaked box\\fi\\par\n",
        "\\vfill\\eject\\end",
    );
    let mut session = Session::start(
        universe,
        "paragraph-discharged-nested-assignments",
        RevisionId::new(1),
        source,
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("cold revision");
    let after_cold = session.pure_memo_stats();
    assert_eq!(
        after_cold.paragraph_unsupported_write_barriers, 0,
        "balanced nested-local assignments must not escape: {after_cold:?}"
    );

    let changed = source.find("changed").expect("changed word");
    let edited = format!(
        "{}altered{}",
        &source[..changed],
        &source[changed + "changed".len()..]
    );
    let before = session.pure_memo_stats();
    let incremental = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(source.as_bytes()),
                range: changed..changed + "changed".len(),
                replacement: "altered".to_owned(),
            },
        )
        .expect("prefix edit");
    let after = session.pure_memo_stats();
    assert!(
        after.paragraph_line_hits > before.paragraph_line_hits,
        "nested-local assignment paragraph should replay: {after:?}"
    );

    let mut cold_universe = template();
    cold_universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let mut cold = Session::start(
        cold_universe,
        "paragraph-discharged-nested-assignments",
        RevisionId::new(2),
        edited,
        usize::MAX,
    )
    .expect("cold comparison starts");
    let expected = cold.cold().expect("cold comparison");
    assert_eq!(incremental.dvi_bytes(), expected.dvi_bytes());
    assert_eq!(incremental.effects, expected.effects);
}

#[test]
fn paragraph_with_source_proven_local_box_consumption_replays() {
    let mut universe = template();
    universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let source = concat!(
        "changed prefix paragraph text\\par\n",
        "local box {\\setbox0=\\hbox{source proven} \\copy0 and \\box0} paragraph\\par\n",
        "\\ifvoid0 stable suffix paragraph\\else leaked box\\fi\\par\n",
        "\\vfill\\eject\\end",
    );
    let mut session = Session::start(
        universe,
        "paragraph-source-proven-local-box",
        RevisionId::new(1),
        source,
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("cold revision");
    assert_eq!(
        session
            .pure_memo_stats()
            .paragraph_unsupported_write_barriers,
        0,
        "consuming a box supplied by the active child group should not barrier"
    );

    let changed = source.find("changed").expect("changed word");
    let edited = format!(
        "{}altered{}",
        &source[..changed],
        &source[changed + "changed".len()..]
    );
    let before = session.pure_memo_stats();
    let incremental = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(source.as_bytes()),
                range: changed..changed + "changed".len(),
                replacement: "altered".to_owned(),
            },
        )
        .expect("prefix edit");
    assert!(
        session.pure_memo_stats().paragraph_line_hits > before.paragraph_line_hits,
        "source-proven local box paragraph should replay"
    );

    let mut cold_universe = template();
    cold_universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let mut cold = Session::start(
        cold_universe,
        "paragraph-source-proven-local-box",
        RevisionId::new(2),
        edited,
        usize::MAX,
    )
    .expect("cold comparison starts");
    let expected = cold.cold().expect("cold comparison");
    assert_eq!(incremental.dvi_bytes(), expected.dvi_bytes());
    assert_eq!(incremental.effects, expected.effects);
}

#[test]
fn paragraph_with_source_proven_local_unboxing_replays() {
    let mut universe = template();
    universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let source = concat!(
        "changed prefix paragraph text\\par\n",
        "local unbox {\\setbox0=\\hbox{source proven} ",
        "\\unhcopy0 and \\unhbox0} paragraph\\par\n",
        "void unbox \\unhbox250 remains replayable\\par\n",
        "\\ifvoid0 stable suffix paragraph\\else leaked box\\fi\\par\n",
        "\\vfill\\eject\\end",
    );
    let mut session = Session::start(
        universe,
        "paragraph-source-proven-local-unbox",
        RevisionId::new(1),
        source,
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("cold revision");
    assert_eq!(
        session
            .pure_memo_stats()
            .paragraph_unsupported_write_barriers,
        0,
        "void and paragraph-local unboxing should not barrier"
    );

    let changed = source.find("changed").expect("changed word");
    let edited = format!(
        "{}altered{}",
        &source[..changed],
        &source[changed + "changed".len()..]
    );
    let before = session.pure_memo_stats();
    let incremental = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(source.as_bytes()),
                range: changed..changed + "changed".len(),
                replacement: "altered".to_owned(),
            },
        )
        .expect("prefix edit");
    assert!(
        session.pure_memo_stats().paragraph_line_hits > before.paragraph_line_hits,
        "void and source-proven local unbox paragraphs should replay"
    );

    let mut cold_universe = template();
    cold_universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let mut cold = Session::start(
        cold_universe,
        "paragraph-source-proven-local-unbox",
        RevisionId::new(2),
        edited,
        usize::MAX,
    )
    .expect("cold comparison starts");
    let expected = cold.cold().expect("cold comparison");
    assert_eq!(incremental.dvi_bytes(), expected.dvi_bytes());
    assert_eq!(incremental.effects, expected.effects);
}

#[test]
fn paragraph_read_only_external_box_replays_and_invalidates() {
    let mut universe = template();
    universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let source = concat!(
        "\\setbox0=\\hbox{old box}\n",
        "changed prefix paragraph text\\par\n",
        "external box \\copy0 and \\unhcopy0 paragraph\\par\n",
        "\\vfill\\eject\\end",
    );
    let mut session = Session::start(
        universe,
        "paragraph-read-only-external-box",
        RevisionId::new(1),
        source,
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("cold revision");
    assert_eq!(
        session
            .pure_memo_stats()
            .paragraph_unsupported_write_barriers,
        0,
        "read-only external boxes should be dependencies, not write barriers"
    );

    let changed = source.find("changed").expect("changed word");
    let edited = format!(
        "{}altered{}",
        &source[..changed],
        &source[changed + "changed".len()..]
    );
    let before_reuse = session.pure_memo_stats();
    session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(source.as_bytes()),
                range: changed..changed + "changed".len(),
                replacement: "altered".to_owned(),
            },
        )
        .expect("prefix edit");
    assert!(
        session.pure_memo_stats().paragraph_line_hits > before_reuse.paragraph_line_hits,
        "unchanged external box reads should replay"
    );
    let old = edited.find("old box").expect("old box text");
    let replacement = "new wider box";
    let redefined = format!(
        "{}{replacement}{}",
        &edited[..old],
        &edited[old + "old box".len()..]
    );
    let before_invalidation = session.pure_memo_stats();
    let incremental = session
        .advance(
            RevisionId::new(3),
            Edit {
                base_revision: RevisionId::new(2),
                expected_hash: ContentHash::from_bytes(edited.as_bytes()),
                range: old..old + "old box".len(),
                replacement: replacement.to_owned(),
            },
        )
        .expect("box content edit");
    assert!(
        session.pure_memo_stats().paragraph_validation_misses
            > before_invalidation.paragraph_validation_misses,
        "changing an external box must invalidate retained readers"
    );

    let mut cold_universe = template();
    cold_universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let mut cold = Session::start(
        cold_universe,
        "paragraph-read-only-external-box",
        RevisionId::new(3),
        redefined,
        usize::MAX,
    )
    .expect("cold comparison starts");
    let expected = cold.cold().expect("cold comparison");
    assert_eq!(incremental.dvi_bytes(), expected.dvi_bytes());
    assert_eq!(incremental.effects, expected.effects);
}

#[test]
fn paragraph_vertical_boxes_are_opaque_break_inputs() {
    let mut universe = template();
    universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let source = concat!(
        "\\setbox0=\\vbox{\\hbox{old vertical box}}\n",
        "changed prefix paragraph text\\par\n",
        "local \\vbox{\\hbox{built vertical box}} paragraph\\par\n",
        "external \\copy0 paragraph\\par\n",
        "\\vfill\\eject\\end",
    );
    let mut session = Session::start(
        universe,
        "paragraph-vertical-box-break-input",
        RevisionId::new(1),
        source,
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("cold revision");

    let prefix = "% preserve vertical-box paragraphs\n";
    let before_reuse = session.pure_memo_stats();
    session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(source.as_bytes()),
                range: 0..0,
                replacement: prefix.to_owned(),
            },
        )
        .expect("prefix edit");
    let after_reuse = session.pure_memo_stats();
    assert!(
        after_reuse.paragraph_line_hits >= before_reuse.paragraph_line_hits + 2,
        "local and read-only external vertical boxes should both replay (before={before_reuse:?}, after={after_reuse:?})",
    );

    let edited = format!("{prefix}{source}");
    let old = edited.find("old vertical box").expect("old box text");
    let replacement = "new wider vertical box";
    let redefined = format!(
        "{}{replacement}{}",
        &edited[..old],
        &edited[old + "old vertical box".len()..]
    );
    let before_invalidation = session.pure_memo_stats();
    let incremental = session
        .advance(
            RevisionId::new(3),
            Edit {
                base_revision: RevisionId::new(2),
                expected_hash: ContentHash::from_bytes(edited.as_bytes()),
                range: old..old + "old vertical box".len(),
                replacement: replacement.to_owned(),
            },
        )
        .expect("vertical box content edit");
    assert!(
        session.pure_memo_stats().paragraph_validation_misses
            > before_invalidation.paragraph_validation_misses,
        "changing an external vertical box must invalidate its reader"
    );

    let mut cold_universe = template();
    cold_universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let mut cold = Session::start(
        cold_universe,
        "paragraph-vertical-box-break-input",
        RevisionId::new(3),
        redefined,
        usize::MAX,
    )
    .expect("cold comparison starts");
    let expected = cold.cold().expect("cold comparison");
    assert_eq!(incremental.dvi_bytes(), expected.dvi_bytes());
    assert_eq!(incremental.effects, expected.effects);
}

#[test]
fn paragraph_void_box_read_invalidates_when_filled() {
    let mut universe = template();
    universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let source = concat!(
        "changed prefix paragraph text\\par\n",
        "void external box \\unhcopy0 paragraph\\par\n",
        "\\vfill\\eject\\end",
    );
    let mut session = Session::start(
        universe,
        "paragraph-void-external-box",
        RevisionId::new(1),
        source,
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("cold revision");
    let before = session.pure_memo_stats();
    let prefix = "\\setbox0=\\hbox{now present}\n";
    let incremental = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(source.as_bytes()),
                range: 0..0,
                replacement: prefix.to_owned(),
            },
        )
        .expect("fill box before unchanged paragraph");
    assert!(
        session.pure_memo_stats().paragraph_validation_misses > before.paragraph_validation_misses,
        "filling a previously void box must invalidate retained readers"
    );

    let mut cold_universe = template();
    cold_universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let mut cold = Session::start(
        cold_universe,
        "paragraph-void-external-box",
        RevisionId::new(2),
        format!("{prefix}{source}"),
        usize::MAX,
    )
    .expect("cold comparison starts");
    let expected = cold.cold().expect("cold comparison");
    assert_eq!(incremental.dvi_bytes(), expected.dvi_bytes());
    assert_eq!(incremental.effects, expected.effects);
}

#[test]
fn paragraph_with_vadjust_replays_and_tracks_payload_meanings() {
    let mut universe = template();
    universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let source = concat!(
        "\\def\\payload{old}\n",
        "\\def\\marginref#1{\\vadjust{\\setbox0=\\hbox{#1}",
        "\\dimen16=\\ht0 \\advance\\dimen16 by \\dp0 ",
        "\\kern-\\dimen16 \\vbox to \\dimen16{\\hbox to 100pt{\\hfil\\box0}\\vss}}}\n",
        "changed prefix paragraph text\\par\n",
        "paragraph with \\marginref{\\payload} migrating material\\par\n",
        "stable suffix paragraph\\par\n",
        "\\vfill\\eject\\end",
    );
    let mut session = Session::start(
        universe,
        "paragraph-vadjust-replay",
        RevisionId::new(1),
        source,
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("cold revision");

    let changed = source.find("changed").expect("changed word");
    let edited = format!(
        "{}altered{}",
        &source[..changed],
        &source[changed + "changed".len()..]
    );
    let before_replay = session.pure_memo_stats();
    let replayed = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(source.as_bytes()),
                range: changed..changed + "changed".len(),
                replacement: "altered".to_owned(),
            },
        )
        .expect("prefix edit");
    assert!(
        session.pure_memo_stats().paragraph_line_hits > before_replay.paragraph_line_hits,
        "paragraph containing migrating adjust material should replay"
    );

    let mut replay_cold_universe = template();
    replay_cold_universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let mut replay_cold = Session::start(
        replay_cold_universe,
        "paragraph-vadjust-replay",
        RevisionId::new(2),
        edited.clone(),
        usize::MAX,
    )
    .expect("replay cold comparison starts");
    let replay_expected = replay_cold.cold().expect("replay cold comparison");
    assert_eq!(replayed.dvi_bytes(), replay_expected.dvi_bytes());
    assert_eq!(replayed.effects, replay_expected.effects);

    let old = source.find("old").expect("old payload body");
    let invalidation_before = session.pure_memo_stats();
    let incremental = session
        .advance(
            RevisionId::new(3),
            Edit {
                base_revision: RevisionId::new(2),
                expected_hash: ContentHash::from_bytes(edited.as_bytes()),
                range: old..old + "old".len(),
                replacement: "new".to_owned(),
            },
        )
        .expect("payload meaning edit");
    assert!(
        session.pure_memo_stats().paragraph_validation_misses
            > invalidation_before.paragraph_validation_misses,
        "changing the meaning used to construct vadjust must invalidate reuse"
    );

    let redefined = format!("{}new{}", &edited[..old], &edited[old + "old".len()..]);
    let mut cold_universe = template();
    cold_universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let mut cold = Session::start(
        cold_universe,
        "paragraph-vadjust-replay",
        RevisionId::new(3),
        redefined,
        usize::MAX,
    )
    .expect("cold comparison starts");
    let expected = cold.cold().expect("cold comparison");
    assert_eq!(incremental.dvi_bytes(), expected.dvi_bytes());
    assert_eq!(incremental.effects, expected.effects);
}

#[test]
fn paragraph_box_reads_without_an_active_local_definition_remain_barriered() {
    let assert_barrier = |name: &str, source: &str| {
        let mut universe = template();
        universe.enable_pure_memo(tex_state::PureMemoConfig::default());
        let mut session = Session::start(universe, name, RevisionId::new(1), source, usize::MAX)
            .expect("session starts");
        session.cold().expect("cold revision");
        let stats = session.pure_memo_stats();
        assert!(
            stats.paragraph_unsupported_write_barriers > 0,
            "{name} must remain a barrier: {stats:?}"
        );
    };
    assert_barrier(
        "paragraph-outer-box-consumption",
        "\\setbox0=\\hbox{outer}\nouter box \\box0 paragraph\\par\n\\vfill\\eject\\end",
    );
    assert_barrier(
        "paragraph-outer-box-unbox",
        "\\setbox0=\\hbox{outer}\nouter box \\unhbox0 paragraph\\par\n\\vfill\\eject\\end",
    );
    assert_barrier(
        "paragraph-global-box-definition",
        "global box {\\global\\setbox1=\\hbox{global} \\box1} paragraph\\par\n\\vfill\\eject\\end",
    );
    assert_barrier(
        "paragraph-entry-box-definition",
        "entry box \\setbox2=\\hbox{entry} \\box2 paragraph\\par\n\\vfill\\eject\\end",
    );
}

#[test]
fn nested_local_let_tracks_the_rhs_meaning() {
    let mut universe = template();
    universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let source = concat!(
        "\\def\\original{old}\n",
        "changed prefix paragraph text\\par\n",
        "nested local {\\let\\temporary=\\original \\temporary} meaning paragraph\\par\n",
        "stable suffix paragraph\\par\n",
        "\\vfill\\eject\\end",
    );
    let mut session = Session::start(
        universe,
        "paragraph-nested-local-let-read",
        RevisionId::new(1),
        source,
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("cold revision");
    let changed = source.find("old").expect("old macro body");
    let edited = format!(
        "{}new{}",
        &source[..changed],
        &source[changed + "old".len()..]
    );
    let incremental = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(source.as_bytes()),
                range: changed..changed + "old".len(),
                replacement: "new".to_owned(),
            },
        )
        .expect("macro-body edit");
    let mut cold_universe = template();
    cold_universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let mut cold = Session::start(
        cold_universe,
        "paragraph-nested-local-let-read",
        RevisionId::new(2),
        edited,
        usize::MAX,
    )
    .expect("cold comparison starts");
    let expected = cold.cold().expect("cold comparison");
    assert_eq!(incremental.dvi_bytes(), expected.dvi_bytes());
    assert_eq!(incremental.effects, expected.effects);
}

#[test]
fn box_local_definition_does_not_hide_a_later_outer_meaning_read() {
    let mut universe = template();
    universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let source = concat!(
        "\\def\\word{old}\n",
        "changed prefix paragraph text\\par\n",
        "boxed \\hbox{\\def\\word{inner}\\word} then outer \\word paragraph\\par\n",
        "stable suffix paragraph\\par\n",
        "\\vfill\\eject\\end",
    );
    let mut session = Session::start(
        universe,
        "paragraph-box-local-meaning-scope",
        RevisionId::new(1),
        source,
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("cold revision");
    assert_eq!(
        session
            .pure_memo_stats()
            .paragraph_unsupported_write_barriers,
        0,
        "the balanced box-local definition should not barrier the paragraph"
    );

    let changed = source.find("old").expect("old macro body");
    let edited = format!(
        "{}new{}",
        &source[..changed],
        &source[changed + "old".len()..]
    );
    let before = session.pure_memo_stats();
    let incremental = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(source.as_bytes()),
                range: changed..changed + "old".len(),
                replacement: "new".to_owned(),
            },
        )
        .expect("macro-body edit");
    assert!(
        session.pure_memo_stats().paragraph_validation_misses > before.paragraph_validation_misses,
        "the changed outer meaning should invalidate an eligible paragraph"
    );

    let mut cold_universe = template();
    cold_universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let mut cold = Session::start(
        cold_universe,
        "paragraph-box-local-meaning-scope",
        RevisionId::new(2),
        edited,
        usize::MAX,
    )
    .expect("cold comparison starts");
    let expected = cold.cold().expect("cold comparison");
    assert_eq!(incremental.dvi_bytes(), expected.dvi_bytes());
    assert_eq!(incremental.effects, expected.effects);
}

#[test]
fn paragraph_with_entry_depth_arithmetic_remains_barriered() {
    let mut universe = template();
    universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let source = concat!(
        "\\dimen0=3pt\n",
        "changed prefix paragraph text\\par\n",
        "entry arithmetic \\advance\\dimen0 by 2pt paragraph\\par\n",
        "\\hrule height\\dimen0\n",
        "\\vfill\\eject\\end",
    );
    let mut session = Session::start(
        universe,
        "paragraph-entry-depth-arithmetic",
        RevisionId::new(1),
        source,
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("cold revision");
    assert!(
        session
            .pure_memo_stats()
            .paragraph_unsupported_write_barriers
            > 0,
        "entry-depth arithmetic must remain an escaping-write barrier"
    );
}

#[test]
fn paragraph_with_global_assignment_inside_nested_group_remains_barriered() {
    let mut universe = template();
    universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let source = concat!(
        "changed prefix paragraph text\\par\n",
        "nested global {\\global\\advance\\dimen0 by 5pt} assignment paragraph\\par\n",
        "\\hrule height\\dimen0\n",
        "\\vfill\\eject\\end",
    );
    let mut session = Session::start(
        universe,
        "paragraph-nested-global-assignment",
        RevisionId::new(1),
        source,
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("cold revision");
    assert!(
        session
            .pure_memo_stats()
            .paragraph_unsupported_write_barriers
            > 0,
        "a nested global write must remain visible after group exit"
    );
}

#[test]
fn paragraph_with_unsupported_future_write_executes_cold() {
    let mut universe = template();
    universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let source = concat!(
        "changed prefix paragraph text\\par\n",
        "future write paragraph text \\dimen0=5pt\\par\n",
        "\\hrule height\\dimen0\n",
        "stable suffix paragraph text\\par\n",
        "\\vfill\\eject\\end",
    );
    let mut session = Session::start(
        universe,
        "paragraph-unsupported-future-write",
        RevisionId::new(1),
        source,
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("cold revision");
    assert!(
        session
            .pure_memo_stats()
            .paragraph_unsupported_write_barriers
            > 0
    );

    let changed = source.find("changed").expect("changed word");
    let edited = format!(
        "{}altered{}",
        &source[..changed],
        &source[changed + "changed".len()..]
    );
    let incremental = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(source.as_bytes()),
                range: changed..changed + "changed".len(),
                replacement: "altered".to_owned(),
            },
        )
        .expect("prefix edit");

    let mut cold_universe = template();
    cold_universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let mut cold = Session::start(
        cold_universe,
        "paragraph-unsupported-future-write",
        RevisionId::new(2),
        edited,
        usize::MAX,
    )
    .expect("cold comparison starts");
    let expected = cold.cold().expect("cold comparison");
    assert_eq!(incremental.dvi_bytes(), expected.dvi_bytes());
    assert_eq!(incremental.effects, expected.effects);
}

#[test]
fn paragraph_consuming_vertical_afterassignment_executes_cold() {
    let mut universe = template();
    universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let source = concat!(
        "\\def\\setfuture{\\dimen0=5pt}\n",
        "changed prefix paragraph text\\par\n",
        "\\afterassignment\\setfuture\n",
        "afterassignment paragraph \\count0=1 text\\par\n",
        "\\hrule height\\dimen0\n",
        "stable suffix paragraph text\\par\n",
        "\\vfill\\eject\\end",
    );
    let mut session = Session::start(
        universe,
        "paragraph-afterassignment-consumption",
        RevisionId::new(1),
        source,
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("cold revision");
    assert!(
        session
            .pure_memo_stats()
            .paragraph_unsupported_write_barriers
            > 0
    );

    let changed = source.find("changed").expect("changed word");
    let edited = format!(
        "{}altered{}",
        &source[..changed],
        &source[changed + "changed".len()..]
    );
    let incremental = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(source.as_bytes()),
                range: changed..changed + "changed".len(),
                replacement: "altered".to_owned(),
            },
        )
        .expect("prefix edit");

    let mut cold_universe = template();
    cold_universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let mut cold = Session::start(
        cold_universe,
        "paragraph-afterassignment-consumption",
        RevisionId::new(2),
        edited,
        usize::MAX,
    )
    .expect("cold comparison starts");
    let expected = cold.cold().expect("cold comparison");
    assert_eq!(incremental.dvi_bytes(), expected.dvi_bytes());
    assert_eq!(incremental.effects, expected.effects);
}

#[test]
fn paragraph_consuming_shifted_box_register_executes_cold() {
    let mut universe = template();
    universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let source = concat!(
        "\\setbox0=\\hbox{saved box}\n",
        "changed prefix paragraph text\\par\n",
        "box-consuming paragraph \\raise1pt\\box0 text\\par\n",
        "\\ifvoid0 void\\else nonvoid\\fi\\par\n",
        "\\vfill\\eject\\end",
    );
    let mut session = Session::start(
        universe,
        "paragraph-box-consumption",
        RevisionId::new(1),
        source,
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("cold revision");
    assert!(
        session
            .pure_memo_stats()
            .paragraph_unsupported_write_barriers
            > 0
    );

    let changed = source.find("changed").expect("changed word");
    let edited = format!(
        "{}altered{}",
        &source[..changed],
        &source[changed + "changed".len()..]
    );
    let incremental = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(source.as_bytes()),
                range: changed..changed + "changed".len(),
                replacement: "altered".to_owned(),
            },
        )
        .expect("prefix edit");

    let mut cold_universe = template();
    cold_universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let mut cold = Session::start(
        cold_universe,
        "paragraph-box-consumption",
        RevisionId::new(2),
        edited,
        usize::MAX,
    )
    .expect("cold comparison starts");
    let expected = cold.cold().expect("cold comparison");
    assert_eq!(incremental.dvi_bytes(), expected.dvi_bytes());
    assert_eq!(incremental.effects, expected.effects);
}

#[test]
fn paragraph_replay_restores_final_line_badness() {
    let mut universe = template();
    universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let source = concat!(
        "\\hsize=55pt\n",
        "changed prefix with several words of changing width\\par\n",
        "\\parfillskip=0pt x\\par\n",
        "reported badness \\the\\badness\\par\n",
        "\\vfill\\eject\\end",
    );
    let mut session = Session::start(
        universe,
        "paragraph-last-badness",
        RevisionId::new(1),
        source,
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("cold revision");

    let changed = source.find("changed").expect("changed word");
    let edited = format!(
        "{}altered{}",
        &source[..changed],
        &source[changed + "changed".len()..]
    );
    let before = session.pure_memo_stats().paragraph_line_hits;
    let incremental = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(source.as_bytes()),
                range: changed..changed + "changed".len(),
                replacement: "altered".to_owned(),
            },
        )
        .expect("prefix edit");
    assert!(session.pure_memo_stats().paragraph_line_hits > before);

    let mut cold_universe = template();
    cold_universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let mut cold = Session::start(
        cold_universe,
        "paragraph-last-badness",
        RevisionId::new(2),
        edited,
        usize::MAX,
    )
    .expect("cold comparison starts");
    let expected = cold.cold().expect("cold comparison");
    assert_eq!(incremental.dvi_bytes(), expected.dvi_bytes());
    assert_eq!(incremental.effects, expected.effects);
}

#[test]
fn paragraph_with_inline_math_replays_with_explicit_math_dependencies() {
    let mut universe = template();
    universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let source = concat!(
        "changed prefix paragraph text\\par\n",
        "inline math $\\fam=0 x+y$ paragraph text\\par\n",
        "stable suffix paragraph text\\par\n",
        "\\vfill\\eject\\end",
    );
    let mut session = Session::start(
        universe,
        "paragraph-inline-math-replay",
        RevisionId::new(1),
        source,
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("cold revision");
    let inline_region = session
        .pure_memo
        .accepted_paragraphs()
        .iter()
        .find(|region| {
            region.dependencies().any(|dependency| {
                matches!(
                    dependency.key,
                    tex_state::DependencyKey::Code {
                        table: tex_state::DependencyCodeTable::Mathcode,
                        ..
                    }
                )
            })
        })
        .expect("inline paragraph records exact math-code reads");
    assert!(inline_region.dependencies().all(|dependency| {
        !matches!(
            dependency.key,
            tex_state::DependencyKey::CodeGeneration(
                tex_state::DependencyCodeTable::Mathcode | tex_state::DependencyCodeTable::Delcode
            )
        )
    }));
    let family_dependencies = inline_region
        .dependencies()
        .filter(|dependency| {
            matches!(
                dependency.key,
                tex_state::DependencyKey::Cell {
                    bank: tex_state::DependencyBank::MathFamilyFont,
                    ..
                }
            )
        })
        .count();
    // This fixture has no installed math fonts, so TeX replaces the formula
    // after checking the six family-2/family-3 parameter bindings. The exact
    // projection must not retain the other 42 family cells.
    assert_eq!(family_dependencies, 6);
    let before = session.pure_memo_stats();
    assert_eq!(before.paragraph_display_math_barriers, 0, "{before:?}");

    let changed = source.find("changed").expect("changed word");
    let edited = format!(
        "{}altered{}",
        &source[..changed],
        &source[changed + "changed".len()..]
    );
    let incremental = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(source.as_bytes()),
                range: changed..changed + "changed".len(),
                replacement: "altered".to_owned(),
            },
        )
        .expect("prefix edit");
    let after = session.pure_memo_stats();
    assert!(after.paragraph_hits > before.paragraph_hits, "{after:?}");

    let mut cold_universe = template();
    cold_universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let mut cold = Session::start(
        cold_universe,
        "paragraph-inline-math-replay",
        RevisionId::new(2),
        edited,
        usize::MAX,
    )
    .expect("cold comparison starts");
    let expected = cold.cold().expect("cold comparison");
    assert_eq!(incremental.dvi_bytes(), expected.dvi_bytes());
    assert_eq!(incremental.effects, expected.effects);
}

#[test]
fn inline_math_dependency_change_rejects_retained_lines() {
    let mut universe = template();
    universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let source = concat!(
        "\\mathsurround=0pt\n",
        "prefix paragraph text\\par\n",
        "inline math $x+y$ paragraph text\\par\n",
        "stable suffix paragraph text\\par\n",
        "\\vfill\\eject\\end",
    );
    let mut session = Session::start(
        universe,
        "paragraph-inline-math-dependency",
        RevisionId::new(1),
        source,
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("cold revision");
    let before = session.pure_memo_stats();

    let value = source.find("0pt").expect("math surround value");
    let edited = format!("{}5{}", &source[..value], &source[value + 1..]);
    let incremental = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(source.as_bytes()),
                range: value..value + 1,
                replacement: "5".to_owned(),
            },
        )
        .expect("math dependency edit");
    let after = session.pure_memo_stats();
    assert!(
        after.paragraph_validation_misses > before.paragraph_validation_misses,
        "{after:?}"
    );

    let mut cold_universe = template();
    cold_universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let mut cold = Session::start(
        cold_universe,
        "paragraph-inline-math-dependency",
        RevisionId::new(2),
        edited,
        usize::MAX,
    )
    .expect("cold comparison starts");
    let expected = cold.cold().expect("cold comparison");
    assert_eq!(incremental.dvi_bytes(), expected.dvi_bytes());
    assert_eq!(incremental.effects, expected.effects);
}

#[test]
fn inline_math_family_binding_change_rejects_retained_lines() {
    let mut universe = template();
    universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let source = concat!(
        "\\font\\matha=cmr10 \\font\\mathb=cmtt10 ",
        "\\font\\mathsy=cmsy10 \\font\\mathex=cmex10\n",
        "\\textfont2=\\mathsy \\scriptfont2=\\mathsy ",
        "\\scriptscriptfont2=\\mathsy\n",
        "\\textfont3=\\mathex \\scriptfont3=\\mathex ",
        "\\scriptscriptfont3=\\mathex\n",
        "prefix paragraph \\textfont0=\\matha text\\par\n",
        "inline math $\\mathchar\"0078+\\mathchar\"0079$ paragraph text\\par\n",
        "stable suffix paragraph text\\par\n",
        "\\vfill\\eject\\end",
    );
    let mut session = Session::start(
        universe,
        "paragraph-inline-math-family",
        RevisionId::new(1),
        source,
        usize::MAX,
    )
    .expect("session starts");
    session
        .register_input_file(Path::new("cmr10.tfm"), CMR10.to_vec())
        .expect("roman font fixture");
    session
        .register_input_file(Path::new("cmtt10.tfm"), CMTT10.to_vec())
        .expect("typewriter font fixture");
    session
        .register_input_file(Path::new("cmsy10.tfm"), CMSY10.to_vec())
        .expect("symbol font fixture");
    session
        .register_input_file(Path::new("cmex10.tfm"), CMEX10.to_vec())
        .expect("extension font fixture");
    session.cold().expect("cold revision");
    let before = session.pure_memo_stats();
    let inline_region = session
        .pure_memo
        .accepted_paragraphs()
        .iter()
        .find(|region| {
            region.dependencies().any(|dependency| {
                matches!(
                    dependency.key,
                    tex_state::DependencyKey::Cell {
                        bank: tex_state::DependencyBank::MathFamilyFont,
                        index: 0,
                    }
                )
            })
        })
        .expect("inline paragraph records its text-family binding");
    assert!(inline_region.barriers.is_empty());

    let font = source
        .find("textfont0=\\matha")
        .expect("text font assignment")
        + "textfont0=\\math".len();
    let edited = format!("{}b{}", &source[..font], &source[font + 1..]);
    let incremental = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(source.as_bytes()),
                range: font..font + 1,
                replacement: "b".to_owned(),
            },
        )
        .expect("math-family binding edit");
    let after = session.pure_memo_stats();
    assert!(
        after.paragraph_validation_misses > before.paragraph_validation_misses,
        "the changed family binding must invalidate the inline paragraph: {after:?}"
    );

    let mut cold_universe = template();
    cold_universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let mut cold = Session::start(
        cold_universe,
        "paragraph-inline-math-family",
        RevisionId::new(2),
        edited,
        usize::MAX,
    )
    .expect("cold comparison starts");
    cold.register_input_file(Path::new("cmr10.tfm"), CMR10.to_vec())
        .expect("roman font fixture");
    cold.register_input_file(Path::new("cmtt10.tfm"), CMTT10.to_vec())
        .expect("typewriter font fixture");
    cold.register_input_file(Path::new("cmsy10.tfm"), CMSY10.to_vec())
        .expect("symbol font fixture");
    cold.register_input_file(Path::new("cmex10.tfm"), CMEX10.to_vec())
        .expect("extension font fixture");
    let expected = cold.cold().expect("cold comparison");
    assert_eq!(incremental.dvi_bytes(), expected.dvi_bytes());
    assert_eq!(incremental.effects, expected.effects);
}

#[test]
fn paragraph_replay_enters_display_continuation_after_skipped_delimiters() {
    let mut universe = template();
    universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let source = concat!(
        "changed paragraph text\\par\n",
        "text before display $$$$ text after display\\par\n",
        "stable suffix paragraph\\par\n",
        "\\vfill\\eject\\end",
    );
    let mut session = Session::start(
        universe,
        "paragraph-display-continuation",
        RevisionId::new(1),
        source.to_owned(),
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("cold revision");
    let before = session.pure_memo_stats();
    assert_eq!(before.paragraph_display_math_barriers, 0, "{before:?}");

    let changed = source.find("changed").expect("changed word");
    let edited = format!(
        "{}altered{}",
        &source[..changed],
        &source[changed + "changed".len()..]
    );
    let incremental = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(source.as_bytes()),
                range: changed..changed + "changed".len(),
                replacement: "altered".to_owned(),
            },
        )
        .expect("prefix paragraph edit");
    let after = session.pure_memo_stats();
    assert!(after.paragraph_hits > before.paragraph_hits, "{after:?}");

    let mut cold_universe = template();
    cold_universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let mut cold = Session::start(
        cold_universe,
        "paragraph-display-continuation",
        RevisionId::new(2),
        edited,
        usize::MAX,
    )
    .expect("cold comparison starts");
    let expected = cold.cold().expect("cold comparison");
    assert_eq!(incremental.dvi_bytes(), expected.dvi_bytes());
    assert_eq!(incremental.effects, expected.effects);
}

#[test]
fn display_continuation_replay_rebinds_introduced_macro_token_list() {
    let mut universe = template();
    universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let removed = "\\def\\unused{allocation slot}\\n";
    let source = format!(
        "{removed}{}",
        concat!(
            "\\def\\showdisplay{$$$$}\n",
            "prefix paragraph text\\par\n",
            "text before display \\showdisplay text after display\\par\n",
            "stable suffix paragraph\\par\n",
            "\\vfill\\eject\\end",
        )
    );
    let mut session = Session::start(
        universe,
        "paragraph-display-token-list-rebind",
        RevisionId::new(1),
        source.clone(),
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("cold revision");
    let before = session.pure_memo_stats();

    let incremental = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(source.as_bytes()),
                range: 0..removed.len(),
                replacement: String::new(),
            },
        )
        .expect("token-list allocation-shifting edit");
    let after = session.pure_memo_stats();
    assert!(after.paragraph_hits > before.paragraph_hits, "{after:?}");

    let edited = &source[removed.len()..];
    let mut cold_universe = template();
    cold_universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let mut cold = Session::start(
        cold_universe,
        "paragraph-display-token-list-rebind",
        RevisionId::new(2),
        edited.to_owned(),
        usize::MAX,
    )
    .expect("cold comparison starts");
    let expected = cold.cold().expect("cold comparison");
    assert_eq!(incremental.dvi_bytes(), expected.dvi_bytes());
    assert_eq!(incremental.effects, expected.effects);
}

#[test]
fn root_compacted_paragraph_does_not_replay_after_entering_a_live_group() {
    let mut universe = template();
    universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let target = "ownership paragraph \\global\\count0=1 \\count0=0 text\\par\n";
    let source = format!(
        "prefix paragraph text\\par\n{target}\\endgroup\nroot value \\the\\count0\\par\n\\vfill\\eject\\end"
    );
    let mut session = Session::start(
        universe,
        "paragraph-root-to-live-group",
        RevisionId::new(1),
        source.clone(),
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("cold revision");

    let insertion = source.find(target).expect("target paragraph");
    let before = session.pure_memo_stats();
    let incremental = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(source.as_bytes()),
                range: insertion..insertion,
                replacement: "\\begingroup\n".to_owned(),
            },
        )
        .expect("group insertion");
    let after = session.pure_memo_stats();
    assert!(
        after.paragraph_validation_misses > before.paragraph_validation_misses,
        "the root-compacted record must be rejected at live-group entry: {after:?}"
    );

    let edited = format!(
        "{}\\begingroup\n{}",
        &source[..insertion],
        &source[insertion..]
    );
    let mut cold_universe = template();
    cold_universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let mut cold = Session::start(
        cold_universe,
        "paragraph-root-to-live-group",
        RevisionId::new(2),
        edited,
        usize::MAX,
    )
    .expect("cold comparison starts");
    let expected = cold.cold().expect("cold comparison");
    assert_eq!(incremental.dvi_bytes(), expected.dvi_bytes());
    assert_eq!(incremental.effects, expected.effects);
}

#[test]
fn paragraph_replays_exact_live_group_ownership_script() {
    let mut universe = template();
    universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let source = concat!(
        "\\begingroup\n",
        "changed prefix paragraph text\\par\n",
        "ownership paragraph \\global\\count0=1 \\count0=0 text\\par\n",
        "\\endgroup\n",
        "root value \\the\\count0\\par\n",
        "\\vfill\\eject\\end",
    );
    let mut session = Session::start(
        universe,
        "paragraph-live-group-ownership",
        RevisionId::new(1),
        source,
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("cold revision");
    assert_eq!(
        session
            .pure_memo_stats()
            .paragraph_unsupported_group_transition_barriers,
        0
    );

    let changed = source.find("changed").expect("changed word");
    let edited = format!(
        "{}altered{}",
        &source[..changed],
        &source[changed + "changed".len()..]
    );
    let incremental = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(source.as_bytes()),
                range: changed..changed + "changed".len(),
                replacement: "altered".to_owned(),
            },
        )
        .expect("prefix edit");
    assert!(session.pure_memo_stats().paragraph_line_hits > 0);

    let mut cold_universe = template();
    cold_universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let mut cold = Session::start(
        cold_universe,
        "paragraph-live-group-ownership",
        RevisionId::new(2),
        edited,
        usize::MAX,
    )
    .expect("cold comparison starts");
    let expected = cold.cold().expect("cold comparison");
    assert_eq!(incremental.dvi_bytes(), expected.dvi_bytes());
    assert_eq!(incremental.effects, expected.effects);
}

#[test]
fn paragraph_front_end_keys_macro_paragraphs_before_expansion() {
    let mut universe = template();
    universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let source = "\\def\\body#1{macro #1 paragraph text}\n\\body{one}\\par\n\\body{two}\\par\n\\body{three}\\par\n\\vfill\\eject\\end";
    let mut session = Session::start(
        universe,
        "paragraph-macro-raw-key",
        RevisionId::new(1),
        source.to_owned(),
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("cold revision");
    let before = session.pure_memo_stats();

    let inserted = "% unchanged macro paragraphs keep fragment identity\n";
    let incremental = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(source.as_bytes()),
                range: 0..0,
                replacement: inserted.to_owned(),
            },
        )
        .expect("prefix edit");
    let after = session.pure_memo_stats();
    assert!(
        after.paragraph_lookups >= before.paragraph_lookups + 3,
        "{after:?}"
    );
    assert!(
        after.paragraph_hits >= before.paragraph_hits + 2,
        "{after:?}"
    );

    let mut cold_universe = template();
    cold_universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let mut cold = Session::start(
        cold_universe,
        "paragraph-macro-raw-key",
        RevisionId::new(2),
        format!("{inserted}{source}"),
        usize::MAX,
    )
    .expect("cold comparison");
    let cold_output = cold.cold().expect("cold edited revision");
    assert_eq!(
        incremental.dvi_bytes().expect("incremental DVI"),
        cold_output.dvi_bytes().expect("cold DVI")
    );
}

#[test]
fn paragraph_macro_frame_transitions_replay_across_carried_generations() {
    let mut universe = template();
    universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let source = concat!(
        "\\def\\body{first macro paragraph\\par second macro paragraph\\par third macro paragraph\\par}\n",
        "\\body\\vfill\\eject\\end",
    );
    let mut session = Session::start(
        universe,
        "paragraph-macro-frame-transition",
        RevisionId::new(1),
        source.to_owned(),
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("cold revision");

    let first_prefix = "% first prefix edit\n";
    let before_first = session.pure_memo_stats();
    session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(source.as_bytes()),
                range: 0..0,
                replacement: first_prefix.to_owned(),
            },
        )
        .expect("first header edit");
    let after_first = session.pure_memo_stats();
    assert!(
        after_first.paragraph_hits >= before_first.paragraph_hits + 2,
        "the unchanged macro-body suffix should replay: {after_first:?}"
    );

    let second_source = format!("{first_prefix}{source}");
    let second_prefix = "% second prefix edit\n";
    let before_second = session.pure_memo_stats();
    let incremental = session
        .advance(
            RevisionId::new(3),
            Edit {
                base_revision: RevisionId::new(2),
                expected_hash: ContentHash::from_bytes(second_source.as_bytes()),
                range: 0..0,
                replacement: second_prefix.to_owned(),
            },
        )
        .expect("second header edit");
    let after_second = session.pure_memo_stats();
    assert!(
        after_second.paragraph_hits >= before_second.paragraph_hits + 2,
        "carried input recipes must ignore revision-local token-list handles: {after_second:?}"
    );

    let third_source = format!("{second_prefix}{second_source}");
    let mut cold_universe = template();
    cold_universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let mut cold = Session::start(
        cold_universe,
        "paragraph-macro-frame-transition",
        RevisionId::new(3),
        third_source,
        usize::MAX,
    )
    .expect("cold comparison starts");
    let expected = cold.cold().expect("cold comparison");
    assert_eq!(incremental.dvi_bytes(), expected.dvi_bytes());
    assert_eq!(incremental.effects, expected.effects);
}

#[test]
fn macro_started_group_transitions_are_paragraph_replay_barriers() {
    for (name, source) in [
        (
            "macro-open-group",
            concat!(
                "\\def\\body{macro text \\begingroup grouped paragraph text\\par}\n",
                "\\body\\endgroup\n",
                "\\vfill\\eject\\end",
            ),
        ),
        (
            "macro-aftergroup-entry-frame",
            concat!(
                "\\def\\mark{\\global\\count0=7}\n",
                "\\def\\body{macro text \\aftergroup\\mark aftergroup paragraph text\\par}\n",
                "\\begingroup\\body\\endgroup\n",
                "\\vfill\\eject\\end",
            ),
        ),
    ] {
        let mut universe = template();
        universe.enable_pure_memo(tex_state::PureMemoConfig::default());
        let mut session = Session::start(
            universe,
            name,
            RevisionId::new(1),
            source.to_owned(),
            usize::MAX,
        )
        .expect("session starts");
        session.cold().expect("cold revision");

        let stats = session.pure_memo_stats();
        assert_eq!(
            stats.paragraph_unsupported_group_transition_barriers, 1,
            "{name}: {stats:?}"
        );
        assert!(
            session.pure_memo.accepted_paragraphs().is_empty(),
            "{name}: a macro-started group transition must not publish a replay record"
        );
    }
}

#[test]
fn rooted_paragraph_replays_with_a_live_macro_suffix() {
    let mut universe = template();
    universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let source = concat!(
        "\\def\\tail{macro ending text\\par second macro paragraph\\par}\n",
        "root paragraph begins here \\tail",
        "\\vfill\\eject\\end",
    );
    let mut session = Session::start(
        universe,
        "rooted-paragraph-macro-suffix",
        RevisionId::new(1),
        source.to_owned(),
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("cold revision");

    let prefix = "% preserve rooted paragraph pieces\n";
    let before = session.pure_memo_stats();
    let incremental = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(source.as_bytes()),
                range: 0..0,
                replacement: prefix.to_owned(),
            },
        )
        .expect("prefix edit");
    let after = session.pure_memo_stats();
    assert!(
        after.paragraph_line_hits >= before.paragraph_line_hits + 2,
        "both the rooted paragraph and its live macro suffix should replay: {after:?}"
    );

    let mut cold_universe = template();
    cold_universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let mut cold = Session::start(
        cold_universe,
        "rooted-paragraph-macro-suffix",
        RevisionId::new(2),
        format!("{prefix}{source}"),
        usize::MAX,
    )
    .expect("cold comparison starts");
    let expected = cold.cold().expect("cold comparison");
    assert_eq!(incremental.dvi_bytes(), expected.dvi_bytes());
    assert_eq!(incremental.effects, expected.effects);
}

#[test]
fn paragraph_mode_reads_are_discharged_by_the_replay_boundary() {
    let mut universe = template();
    universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let source = concat!(
        "\\def\\entrymode{\\ifvmode vertical\\else wrong\\fi}\n",
        "\\entrymode \\ifhmode horizontal\\else wrong\\fi paragraph\\par\n",
        "stable suffix paragraph\\par\n",
        "\\vfill\\eject\\end",
    );
    let mut session = Session::start(
        universe,
        "paragraph-structural-mode",
        RevisionId::new(1),
        source.to_owned(),
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("cold revision");

    let prefix = "% preserve mode-sensitive paragraphs\n";
    let before = session.pure_memo_stats();
    let incremental = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(source.as_bytes()),
                range: 0..0,
                replacement: prefix.to_owned(),
            },
        )
        .expect("prefix edit");
    assert!(
        session.pure_memo_stats().paragraph_line_hits >= before.paragraph_line_hits + 2,
        "mode-sensitive paragraphs should replay from the same vertical boundary"
    );

    let mut cold_universe = template();
    cold_universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let mut cold = Session::start(
        cold_universe,
        "paragraph-structural-mode",
        RevisionId::new(2),
        format!("{prefix}{source}"),
        usize::MAX,
    )
    .expect("cold comparison starts");
    let expected = cold.cold().expect("cold comparison");
    assert_eq!(incremental.dvi_bytes(), expected.dvi_bytes());
    assert_eq!(incremental.effects, expected.effects);
}

#[test]
fn paragraph_front_end_rejects_changed_raw_span_before_reusing_later_macros() {
    let mut universe = template();
    universe.enable_pure_memo(all_memo_layers());
    let source = "\\def\\body#1{macro #1 paragraph text}\n\\body{one}\\par\n\\body{two\ncontinued}\\par\n\\body{three}\\par\n\\vfill\\eject\\end";
    let mut session = Session::start(
        universe,
        "paragraph-macro-span-validation",
        RevisionId::new(1),
        source.to_owned(),
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("cold revision");
    let before = session.pure_memo_stats();
    let start = source.find("continued").expect("changed continuation");

    let incremental = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(source.as_bytes()),
                range: start..start + "continued".len(),
                replacement: "altered".to_owned(),
            },
        )
        .expect("middle-of-paragraph edit");
    let after = session.pure_memo_stats();
    assert!(
        after.paragraph_hits == before.paragraph_hits + 1 || incremental.reuse.suffixes_adopted > 0,
        "the changed paragraph must miss, then the later stable work must be reused by paragraph memo or exact suffix adoption: {after:?}"
    );

    let mut edited = source.to_owned();
    edited.replace_range(start..start + "continued".len(), "altered");
    let mut cold_universe = template();
    cold_universe.enable_pure_memo(all_memo_layers());
    let mut cold = Session::start(
        cold_universe,
        "paragraph-macro-span-validation",
        RevisionId::new(2),
        edited,
        usize::MAX,
    )
    .expect("cold comparison");
    let cold_output = cold.cold().expect("cold edited revision");
    assert_eq!(incremental.effects, cold_output.effects);
    assert_eq!(
        incremental.dvi_bytes().expect("incremental DVI"),
        cold_output.dvi_bytes().expect("cold DVI")
    );
}

#[test]
fn paragraph_post_break_reuse_tiers_match_cold_for_layout_and_hyphenation_changes() {
    fn run_edit(
        source: &str,
        range: std::ops::Range<usize>,
        replacement: &str,
        prepare: fn(&mut Universe),
    ) -> (tex_state::PureMemoStats, Vec<u8>) {
        let mut universe = template();
        prepare(&mut universe);
        universe.enable_pure_memo(tex_state::PureMemoConfig::default());
        let mut session = Session::start(
            universe,
            "paragraph-post-break-tiers",
            RevisionId::new(1),
            source,
            usize::MAX,
        )
        .expect("session starts");
        session
            .register_input_file(Path::new("cmr10.tfm"), CMR10.to_vec())
            .expect("font fixture");
        session.cold().expect("cold paragraph generation");
        let edited = format!(
            "{}{}{}",
            &source[..range.start],
            replacement,
            &source[range.end..]
        );
        let output = session
            .advance(
                RevisionId::new(2),
                Edit {
                    base_revision: RevisionId::new(1),
                    expected_hash: ContentHash::from_bytes(source.as_bytes()),
                    range,
                    replacement: replacement.to_owned(),
                },
            )
            .expect("edited generation");
        let mut cold_universe = template();
        prepare(&mut cold_universe);
        cold_universe.enable_pure_memo(tex_state::PureMemoConfig::default());
        let mut cold = Session::start(
            cold_universe,
            "paragraph-post-break-tiers",
            RevisionId::new(2),
            edited,
            usize::MAX,
        )
        .expect("cold comparison starts");
        cold.register_input_file(Path::new("cmr10.tfm"), CMR10.to_vec())
            .expect("cold font fixture");
        let expected = cold.cold().expect("cold comparison");
        let actual = output.dvi_bytes().expect("incremental DVI");
        assert_eq!(actual, expected.dvi_bytes().expect("cold DVI"));
        assert_eq!(output.effects, expected.effects);
        assert_eq!(output.artifacts, expected.artifacts);
        (session.pure_memo_stats(), actual)
    }

    let prose = "hyphenation demonstration hyphenation demonstration hyphenation demonstration";
    let source = format!(
        "\\font\\tenrm=cmr10\\relax \\tenrm \\hsize=70pt \\vsize=1000pt \\hyphenation{{hy-phen-a-tion}}\n{prose}\\par\n{prose}\\par\n\\end"
    );
    let hsize = source.find("70pt").expect("hsize value");
    let (layout, _) = run_edit(&source, hsize..hsize + 2, "45", |_| {});
    assert_eq!(layout.paragraph_hits, 0, "{layout:?}");
    assert_eq!(layout.paragraph_line_hits, 0, "{layout:?}");
    assert_eq!(
        layout.paragraph_validation_failure_reasons
            [tex_state::ParagraphValidationFailure::BreakDependency as usize],
        1,
        "{layout:?}"
    );

    let hyphens = source.find("hy-phen-a-tion").expect("exception");
    let (hyphenation, _) = run_edit(
        &source,
        hyphens..hyphens + "hy-phen-a-tion".len(),
        "hyphen-ation",
        |_| {},
    );
    assert_eq!(hyphenation.paragraph_hits, 0, "{hyphenation:?}");
    assert_eq!(hyphenation.paragraph_line_hits, 0, "{hyphenation:?}");
    assert_eq!(
        hyphenation.paragraph_validation_failure_reasons
            [tex_state::ParagraphValidationFailure::BreakDependency as usize],
        1,
        "{hyphenation:?}"
    );

    let insertion = source.find(prose).expect("first paragraph");
    let (full, _) = run_edit(&source, insertion..insertion, "\\count77=1 ", |_| {});
    assert!(full.paragraph_line_hits > 0, "{full:?}");

    let font_dimen_source = format!(
        "\\font\\tenrm=cmr10\\relax \\fontdimen2\\tenrm=3pt \\tenrm \\hsize=70pt\n{prose}\\par\n{prose}\\par\n\\end"
    );
    let font_dimen = font_dimen_source.find("3pt").expect("font dimension value");
    let (font_parameter, _) = run_edit(&font_dimen_source, font_dimen..font_dimen + 1, "9", |_| {});
    assert_eq!(font_parameter.paragraph_line_hits, 0, "{font_parameter:?}");
    assert_eq!(
        font_parameter.paragraph_validation_failure_reasons
            [tex_state::ParagraphValidationFailure::BreakDependency as usize],
        1,
        "{font_parameter:?}"
    );

    let sfcode_source = format!(
        "\\font\\tenrm=cmr10\\relax \\tenrm \\hsize=70pt \\sfcode`.=1000\nSentence. {prose}\\par\nSentence. {prose}\\par\n\\end"
    );
    let sfcode = sfcode_source.find("1000").expect("sfcode value");
    let (code_table, _) = run_edit(&sfcode_source, sfcode..sfcode + 1, "3", |_| {});
    assert_eq!(code_table.paragraph_line_hits, 0, "{code_table:?}");
    assert_eq!(
        code_table.paragraph_validation_failure_reasons
            [tex_state::ParagraphValidationFailure::BreakDependency as usize],
        1,
        "{code_table:?}"
    );

    let line_dimension_source = format!(
        "\\font\\tenrm=cmr10\\relax \\tenrm \\hsize=70pt \\pdfeachlineheight=20pt\n{prose}\\par\n{prose}\\par\n\\end"
    );
    let line_height = line_dimension_source.find("20pt").expect("PDF line height");
    let (line_dimension, _) = run_edit(
        &line_dimension_source,
        line_height..line_height + 2,
        "30",
        install_pdf_paragraph_test_parameters,
    );
    assert_eq!(line_dimension.paragraph_line_hits, 0, "{line_dimension:?}");
    assert_eq!(
        line_dimension.paragraph_validation_failure_reasons
            [tex_state::ParagraphValidationFailure::BreakDependency as usize],
        1,
        "{line_dimension:?}"
    );

    let prev_graf_source = format!(
        "\\font\\tenrm=cmr10\\relax \\tenrm \\hsize=70pt \\hangindent=10pt \\hangafter=0 \\prevgraf=0\n{prose}\\par\n{prose}\\par\n\\end"
    );
    let prev_graf = prev_graf_source.find("prevgraf=0").expect("prevgraf") + "prevgraf=".len();
    let (line_offset, _) = run_edit(&prev_graf_source, prev_graf..prev_graf + 1, "3", |_| {});
    assert!(line_offset.paragraph_line_hits > 0, "{line_offset:?}");
    assert_eq!(
        line_offset.paragraph_validation_failure_reasons
            [tex_state::ParagraphValidationFailure::BreakDependency as usize],
        0,
        "{line_offset:?}"
    );
}

#[test]
fn paragraph_recording_rejects_pdf_microtype_until_font_code_dependencies_are_complete() {
    let mut universe = template();
    install_pdf_paragraph_test_parameters(&mut universe);
    universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let source = concat!(
        "\\font\\tenrm=cmr10\\relax \\tenrm \\pdfadjustspacing=1\n",
        "microtype paragraph words microtype paragraph words\\par\n",
        "another microtype paragraph words\\par\n",
        "\\end",
    );
    let mut session = Session::start(
        universe,
        "paragraph-pdf-microtype-barrier",
        RevisionId::new(1),
        source,
        usize::MAX,
    )
    .expect("session starts");
    session
        .register_input_file(Path::new("cmr10.tfm"), CMR10.to_vec())
        .expect("font fixture");
    session.cold().expect("microtype paragraphs execute");
    assert!(
        session.pure_memo.accepted_paragraphs().is_empty(),
        "finished lines must not be retained until mutable PDF font-code dependencies are tracked"
    );
}

#[test]
fn paragraph_hlist_mount_rejects_unsupported_graph_before_replay() {
    let source = concat!(
        "\\font\\tenrm=cmr10\\relax \\tenrm \\hsize=70pt\n",
        "marked paragraph \\mark{one} words marked paragraph words\\par\n",
        "marked paragraph \\mark{two} words marked paragraph words\\par\n",
        "\\vfill\\eject\\end",
    )
    .to_owned();
    let mut universe = template();
    universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let mut session = Session::start(
        universe,
        "paragraph-unsupported-hlist-mount",
        RevisionId::new(1),
        source.clone(),
        usize::MAX,
    )
    .expect("session starts");
    session
        .register_input_file(Path::new("cmr10.tfm"), CMR10.to_vec())
        .expect("font fixture");
    session.cold().expect("cold paragraph generation");
    let before = session.pure_memo_stats();
    let mark_payload = source.find("one").expect("first mark payload");
    let edited = source.replacen("one", "uno", 1);
    let output = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(source.as_bytes()),
                range: mark_payload..mark_payload + 3,
                replacement: "uno".to_owned(),
            },
        )
        .expect("unsupported graph executes cold");
    let after = session.pure_memo_stats();
    assert!(
        after.paragraph_validation_misses > before.paragraph_validation_misses,
        "unsupported mounted closure must be a typed validation miss: {after:?}"
    );
    let retained_result = tex_state::ParagraphValidationFailure::RetainedResult as usize;
    assert!(
        after.paragraph_validation_failure_reasons[retained_result]
            > before.paragraph_validation_failure_reasons[retained_result],
        "mark-bearing graph must fail retained-result mount validation: {after:?}"
    );
    let mut cold_universe = template();
    cold_universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let mut cold = Session::start(
        cold_universe,
        "paragraph-unsupported-hlist-mount",
        RevisionId::new(2),
        edited,
        usize::MAX,
    )
    .expect("cold comparison starts");
    cold.register_input_file(Path::new("cmr10.tfm"), CMR10.to_vec())
        .expect("cold font fixture");
    let expected = cold.cold().expect("cold comparison");
    assert_eq!(output.effects, expected.effects);
    assert_eq!(output.artifacts, expected.artifacts);
    assert_eq!(
        output.dvi_bytes().expect("incremental DVI"),
        expected.dvi_bytes().expect("cold DVI")
    );
}

#[test]
fn break_dependency_cold_fallback_keeps_current_output_provenance() {
    let prose = "stable paragraph words stable paragraph words";
    let source = format!(
        "\\font\\tenrm=cmr10\\relax \\tenrm \\hsize=70pt \\vsize=40pt\n{prose}\\par\n{prose}\\par\n\\vfill\\eject\\end"
    );
    let second_start = source.rfind(prose).expect("second stable paragraph");
    let second_end = second_start + prose.len();
    let mut universe = template();
    universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let mut session = Session::start(
        universe,
        "paragraph-break-fallback-provenance",
        RevisionId::new(1),
        source.clone(),
        usize::MAX,
    )
    .expect("session starts");
    session
        .register_input_file(Path::new("cmr10.tfm"), CMR10.to_vec())
        .expect("font fixture");
    let initial_dvi = session
        .cold()
        .expect("cold paragraph generation")
        .dvi_bytes()
        .expect("initial DVI")
        .to_vec();
    let before = session.pure_memo_stats();
    let hsize = source.find("70pt").expect("hsize value");
    session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(source.as_bytes()),
                range: hsize..hsize + 2,
                replacement: "45".to_owned(),
            },
        )
        .expect("layout-changing edit");
    let after = session.pure_memo_stats();
    let break_index = tex_state::ParagraphValidationFailure::BreakDependency as usize;
    assert_eq!(
        after.paragraph_validation_failure_reasons[break_index]
            - before.paragraph_validation_failure_reasons[break_index],
        1,
        "a changed break dependency must disable replay after one miss: {after:?}"
    );
    assert_eq!(after.paragraph_hits, before.paragraph_hits, "{after:?}");
    assert!(
        !session.pure_memo.accepted_paragraphs().is_empty(),
        "the cold fallback must preserve, not rebuild or discard, the prior accepted history"
    );
    assert!(
        (1..=session.artifacts.len() as u32)
            .flat_map(|page| (0..256).map(move |event| (page, event)))
            .any(|(page, event)| {
                matches!(
                    session.rendered_source_origin(page, event, None),
                    Ok(Some(LayoutResolvedOrigin::Current {
                        doc_offset_lo,
                        doc_offset_hi,
                        ..
                    })) if doc_offset_lo < second_end as u64
                        && doc_offset_hi > second_start as u64
                )
            }),
        "ordinary cold line breaking and shipout must observe current provenance"
    );

    let changed_source = source.replacen("70pt", "45pt", 1);
    let hits_before_inverse = session.pure_memo_stats().paragraph_line_hits;
    let inverse = session
        .advance(
            RevisionId::new(3),
            Edit {
                base_revision: RevisionId::new(2),
                expected_hash: ContentHash::from_bytes(changed_source.as_bytes()),
                range: hsize..hsize + 2,
                replacement: "70".to_owned(),
            },
        )
        .expect("inverse layout edit");
    assert_eq!(inverse.dvi_bytes().expect("inverse DVI"), initial_dvi);
    assert!(
        session.pure_memo_stats().paragraph_line_hits > hits_before_inverse,
        "restoring the break dependency must make preserved history reusable"
    );
}

#[test]
fn paragraph_entry_validation_rejects_changed_indent_then_backdates_equal_state() {
    let prose = "stable paragraph words stable paragraph words stable paragraph words";
    let source = format!(
        "\\font\\tenrm=cmr10\\relax \\tenrm \\parindent=10pt\n{prose}\\par\n{prose}\\par\n{prose}\\par\n\\vfill\\eject\\end"
    );
    let mut universe = template();
    universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let mut session = Session::start(
        universe,
        "paragraph-entry-validation",
        RevisionId::new(1),
        source.clone(),
        usize::MAX,
    )
    .expect("session starts");
    session
        .register_input_file(Path::new("cmr10.tfm"), CMR10.to_vec())
        .expect("font fixture");
    let initial = session
        .cold()
        .expect("cold paragraph generation")
        .dvi_bytes()
        .expect("initial DVI");

    let indent = source.find("10pt").expect("indent value");
    let before_changed = session.pure_memo_stats();
    let changed_source = source.replacen("10pt", "20pt", 1);
    let changed = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(source.as_bytes()),
                range: indent..indent + 2,
                replacement: "20".to_owned(),
            },
        )
        .expect("changed relevant entry state");
    let after_changed = session.pure_memo_stats();
    assert_eq!(
        after_changed.paragraph_hits, before_changed.paragraph_hits,
        "changed parindent must reject retained hlists: {after_changed:?}"
    );
    assert!(
        after_changed.paragraph_validation_misses > before_changed.paragraph_validation_misses,
        "changed parindent must be reported as a typed validation miss"
    );
    assert!(
        session
            .pure_memo
            .accepted_paragraphs()
            .iter()
            .all(|region| region.dependencies().count() == region.dependency_ordinals.len()),
        "cold fallback after a changed stamp must re-intern every observation ordinal"
    );

    let before_equal = after_changed;
    let equal_source = changed_source.replacen("20pt", "020pt", 1);
    let equal = session
        .advance(
            RevisionId::new(3),
            Edit {
                base_revision: RevisionId::new(2),
                expected_hash: ContentHash::from_bytes(changed_source.as_bytes()),
                range: indent..indent + 2,
                replacement: "020".to_owned(),
            },
        )
        .expect("semantically equal entry state");
    let after_equal = session.pure_memo_stats();
    assert!(
        after_equal.paragraph_hits > before_equal.paragraph_hits,
        "equal parindent after a stamp change must backdate and hit: {after_equal:?}"
    );

    let mut cold_universe = template();
    cold_universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let mut cold = Session::start(
        cold_universe,
        "paragraph-entry-validation",
        RevisionId::new(3),
        equal_source,
        usize::MAX,
    )
    .expect("cold comparison starts");
    cold.register_input_file(Path::new("cmr10.tfm"), CMR10.to_vec())
        .expect("cold font fixture");
    let expected = cold.cold().expect("cold comparison");
    assert_eq!(
        equal.dvi_bytes().expect("incremental DVI"),
        expected.dvi_bytes().expect("cold DVI")
    );
    assert_ne!(
        changed.dvi_bytes().expect("changed DVI"),
        initial,
        "fixture must make the relevant indentation change observable"
    );
}

#[test]
fn stateful_paragraph_redo_survives_a_later_prefix_edit() {
    let mut universe = template();
    universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let source = concat!(
        "stateful \\count5=41 \\language=7 paragraph text\\par\n",
        "stateful \\count5=42 \\language=7 paragraph text\\par\n",
        "stateful \\count5=43 \\language=7 paragraph text\\par\n",
        "stateful \\count5=44 \\language=7 paragraph text\\par\n",
        "\\vfill\\eject\\end",
    )
    .to_owned();
    let mut session = Session::start(
        universe,
        "stateful-paragraph-redo",
        RevisionId::new(1),
        source.clone(),
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("cold revision");
    let before = session.pure_memo_stats();
    assert_eq!(
        before.paragraph_mutations_replayed, 0,
        "cold generation cannot replay itself: {before:?}"
    );

    let incremental = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(source.as_bytes()),
                range: 0..0,
                replacement: "\\count99=3 ".to_owned(),
            },
        )
        .expect("prefix edit");
    let after = session.pure_memo_stats();
    assert!(
        after.paragraph_mutations_replayed > before.paragraph_mutations_replayed,
        "before={before:?} after={after:?}"
    );

    let edited = format!("\\count99=3 {source}");
    let mut cold_universe = template();
    cold_universe.enable_pure_memo(tex_state::PureMemoConfig::default());
    let mut cold = Session::start(
        cold_universe,
        "stateful-paragraph-redo",
        RevisionId::new(2),
        edited,
        usize::MAX,
    )
    .expect("cold comparison");
    assert_eq!(
        incremental.dvi_bytes().expect("incremental DVI"),
        cold.cold()
            .expect("cold edited revision")
            .dvi_bytes()
            .expect("cold DVI")
    );
}

fn assert_semantic_edit_matches_cold(name: &str, original: &str, edited: &str) -> ReuseMetrics {
    let mut session = Session::start(template(), name, RevisionId::new(1), original, usize::MAX)
        .expect("incremental session");
    session
        .register_input_file(Path::new("cmr10.tfm"), CMR10.to_vec())
        .expect("incremental font");
    session.cold().expect("initial cold run");
    let incremental = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(original.as_bytes()),
                range: 0..original.len(),
                replacement: edited.to_owned(),
            },
        )
        .expect("semantic edit");

    let mut cold = Session::start(template(), name, RevisionId::new(2), edited, usize::MAX)
        .expect("comparison session");
    cold.register_input_file(Path::new("cmr10.tfm"), CMR10.to_vec())
        .expect("comparison font");
    let cold = cold.cold().expect("comparison cold run");
    assert_eq!(incremental.dvi_pages, cold.dvi_pages, "{name}: DVI plans");
    assert_eq!(incremental.artifacts, cold.artifacts, "{name}: artifacts");
    assert_eq!(incremental.effects, cold.effects, "{name}: effects");
    incremental.reuse
}

#[test]
fn every_box_hooks_match_cold_execution_after_retained_paragraph_edit() {
    let original = "\\font\\tenrm=cmr10\\relax\\tenrm\n\\everyhbox{\\global\\advance\\count20 by1}\\setbox0=\\hbox{X}\nalpha\\par\n\\message{HOOKS=\\the\\count20}\\shipout\\hbox{\\copy0}\\end";
    let edited = original.replace("alpha", "omega");
    let mut session = Session::start(
        template(),
        "every-box-retained",
        RevisionId::new(1),
        original.to_owned(),
        usize::MAX,
    )
    .expect("incremental session");
    session
        .register_input_file(Path::new("cmr10.tfm"), CMR10.to_vec())
        .expect("incremental font");
    let initial = session.cold().expect("initial cold run");
    assert!(initial.effects.iter().any(|effect| matches!(
        effect,
        tex_state::EffectRecord::StreamWrite { text, .. } if text.contains("HOOKS=1")
    )));
    let incremental = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(original.as_bytes()),
                range: 0..original.len(),
                replacement: edited.clone(),
            },
        )
        .expect("retained edit");
    let mut cold = Session::start(
        template(),
        "every-box-retained",
        RevisionId::new(2),
        edited,
        usize::MAX,
    )
    .expect("cold comparison");
    cold.register_input_file(Path::new("cmr10.tfm"), CMR10.to_vec())
        .expect("cold font");
    let expected = cold.cold().expect("cold edited run");
    assert_eq!(incremental.dvi_pages, expected.dvi_pages);
    assert_eq!(incremental.artifacts, expected.artifacts);
    assert_eq!(incremental.effects, expected.effects);
    let reuse = incremental.reuse;
    assert!(
        reuse.restart_boundary.is_some() || reuse.suffixes_adopted > 0,
        "the comparison should exercise retained execution: {reuse:?}"
    );
}

#[test]
fn cold_history_contains_only_named_restartable_boundaries() {
    let text = source("a");
    let mut session = Session::start(template(), "test", RevisionId::new(1), text, usize::MAX)
        .expect("session starts");
    let output = session.cold().expect("cold execution succeeds");
    assert_eq!(output.history[0].key().boundary, EngineBoundary::JobStart);
    assert!(session.substrate.is_some());
    assert_eq!(output.artifacts.len(), 2);
}

#[test]
fn live_retention_charges_query_caches_to_their_owners() {
    let text = "\\font\\tenrm=cmr10\\relax\\tenrm\\shipout\\hbox{A}\\end";
    let mut session = Session::start(template(), "retention-query", RevisionId::new(1), text, 0)
        .expect("session starts");
    session
        .register_input_file(Path::new("cmr10.tfm"), CMR10.to_vec())
        .expect("font fixture registers");
    let accepted = session.cold().expect("cold execution succeeds");
    let before = session.retention_metrics().expect("accepted retention");
    assert_eq!(before, accepted.retention);
    assert_eq!(session.render_maps.borrow().retained_bytes(), 0);

    let event = (0..32)
        .find(|&event| {
            session
                .rendered_origin(1, event, Some(0))
                .expect("render lookup")
                .is_some()
        })
        .expect("source-backed text event");
    let output_id = session.output_id();
    session
        .rendered_source_location(1, event, Some(0), output_id, RevisionId::new(1))
        .expect("source query")
        .expect("mapped source");
    session
        .rendered_source_location(1, event, Some(0), output_id, RevisionId::new(1))
        .expect("repeated source query")
        .expect("mapped source");
    assert_eq!(session.page_lowerings(1), 1);

    let after = session.retention_metrics().expect("live retention");
    let line_index_bytes = after.diagnostic_bytes - before.diagnostic_bytes;
    let page_map_bytes = session.render_maps.borrow().retained_bytes();
    assert!(line_index_bytes > 0);
    assert!(page_map_bytes > 0);
    assert_eq!(after.output_bytes - before.output_bytes, page_map_bytes);
    assert_eq!(
        after.protected_overage_bytes - before.protected_overage_bytes,
        line_index_bytes,
        "only checkpoint-owned diagnostics count against the checkpoint budget"
    );
    assert_eq!(
        accepted.retention, before,
        "accepted output is point-in-time"
    );

    session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(text.as_bytes()),
                range: 0..text.len(),
                replacement: "\\input missing\\end".to_owned(),
            },
        )
        .expect_err("missing input rolls the attempted revision back");
    assert_eq!(session.page_lowerings(1), 0, "rollback drops page maps");
}

#[test]
fn rendered_source_queries_reject_another_revision_one_session() {
    let mut first = Session::start(
        template(),
        "first-output",
        RevisionId::new(1),
        "\\font\\tenrm=cmr10\\relax\\shipout\\hbox{\\tenrm A}\\end",
        usize::MAX,
    )
    .expect("first session");
    first
        .register_input_file(Path::new("cmr10.tfm"), CMR10.to_vec())
        .expect("first font");
    first.cold().expect("first output");
    let first_event = (0..32)
        .find(|&event| {
            first
                .rendered_origin(1, event, Some(0))
                .expect("first render lookup")
                .is_some()
        })
        .expect("first source-backed event");

    let mut second = Session::start(
        template(),
        "second-output",
        RevisionId::new(1),
        "\\font\\tenrm=cmr10\\relax\\shipout\\hbox{\\vrule\\tenrm BBB}\\end",
        usize::MAX,
    )
    .expect("second session");
    second
        .register_input_file(Path::new("cmr10.tfm"), CMR10.to_vec())
        .expect("second font");
    second.cold().expect("second output");

    assert_ne!(first.output_id(), second.output_id());
    assert_eq!(
        second
            .rendered_source_location(
                1,
                first_event,
                Some(0),
                first.output_id(),
                RevisionId::new(1),
            )
            .expect("cross-session query"),
        Some(RenderedSourceResult::OutputMismatch {
            accepted: second.output_id(),
        })
    );
    assert_eq!(second.page_lowerings(1), 0, "mismatch must precede lookup");
}

#[test]
fn accepted_history_retains_live_identities_for_direct_convergence() {
    let text = source("a");
    let mut session = Session::start(
        template(),
        "test",
        RevisionId::new(1),
        text.clone(),
        usize::MAX,
    )
    .expect("session starts");
    let cold = session.cold().expect("cold execution succeeds");
    assert!(
        cold.history
            .iter()
            .all(|record| record.checkpoint().has_exact_state_identity()),
        "cold history must capture canonical identities while each boundary is live"
    );
    let output = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(text.as_bytes()),
                range: 0..0,
                replacement: String::new(),
            },
        )
        .expect("no-op revision succeeds");
    assert_eq!(
        output.reuse.convergence_boundary,
        cold.history.get(1).map(BoundaryRecord::key)
    );
    assert!(output.reuse.pages_reused > 0);
    assert_eq!(output.reuse.same_history_stop, SameHistoryStop::Matched);
    assert_eq!(output.reuse.same_history_attempts, 1);
    assert_eq!(output.reuse.same_history_hash_mismatches, 0);
    assert!(output.reuse.reexecuted_bytes > 0);
    assert!(output.reuse.reexecuted_tokens > 0);
    assert!(
        output
            .history
            .iter()
            .all(|record| record.checkpoint().has_exact_state_identity()),
        "convergence must retain the already captured identities"
    );
    assert_eq!(
        output.dvi_bytes().expect("incremental DVI"),
        cold.dvi_bytes().expect("cold DVI")
    );
}

#[test]
fn semantic_edit_scenario_matrix_is_cold_identical_without_false_convergence() {
    let cases = [
        (
            "paragraph-content",
            "\\font\\f=cmr10\\f \\setbox0=\\vbox{alpha beta\\par}\\shipout\\box0\\end",
            "\\font\\f=cmr10\\f \\setbox0=\\vbox{alpha gamma\\par}\\shipout\\box0\\end",
        ),
        (
            "page-number-read",
            "\\count0=1\\shipout\\hbox{\\write16{page \\the\\count0}}\\end",
            "\\count0=2\\shipout\\hbox{\\write16{page \\the\\count0}}\\end",
        ),
        (
            "mark",
            "\\shipout\\vbox{\\mark{A}\\hrule height1pt}\\end",
            "\\shipout\\vbox{\\mark{B}\\hrule height1pt}\\end",
        ),
        (
            "deferred-write",
            "\\shipout\\hbox{\\write16{alpha}}\\end",
            "\\shipout\\hbox{\\write16{beta}}\\end",
        ),
        (
            "page-count",
            "\\shipout\\vbox{\\hrule height1pt}\\end",
            "\\shipout\\vbox{\\hrule height1pt}\\shipout\\vbox{\\hrule height2pt}\\end",
        ),
        (
            "output-routine",
            "\\count0=1\\output={\\global\\advance\\count0 by 1\\shipout\\box255}\\topskip=0pt\\vsize=1pt\\hrule height2pt\\penalty-10000\\end",
            "\\count0=2\\output={\\global\\advance\\count0 by 1\\shipout\\box255}\\topskip=0pt\\vsize=1pt\\hrule height2pt\\penalty-10000\\end",
        ),
        (
            "footnote-insertion",
            "\\output={\\shipout\\box255}\\topskip=0pt\\vsize=5pt\\insert7{\\hrule height1pt}\\hrule height10pt\\penalty-10000\\end",
            "\\output={\\shipout\\box255}\\topskip=0pt\\vsize=5pt\\insert7{\\hrule height2pt}\\hrule height10pt\\penalty-10000\\end",
        ),
    ];
    for (name, original, edited) in cases {
        let metrics = assert_semantic_edit_matches_cold(name, original, edited);
        assert_eq!(metrics.convergence_boundary, None, "{name}");
        assert_ne!(
            metrics.same_history_stop,
            SameHistoryStop::Matched,
            "{name}"
        );
        assert_eq!(metrics.pages_reused, 0, "{name}");
        assert!(metrics.reexecuted_commands > 0, "{name}");
    }
}

#[test]
fn multi_page_baseline_distinguishes_comment_and_semantic_edits() {
    let original = multi_page_source(20);
    let comment_at = original.find("page 10").expect("middle comment") + "page ".len();
    let mut comment_session = Session::start(
        template(),
        "comment-baseline",
        RevisionId::new(1),
        original.clone(),
        usize::MAX,
    )
    .expect("comment session");
    comment_session.cold().expect("comment cold");
    let comment = comment_session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(original.as_bytes()),
                range: comment_at..comment_at + 2,
                replacement: "XX".to_owned(),
            },
        )
        .expect("comment edit");
    assert_eq!(comment.reuse.same_history_stop, SameHistoryStop::Matched);
    assert!(comment.reuse.pages_reused > 0);

    let width_at = original.find("width 20pt").expect("middle width") + "width ".len();
    let mut semantic_session = Session::start(
        template(),
        "semantic-baseline",
        RevisionId::new(1),
        original.clone(),
        usize::MAX,
    )
    .expect("semantic session");
    semantic_session.cold().expect("semantic cold");
    let semantic = semantic_session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(original.as_bytes()),
                range: width_at..width_at + 2,
                replacement: "21".to_owned(),
            },
        )
        .expect("semantic edit");
    // Probabilistic state convergence may legitimately match at the terminal
    // boundary after all changed pages have already been reexecuted. In this
    // observed run no old page suffix is adopted.
    assert_eq!(semantic.reuse.same_history_stop, SameHistoryStop::Matched);
    assert!(semantic.reuse.pages_reused > 0);
    assert!(semantic.reuse.pages_retyped > 0);
    assert!(semantic.reuse.pages_retyped < 20);
    assert_eq!(
        semantic.reuse.pages_retained_prefix
            + semantic.reuse.pages_retyped
            + semantic.reuse.pages_reused,
        20
    );
    assert_eq!(semantic.reuse.trace_nodes_walked, 2);
    assert_eq!(semantic.reuse.trace_leaf_hits, semantic.reuse.pages_reused);
    assert_eq!(semantic.reuse.trace_subtree_hits, 1);
    assert!(semantic.reuse.trace_retained_bytes > 0);
    assert_eq!(
        semantic.reuse.convergence_boundary.map(|key| key.boundary),
        Some(EngineBoundary::ShipoutComplete)
    );
    assert!(semantic.reuse.same_history_hash_mismatches > 0);
    assert!(semantic.reuse.reexecuted_bytes < original.len());
}

#[test]
fn edit_before_earliest_retained_checkpoint_falls_back_to_cold_execution() {
    let original = multi_page_source(8);
    let mut session = Session::start(
        template(),
        "missing-prefix-fallback",
        RevisionId::new(1),
        original.clone(),
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("cold revision");
    session.history.remove(0);
    assert!(session.history[0].key.position > 0);

    let width = original.find("width 10pt").expect("first page width") + "width ".len();
    let accepted = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(original.as_bytes()),
                range: width..width + 2,
                replacement: "11".to_owned(),
            },
        )
        .expect("edit falls back to a full execution");
    assert_eq!(accepted.reuse.restart_boundary, None);
    assert_eq!(accepted.reuse.pages_reused, 0);

    let edited = session.source().to_owned();
    let mut cold = Session::start(
        template(),
        "missing-prefix-fallback",
        RevisionId::new(2),
        edited,
        usize::MAX,
    )
    .expect("cold comparison starts");
    let cold = cold.cold().expect("cold comparison executes");
    assert_eq!(
        accepted.dvi_bytes().expect("incremental DVI"),
        cold.dvi_bytes().expect("cold DVI")
    );
}

#[test]
fn unchanged_unicode_crlf_span_identity_survives_multiple_surrounding_edits() {
    let original = "% α\r\n% keep\r\n% ω\r\n\\end";
    let keep = original.find("keep").expect("keep span");
    let mut session = Session::start(
        template(),
        "stable-spans",
        RevisionId::new(1),
        original,
        usize::MAX,
    )
    .expect("session");
    session.cold().expect("cold run");
    let initial = root_span_at(&session, keep..keep + 4);

    let alpha = session.source().find('α').expect("alpha");
    let hash = session.content_hash();
    session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: hash,
                range: alpha..alpha + 'α'.len_utf8(),
                replacement: "prefix".to_owned(),
            },
        )
        .expect("prefix edit");
    let keep = session.source().find("keep").expect("mapped keep");
    assert_eq!(root_span_at(&session, keep..keep + 4), initial);

    let omega = session.source().find('ω').expect("omega");
    let hash = session.content_hash();
    session
        .advance(
            RevisionId::new(3),
            Edit {
                base_revision: RevisionId::new(2),
                expected_hash: hash,
                range: omega..omega + 'ω'.len_utf8(),
                replacement: "suffix-long".to_owned(),
            },
        )
        .expect("suffix edit");
    let keep = session.source().find("keep").expect("mapped keep");
    assert_eq!(root_span_at(&session, keep..keep + 4), initial);

    let hash = session.content_hash();
    session
        .advance(
            RevisionId::new(4),
            Edit {
                base_revision: RevisionId::new(3),
                expected_hash: hash,
                range: keep..keep + 4,
                replacement: "keep".to_owned(),
            },
        )
        .expect("equal-byte replacement");
    let replaced = root_span_at(&session, keep..keep + 4);
    assert_ne!(replaced.piece(), initial.piece());
    assert_eq!(replaced.content(), initial.content());
}

#[test]
fn reused_suffix_origin_resolves_at_current_offset_after_earlier_insert() {
    let body = source("a");
    let original = format!("%a\n{body}");
    let body_offset = original.find("\\shipout").expect("shipout offset");
    let initial_piece = session_piece_origin_setup(&original, body_offset);
    let (mut session, origin) = initial_piece;
    session.cold().expect("cold execution succeeds");
    let inserted = " longer";
    let output = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(original.as_bytes()),
                range: 2..2,
                replacement: inserted.to_owned(),
            },
        )
        .expect("insertion converges");
    assert!(output.reuse.pages_reused > 0);
    assert_eq!(
        session
            .substrate
            .as_ref()
            .expect("accepted substrate")
            .resolve_layout_origin(origin, &session.fragments, &session.layout),
        LayoutResolvedOrigin::Current {
            path: "<editor>".to_owned(),
            doc_offset_lo: (body_offset + inserted.len()) as u64,
            doc_offset_hi: (body_offset + inserted.len() + 1) as u64,
            line: 2,
            column: 1,
        }
    );
}

#[test]
fn convergent_old_substrate_resolves_new_fragment_origins() {
    let body = source("a");
    let original = format!("%a\n{body}");
    let mut session = Session::start(
        template(),
        "scratch-origin",
        RevisionId::new(1),
        original.clone(),
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("cold execution succeeds");
    let old_substrate = session.substrate.as_ref().expect("substrate") as *const _;
    let output = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(original.as_bytes()),
                range: 1..2,
                replacement: "b".to_owned(),
            },
        )
        .expect("edit converges");
    assert!(output.reuse.convergence_boundary.is_some());
    assert_eq!(
        session.substrate.as_ref().expect("retained substrate") as *const _,
        old_substrate,
        "convergence must retain the old substrate"
    );
    let new_piece = session.layout.pieces().first().expect("replacement piece");
    let origin = session
        .fragments
        .registration(new_piece.fragment())
        .expect("new fragment registration")
        .direct_origin(1, 2)
        .expect("new fragment origin");
    assert!(matches!(
        session
            .substrate
            .as_ref()
            .expect("retained substrate")
            .resolve_layout_origin(origin, &session.fragments, &session.layout),
        LayoutResolvedOrigin::Current {
            doc_offset_lo: 1,
            doc_offset_hi: 2,
            ..
        }
    ));
}

#[test]
fn convergent_adopted_char_artifact_keeps_current_and_deleted_provenance() {
    let original =
        "\\font\\tenrm=cmr10\\relax\\tenrm %a\n\\shipout\\hbox{\\char65}\\shipout\\hbox{B}\\end";
    let mut session = Session::start(
        template(),
        "scratch-char-origin",
        RevisionId::new(1),
        original,
        usize::MAX,
    )
    .expect("session starts");
    session
        .register_input_file(Path::new("cmr10.tfm"), CMR10.to_vec())
        .expect("font fixture registers");
    session.cold().expect("cold execution succeeds");

    let first = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(original.as_bytes()),
                range: original.find("%a").expect("comment") + 1
                    ..original.find("%a").expect("comment") + 2,
                replacement: "b".to_owned(),
            },
        )
        .expect("comment edit converges");
    assert_eq!(first.reuse.pages_retyped, 1);
    assert_eq!(first.reuse.pages_reused, 1);
    let event = (0..32)
        .find(|&event| {
            session
                .rendered_origin(1, event, None)
                .expect("render lookup")
                .is_some()
        })
        .expect("char text event");
    assert_eq!(
        session
            .rendered_source_origin(1, event, None)
            .expect("render source lookup"),
        Some(LayoutResolvedOrigin::Current {
            path: "<editor>".to_owned(),
            doc_offset_lo: 47,
            doc_offset_hi: 52,
            line: 2,
            column: 15,
        })
    );

    let revision_two = session.source.clone();
    let inserted = " longer";
    let insert_at = revision_two.find('\n').expect("comment newline");
    let third = session
        .advance(
            RevisionId::new(3),
            Edit {
                base_revision: RevisionId::new(2),
                expected_hash: ContentHash::from_bytes(revision_two.as_bytes()),
                range: insert_at..insert_at,
                replacement: inserted.to_owned(),
            },
        )
        .expect("earlier insertion converges");
    assert!(third.reuse.pages_reused > 0);
    assert_eq!(session.page_lowerings(1), 0, "accept drops old page maps");
    let b_event = (0..32)
        .find(|&event| {
            session
                .rendered_origin(2, event, None)
                .expect("render lookup")
                .is_some()
        })
        .expect("reused B text event");
    let b_origin = session
        .rendered_origin(2, b_event, None)
        .expect("render lookup")
        .expect("B render origin");
    let b_offset = session.source.find("{B}").expect("B box") + 1;
    assert_eq!(
        session
            .rendered_source_location(2, b_event, None, session.output_id(), RevisionId::new(3),)
            .expect("render source lookup"),
        Some(RenderedSourceResult::Current(
            tex_state::ResolvedSourceLocation {
                path: "<editor>".to_owned(),
                start: b_offset as u64,
                end: (b_offset + 1) as u64,
                line: 2,
                column: (b_offset - session.source.find('\n').expect("newline")) as u32,
            }
        ))
    );

    let revision_three = session.source.clone();
    let char_line_start = revision_three
        .find("\\shipout\\hbox{\\char65}")
        .expect("char line");
    let char_line_end = revision_three[char_line_start..]
        .find("\\shipout\\hbox{B}")
        .map(|offset| char_line_start + offset)
        .expect("second shipout");
    let char_line = revision_three[char_line_start..char_line_end].to_owned();
    let fourth = session
        .advance(
            RevisionId::new(4),
            Edit {
                base_revision: RevisionId::new(3),
                expected_hash: ContentHash::from_bytes(revision_three.as_bytes()),
                range: char_line_start..char_line_end,
                replacement: char_line,
            },
        )
        .expect("equivalent char edit converges");
    assert!(fourth.reuse.convergence_boundary.is_some());
    assert!(fourth.reuse.pages_reused > 0);
    assert_eq!(session.page_lowerings(2), 0, "accept drops old page maps");
    assert_eq!(
        session
            .substrate
            .as_ref()
            .expect("retained substrate")
            .resolve_layout_origin(b_origin, &session.fragments, &session.layout),
        LayoutResolvedOrigin::Deleted { minted_revision: 1 }
    );
    assert_eq!(
        session
            .rendered_source_location(2, b_event, None, session.output_id(), RevisionId::new(4),)
            .expect("deleted render source lookup"),
        Some(RenderedSourceResult::Deleted { minted_revision: 1 })
    );
}

#[test]
fn reminted_line_positions_resolve_typed_deleted() {
    let original = format!("%a\n{}", source("a"));
    let (mut session, origin) = session_piece_origin_setup(&original, 1);
    session.cold().expect("cold execution succeeds");
    session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(original.as_bytes()),
                range: 1..2,
                replacement: "b".to_owned(),
            },
        )
        .expect("edit succeeds");
    assert_eq!(
        session
            .substrate
            .as_ref()
            .expect("accepted substrate")
            .resolve_layout_origin(origin, &session.fragments, &session.layout),
        LayoutResolvedOrigin::Deleted { minted_revision: 1 }
    );
}

#[test]
fn convergent_advance_prunes_fully_replaced_fragment_bytes() {
    let original = source("a");
    let mut session = Session::start(
        template(),
        "convergent-prune",
        RevisionId::new(1),
        original.clone(),
        usize::MAX,
    )
    .expect("session starts");
    let initial = session.layout.pieces()[0].fragment();
    session.cold().expect("cold run");
    let output = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(original.as_bytes()),
                range: 0..original.len(),
                replacement: original.clone(),
            },
        )
        .expect("semantically unchanged edit converges");

    assert!(output.reuse.convergence_boundary.is_some());
    assert_eq!(session.fragments.bytes(initial), None);
    assert_eq!(session.fragments.source_bytes(), session.source.len());
    assert_eq!(
        output.retention.diagnostic_bytes,
        session.diagnostic_retained_bytes()
    );
}

#[test]
fn nonconvergent_advance_prunes_fully_replaced_fragment_bytes() {
    let original = persistent_source(1);
    let replacement = persistent_source(29);
    let mut session = Session::start(
        template(),
        "nonconvergent-prune",
        RevisionId::new(1),
        original.clone(),
        usize::MAX,
    )
    .expect("session starts");
    let initial = session.layout.pieces()[0].fragment();
    session.cold().expect("cold run");
    let output = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(original.as_bytes()),
                range: 0..original.len(),
                replacement: replacement.clone(),
            },
        )
        .expect("semantic edit succeeds");

    assert_eq!(output.reuse.convergence_boundary, None);
    assert_ne!(output.reuse.same_history_stop, SameHistoryStop::Matched);
    assert_eq!(output.reuse.pages_reused, 0);
    assert!(output.reuse.reexecuted_bytes > 0);
    assert!(output.reuse.reexecuted_tokens > 0);
    assert!(
        session
            .history
            .iter()
            .all(|record| record.checkpoint().has_exact_state_identity()),
        "a nonconvergent revision must publish live identities for its new accepted history"
    );
    assert_eq!(session.fragments.bytes(initial), None);
    assert_eq!(session.fragments.source_bytes(), replacement.len());
}

#[derive(Default)]
struct StagedInputResolver {
    files: BTreeMap<String, String>,
}

impl InputResolver for StagedInputResolver {
    fn open_input(
        &mut self,
        _input: &mut dyn InputReadState,
        name: &str,
        request_index: u64,
    ) -> tex_expand::ResourceResult<Box<dyn InputSource>> {
        Ok(self.files.get(name).cloned().map_or_else(
            || {
                tex_expand::ResourceLookup::NeedResource(tex_expand::ResourceNeed::new(
                    request_index,
                ))
            },
            |source| {
                tex_expand::ResourceLookup::Available(
                    Box::new(MemoryInput::new(source)) as Box<dyn InputSource>
                )
            },
        ))
    }
}

#[test]
fn multi_round_resource_retry_drops_orphan_fragment_bytes_and_keeps_parity() {
    let original = "\\end".to_owned();
    let replacement = "\\input one \\input two \\end".to_owned();
    let mut session = Session::start(
        template(),
        "resource-retry",
        RevisionId::new(1),
        original.clone(),
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("cold run");
    let edit = Edit {
        base_revision: RevisionId::new(1),
        expected_hash: ContentHash::from_bytes(original.as_bytes()),
        range: 0..original.len(),
        replacement: replacement.clone(),
    };
    let mut inputs = StagedInputResolver::default();
    let mut fonts = DirectFontResolver;
    let initial_live_bytes = session.fragments.source_bytes();
    let mut peak_live_bytes = initial_live_bytes;

    for (name, contents) in [
        ("one", "\\shipout\\vbox{\\hrule height 1pt}"),
        ("two", "\\shipout\\vbox{\\hrule height 2pt}"),
    ] {
        session
            .advance_with_resolvers(RevisionId::new(2), edit.clone(), &mut inputs, &mut fonts)
            .expect_err("unresolved input rejects this attempt");
        peak_live_bytes = peak_live_bytes.max(session.fragments.source_bytes());
        assert_eq!(session.fragments.source_bytes(), initial_live_bytes);
        inputs.files.insert(name.to_owned(), contents.to_owned());
    }
    assert_eq!(peak_live_bytes, initial_live_bytes);

    let accepted = session
        .advance_with_resolvers(RevisionId::new(2), edit, &mut inputs, &mut fonts)
        .expect("fully provisioned retry succeeds");
    assert_eq!(session.fragments.source_bytes(), replacement.len());
    assert_eq!(
        session.fragments.len(),
        2,
        "failed candidates retain no fragment metadata"
    );

    let mut cold = Session::start(
        template(),
        "resource-retry",
        RevisionId::new(2),
        replacement,
        usize::MAX,
    )
    .expect("cold session");
    let mut cold_inputs = inputs;
    let cold = cold
        .cold_with_resolvers(&mut cold_inputs, &mut fonts)
        .expect("cold comparison succeeds");
    assert_eq!(
        accepted.dvi_bytes().expect("incremental DVI"),
        cold.dvi_bytes().expect("cold DVI")
    );
}

#[test]
fn repeated_fatal_advance_drops_orphan_fragment_bytes_before_later_accept() {
    let original = "\\end".to_owned();
    let replacement = persistent_source(17);
    let mut session = Session::start(
        template(),
        "fatal-retry",
        RevisionId::new(1),
        original.clone(),
        usize::MAX,
    )
    .expect("session starts");
    let edit = Edit {
        base_revision: RevisionId::new(1),
        expected_hash: ContentHash::from_bytes(original.as_bytes()),
        range: 0..original.len(),
        replacement: replacement.clone(),
    };
    let initial_live_bytes = session.fragments.source_bytes();
    let mut peak_live_bytes = initial_live_bytes;

    for _ in 0..4 {
        let error = session
            .advance(RevisionId::new(2), edit.clone())
            .expect_err("advance without an accepted substrate is fatal");
        assert!(matches!(error, SessionError::MissingAcceptedSubstrate));
        peak_live_bytes = peak_live_bytes.max(session.fragments.source_bytes());
        assert_eq!(session.fragments.source_bytes(), initial_live_bytes);
    }
    assert_eq!(peak_live_bytes, initial_live_bytes);
    assert_eq!(
        session.fragments.len(),
        1,
        "failed candidates retain no fragment metadata"
    );

    session
        .cold()
        .expect("initial revision can still be accepted");
    let accepted = session
        .advance(RevisionId::new(2), edit)
        .expect("same pending edit later succeeds");
    assert_eq!(session.fragments.source_bytes(), replacement.len());
    assert_eq!(session.fragments.len(), 2);

    let mut cold = Session::start(
        template(),
        "fatal-retry",
        RevisionId::new(2),
        replacement,
        usize::MAX,
    )
    .expect("cold session");
    let cold = cold.cold().expect("cold comparison succeeds");
    assert_eq!(
        accepted.dvi_bytes().expect("incremental DVI"),
        cold.dvi_bytes().expect("cold DVI")
    );
}

#[test]
fn alternating_edits_keep_source_backing_bytes_bounded() {
    let mut text = persistent_source(1);
    let initial_len = text.len();
    let mut session = Session::start(
        template(),
        "balanced-pruning",
        RevisionId::new(1),
        text.clone(),
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("cold run");

    for step in 1..=64_u64 {
        let range;
        let replacement;
        if step % 2 == 1 {
            range = 0..0;
            replacement = " ".to_owned();
        } else {
            range = 0..1;
            replacement = String::new();
        }
        let edit = Edit {
            base_revision: RevisionId::new(step),
            expected_hash: ContentHash::from_bytes(text.as_bytes()),
            range: range.clone(),
            replacement: replacement.clone(),
        };
        text.replace_range(range, &replacement);
        let output = session
            .advance(RevisionId::new(step + 1), edit)
            .expect("balanced edit succeeds");
        assert_eq!(session.fragments.source_bytes(), text.len());
        assert_eq!(
            output.retention.diagnostic_bytes,
            session.diagnostic_retained_bytes()
        );
    }
    assert_eq!(text.len(), initial_len);
    assert_eq!(session.fragments.source_bytes(), initial_len);
    assert_eq!(session.fragments.len(), 65);
}

#[test]
fn keystroke_storm_tracks_cumulative_headroom_without_pinning_old_lines() {
    let body = source("a");
    let mut text = format!("%\n{body}");
    let initial_len = text.len();
    let mut session = Session::start(
        template(),
        "keystroke-storm",
        RevisionId::new(1),
        text.clone(),
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("cold run");
    let mut expected_reserved = initial_len as u64 + 1;

    for step in 1..=128_u64 {
        let insert_at = text.find('\n').expect("comment terminator");
        let edit = Edit {
            base_revision: RevisionId::new(step),
            expected_hash: ContentHash::from_bytes(text.as_bytes()),
            range: insert_at..insert_at,
            replacement: "x".to_owned(),
        };
        text.insert(insert_at, 'x');
        expected_reserved += (insert_at + 3) as u64;
        session
            .advance(RevisionId::new(step + 1), edit)
            .expect("keystroke edit succeeds");
        assert!(session.fragments.source_bytes() <= initial_len + insert_at + 2);
    }

    assert_eq!(
        session.fragments.reserved_position_bytes(),
        expected_reserved
    );
    let projected_typical_session = 100_000_u64 * 101;
    assert!(projected_typical_session < (1_u64 << 31) / 100);
}

#[test]
fn separated_line_edits_exercise_pathological_piece_growth_bound() {
    let mut text = (0..64).map(|_| "%a\n").collect::<String>();
    text.push_str("\\end");
    let mut session = Session::start(
        template(),
        "piece-growth",
        RevisionId::new(1),
        text.clone(),
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("cold run");

    for step in 0..32_u64 {
        let edit_at = step as usize * 6 + 1;
        let before = session.layout.pieces().len();
        let replacement = if step % 2 == 0 { "b" } else { "c" };
        let edit = Edit {
            base_revision: RevisionId::new(step + 1),
            expected_hash: ContentHash::from_bytes(text.as_bytes()),
            range: edit_at..edit_at + 1,
            replacement: replacement.to_owned(),
        };
        text.replace_range(edit_at..edit_at + 1, replacement);
        session
            .advance(RevisionId::new(step + 2), edit)
            .expect("separated line edit succeeds");
        assert!(session.layout.pieces().len() <= before + 2);
    }
    assert_eq!(session.layout.pieces().len(), 64);
    assert_eq!(session.fragments.source_bytes(), text.len() + 32 * 3);
}

fn session_piece_origin_setup(
    source: &str,
    offset: usize,
) -> (Session, tex_state::token::OriginId) {
    let session = Session::start(
        template(),
        "layout-origin",
        RevisionId::new(1),
        source,
        usize::MAX,
    )
    .expect("session starts");
    let piece = session.layout.pieces().first().expect("initial piece");
    let origin = session
        .fragments
        .registration(piece.fragment())
        .expect("initial fragment registration")
        .direct_origin(offset as u64, offset as u64 + 1)
        .expect("initial fragment origin");
    (session, origin)
}

#[test]
fn adopted_old_suffix_remains_restartable_on_the_next_edit() {
    let body = source("a");
    let original = format!("%a\n{body}");
    let text = format!("%a much longer comment\n{body}");
    let mut session = Session::start(
        template(),
        "test",
        RevisionId::new(1),
        original.clone(),
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("cold execution succeeds");
    let adopted = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(original.as_bytes()),
                range: 2..2,
                replacement: " much longer comment".to_owned(),
            },
        )
        .expect("length-changing revision converges");
    assert_eq!(
        adopted.reuse.execution_path,
        RevisionExecutionPath::FastEdit
    );
    assert!(adopted.reuse.convergence_boundary.is_some());
    let output = session
        .advance(
            RevisionId::new(3),
            Edit {
                base_revision: RevisionId::new(2),
                expected_hash: ContentHash::from_bytes(text.as_bytes()),
                range: 0..0,
                replacement: String::new(),
            },
        )
        .expect("mapped adopted history remains restartable");
    assert!(output.reuse.convergence_boundary.is_some());

    let mut cold = Session::start(template(), "test", RevisionId::new(3), text, usize::MAX)
        .expect("cold session");
    let cold = cold.cold().expect("cold execution");
    assert_eq!(
        output.dvi_bytes().expect("incremental DVI"),
        cold.dvi_bytes().expect("cold DVI")
    );
}

#[test]
fn edited_output_is_byte_identical_to_a_fresh_cold_session() {
    let original = source("a");
    let replacement = source("longer");
    let mut incremental = Session::start(
        template(),
        "test",
        RevisionId::new(1),
        original.clone(),
        usize::MAX,
    )
    .expect("session starts");
    incremental.cold().expect("initial run");
    let edited = incremental
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(original.as_bytes()),
                range: 0..original.len(),
                replacement: replacement.clone(),
            },
        )
        .expect("edit succeeds");
    assert_eq!(edited.reuse.execution_path, RevisionExecutionPath::SlowEdit);

    let mut cold = Session::start(
        template(),
        "test",
        RevisionId::new(2),
        replacement,
        usize::MAX,
    )
    .expect("cold session starts");
    let cold = cold.cold().expect("cold run");
    assert_eq!(
        edited.dvi_bytes().expect("edited DVI"),
        cold.dvi_bytes().expect("cold DVI")
    );
}

#[test]
fn edits_inside_nonrestartable_constructs_replay_from_the_preceding_boundary() {
    let cases = [
        ("scanner", "\\count0=1 \\end"),
        ("box", "\\setbox0=\\hbox{\\count0=1}\\end"),
        (
            "alignment",
            "\\setbox0=\\vbox{\\halign{#\\cr \\count0=1\\cr}}\\end",
        ),
        ("inline math", "\\setbox0=\\hbox{$\\count0=1$}\\end"),
    ];
    for (name, original) in cases {
        let edit_at = original.find("=1").expect("marked edit") + 1;
        let mut session =
            Session::start(template(), name, RevisionId::new(1), original, usize::MAX)
                .expect("incremental session");
        session
            .cold()
            .unwrap_or_else(|error| panic!("{name} cold run failed: {error}"));
        let incremental = session
            .advance(
                RevisionId::new(2),
                Edit {
                    base_revision: RevisionId::new(1),
                    expected_hash: ContentHash::from_bytes(original.as_bytes()),
                    range: edit_at..edit_at + 1,
                    replacement: "2".to_owned(),
                },
            )
            .unwrap_or_else(|error| panic!("{name} incremental run failed: {error}"));
        assert_eq!(
            incremental.reuse.restart_boundary.map(|key| key.boundary),
            Some(EngineBoundary::JobStart),
            "{name} must replay from JobStart"
        );

        let mut edited = original.to_owned();
        edited.replace_range(edit_at..edit_at + 1, "2");
        let mut cold = Session::start(template(), name, RevisionId::new(2), edited, usize::MAX)
            .expect("cold comparison session");
        let cold = cold
            .cold()
            .unwrap_or_else(|error| panic!("{name} comparison run failed: {error}"));
        assert_eq!(
            incremental.dvi_pages, cold.dvi_pages,
            "{name} edit differs from cold"
        );
    }
}

#[test]
fn promoted_prefix_records_remain_restartable_on_the_next_edit() {
    let first = persistent_source(1);
    let second = persistent_source(2);
    let third = persistent_source(3);
    let mut session = Session::start(
        template(),
        "test",
        RevisionId::new(1),
        first.clone(),
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("initial run");
    let promoted = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(first.as_bytes()),
                range: 0..first.len(),
                replacement: second.clone(),
            },
        )
        .expect("first promotion succeeds");
    assert_eq!(promoted.reuse.convergence_boundary, None);
    let incrementally_edited = session
        .advance(
            RevisionId::new(3),
            Edit {
                base_revision: RevisionId::new(2),
                expected_hash: ContentHash::from_bytes(second.as_bytes()),
                range: 0..second.len(),
                replacement: third.clone(),
            },
        )
        .expect("retargeted prefix restores on the next edit");

    let mut cold = Session::start(template(), "test", RevisionId::new(3), third, usize::MAX)
        .expect("cold session starts");
    let cold = cold.cold().expect("cold run");
    assert_eq!(
        incrementally_edited.dvi_bytes().expect("incremental DVI"),
        cold.dvi_bytes().expect("cold DVI")
    );
}

#[test]
fn fast_scripted_edit_sequence_matches_cold_every_revision() {
    scripted_edit_sequence(32);
}

#[test]
#[ignore = "explicit 1000-edit incremental fuzz tier"]
fn thousand_edit_scripted_fuzz_matches_cold_every_revision() {
    scripted_edit_sequence(1_000);
}

fn scripted_edit_sequence(edits: u64) {
    let mut text = persistent_source(1);
    let template = template();
    let mut session = Session::start(
        template.clone(),
        "fuzz",
        RevisionId::new(1),
        text.clone(),
        usize::MAX,
    )
    .expect("incremental session");
    session.cold().expect("initial run");
    let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
    for step in 1..=edits {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let next = persistent_source((seed % 97 + 1) as usize);
        let revision = RevisionId::new(step + 1);
        let incremental = session
            .advance(
                revision,
                Edit {
                    base_revision: RevisionId::new(step),
                    expected_hash: ContentHash::from_bytes(text.as_bytes()),
                    range: 0..text.len(),
                    replacement: next.clone(),
                },
            )
            .expect("scripted incremental edit");
        let mut cold = Session::start(template.clone(), "fuzz", revision, next.clone(), usize::MAX)
            .expect("cold session");
        let cold = cold.cold().expect("cold execution");
        assert_eq!(
            incremental.dvi_bytes().expect("incremental DVI"),
            cold.dvi_bytes().expect("cold DVI"),
            "revision {} differs",
            revision.raw()
        );
        text = next;
    }
}

#[test]
fn pruning_protects_job_start_and_newest_and_reports_overage() {
    let text = source("a");
    let mut session =
        Session::start(template(), "test", RevisionId::new(1), text, 0).expect("session starts");
    let output = session.cold().expect("cold execution succeeds");
    assert_eq!(
        output.history.first().expect("job start").key().boundary,
        EngineBoundary::JobStart
    );
    assert!(output.history.len() <= 2);
    assert!(output.retention.protected_overage_bytes > 0);
    assert!(output.retention.output_bytes > 0);
}

#[test]
fn stale_revision_and_hash_are_actionable_errors() {
    let text = source("a");
    let mut session = Session::start(
        template(),
        "test",
        RevisionId::new(4),
        text.clone(),
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("cold run");
    let error = session
        .advance(
            RevisionId::new(5),
            Edit {
                base_revision: RevisionId::new(3),
                expected_hash: ContentHash::from_bytes(text.as_bytes()),
                range: 0..0,
                replacement: String::new(),
            },
        )
        .expect_err("stale edit rejected");
    assert!(matches!(error, SessionError::StaleRevision { .. }));
}

#[test]
fn record_rehome_rejects_a_changed_suffix_and_stale_root_revision() {
    let original = source("a");
    let mut session = Session::start(
        template(),
        "authority",
        RevisionId::new(1),
        original.clone(),
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("cold run");
    let substrate = session.substrate.as_ref().expect("accepted substrate");
    let job_start = session.history.first().expect("job start").checkpoint();
    let changed = tex_exec::RootRehomeContext::new(&original, "changed");

    assert_eq!(
        job_start
            .rehome_converged_root(substrate, &changed, 0)
            .expect_err("changed adopted interval is rejected"),
        GenerationForkError::ChangedRootInterval
    );
    let unchanged = tex_exec::RootRehomeContext::new(&original, &original);
    assert_eq!(
        job_start
            .rehome_converged_root(substrate, &unchanged, usize::MAX)
            .expect_err("invalid mapped anchor is rejected"),
        GenerationForkError::InvalidMappedAnchor
    );
    let stale = tex_exec::RootRehomeContext::new("stale revision", &original);
    assert_eq!(
        job_start
            .rehome_unchanged_prefix(substrate, &stale)
            .expect_err("stale root revision is rejected"),
        GenerationForkError::RootRevisionMismatch
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // Deliberately mutates a real dependency between revisions.
fn changed_included_input_rejects_checkpoint_reuse() {
    let directory = tempfile::tempdir().expect("temporary input directory");
    let included = directory.path().join("included.tex");
    std::fs::write(&included, b"\\count0=1\n").expect("seed include");
    let root = format!("\\input {} \\end", included.display());
    let mut universe = Universe::with_world(tex_state::World::real_with_artifact_dir(
        directory.path().join("artifacts"),
    ));
    tex_exec::install_unexpandable_primitives(&mut universe);
    tex_expand::install_expandable_primitives(&mut universe);
    let mut session = Session::start(
        universe,
        "include",
        RevisionId::new(1),
        root.clone(),
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("cold include run");
    std::fs::write(&included, b"\\count0=2\n").expect("change include");
    let error = session
        .advance(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(root.as_bytes()),
                range: 0..0,
                replacement: String::new(),
            },
        )
        .expect_err("changed include rejects retained reuse");
    assert!(matches!(error, SessionError::World(_)));
}

#[test]
fn finalize_materializes_session_effects_once_and_consumes_session() {
    let text = "\\message{retained hello}\\end";
    let mut session = Session::start(template(), "finalize", RevisionId::new(1), text, usize::MAX)
        .expect("session starts");
    let output = session.cold().expect("cold run");
    assert!(!output.effects.is_empty());
    let world = session.finalize().expect("session finalizes once");
    assert!(
        std::str::from_utf8(world.memory_terminal_output().expect("terminal output"))
            .expect("UTF-8 output")
            .contains("retained hello")
    );
}

#[test]
fn finalize_installs_spliced_accepted_artifacts() {
    let original = source("a");
    let replacement = source("longer");
    let mut session = Session::start(
        template(),
        "finalize-artifacts",
        RevisionId::new(1),
        original.clone(),
        usize::MAX,
    )
    .expect("session starts");
    let old = session.cold().expect("cold run");
    let mut alternate = Session::start(
        template(),
        "alternate-artifacts",
        RevisionId::new(1),
        replacement,
        usize::MAX,
    )
    .expect("alternate session");
    let expected = alternate.cold().expect("alternate run").artifacts;
    assert_ne!(expected[0].hash(), old.artifacts[0].hash());
    // Model the accepted detached sequence after a splice while deliberately
    // retaining the old frozen substrate.
    session.artifacts = expected.clone();
    let world = session.finalize().expect("session finalizes");
    assert_eq!(world.committed_artifacts(), expected);
    for artifact in expected {
        assert_eq!(
            world
                .read_artifact(artifact.hash())
                .expect("accepted artifact is published"),
            Some(artifact.bytes().to_vec())
        );
    }
}
