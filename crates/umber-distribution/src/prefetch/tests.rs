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
fn literal_hint_normalization_uses_kind_defaults_only_for_final_components() {
    let cases = [
        (LiteralHintKind::DocumentClass, "article", "article.cls"),
        (
            LiteralHintKind::DocumentClass,
            "latex/base/article.cls",
            "latex/base/article.cls",
        ),
        (LiteralHintKind::Package, "vendor/pkg", "vendor/pkg.sty"),
        (LiteralHintKind::Package, "vendor/pkg.sty", "vendor/pkg.sty"),
        (
            LiteralHintKind::Input,
            "chapters.v1/intro",
            "chapters.v1/intro.tex",
        ),
        (
            LiteralHintKind::Input,
            "chapters/intro.ltx",
            "chapters/intro.ltx",
        ),
        (
            LiteralHintKind::IncludeGraphics,
            "figures/plot",
            "figures/plot",
        ),
    ];
    for (kind, name, expected) in cases {
        assert_eq!(normalize_literal_hint_name(kind, name), expected);
    }
}

#[test]
fn literal_hints_enqueue_typed_default_keys_and_preserve_lookup_spelling() {
    let source = r#"\documentclass[11pt]{latex/base/article.cls}
\usepackage{vendor/amsmath.sty, vendor/graphicx}
\RequirePackage{vendor/keyval}
\input{chapters.v1/intro}
\include{parts/chapter.tex}
\includegraphics[width=2cm]{figures/plot}
"#;
    let mut policy = PrefetchPolicy::new(PrefetchBudget::default());
    assert_eq!(policy.enqueue_literal_hints(source), 7);
    let requests = policy.drain(64);
    assert_eq!(
        requests
            .iter()
            .map(|request| request.key.as_str())
            .collect::<Vec<_>>(),
        [
            "tex:latex/base/article.cls",
            "tex:vendor/amsmath.sty",
            "tex:vendor/graphicx.sty",
            "tex:vendor/keyval.sty",
            "tex:chapters.v1/intro.tex",
            "tex:parts/chapter.tex",
            "tex:figures/plot",
        ]
    );
    assert_eq!(
        requests
            .iter()
            .map(|request| request.original_spelling.as_str())
            .collect::<Vec<_>>(),
        [
            "latex/base/article.cls",
            "vendor/amsmath.sty",
            "vendor/graphicx",
            "vendor/keyval",
            "chapters.v1/intro",
            "parts/chapter.tex",
            "figures/plot",
        ]
    );
    assert!(
        requests[..6]
            .iter()
            .all(|request| request.class == PrefetchClass::SmallRuntime)
    );
    assert_eq!(requests[6].class, PrefetchClass::Image);
    assert_eq!(
        requests[2]
            .file_key
            .as_ref()
            .expect("typed request")
            .normalized_name,
        "vendor/graphicx.sty"
    );
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
fn font_metric_hints_extract_direct_and_literal_scaled_names() {
    let source = r#"
\DeclareFontShape {OT1}{ptm}{m}{n}{<-> ptmr7t}
\DeclareFontShape{OT1}{ptm}{b}{n}{<->s*[1.04]ptmb7t.tfm}
\DeclareFontShape{OT1}{ptm}{m}{it}{<-> s * [1.0] ptmri7t}
"#;
    let hints = extract_literal_hints(source, LiteralHintLimits::default());
    assert_eq!(
        hints
            .iter()
            .map(|hint| (
                hint.kind,
                hint.name.as_str(),
                hint.original_spelling.as_str()
            ))
            .collect::<Vec<_>>(),
        [
            (LiteralHintKind::FontMetric, "ptmr7t", "ptmr7t"),
            (LiteralHintKind::FontMetric, "ptmb7t.tfm", "ptmb7t.tfm"),
            (LiteralHintKind::FontMetric, "ptmri7t", "ptmri7t"),
        ]
    );
    assert_eq!(LiteralHintKind::FontMetric.command(), "DeclareFontShape");
    assert_eq!(LiteralHintKind::FontMetric.file_kind(), FileKind::Tfm);
    assert_eq!(
        normalize_literal_hint_name(LiteralHintKind::FontMetric, "ptmb7t.tfm.tfm"),
        "ptmb7t.tfm"
    );
}

#[test]
fn font_metric_hints_skip_dynamic_alias_and_trailing_payloads() {
    let source = r#"
\DeclareFontShape{OT1}{ptm}{m}{n}{<-> ssub * ptm/m/n}
\DeclareFontShape{OT1}{ptm}{m}{n}{<-> sub * ptm/m/n}
\DeclareFontShape{OT1}{ptm}{m}{n}{<-> gen * ptm}
\DeclareFontShape{OT1}{ptm}{m}{n}{<-> fixed * ptm}
\DeclareFontShape{OT1}{ptm}{m}{n}{<5-10> ptm}
\DeclareFontShape{OT1}{ptm}{m}{n}{<-> s * [\scale] ptm}
\DeclareFontShape{OT1}{ptm}{m}{n}{<-> s * [1.04] \fontname}
\DeclareFontShape{OT1}{ptm}{m}{n}{<-> \input{nested}}
\DeclareFontShape{OT1}{ptm}{m}{n}{<-> ptm extra}
"#;
    assert!(extract_literal_hints(source, LiteralHintLimits::default()).is_empty());
}

#[test]
fn font_metric_hints_honor_comments_group_bounds_and_literal_limits() {
    let source = r#"
\DeclareFontShape% declaration comment
  {OT1}% family comment
  {ptm}{m}{n}% series comment
  {<-> s % scale marker comment
    * [1.04] % scale comment
    ptmb7t}% payload comment
\DeclareFontShape{OT1}{ptm}{m}{n}{<-> ptmr7t trailing}
\DeclareFontShape{OT1}{ptm}{m}{n}{<-> ptm{nested}}
"#;
    let hints = extract_literal_hints(
        source,
        LiteralHintLimits {
            max_hints: 1,
            max_name_bytes: 6,
        },
    );
    assert_eq!(
        hints
            .iter()
            .map(|hint| hint.name.as_str())
            .collect::<Vec<_>>(),
        ["ptmb7t"]
    );

    let unbalanced = r#"\DeclareFontShape{OT1}{ptm}{m}{n}{<-> ptmr7t"#;
    assert!(extract_literal_hints(unbalanced, LiteralHintLimits::default()).is_empty());
}

#[test]
fn font_metric_prefetch_requests_use_tfm_identity_and_font_budget_class() {
    let source = r#"
\DeclareFontShape{OT1}{ptm}{m}{n}{<-> ptmr7t}
\DeclareFontShape{OT1}{ptm}{b}{n}{<-> s*[1.04]ptmr7t.tfm}
"#;
    let mut policy = PrefetchPolicy::new(PrefetchBudget {
        max_files: 1,
        ..PrefetchBudget::default()
    });
    assert_eq!(policy.enqueue_literal_hints(source), 1);
    let requests = policy.drain(2);
    let [request] = requests.as_slice() else {
        panic!("expected one bounded font request");
    };
    assert_eq!(request.key, "tfm:ptmr7t.tfm");
    assert_eq!(request.class, PrefetchClass::Font);
    assert_eq!(request.original_spelling, "ptmr7t");
    assert_eq!(
        request.file_key,
        Some(PrefetchFileKey::new("tex", "tfm", "ptmr7t.tfm").expect("typed TFM key"))
    );
}

#[test]
fn admitted_font_definition_scans_typed_metrics_with_bounded_closure() {
    let source = br#"
\DeclareFontShape{OT1}{ptm}{m}{n}{<-> ptmr7t}
\DeclareFontShape{OT1}{ptm}{b}{n}{<-> ptmb7t}
\DeclareFontShape{OT1}{ptm}{m}{it}{<-> ptmri7t}
\DeclareFontShape{OT1}{ptm}{b}{it}{<-> ptmbi7t}
"#;
    let definition_key =
        PrefetchFileKey::new("tex", "tex", "ot1ptm.fd").expect("typed font definition key");
    let definition = PrefetchRequest::for_file_key(
        definition_key,
        "tex:ot1ptm.fd",
        "ot1ptm.fd",
        "literal",
        false,
    );
    assert_eq!(definition.class, PrefetchClass::SmallRuntime);
    assert_eq!(
        PrefetchClass::for_key("tex:OT1PTM.FD"),
        PrefetchClass::SmallRuntime
    );
    assert_eq!(
        PrefetchClass::for_key("tex:ptmr7t.tfm"),
        PrefetchClass::Other
    );
    assert_eq!(
        PrefetchClass::for_key("tex:ptmr7t.pfb"),
        PrefetchClass::Other
    );

    let mut policy = PrefetchPolicy::new(PrefetchBudget {
        max_followup_depth: 1,
        max_followup_hints: 4,
        max_runtime_scan_bytes: source.len() as u64,
        ..PrefetchBudget::default()
    });
    assert!(policy.enqueue(definition.clone()));
    let drained = policy.drain(1);
    assert_eq!(drained.len(), 1);
    assert_eq!(drained[0], definition);

    // Use the normal request class derived from the transport key. This is
    // the same admission path used by native and WASM hosts; no runtime class
    // override is supplied here.
    policy.admitted_request_with_metadata(&definition, source, std::iter::empty());
    assert_eq!(policy.metrics().scanned_runtime_bytes, source.len() as u64);
    assert_eq!(policy.metrics().followup_hints, 4);

    let metrics = policy.drain(8);
    let expected_names = ["ptmr7t.tfm", "ptmb7t.tfm", "ptmri7t.tfm", "ptmbi7t.tfm"];
    assert_eq!(
        metrics
            .iter()
            .map(|request| request.key.as_str())
            .collect::<Vec<_>>(),
        [
            "tfm:ptmr7t.tfm",
            "tfm:ptmb7t.tfm",
            "tfm:ptmri7t.tfm",
            "tfm:ptmbi7t.tfm",
        ]
    );
    assert!(metrics.iter().zip(expected_names).all(|(request, name)| {
        request.class == PrefetchClass::Font
            && request.depth() == 1
            && request.file_key
                == Some(PrefetchFileKey::new("tex", "tfm", name).expect("typed TFM key"))
    }));
    assert!(policy.drain(8).is_empty());
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
        file_key: None,
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
        file_key: None,
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
fn admitted_typed_runtime_closure_normalizes_package_and_input_children() {
    let root_key = PrefetchFileKey::new("tex", "tex", "root.sty").expect("root key");
    let root =
        PrefetchRequest::for_file_key(root_key, "tex:root.sty", "root.sty", "literal", false);
    let mut policy = PrefetchPolicy::new(PrefetchBudget::default());
    assert!(policy.enqueue(root.clone()));
    assert_eq!(policy.drain(1).as_slice(), std::slice::from_ref(&root));
    policy.admitted_request_with_class(
        &root,
        PrefetchClass::SmallRuntime,
        br#"\RequirePackage{child}\input{chapters/intro}"#,
        std::iter::empty(),
    );
    let children = policy.drain(64);
    assert_eq!(
        children
            .iter()
            .map(|request| request.key.as_str())
            .collect::<Vec<_>>(),
        ["tex:child.sty", "tex:chapters/intro.tex"]
    );
    assert!(children.iter().all(|request| request.file_key.is_some()));
}

#[test]
fn admitted_literal_children_precede_metadata_peers() {
    let mut policy = PrefetchPolicy::new(PrefetchBudget::default());
    let metadata = PrefetchRequest::new("tex:metadata.sty", "metadata.sty", "metadata", false);
    assert!(policy.enqueue_with_priority(metadata, 0));
    policy.admitted("tex:root.sty", br#"\input{literal-child.tex}"#);
    let requests = policy.drain_prefetch_wave(64);
    assert_eq!(
        requests
            .iter()
            .map(|request| request.key.as_str())
            .collect::<Vec<_>>(),
        ["tex:literal-child.tex"]
    );
    assert_eq!(
        policy
            .drain_prefetch_wave(64)
            .iter()
            .map(|request| request.key.as_str())
            .collect::<Vec<_>>(),
        ["tex:metadata.sty"]
    );
}

#[test]
fn metadata_origin_is_a_leaf_for_catalogue_peer_traversal() {
    let parent_key = PrefetchFileKey::new("tex", "tex", "metadata-parent.sty").expect("parent key");
    let parent = PrefetchRequest::for_file_key(
        parent_key,
        "tex:metadata-parent.sty",
        "metadata-parent.sty",
        "metadata",
        false,
    )
    .with_origin(PrefetchOrigin::Metadata)
    .with_depth(1);
    let peer = PrefetchRequest::new(
        "tex:metadata-peer.sty",
        "metadata-peer.sty",
        "metadata",
        false,
    );
    let mut policy = PrefetchPolicy::new(PrefetchBudget::default());
    assert!(policy.enqueue(parent.clone()));
    assert_eq!(policy.drain(1).as_slice(), std::slice::from_ref(&parent));
    policy.admitted_request_with_class(&parent, PrefetchClass::SmallRuntime, b"metadata", [peer]);
    assert!(policy.drain(64).is_empty());
}

#[test]
fn duplicate_request_keeps_strongest_discovery_origin() {
    let key = PrefetchFileKey::new("tex", "tex", "shared.sty").expect("file key");
    let metadata = PrefetchRequest::for_file_key(
        key.clone(),
        "tex:shared.sty",
        "shared.sty",
        "metadata",
        false,
    )
    .with_origin(PrefetchOrigin::Metadata)
    .with_depth(3);
    let literal =
        PrefetchRequest::for_file_key(key, "tex:shared.sty", "shared.sty", "literal", false)
            .with_origin(PrefetchOrigin::Literal)
            .with_depth(0);
    let mut policy = PrefetchPolicy::new(PrefetchBudget::default());
    assert!(policy.enqueue_with_priority(metadata, 0));
    assert!(policy.enqueue_with_priority(literal, 2));
    let drained = policy.drain(1);
    let [request] = drained.as_slice() else {
        panic!("expected one deduplicated request");
    };
    assert_eq!(request.origin(), PrefetchOrigin::Literal);
    assert_eq!(request.depth(), 0);
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
    assert_eq!(
        closure
            .iter()
            .map(PrefetchRequest::depth)
            .collect::<Vec<_>>(),
        [1, 2]
    );
}

#[test]
fn queue_insertion_does_not_consume_optional_file_capacity() {
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
    assert!(policy.enqueue(PrefetchRequest::new(
        "tex:hint.sty",
        "hint.sty",
        "literal",
        false,
    )));
    assert_eq!(policy.drain(64).len(), 1);
    assert_eq!(policy.drain(64).len(), 1);
}

#[test]
fn deferred_optional_requests_wait_for_the_next_phase() {
    let mut policy = PrefetchPolicy::new(PrefetchBudget {
        max_files: 0,
        ..PrefetchBudget::default()
    });
    let request = PrefetchRequest::new("tex:deferred.sty", "deferred.sty", "literal", false);
    assert!(policy.enqueue(request.clone()));
    let drained_batch = policy.drain(1);
    let [drained] = drained_batch.as_slice() else {
        panic!("expected one drained request");
    };
    assert!(policy.defer(drained.clone()));
    assert!(
        policy.drain(1).is_empty(),
        "deferred work is not immediately drainable"
    );
    policy.begin_phase();
    assert_eq!(policy.drain(1), [request]);
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

#[test]
fn semantic_kinds_do_not_alias_when_transport_key_is_shared() {
    let key = |kind| PrefetchFileKey::new("tex", kind, "same-name").expect("semantic key");
    let candidate = |kind| PrefetchCandidate {
        key: "tex:same-name".to_owned(),
        object: ObjectEntry {
            object: "shared-object".to_owned(),
            ahash64: "0123456789abcdef".to_owned(),
            bytes: 12,
        },
        class: PrefetchClass::Other,
        required: false,
        file_key: Some(key(kind)),
    };
    let mut policy = PrefetchPolicy::new(PrefetchBudget {
        max_files: 4,
        max_bytes: 12,
        max_runtime_bytes: 12,
        ..PrefetchBudget::default()
    });
    let selection = policy.select_prefetch_group([], [candidate("vf"), candidate("font-program")]);
    assert_eq!(selection.hints.len(), 2);
    assert_eq!(selection.prefetch_bytes, 12);
}

#[test]
fn selection_budget_spans_waves_and_renews_for_a_new_phase() {
    let candidate = |name: &str, object: &str, bytes: u64| PrefetchCandidate {
        key: format!("tex:{name}"),
        object: ObjectEntry {
            object: object.to_owned(),
            ahash64: format!("{bytes:016x}"),
            bytes,
        },
        class: PrefetchClass::SmallRuntime,
        required: false,
        file_key: Some(PrefetchFileKey::new("tex", "tex", name).expect("key")),
    };
    let mut policy = PrefetchPolicy::new(PrefetchBudget {
        max_files: 8,
        max_bytes: 100,
        max_runtime_bytes: 60,
        ..PrefetchBudget::default()
    });
    assert_eq!(
        policy
            .select_prefetch_group([], [candidate("one.sty", "one", 60)])
            .prefetch_bytes,
        60
    );
    let second = policy.select_prefetch_group(
        [],
        [
            candidate("alias.sty", "one", 60),
            candidate("two.sty", "two", 1),
        ],
    );
    assert_eq!(second.hints.len(), 1);
    assert_eq!(second.hints[0].key, "tex:alias.sty");
    assert_eq!(second.prefetch_bytes, 0);
    assert!(
        policy
            .select_prefetch_group([], [candidate("three.sty", "three", 1)])
            .hints
            .is_empty()
    );
    policy.begin_phase();
    let renewed = policy.select_prefetch_group([], [candidate("three.sty", "three", 1)]);
    assert_eq!(renewed.hints.len(), 1);
    assert_eq!(renewed.prefetch_bytes, 1);
}

#[test]
fn drained_optional_keys_are_not_rediscovered_but_demand_can_requeue() {
    let optional = PrefetchRequest::new("tex:declined.sty", "declined.sty", "literal", false);
    let demand = PrefetchRequest::new("tex:declined.sty", "declined.sty", "required", true);
    let mut policy = PrefetchPolicy::new(PrefetchBudget::default());
    assert!(policy.enqueue(optional.clone()));
    assert_eq!(policy.drain(1), vec![optional]);
    assert!(!policy.enqueue(PrefetchRequest::new(
        "tex:declined.sty",
        "again.sty",
        "runtime",
        false,
    )));
    assert!(policy.enqueue(demand.clone()));
    assert_eq!(policy.drain(1), vec![demand]);
}
