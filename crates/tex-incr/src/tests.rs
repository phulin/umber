use super::*;
use tex_state::RootSpanId;

mod long_session;

const CMR10: &[u8] = include_bytes!("../../tex-fonts/tests/fixtures/cm/cmr10.tfm");

fn template() -> Universe {
    let mut universe = Universe::with_world(tex_state::World::memory()).with_plain_catcodes();
    // tex.web §75 starts a job in `error_stop_mode`, and §82 enters §83's
    // dialog on that alone. These sessions run a memory terminal with nothing
    // in it, which §71 answers with `fatal_error`; they are about incremental
    // reuse, not the terminal, so they run the job a `\nonstopmode` document
    // runs.
    universe.set_interaction_mode(tex_state::InteractionMode::Nonstop);
    tex_exec::install_unexpandable_primitives(&mut universe);
    tex_command::install_tex82_expandable_primitives(&mut universe);
    universe
}

fn template_without_preinstalled_primitives() -> Universe {
    let mut universe = Universe::with_world(tex_state::World::memory()).with_plain_catcodes();
    universe.set_interaction_mode(tex_state::InteractionMode::Nonstop);
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
fn cold_history_contains_only_named_restartable_boundaries() {
    let text = source("a");
    let mut session = Session::start(template(), "test", RevisionId::new(1), text, usize::MAX)
        .expect("session starts");
    let output = session.cold().expect("cold execution succeeds");
    assert_eq!(
        session.history()[0].key().boundary,
        EngineBoundary::JobStart
    );
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
                .has_rendered_origin(1, event, Some(0))
                .expect("render lookup")
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
fn zero_render_cache_budget_rebuilds_without_changing_output() {
    let text = "\\font\\tenrm=cmr10\\relax\\tenrm\\shipout\\hbox{A}\\end";
    let mut session = Session::start(
        template(),
        "render-cache-budget",
        RevisionId::new(1),
        text,
        usize::MAX,
    )
    .expect("session starts");
    session.set_render_cache_budget(0);
    session
        .register_input_file(Path::new("cmr10.tfm"), CMR10.to_vec())
        .expect("font fixture registers");
    let accepted = session.cold().expect("cold execution succeeds");
    let event = (0..32)
        .find(|&event| {
            session
                .has_rendered_origin(1, event, Some(0))
                .expect("render lookup")
        })
        .expect("source-backed text event");
    let first = session
        .rendered_source_origin(1, event, Some(0))
        .expect("first source query");
    let lowerings = session.page_lowerings(1);
    let second = session
        .rendered_source_origin(1, event, Some(0))
        .expect("second source query");
    assert_eq!(first, second);
    assert_eq!(session.page_lowerings(1), lowerings + 1);
    assert_eq!(session.render_maps.borrow().retained_bytes(), 0);

    session.evict_rebuildable_caches();
    let current = session.output(ReuseMetrics::default(), accepted.retention);
    assert_eq!(current.effects, accepted.effects);
    assert_eq!(current.artifacts, accepted.artifacts);
    assert_eq!(current.dvi_pages, accepted.dvi_pages);
}

#[test]
fn published_output_is_detached_from_the_session_lifetime() {
    let accepted = {
        let text = "\\font\\tenrm=cmr10\\relax\\tenrm\\shipout\\hbox{A}\\end";
        let mut session = Session::start(
            template(),
            "detached-output",
            RevisionId::new(1),
            text,
            usize::MAX,
        )
        .expect("session starts");
        session
            .register_input_file(Path::new("cmr10.tfm"), CMR10.to_vec())
            .expect("font fixture registers");
        let accepted = session.cold().expect("cold execution succeeds");
        assert!(!session.history().is_empty());
        accepted
    };

    assert_eq!(accepted.artifacts.len(), 1);
    assert!(accepted.artifacts[0].render_provenance_bytes() > 0);
    assert!(!accepted.artifacts[0].bytes().is_empty());
    assert!(!accepted.dvi_bytes().expect("detached DVI").is_empty());
}

#[test]
fn dropping_prepared_revision_discards_all_provisional_output() {
    let original = source("a");
    let replacement = source("b");
    let mut session = Session::start(
        template(),
        "provisional-output",
        RevisionId::new(1),
        original.clone(),
        usize::MAX,
    )
    .expect("session starts");
    session
        .register_input_file(Path::new("cmr10.tfm"), CMR10.to_vec())
        .expect("font fixture registers");
    session.cold().expect("initial revision accepts");
    let accepted_output = session.output.clone();
    let accepted_history = session
        .history()
        .iter()
        .map(BoundaryRecord::key)
        .collect::<Vec<_>>();

    let pending = session
        .prepare_revision_with_resolvers(
            RevisionId::new(2),
            Edit {
                base_revision: RevisionId::new(1),
                expected_hash: ContentHash::from_bytes(original.as_bytes()),
                range: 0..original.len(),
                replacement,
            },
            &mut DirectResourceHost,
        )
        .expect("edited revision prepares");
    assert!(!pending.artifacts().is_empty());
    drop(pending);

    assert_eq!(session.revision(), RevisionId::new(1));
    assert_eq!(session.output, accepted_output);
    assert_eq!(
        session
            .history()
            .iter()
            .map(BoundaryRecord::key)
            .collect::<Vec<_>>(),
        accepted_history
    );
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
                .has_rendered_origin(1, event, Some(0))
                .expect("first render lookup")
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
fn no_op_revision_converges_and_preserves_cold_output() {
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
    let expected_convergence = session.history().get(1).map(BoundaryRecord::key);
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
    assert_eq!(output.reuse.convergence_boundary, expected_convergence);
    assert!(output.reuse.pages_reused > 0);
    assert_eq!(output.reuse.same_history_stop, SameHistoryStop::Matched);
    assert_eq!(output.reuse.same_history_attempts, 1);
    assert_eq!(output.reuse.same_history_hash_mismatches, 0);
    assert!(output.reuse.reexecuted_bytes > 0);
    assert!(output.reuse.reexecuted_tokens > 0);
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
                .has_rendered_origin(1, event, None)
                .expect("render lookup")
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
                .has_rendered_origin(2, event, None)
                .expect("render lookup")
        })
        .expect("reused B text event");
    let b_origin = session
        .rendered_artifact_origin(2, b_event, None)
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
    let resolved = match b_origin {
        ArtifactOrigin::Rooted(origin) => session
            .substrate
            .as_ref()
            .expect("retained substrate")
            .resolve_layout_rooted_origin(&origin, &session.fragments, &session.layout),
        ArtifactOrigin::Stable(span) => session
            .substrate
            .as_ref()
            .expect("retained substrate")
            .resolve_stable_layout_origin(span, &session.fragments, &session.layout),
        ArtifactOrigin::Live(_) | ArtifactOrigin::Unknown => {
            panic!("direct shipout must publish an artifact-owned root or recipe")
        }
    };
    assert_eq!(
        resolved,
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
    assert_eq!(session.fragments.bytes(initial), None);
    assert_eq!(session.fragments.source_bytes(), replacement.len());
}

#[derive(Default)]
struct StagedInputResolver {
    files: BTreeMap<String, String>,
}

#[derive(Default)]
struct StagedResourceHost<'a> {
    files: Option<&'a mut BTreeMap<String, String>>,
}

impl<'a> StagedResourceHost<'a> {
    fn new(inputs: &'a mut StagedInputResolver) -> Self {
        Self {
            files: Some(&mut inputs.files),
        }
    }
}

impl ResourceHost for StagedResourceHost<'_> {
    fn fulfill(&mut self, world: &mut ResourceWorld<'_>, need: &ResourceNeed) -> ResourceOutcome {
        match need {
            ResourceNeed::Input { name, .. } => self
                .files
                .as_deref()
                .and_then(|files| {
                    files
                        .get(name)
                        .or_else(|| name.strip_suffix(".tex").and_then(|stem| files.get(stem)))
                })
                .map_or(ResourceOutcome::Unavailable, |source| {
                    ResourceOutcome::Fulfilled(ResourceFulfillment::input(
                        name,
                        RegisteredSourceKind::Generated,
                        Arc::<[u8]>::from(source.as_bytes()),
                    ))
                }),
            ResourceNeed::InputProbe { request } => self
                .files
                .as_deref()
                .and_then(|files| {
                    files.get(&request.name).or_else(|| {
                        request
                            .name
                            .strip_suffix(".tex")
                            .and_then(|stem| files.get(stem))
                    })
                })
                .map_or(ResourceOutcome::Unavailable, |source| {
                    ResourceOutcome::Fulfilled(ResourceFulfillment::InputProbe {
                        request: request.clone(),
                        resource: tex_command::FileEnquiryResource::new(
                            tex_command::SourceRegistration::new(
                                RegisteredSourceKind::Generated,
                                Arc::<[u8]>::from(source.as_bytes()),
                            ),
                            None,
                        ),
                    })
                }),
            ResourceNeed::Font { request } => world
                .read_file(canonical_font_resource_path(&request.name))
                .ok()
                .map_or(ResourceOutcome::Unavailable, |metrics| {
                    ResourceOutcome::Fulfilled(ResourceFulfillment::Font {
                        request: request.clone(),
                        resource: Box::new(tex_command::FontResource::Tfm {
                            metrics,
                            opentype: None,
                        }),
                    })
                }),
            ResourceNeed::PdfImage { .. } => ResourceOutcome::Unavailable,
        }
    }
}

