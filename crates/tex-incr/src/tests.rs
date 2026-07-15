use super::*;

const CMR10: &[u8] = include_bytes!("../../tex-fonts/tests/fixtures/cm/cmr10.tfm");

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
    session
        .rendered_source_location(1, event, Some(0), RevisionId::new(1))
        .expect("source query")
        .expect("mapped source");
    session
        .rendered_source_location(1, event, Some(0), RevisionId::new(1))
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
            .rendered_source_location(2, b_event, None, RevisionId::new(3))
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
            .rendered_source_location(2, b_event, None, RevisionId::new(4))
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
        _request_index: u64,
    ) -> Result<Box<dyn InputSource>, String> {
        self.files
            .get(name)
            .cloned()
            .map(MemoryInput::new)
            .map(|source| Box::new(source) as Box<dyn InputSource>)
            .ok_or_else(|| format!("resource {name} is not available yet"))
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
    assert_eq!(session.fragments.len(), 4, "failed ids remain burned");

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
    assert_eq!(session.fragments.len(), 5, "failed ids remain burned");

    session
        .cold()
        .expect("initial revision can still be accepted");
    let accepted = session
        .advance(RevisionId::new(2), edit)
        .expect("same pending edit later succeeds");
    assert_eq!(session.fragments.source_bytes(), replacement.len());
    assert_eq!(session.fragments.len(), 6);

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
