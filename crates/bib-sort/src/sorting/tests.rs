use bib_model::{
    BibSourceLocation, DataListId, EntryBuilder, EntryId, EntryType, FieldId, FieldProvenance,
    FieldValue, FieldValueStage, Literal, ProcessedSectionBuilder, SectionId, SourceSpan,
    VirtualPath,
};

use super::*;

fn entry(id: &str, title: &str, year: i64) -> Entry {
    let id = EntryId::new(id).expect("test value is valid");
    let path = VirtualPath::user("test.bib").expect("test value is valid");
    let source = BibSourceLocation::new(
        path,
        SourceSpan {
            byte_start: 0,
            byte_end: 1,
            line: 1,
            column: 1,
        },
    )
    .expect("test value is valid");
    let mut builder = EntryBuilder::new(
        id,
        EntryType::new("book").expect("test value is valid"),
        source.clone(),
    );
    builder
        .field(
            FieldId::new("title").expect("test value is valid"),
            FieldValue::Literal(Literal::new(title)),
            FieldValueStage::Normalized,
            FieldProvenance::Datasource(source.clone()),
        )
        .expect("test value is valid")
        .field(
            FieldId::new("year").expect("test value is valid"),
            FieldValue::Integer(year),
            FieldValueStage::Normalized,
            FieldProvenance::Datasource(source),
        )
        .expect("test value is valid");
    builder.freeze()
}

fn section() -> bib_model::ProcessedSection {
    let mut section = ProcessedSectionBuilder::new(SectionId::new(0));
    for value in [
        entry("a", "Örn", 10),
        entry("b", "Anders", 2),
        entry("c", "Anders", 2),
    ] {
        section.entry(value).expect("test value is valid");
    }
    section.freeze()
}

#[test]
fn stable_numeric_sort_and_filter() {
    let section = section();
    let mut year = SortComponent::ascending(SortField::Field(
        FieldId::new("year").expect("test value is valid"),
    ));
    year.options.numeric = true;
    let title = SortComponent::ascending(SortField::Field(
        FieldId::new("title").expect("test value is valid"),
    ));
    let template = SortTemplate::new([year, title]).expect("test value is valid");
    let list = DataListBuilder::new(
        &section,
        DataListId::new("main").expect("test value is valid"),
        template,
    )
    .filter(DataListFilter::EntryType("book".into()))
    .build()
    .expect("test value is valid");
    assert_eq!(
        list.entries().map(EntryId::as_str).collect::<Vec<_>>(),
        ["b", "c", "a"]
    );
}

#[test]
fn pinned_swedish_tailoring_and_case_order() {
    let section = section();
    let mut title = SortComponent::ascending(SortField::Field(
        FieldId::new("title").expect("test value is valid"),
    ));
    title.options.locale = Locale::Swedish;
    title.options.case_order = CaseOrder::UpperFirst;
    let sorted = DataListBuilder::new(
        &section,
        DataListId::new("swedish").expect("test value is valid"),
        SortTemplate::new([title]).expect("test value is valid"),
    )
    .sorted_entries()
    .expect("test value is valid");
    assert_eq!(
        sorted
            .iter()
            .map(|entry| entry.id.as_str())
            .collect::<Vec<_>>(),
        ["b", "c", "a"]
    );
}

#[test]
fn missing_final_padding_substrings_and_limits_are_explicit() {
    let section = section();
    let mut component = SortComponent::ascending(SortField::EntryId);
    component.options.substring = Some((0, 1));
    component.options.pad_width = Some(5);
    component.options.pad_char = 'x';
    let error = DataListBuilder::new(
        &section,
        DataListId::new("bounded").expect("test value is valid"),
        SortTemplate::new([component]).expect("test value is valid"),
    )
    .limits(DataListLimits {
        maximum_entries: 2,
        ..DataListLimits::default()
    })
    .build()
    .expect_err("operation must fail");
    assert_eq!(error, SortError::TooManyEntries);
}

#[test]
fn list_items_initials_hashes_and_skip_modes_are_exact() {
    let values = [Literal::new("A"), Literal::new("B"), Literal::new("C")];
    let (visible, more) = limit_literal_list(&values, 2, 1);
    assert_eq!(visible, [Literal::new("A")]);
    assert!(more);
    assert_eq!(list_initial("örjan"), "Ö");
    assert_eq!(list_initial_hash("A"), "7fc56270e7a70fa81a5935b72eacbe29");
    assert!(!EntryDisposition::DataOnly.appears_in_list());
    assert!(!EntryDisposition::SkipLabels.computes_labels());
}