#[test]
fn candidate_root_eof_is_one_fatal_completion() {
    let session = Session::start(
        template_without_preinstalled_primitives(),
        "missing-end",
        RevisionId::new(1),
        "\\endinput",
        usize::MAX,
    )
    .expect("session starts");
    let mut candidate = session.start_cold_candidate().expect("cold candidate");

    assert!(matches!(
        candidate
            .drive_with_resource_resolvers(
                &mut StagedResourceHost::default(),
                &Cancellation::new(),
            )
            .expect("fatal root EOF reaches completion"),
        RevisionCandidateResult::Complete
    ));
    let effects = candidate
        .completed_universe_mut()
        .expect("completed candidate exposes its universe")
        .world()
        .effect_records()
        .len();
    assert_eq!(
        candidate
            .completed_universe_mut()
            .expect("completed candidate exposes its universe")
            .world()
            .error_channel()
            .history(),
        tex_state::print::ErrorHistory::FatalErrorStop
    );

    assert!(matches!(
        candidate
            .drive_with_resource_resolvers(
                &mut StagedResourceHost::default(),
                &Cancellation::new(),
            )
            .expect("completion remains latched"),
        RevisionCandidateResult::Complete
    ));
    assert_eq!(
        candidate
            .completed_universe_mut()
            .expect("completed candidate exposes its universe")
            .world()
            .effect_records()
            .len(),
        effects
    );
    let telemetry = candidate.execution_telemetry();
    assert_eq!(telemetry.local_step_retries, 0);
    assert_eq!(telemetry.replayed_delivered_tokens, 0);
    assert_eq!(telemetry.replayed_dispatches, 0);
}

