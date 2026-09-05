use super::*;

#[test]
fn literal_hints_ignore_comments_and_expand_package_lists_only() {
    let source = r#"\documentclass[11pt]{article}
\usepackage{amsmath, graphicx} % \input{ignored}
\RequirePackage foo
\input{chapter-one}
\includegraphics[width=2cm]{figures/one}
"#;
    let hints = extract_literal_hints(source, LiteralHintLimits::default());
    let names = hints
        .iter()
        .map(|hint| hint.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "article",
            "amsmath",
            "graphicx",
            "foo",
            "chapter-one",
            "figures/one"
        ]
    );
    assert_eq!(hints[0].kind, LiteralHintKind::DocumentClass);
    assert_eq!(hints[5].kind, LiteralHintKind::IncludeGraphics);
}

#[test]
fn malformed_or_dynamic_arguments_are_not_hints() {
    let source = r#"\usepackage{foo % missing close
\input \macro
\usepackage{ok}
"#;
    let hints = extract_literal_hints(source, LiteralHintLimits::default());
    assert_eq!(
        hints
            .iter()
            .map(|hint| hint.name.as_str())
            .collect::<Vec<_>>(),
        ["ok"]
    );
}

#[test]
fn group_selection_keeps_required_and_separate_class_budgets() {
    let entry = |key: &str, bytes: u64| PrefetchCandidate {
        key: key.to_owned(),
        object: ObjectEntry {
            object: format!("ahash64-v1-{key:0<16}"),
            ahash64: "0123456789abcdef".to_owned(),
            bytes,
        },
        class: PrefetchClass::for_key(key),
        required: false,
    };
    let selected = select_prefetch_group(
        [entry("tex:required.tex", 100)],
        [
            entry("tex:runtime.sty", 50),
            entry("tfm:font.tfm", 60),
            entry("tex:large.sty", 60),
        ],
        PrefetchBudget {
            max_files: 8,
            max_bytes: 200,
            max_runtime_bytes: 50,
            max_font_bytes: 60,
            max_image_bytes: 0,
            max_document_bytes: 0,
            ..PrefetchBudget::default()
        },
    );
    assert_eq!(selected.required.len(), 1);
    assert_eq!(
        selected
            .hints
            .iter()
            .map(|candidate| candidate.key.as_str())
            .collect::<Vec<_>>(),
        ["tex:runtime.sty", "tfm:font.tfm",]
    );
    assert_eq!(selected.demand_bytes, 100);
    assert_eq!(selected.prefetch_bytes, 110);
}

#[test]
fn group_selection_charges_shared_payload_once() {
    let candidate = |key: &str| PrefetchCandidate {
        key: key.to_owned(),
        object: ObjectEntry {
            object: "ahash64-v1-shared".to_owned(),
            ahash64: "0123456789abcdef".to_owned(),
            bytes: 100,
        },
        class: PrefetchClass::SmallRuntime,
        required: false,
    };
    let selected = select_prefetch_group(
        [],
        [candidate("tex:one.sty"), candidate("tex:alias.sty")],
        PrefetchBudget {
            max_bytes: 100,
            max_runtime_bytes: 100,
            ..PrefetchBudget::default()
        },
    );
    assert_eq!(selected.hints.len(), 2);
    assert_eq!(selected.prefetch_bytes, 100);
}

