use super::*;

fn template() -> Universe {
    let mut universe = Universe::with_world(tex_state::World::memory());
    tex_exec::install_unexpandable_primitives(&mut universe);
    tex_expand::install_expandable_primitives(&mut universe);
    universe
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
fn no_op_revision_converges_at_first_eligible_boundary() {
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
    assert_eq!(
        output.dvi_bytes().expect("incremental DVI"),
        cold.dvi_bytes().expect("cold DVI")
    );
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

    assert_eq!(
        job_start
            .rehome_converged_root(substrate, &original, "changed", 0)
            .expect_err("changed adopted interval is rejected"),
        GenerationForkError::ChangedRootInterval
    );
    assert_eq!(
        job_start
            .rehome_unchanged_prefix(substrate, "stale revision", &original)
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