#[test]
fn candidate_retries_staged_missing_input_without_losing_state() {
    let mut session = Session::start(
        template_without_preinstalled_primitives(),
        "staged-canonical-input",
        RevisionId::new(1),
        "\\input child \\end",
        usize::MAX,
    )
    .expect("session starts");
    let mut inputs = StagedInputResolver::default();
    let mut candidate = session.start_cold_candidate().expect("cold candidate");
    let awaiting = candidate
        .drive_with_resource_resolvers(
            &mut DecliningStagedResourceHost::new(&mut inputs),
            &Cancellation::new(),
        )
        .expect("missing input suspends");
    assert!(matches!(
        awaiting,
        RevisionCandidateResult::AwaitingResources(ResourceNeed::Input { ref name, .. })
            if name == "child.tex"
    ));
    inputs.files.insert(
        "child".to_owned(),
        "\\message{child-ok}\\shipout\\vbox{\\hrule height1pt width2pt}".to_owned(),
    );
    assert!(matches!(
        candidate
            .drive_with_resource_resolvers(
                &mut DecliningStagedResourceHost::new(&mut inputs),
                &Cancellation::new(),
            )
            .expect("provisioned retry completes"),
        RevisionCandidateResult::Complete
    ));
    let telemetry = candidate.execution_telemetry();
    assert_eq!(telemetry.suspensions, 1);
    assert_eq!(telemetry.local_step_retries, 1);
    assert_eq!(telemetry.replayed_delivered_tokens, 0);
    assert_eq!(telemetry.replayed_dispatches, 0);
    let accepted = session
        .accept_cold_candidate(candidate)
        .expect("completed retry accepts");

    let mut cold = Session::start(
        template_without_preinstalled_primitives(),
        "staged-canonical-input",
        RevisionId::new(1),
        "\\input child \\end",
        usize::MAX,
    )
    .expect("cold comparison starts");
    cold.register_input_file(
        Path::new("child.tex"),
        b"\\message{child-ok}\\shipout\\vbox{\\hrule height1pt width2pt}".to_vec(),
    )
    .expect("cold comparison input registers");
    let cold = cold.cold().expect("cold comparison succeeds");
    assert_eq!(accepted.effects, cold.effects);
    assert_eq!(accepted.artifacts, cold.artifacts);
    assert_eq!(accepted.dvi_pages, cold.dvi_pages);
    assert_eq!(
        accepted.dvi_bytes().expect("retried DVI"),
        cold.dvi_bytes().expect("cold DVI")
    );
}

