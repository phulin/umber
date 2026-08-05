use bib_model::{
    BibDiagnostic, BibSourceLocation, Entry, EntryBuilder, EntryId, EntryType, FieldId,
    FieldProvenance, FieldValue, FieldValueStage, Literal, SectionId, SourceSpan, VirtualPath,
};

use super::*;
use crate::biber::EntryEditor;

#[derive(Default)]
struct PassFixture {
    entries: Vec<Entry>,
    aliases: Vec<(EntryId, EntryId)>,
    sections: Vec<SectionSpec>,
    maps: Vec<SourceMap>,
    data_model: DataModel,
}

struct PassResult {
    sections: Vec<GraphSection>,
    diagnostics: Vec<BibDiagnostic>,
}

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
fn process(input: PassFixture, options: GraphOptions) -> PassResult {
    let (sections, diagnostics) = RelationshipPass::new(options)
        .process(
            input.entries.iter().map(EntryEditor::from_entry).collect(),
            input.aliases,
            input.sections,
            &input.maps,
            &input.data_model,
        )
        .expect("valid graph test fixture");
    PassResult {
        sections,
        diagnostics,
    }
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
        PassFixture {
            entries,
            aliases: vec![(id("p-alias"), id("parent"))],
            sections: vec![SectionSpec {
                id: SectionId::new(2),
                cited: vec![id("set"), id("root")],
                include_all: false,
                min_crossrefs: Some(2),
            }],
            ..PassFixture::default()
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
        PassFixture {
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
            ..PassFixture::default()
        },
        GraphOptions::default(),
    );
    let child = output.sections[0]
        .entries
        .iter()
        .find(|entry| entry.id() == &id("c"))
        .expect("valid graph test fixture");
    assert_eq!(child.field(&field("publisher")), Some(&literal("X Press")));
    assert_eq!(child.field(&field("year")), Some(&literal("2026")));
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
        PassFixture {
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
            ..PassFixture::default()
        },
        GraphOptions::default(),
    );
    assert_eq!(output.sections[0].entries.len(), 2);
    assert!(
        output.sections[0]
            .entries
            .iter()
            .all(|entry| entry.field(&field("title")) == Some(&literal("New")))
    );
}

#[test]
fn cycles_are_diagnosed_deterministically_and_processing_terminates() {
    let output = process(
        PassFixture {
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
            ..PassFixture::default()
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
        match RelationshipPass::new(GraphOptions {
            limits: GraphLimits {
                max_entries: 1,
                ..GraphLimits::default()
            },
            ..GraphOptions::default()
        })
        .process(
            [&entry("a", &[]), &entry("b", &[])]
                .into_iter()
                .map(EntryEditor::from_entry)
                .collect(),
            Vec::new(),
            Vec::new(),
            &[],
            &DataModel::default(),
        ) {
            Err(error) => error,
            Ok(_) => panic!("graph processing must fail"),
        }
    };
    assert_eq!(error, GraphError::Limit("entry limit exceeded"));
}
