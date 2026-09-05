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
