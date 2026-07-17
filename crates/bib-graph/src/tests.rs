use bib_model::{
    BibConfigurationBuilder, BibSourceLocation, COMPATIBILITY_VERSION, EntryBuilder, EntryId,
    EntryType, FieldId, FieldProvenance, FieldValue, FieldValueStage, Literal, SectionId,
    SourceSpan, VirtualPath,
};
use bib_unicode::UnicodeData;

use super::*;

fn id(value: &str) -> EntryId {
    EntryId::new(value).expect("valid graph test fixture")
}
fn field(value: &str) -> FieldId {
    FieldId::new(value).expect("valid graph test fixture")
}
fn kind(value: &str) -> EntryType {
    EntryType::new(value).expect("valid graph test fixture")
}
fn source() -> BibSourceLocation {
    BibSourceLocation::new(
        VirtualPath::user("refs.bib").expect("valid graph test fixture"),
        SourceSpan {
            byte_start: 0,
            byte_end: 1,
            line: 1,
            column: 1,
        },
    )
    .expect("valid graph test fixture")
}
fn entry(key: &str, fields: &[(&str, FieldValue)]) -> bib_model::Entry {
    let source = source();
    let mut builder = EntryBuilder::new(id(key), kind("book"), source.clone());
    for (name, value) in fields {
        builder
            .field(
                field(name),
                value.clone(),
                FieldValueStage::RawDecoded,
                FieldProvenance::Datasource(source.clone()),
            )
            .expect("valid graph test fixture");
    }
    builder.freeze()
}
fn literal(value: &str) -> FieldValue {
    FieldValue::Literal(Literal::new(value))
}
fn keys(values: &[&str]) -> FieldValue {
    FieldValue::KeyList(values.iter().map(|value| id(value)).collect())
}
fn process(input: GraphInput, options: GraphOptions) -> GraphOutput {
    let configuration = BibConfigurationBuilder::new(COMPATIBILITY_VERSION).freeze();
    let unicode = UnicodeData::pinned();
    GraphProcessor::new(GraphContext::new(&configuration, &unicode), options)
        .process(input)
        .expect("valid graph test fixture")
}

#[test]
fn closure_resolves_aliases_sets_related_and_crossref_thresholds_in_source_order() {
    let entries = vec![
        entry("parent", &[("title", literal("P"))]),
        entry("one", &[("crossref", keys(&["parent"]))]),
        entry("two", &[("crossref", keys(&["p-alias"]))]),
        entry("set", &[("entryset", keys(&["one", "two"]))]),
        entry("related", &[("title", literal("R"))]),
        entry("root", &[("related", keys(&["related"]))]),
    ];
    let output = process(
        GraphInput {
            entries,
            aliases: vec![(id("p-alias"), id("parent"))],
            sections: vec![SectionSpec {
                id: SectionId::new(2),
                cited: vec![id("set"), id("root")],
                include_all: false,
                min_crossrefs: Some(2),
            }],
            ..GraphInput::default()
        },
        GraphOptions::default(),
    );
    let actual = output.sections[0]
        .entries
        .iter()
        .map(|entry| entry.id().as_str())
        .collect::<Vec<_>>();
    assert_eq!(actual, ["parent", "one", "two", "set", "related", "root"]);
    assert_eq!(
        output.sections[0].original_citekeys,
        [id("set"), id("root")]
    );
}

#[test]
fn xdata_and_crossref_inherit_in_declared_order_with_provenance_then_validate() {
    let output = process(
        GraphInput {
            entries: vec![
                entry(
                    "x",
                    &[
                        ("publisher", literal("X Press")),
                        ("location", literal("X City")),
                    ],
                ),
                entry(
                    "p",
                    &[
                        ("publisher", literal("Parent Press")),
                        ("year", literal("2026")),
                    ],
                ),
                entry(
                    "c",
                    &[
                        ("xdata", keys(&["x"])),
                        ("crossref", keys(&["p"])),
                        ("title", literal("Child")),
                    ],
                ),
            ],
            sections: vec![SectionSpec {
                id: SectionId::new(0),
                cited: vec![id("c")],
                include_all: false,
                min_crossrefs: Some(99),
            }],
            data_model: DataModel {
                rules: vec![ValidationRule {
                    entry_type: Some(kind("book")),
                    constraint: DataConstraint::Mandatory(field("year")),
                }],
            },
            ..GraphInput::default()
        },
        GraphOptions::default(),
    );
    let child = output.sections[0]
        .entries
        .iter()
        .find(|entry| entry.id() == &id("c"))
        .expect("valid graph test fixture");
    assert_eq!(
        child.fields().get(&field("publisher")),
        Some(&literal("X Press"))
    );
    assert_eq!(child.fields().get(&field("year")), Some(&literal("2026")));
    let inherited = child
        .fields()
        .iter()
        .find(|value| value.id() == &field("year"))
        .expect("valid graph test fixture");
    assert!(
        matches!(inherited.provenance(), FieldProvenance::Inherited { parent, .. } if parent.entry() == &id("p"))
    );
    assert!(
        output.diagnostics.is_empty(),
        "validation must run after inheritance"
    );
}

#[test]
fn sourcemaps_transform_alias_and_clone_without_mutating_the_source() {
    let output = process(
        GraphInput {
            entries: vec![entry("a", &[("title", literal("Old"))])],
            sections: vec![SectionSpec {
                id: SectionId::new(0),
                cited: vec![id("alias"), id("clone")],
                include_all: false,
                min_crossrefs: None,
            }],
            maps: vec![SourceMap {
                steps: vec![SourceMapStep {
                    matches: vec![MapMatch::FieldEquals(field("title"), "Old".into())],
                    actions: vec![
                        MapAction::Set(field("title"), literal("New")),
                        MapAction::AddAlias("alias".into()),
                        MapAction::CloneAs("clone".into()),
                    ],
                    final_step: true,
                }],
            }],
            ..GraphInput::default()
        },
        GraphOptions::default(),
    );
    assert_eq!(output.sections[0].entries.len(), 2);
    assert!(
        output.sections[0]
            .entries
            .iter()
            .all(|entry| entry.fields().get(&field("title")) == Some(&literal("New")))
    );
}

#[test]
fn cycles_are_diagnosed_deterministically_and_processing_terminates() {
    let output = process(
        GraphInput {
            entries: vec![
                entry("a", &[("crossref", keys(&["b"]))]),
                entry("b", &[("crossref", keys(&["a"]))]),
            ],
            sections: vec![SectionSpec {
                id: SectionId::new(0),
                cited: vec![id("a"), id("b")],
                include_all: false,
                min_crossrefs: None,
            }],
            ..GraphInput::default()
        },
        GraphOptions::default(),
    );
    assert_eq!(output.sections[0].entries.len(), 2);
    assert!(
        output
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "CIRCULAR_INHERITANCE")
    );
}

#[test]
fn graph_work_limits_fail_closed() {
    let error = {
        let configuration = BibConfigurationBuilder::new(COMPATIBILITY_VERSION).freeze();
        let unicode = UnicodeData::pinned();
        GraphProcessor::new(
            GraphContext::new(&configuration, &unicode),
            GraphOptions {
                limits: GraphLimits {
                    max_entries: 1,
                    ..GraphLimits::default()
                },
                ..GraphOptions::default()
            },
        )
        .process(GraphInput {
            entries: vec![entry("a", &[]), entry("b", &[])],
            ..GraphInput::default()
        })
        .expect_err("graph processing must fail")
    };
    assert_eq!(error, GraphError::Limit("entry limit exceeded"));
}
