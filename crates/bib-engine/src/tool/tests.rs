use bib_model::{
    BibSourceLocation, EntryBuilder, EntryId, EntryType, FieldProvenance, FieldValue,
    FieldValueStage, Literal, OutputFormat, OutputRequest, SourceSpan, VirtualPath,
};

use super::{SyntheticTool, TOOL_LIST, TOOL_SECTION, ToolFailureKind};

fn entry(id: &str) -> bib_model::Entry {
    let source = BibSourceLocation::new(
        VirtualPath::user("tool.bib").expect("path"),
        SourceSpan {
            byte_start: 0,
            byte_end: 1,
            line: 1,
            column: 1,
        },
    )
    .expect("source");
    let mut entry = EntryBuilder::new(
        EntryId::new(id).expect("id"),
        EntryType::new("book").expect("type"),
        source.clone(),
    );
    entry
        .field(
            bib_model::FieldId::new("title").expect("field"),
            FieldValue::Literal(Literal::new(id)),
            FieldValueStage::Normalized,
            FieldProvenance::Datasource(source),
        )
        .expect("field");
    entry.freeze()
}

#[test]
fn synthetic_tool_preserves_explicit_order_and_routes_outputs() {
    let mut tool = SyntheticTool::new();
    tool.entries([entry("a"), entry("b")]).expect("unique");
    tool.order([
        EntryId::new("b").expect("id"),
        EntryId::new("a").expect("id"),
    ]);
    let result = tool
        .run([
            OutputRequest::new(
                VirtualPath::user("tool.bib").expect("path"),
                OutputFormat::Bibtex,
            ),
            OutputRequest::new(
                VirtualPath::user("tool.dot").expect("path"),
                OutputFormat::Dot,
            ),
        ])
        .expect("tool runs in process");
    let section = result
        .document()
        .section(bib_model::SectionId::new(TOOL_SECTION))
        .expect("synthetic section");
    let list = section.lists().next().expect("tool list");
    assert_eq!(list.id().as_str(), TOOL_LIST);
    assert_eq!(
        list.entries().map(EntryId::as_str).collect::<Vec<_>>(),
        ["b", "a"]
    );
    assert_eq!(result.files().len(), 2);
}

#[test]
fn synthetic_tool_rejects_incomplete_order_and_duplicate_paths() {
    let mut tool = SyntheticTool::new();
    tool.entries([entry("a"), entry("b")]).expect("unique");
    tool.order([EntryId::new("a").expect("id")]);
    assert_eq!(
        tool.run([]).expect_err("bad order").kind(),
        ToolFailureKind::InvalidOrder
    );

    let mut tool = SyntheticTool::new();
    tool.entry(entry("a")).expect("unique");
    let request = OutputRequest::new(
        VirtualPath::user("same.bib").expect("path"),
        OutputFormat::Bibtex,
    );
    assert_eq!(
        tool.run([request.clone(), request])
            .expect_err("duplicate paths")
            .kind(),
        ToolFailureKind::DuplicateOutputPath
    );
}