#[test]
fn cold_candidate_prunes_speculative_history_before_acceptance_without_changing_output() {
    let text = multi_page_source(32);
    let mut bounded = Session::start(
        template(),
        "bounded-candidate",
        RevisionId::new(1),
        text.clone(),
        0,
    )
    .expect("bounded session starts");
    let mut bounded_candidate = bounded
        .start_cold_candidate()
        .expect("bounded candidate starts");
    assert!(matches!(
        bounded_candidate
            .drive_with_resource_resolvers(&mut StagedResourceHost::default(), &Cancellation::new())
            .expect("bounded candidate completes"),
        RevisionCandidateResult::Complete
    ));
    let CandidateSink::Cold(bounded_sink) = &bounded_candidate.sink else {
        panic!("cold candidate retains a cold history sink");
    };
    assert_eq!(bounded_sink.records.len(), 2);
    assert_eq!(
        bounded_sink.records[0].key().boundary,
        EngineBoundary::JobStart
    );
    assert_eq!(
        bounded_sink.records[1].key().boundary,
        EngineBoundary::ShipoutComplete
    );
    let bounded_output = bounded
        .accept_cold_candidate(bounded_candidate)
        .expect("bounded candidate accepts");

    let mut unbounded = Session::start(
        template(),
        "bounded-candidate",
        RevisionId::new(1),
        text,
        usize::MAX,
    )
    .expect("unbounded session starts");
    let mut unbounded_candidate = unbounded
        .start_cold_candidate()
        .expect("unbounded candidate starts");
    assert!(matches!(
        unbounded_candidate
            .drive_with_resource_resolvers(&mut StagedResourceHost::default(), &Cancellation::new())
            .expect("unbounded candidate completes"),
        RevisionCandidateResult::Complete
    ));
    let CandidateSink::Cold(unbounded_sink) = &unbounded_candidate.sink else {
        panic!("cold candidate retains a cold history sink");
    };
    assert_eq!(unbounded_sink.records.len(), 33);
    let unbounded_output = unbounded
        .accept_cold_candidate(unbounded_candidate)
        .expect("unbounded candidate accepts");

    assert_eq!(bounded_output.dvi_pages, unbounded_output.dvi_pages);
    assert_eq!(bounded_output.artifacts, unbounded_output.artifacts);
    assert_eq!(bounded_output.effects, unbounded_output.effects);
}

#[test]
fn suspended_candidate_keeps_only_protected_history_under_budget() {
    let mut text = multi_page_source(16);
    text.truncate(text.len() - "\\end".len());
    text.push_str("\\input child \\end");
    let mut session = Session::start(
        template_without_preinstalled_primitives(),
        "bounded-suspension",
        RevisionId::new(1),
        text,
        0,
    )
    .expect("session starts");
    let mut inputs = StagedInputResolver::default();
    let mut candidate = session.start_cold_candidate().expect("cold candidate");
    assert!(matches!(
        candidate
            .drive_with_resource_resolvers(
                &mut DecliningStagedResourceHost::new(&mut inputs),
                &Cancellation::new(),
            )
            .expect("candidate suspends"),
        RevisionCandidateResult::AwaitingResources(ResourceNeed::Input { ref name, .. })
            if name == "child.tex"
    ));
    let CandidateSink::Cold(sink) = &candidate.sink else {
        panic!("cold candidate retains a cold history sink");
    };
    assert_eq!(sink.records.len(), 2);
    assert_eq!(sink.records[0].key().boundary, EngineBoundary::JobStart);

    inputs
        .files
        .insert("child".to_owned(), "\\relax".to_owned());
    assert!(matches!(
        candidate
            .drive_with_resource_resolvers(
                &mut DecliningStagedResourceHost::new(&mut inputs),
                &Cancellation::new(),
            )
            .expect("provisioned retry completes"),
        RevisionCandidateResult::Complete
    ));
    session
        .accept_cold_candidate(candidate)
        .expect("completed retry accepts");
}

