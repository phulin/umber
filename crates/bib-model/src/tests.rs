use super::*;

fn source() -> BibSourceLocation {
    let path = VirtualPath::user("references.bib").expect("test path is valid");
    BibSourceLocation::new(
        path,
        SourceSpan {
            byte_start: 4,
            byte_end: 10,
            line: 2,
            column: 1,
        },
    )
    .expect("test span is valid")
}

fn entry(id: &str) -> Entry {
    let id = EntryId::new(id).expect("test identifier is valid");
    let kind = EntryType::new("book").expect("test entry type is valid");
    let field = FieldId::new("title").expect("test field is valid");
    let provenance = FieldProvenance::Datasource(source());
    let mut builder = EntryBuilder::new(id, kind, source());
    builder
        .field(
            field,
            FieldValue::Literal(Literal::new("A title")),
            FieldValueStage::Normalized,
            provenance,
        )
        .expect("field is unique");
    builder.freeze()
}

#[test]
fn identifiers_reject_ambiguous_public_values() {
    assert!(EntryId::new(" key ").is_err());
    assert!(FieldId::new("bad field").is_err());
    assert!(FieldId::new("title").is_ok());
}

#[test]
fn scoped_options_resolve_explicit_precedence() {
    let id = OptionId::new("sorting").expect("test option is valid");
    let mut builder = ScopedOptionsBuilder::new();
    builder
        .push_layer(
            OptionScope::CompiledDefault,
            [(id.clone(), OptionValue::String("nyt".into()))],
        )
        .expect("ordered scope");
    builder
        .push_layer(
            OptionScope::Command,
            [(id.clone(), OptionValue::String("ynt".into()))],
        )
        .expect("ordered scope");
    let options = builder.freeze();
    assert_eq!(
        options.resolve(&id),
        Some(&OptionValue::String("ynt".into()))
    );
}

#[test]
fn frozen_document_preserves_entry_and_list_order() {
    let first = entry("first");
    let second = entry("second");
    let mut section = ProcessedSectionBuilder::new(SectionId::new(0));
    section.entry(first).expect("entry is unique");
    section.entry(second).expect("entry is unique");
    section
        .list(
            DataList::new(
                DataListId::new("main").expect("valid id"),
                [
                    EntryId::new("second").expect("valid id"),
                    EntryId::new("first").expect("valid id"),
                ],
            )
            .expect("list is unique"),
        )
        .expect("entries exist");
    let section = section.freeze();
    assert_eq!(
        section
            .entries()
            .map(|entry| entry.id().as_str())
            .collect::<Vec<_>>(),
        ["first", "second"]
    );
    assert_eq!(
        section
            .lists()
            .next()
            .expect("list exists")
            .entries()
            .map(EntryId::as_str)
            .collect::<Vec<_>>(),
        ["second", "first"]
    );
}

#[test]
fn builders_reject_duplicate_and_dangling_members() {
    let mut section = ProcessedSectionBuilder::new(SectionId::new(0));
    section.entry(entry("known")).expect("entry is unique");
    let list = DataList::new(
        DataListId::new("main").expect("valid id"),
        [EntryId::new("missing").expect("valid id")],
    )
    .expect("list itself is valid");
    assert!(matches!(
        section.list(list),
        Err(BuildError::UnknownListEntry(_))
    ));
}

#[test]
fn section_preserves_alias_and_undefined_key_order() {
    let mut section = ProcessedSectionBuilder::new(SectionId::new(0));
    section.entry(entry("target")).expect("entry is unique");
    section
        .alias(
            EntryId::new("old").expect("valid alias"),
            EntryId::new("target").expect("valid target"),
        )
        .expect("alias is unique and resolved");
    section
        .undefined_key(EntryId::new("missing").expect("valid key"))
        .expect("undefined key is unique");
    let section = section.freeze();
    assert_eq!(
        section
            .aliases()
            .map(|(alias, target)| (alias.as_str(), target.as_str()))
            .collect::<Vec<_>>(),
        [("old", "target")]
    );
    assert_eq!(
        section
            .undefined_keys()
            .map(EntryId::as_str)
            .collect::<Vec<_>>(),
        ["missing"]
    );
}