#[test]
fn admitted_runtime_closure_is_bounded_and_deduplicated() {
    let mut policy = PrefetchPolicy::new(PrefetchBudget {
        max_followup_depth: 1,
        max_followup_hints: 4,
        ..PrefetchBudget::default()
    });
    assert!(policy.enqueue(PrefetchRequest::new(
        "tex:root.sty",
        "root.sty",
        "literal",
        false,
    )));
    let root = policy.drain(64);
    assert_eq!(root.len(), 1);
    policy.admitted("tex:root.sty", br#"\input{child.tex}\input{child.tex}"#);
    let child = policy.drain(64);
    assert_eq!(
        child
            .iter()
            .map(|request| request.key.as_str())
            .collect::<Vec<_>>(),
        ["tex:child.tex"]
    );
    policy.admitted("tex:child.tex", br#"\input{root.sty}\input{grand.tex}"#);
    assert!(policy.drain(64).is_empty());
    assert_eq!(policy.metrics().followup_hints, 1);
}

#[test]
fn admitted_runtime_scan_budget_is_cumulative() {
    let source = br#"\input{child.tex}"#;
    let mut policy = PrefetchPolicy::new(PrefetchBudget {
        max_runtime_scan_bytes: source.len() as u64,
        ..PrefetchBudget::default()
    });
    policy.admitted("tex:first.sty", source);
    policy.admitted("tex:second.sty", source);
    assert_eq!(policy.metrics().scanned_runtime_bytes, source.len() as u64);
    assert_eq!(policy.metrics().followup_hints, 1);
}

#[test]
fn followup_hint_budget_is_shared_across_admitted_runtime_files() {
    let mut policy = PrefetchPolicy::new(PrefetchBudget {
        max_followup_hints: 1,
        ..PrefetchBudget::default()
    });
    policy.admitted("tex:first.sty", br#"\input{first-child.tex}"#);
    policy.admitted("tex:second.sty", br#"\input{second-child.tex}"#);
    assert_eq!(policy.metrics().followup_hints, 1);
    assert_eq!(policy.drain(64).len(), 1);
}

#[test]
fn semantic_image_admission_never_scans_runtime_looking_spelling() {
    let mut policy = PrefetchPolicy::new(PrefetchBudget::default());
    policy.admitted_with_class(
        "tex:figure.sty",
        PrefetchClass::Image,
        br#"\input{not-a-child.tex}"#,
        std::iter::empty(),
    );
    assert_eq!(policy.metrics().scanned_runtime_bytes, 0);
    assert!(policy.drain(64).is_empty());
}

#[test]
fn admitted_dependency_metadata_expands_replay_tiers_without_cycles() {
    let mut policy = PrefetchPolicy::new(PrefetchBudget::default());
    let request = PrefetchRequest::new("tex:root.sty", "root.sty", "required", false);
    let child = PrefetchRequest::new("tex:child.sty", "child.sty", "metadata", false);
    let grandchild =
        PrefetchRequest::new("tex:grandchild.sty", "grandchild.sty", "metadata", false);
    policy.admitted_with_metadata("tex:root.sty", "/tex/root.sty", b"root", [child.clone()]);
    policy.admitted_with_metadata(
        "tex:child.sty",
        "/tex/child.sty",
        b"child",
        [grandchild.clone(), request],
    );
    let closure = policy.dependency_closure("tex:root.sty", 2);
    assert_eq!(
        closure
            .iter()
            .map(|request| request.key.as_str())
            .collect::<Vec<_>>(),
        ["tex:child.sty", "tex:grandchild.sty"]
    );
    assert!(closure.iter().all(|request| request.depth() == 0));
}

#[test]
fn demanded_queue_items_survive_speculative_file_cap() {
    let mut policy = PrefetchPolicy::new(PrefetchBudget {
        max_files: 0,
        ..PrefetchBudget::default()
    });
    assert!(policy.enqueue(PrefetchRequest::new(
        "tex:demand.sty",
        "demand.sty",
        "required",
        true,
    )));
    assert!(!policy.enqueue(PrefetchRequest::new(
        "tex:hint.sty",
        "hint.sty",
        "literal",
        false,
    )));
    assert_eq!(policy.drain(64).len(), 1);
}

#[test]
fn demanded_duplicate_promotes_without_consuming_speculative_capacity() {
    let mut policy = PrefetchPolicy::new(PrefetchBudget {
        max_files: 1,
        ..PrefetchBudget::default()
    });
    assert!(policy.enqueue(PrefetchRequest::new(
        "tex:shared.sty",
        "shared.sty",
        "literal",
        false,
    )));
    assert!(policy.enqueue(PrefetchRequest::new(
        "tex:shared.sty",
        "./shared.sty",
        "required",
        true,
    )));
    assert!(policy.enqueue(PrefetchRequest::new(
        "tex:other.sty",
        "other.sty",
        "literal",
        false,
    )));
    let required = policy.drain(1);
    assert_eq!(required.len(), 1);
    assert!(required[0].required);
    assert_eq!(required[0].original_spelling, "./shared.sty");
    assert_eq!(policy.drain(1).len(), 1);
}

#[test]
fn replay_escalation_uses_region_and_discards_work_not_serials() {
    let mut policy = PrefetchPolicy::new(PrefetchBudget::default());
    let region = PrefetchRegionKey::new("outer-paragraph-end:12:3").expect("region");
    assert!(
        policy
            .note_replay(region.clone(), "tex:one.sty", 10)
            .is_none()
    );
    let escalation = policy
        .note_replay(region, "tex:two.sty", 20)
        .expect("same region escalates different request");
    assert_eq!(escalation.tier, 1);
    let other = PrefetchRegionKey::new("outer-paragraph-end:99:1").expect("region");
    assert!(policy.note_replay(other, "tex:one.sty", 30).is_none());
}