#[test]
fn initex_session_installs_everybox_hooks_for_its_profile() {
    let mut session = Session::start(
        template_without_preinstalled_primitives(),
        "canonical-initex-hooks",
        RevisionId::new(1),
        "\\unless\\iffalse\\message{ETEX=1}\\fi\\everyhbox{\\message{HOOKS=1}}\\setbox0=\\hbox{X}\\end",
        usize::MAX,
    )
    .expect("session starts");
    session.set_command_profile(tex_command::CommandProfile::ETEX26, true);
    let mut candidate = session.start_cold_candidate().expect("cold candidate");
    assert!(matches!(
        candidate
            .drive_with_resource_resolvers(&mut StagedResourceHost::default(), &Cancellation::new())
            .expect("canonical INITEX session completes"),
        RevisionCandidateResult::Complete
    ));
    let accepted = session
        .accept_cold_candidate(candidate)
        .expect("completed candidate accepts");

    assert!(accepted.effects.iter().any(|effect| {
        matches!(effect, tex_state::EffectRecord::StreamWrite { text, .. } if text.contains("HOOKS=1"))
    }));
    assert!(accepted.effects.iter().any(|effect| {
        matches!(effect, tex_state::EffectRecord::StreamWrite { text, .. } if text.contains("ETEX=1"))
    }));
}

struct DecliningStagedResourceHost<'a>(StagedResourceHost<'a>);

impl<'a> DecliningStagedResourceHost<'a> {
    fn new(inputs: &'a mut StagedInputResolver) -> Self {
        Self(StagedResourceHost::new(inputs))
    }
}

impl ResourceHost for DecliningStagedResourceHost<'_> {
    fn fulfill(&mut self, world: &mut ResourceWorld<'_>, need: &ResourceNeed) -> ResourceOutcome {
        match self.0.fulfill(world, need) {
            ResourceOutcome::Unavailable => ResourceOutcome::Declined,
            outcome => outcome,
        }
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
    let initial_live_bytes = session.fragments.source_bytes();
    let mut peak_live_bytes = initial_live_bytes;

    for (name, contents) in [
        ("one", "\\shipout\\vbox{\\hrule height 1pt}"),
        ("two", "\\shipout\\vbox{\\hrule height 2pt}"),
    ] {
        session
            .advance_with_resolvers(
                RevisionId::new(2),
                edit.clone(),
                &mut DecliningStagedResourceHost::new(&mut inputs),
            )
            .expect_err("unresolved input rejects this attempt");
        peak_live_bytes = peak_live_bytes.max(session.fragments.source_bytes());
        assert_eq!(session.fragments.source_bytes(), initial_live_bytes);
        inputs.files.insert(name.to_owned(), contents.to_owned());
    }
    assert_eq!(peak_live_bytes, initial_live_bytes);

    let accepted = session
        .advance_with_resolvers(
            RevisionId::new(2),
            edit,
            &mut DecliningStagedResourceHost::new(&mut inputs),
        )
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
        .cold_with_resolvers(&mut StagedResourceHost::new(&mut cold_inputs))
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
fn restored_suffix_resource_candidate_retries_once_and_rejects_atomically() {
    let original = "\\def\\live#1{#1}\\count0=1\\begingroup\\def\\live#1{#1#1}\\skip0=1pt plus 1fil\\message{group}\\endgroup\\shipout\\vbox{\\hrule height1pt width10pt}\n\\end";
    let value_offset = original.find("\\count0=1").expect("count assignment") + "\\count0=".len();
    let mut accepted_source = original.to_owned();
    accepted_source.replace_range(value_offset..value_offset + 1, "2");
    let mut session = Session::start(
        template(),
        "restored-resource-suffix",
        RevisionId::new(1),
        original,
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
                range: value_offset..value_offset + 1,
                replacement: "2".to_owned(),
            },
        )
        .expect("partial revision accepts");
    assert_eq!(
        adopted.reuse.execution_path,
        RevisionExecutionPath::SlowEdit
    );

    let before_retention = session.retention_metrics().expect("accepted retention");
    let before_output = session.output(ReuseMetrics::default(), before_retention);
    let before_state_hash = session
        .history()
        .last()
        .expect("accepted checkpoint")
        .state_hash();
    let before_layout_pieces = session.layout.pieces().to_vec();
    let before_layout_doc_starts = session.layout.doc_starts().to_vec();
    let end = accepted_source.rfind("\\end").expect("terminal end");
    let mut candidate = session
        .start_advance_candidate(
            RevisionId::new(3),
            Edit {
                base_revision: RevisionId::new(2),
                expected_hash: ContentHash::from_bytes(accepted_source.as_bytes()),
                range: end..accepted_source.len(),
                replacement: "\\input missing \\end".to_owned(),
            },
        )
        .expect("restored suffix candidate starts");
    let RevisionCandidateKind::Incremental { setup, restart, .. } = &candidate.kind else {
        panic!("suffix edit must restore an accepted checkpoint");
    };
    assert_eq!(
        setup.execution_path,
        RevisionExecutionPath::SlowEdit,
        "the regression must use checkpoint restore, not forced JobStart replacement"
    );
    assert_eq!(
        setup.old_history[*restart].key.boundary,
        EngineBoundary::ShipoutComplete
    );

    let mut inputs = StagedInputResolver::default();
    let first_attempt = candidate
        .drive_with_resource_resolvers(
            &mut DecliningStagedResourceHost::new(&mut inputs),
            &Cancellation::new(),
        )
        .expect("missing resource suspends");
    assert!(
        matches!(
            first_attempt,
        RevisionCandidateResult::AwaitingResources(ResourceNeed::Input { ref name, .. })
            if name == "missing.tex"
        ),
        "unexpected first attempt: {first_attempt:?}"
    );
    assert_eq!(candidate.execution_telemetry().suspensions, 1);
    inputs
        .files
        .insert("missing".to_owned(), "\\relax".to_owned());
    assert!(matches!(
        candidate
            .drive_with_resource_resolvers(
                &mut DecliningStagedResourceHost::new(&mut inputs),
                &Cancellation::new(),
            )
            .expect("fulfilled candidate completes"),
        RevisionCandidateResult::Complete
    ));
    let telemetry = candidate.execution_telemetry();
    assert_eq!(telemetry.suspensions, 1);
    assert_eq!(telemetry.local_step_retries, 1);
    assert_eq!(telemetry.replayed_delivered_tokens, 0);
    assert_eq!(telemetry.replayed_dispatches, 0);
    assert_eq!(candidate.suspension_serial(), 1);
    drop(candidate);

    assert_eq!(session.revision(), RevisionId::new(2));
    assert_eq!(session.source(), accepted_source);
    assert_eq!(session.layout.pieces(), before_layout_pieces);
    assert_eq!(session.layout.doc_starts(), before_layout_doc_starts);
    assert_eq!(session.retention_metrics(), Some(before_retention));
    assert_eq!(
        session
            .history()
            .last()
            .expect("accepted checkpoint")
            .state_hash(),
        before_state_hash
    );
    let after_output = session.output(ReuseMetrics::default(), before_retention);
    assert_eq!(after_output.effects, before_output.effects);
    assert_eq!(after_output.artifacts, before_output.artifacts);
    assert_eq!(after_output.dvi_pages, before_output.dvi_pages);
    assert_eq!(
        after_output.dvi_bytes().expect("output DVI"),
        before_output.dvi_bytes().expect("accepted DVI")
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
    assert_eq!(edited.reuse.execution_path, RevisionExecutionPath::FastEdit);

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
        session.history().first().expect("job start").key().boundary,
        EngineBoundary::JobStart
    );
    assert!(session.history().len() <= 2);
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
    ))
    .with_plain_catcodes();
    tex_exec::install_unexpandable_primitives(&mut universe);
    tex_command::install_tex82_expandable_primitives(&mut universe);
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
fn root_framing_name_is_distinct_from_editor_provenance_path() {
    // tex.web §537 prints the startup filename. The editor's canonical VFS
    // path remains the source/provenance identity and must not leak there.
    let mut universe = template();
    let _control = candidate_control(
        &mut universe,
        CandidateControlOptions {
            job_name: "job",
            source_path: "/job/main.tex",
            bytes: b"\\end".to_vec(),
            profile: CommandProfile::TEX82,
            initex: true,
            emit_dvi: true,
            root_framing: SourceFramingPolicy::Canonical,
            root_framing_name: Some("./main.tex"),
        },
    )
    .expect("candidate starts");
    let terminal = universe
        .world()
        .effect_records()
        .iter()
        .filter_map(|effect| match effect {
            EffectRecord::StreamWrite {
                sink: tex_state::PrintSink::Terminal | tex_state::PrintSink::TerminalAndLog,
                text,
            } => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert!(terminal.starts_with("(./main.tex"), "{terminal:?}");
    assert!(!terminal.contains("/job/main.tex"), "{terminal:?}");
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
    session.output = tex_exec::RevisionOutputPatch::recompose(
        session.output.effects().clone(),
        expected.clone(),
        alternate.output.artifacts().publications().to_vec(),
        alternate.output.dvi_pages().to_vec(),
    )
    .expect("replacement output is aligned");
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

#[test]
fn accepted_history_hands_openout_page_to_prepared_finalization() {
    let text = "\\setbox0=\\hbox{\\openout2=original.out \\write2{x}\\closeout2}\
                \\shipout\\copy0\\end";
    let mut session = Session::start(
        template(),
        "prepared-openout",
        RevisionId::new(1),
        text,
        usize::MAX,
    )
    .expect("session starts");
    let accepted = session.cold().expect("revision accepts");
    assert_eq!(accepted.revision, RevisionId::new(1));
    assert_eq!(accepted.artifacts.len(), 1);
    assert!(
        session
            .history()
            .iter()
            .any(|record| record.artifact_prefix() == 1),
        "accepted history owns the logical page prefix"
    );

    let AcceptedUniverseFinalization {
        universe,
        prepared_pages,
    } = session
        .into_accepted_universe()
        .expect("accepted finalization handoff");
    assert!(universe.world().committed_artifacts().is_empty());
    assert!(universe.pdf_pages().is_empty());
    assert_eq!(
        prepared_pages
            .as_ref()
            .expect("prepared suffix")
            .artifacts()
            .len(),
        1
    );
}

#[test]
fn adopted_openout_suffix_rebases_positive_and_negative_effect_prefix_deltas() {
    let text = "\\setbox0=\\hbox{\\openout2=original.out \\write2{x}\\closeout2}\
                \\shipout\\copy0\\end";
    let mut session = Session::start(
        template(),
        "adopted-openout-rebase",
        RevisionId::new(1),
        text,
        usize::MAX,
    )
    .expect("session starts");
    let accepted = session.cold().expect("revision accepts");
    let open_index = accepted
        .effects
        .iter()
        .position(|effect| matches!(effect, EffectRecord::StreamOpen { .. }))
        .expect("accepted output contains OpenOut");
    let original_position = accepted.artifacts[0].open_out_occurrences()[0].1;
    assert_eq!(original_position.raw(), (open_index + 1) as u64);

    // Model adoption after a scratch prefix gained an effect which lowering
    // omitted from the prepared page. The adopted OpenOut remains the first
    // effect in the old suffix, one absolute position later in the joined log.
    let mut positive_effects = accepted.effects.clone();
    positive_effects.insert(
        open_index,
        EffectRecord::StreamWriteBytes {
            sink: tex_state::PrintSink::Log,
            bytes: b"omitted-prefix-model".to_vec(),
        },
    );
    let mut positive_artifacts = accepted.artifacts.clone();
    rebase_and_validate_adopted_artifacts(
        &mut positive_artifacts,
        open_index,
        open_index + 1,
        &positive_effects,
    )
    .expect("positive prefix delta rebases adopted suffix");
    assert_eq!(
        positive_artifacts[0].open_out_occurrences()[0].1.raw(),
        original_position.raw() + 1
    );

    // The inverse splice removes that scratch-prefix effect. Starting with
    // the positively shifted accepted artifact proves subtraction rather than
    // merely reconstructing the original sidecar.
    rebase_and_validate_adopted_artifacts(
        &mut positive_artifacts,
        open_index + 1,
        open_index,
        &accepted.effects,
    )
    .expect("negative prefix delta rebases adopted suffix");
    assert_eq!(
        positive_artifacts[0].open_out_occurrences()[0].1,
        original_position
    );
}
