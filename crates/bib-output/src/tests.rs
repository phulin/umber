use bib_model::{
    BibConfigurationBuilder, BibSourceLocation, COMPATIBILITY_VERSION, DataList, DataListId,
    DataListKind, DateValue, EntryBuilder, EntryId, EntryType, Field, FieldId, FieldProvenance,
    FieldValue, FieldValueStage, Literal, NameBuilder, NameList, NamePartValue, OutputFormat,
    OutputNewline, OutputRequest, ProcessedBibliographyBuilder, ProcessedSectionBuilder, Range,
    RangeEndpoint, SectionId, SourceSpan, Uri, Verbatim, VirtualPath,
};
use bib_unicode::{LegacyEncoding, UnicodeData};

use crate::{BblOutputFailureKind, BblSerializer, OutputContext, Serializer};

fn source() -> BibSourceLocation {
    BibSourceLocation::new(
        VirtualPath::user("full-bbl.bib").expect("valid test path"),
        SourceSpan {
            byte_start: 0,
            byte_end: 1,
            line: 1,
            column: 1,
        },
    )
    .expect("valid test source")
}

fn provenance() -> FieldProvenance {
    FieldProvenance::Datasource(source())
}

fn field(id: &str, value: FieldValue) -> Field {
    Field::new(
        FieldId::new(id).expect("valid field id"),
        value,
        FieldValueStage::Computed,
        provenance(),
    )
}

fn document_with_entry(entry: bib_model::Entry) -> bib_model::ProcessedBibliography {
    let entry_id = entry.id().clone();
    let mut section = ProcessedSectionBuilder::new(SectionId::new(0));
    section.entry(entry).expect("unique entry");
    section
        .list(
            DataList::new(
                DataListId::new("main/global//global/global/global").expect("valid list"),
                [entry_id],
            )
            .expect("valid data list"),
        )
        .expect("known entry");
    let mut document = ProcessedBibliographyBuilder::new(
        BibConfigurationBuilder::new(COMPATIBILITY_VERSION).freeze(),
    );
    document.section(section.freeze()).expect("unique section");
    document.freeze()
}

fn request() -> OutputRequest {
    OutputRequest::new(
        VirtualPath::user("main.bbl").expect("valid output path"),
        OutputFormat::Bbl,
    )
}

fn serialize(
    document: &bib_model::ProcessedBibliography,
    request: &OutputRequest,
) -> Result<bib_model::GeneratedFile, crate::BblOutputFailure> {
    BblSerializer.serialize(
        OutputContext::new(document, &UnicodeData::pinned()),
        request,
    )
}

#[test]
fn full_bbl_fixture_is_byte_exact() {
    let mut name = NameBuilder::new();
    name.family_part(NamePartValue::new(
        Literal::new("Doe"),
        ["D".to_owned()],
        false,
    ));
    name.given_part(NamePartValue::new(
        Literal::new("John"),
        ["J".to_owned()],
        false,
    ));
    let name = name.freeze().expect("complete name");

    let mut entry = EntryBuilder::new(
        EntryId::new("F1").expect("valid entry"),
        EntryType::new("book").expect("valid type"),
        source(),
    );
    let values = [
        field("author", FieldValue::NameList(NameList::new([name], false))),
        field(
            "namehash",
            FieldValue::Literal(Literal::new("bd051a2f7a5f377e3a62581b0e0f8577")),
        ),
        field(
            "fullhash",
            FieldValue::Literal(Literal::new("bd051a2f7a5f377e3a62581b0e0f8577")),
        ),
        field(
            "fullhashraw",
            FieldValue::Literal(Literal::new("bd051a2f7a5f377e3a62581b0e0f8577")),
        ),
        field(
            "bibnamehash",
            FieldValue::Literal(Literal::new("bd051a2f7a5f377e3a62581b0e0f8577")),
        ),
        field(
            "authorbibnamehash",
            FieldValue::Literal(Literal::new("bd051a2f7a5f377e3a62581b0e0f8577")),
        ),
        field(
            "authornamehash",
            FieldValue::Literal(Literal::new("bd051a2f7a5f377e3a62581b0e0f8577")),
        ),
        field(
            "authorfullhash",
            FieldValue::Literal(Literal::new("bd051a2f7a5f377e3a62581b0e0f8577")),
        ),
        field(
            "authorfullhashraw",
            FieldValue::Literal(Literal::new("bd051a2f7a5f377e3a62581b0e0f8577")),
        ),
        field("labelalpha", FieldValue::Literal(Literal::new("\\emph{A}"))),
        field("sortinit", FieldValue::Literal(Literal::new("A"))),
        field(
            "sortinithash",
            FieldValue::Literal(Literal::new("2f401846e2029bad6b3ecc16d50031e2")),
        ),
        field("singletitle", FieldValue::Boolean(true)),
        field(
            "labelnamesource",
            FieldValue::Literal(Literal::new("author")),
        ),
        field(
            "labeltitlesource",
            FieldValue::Literal(Literal::new("title")),
        ),
        field("shorthand", FieldValue::Literal(Literal::new("\\emph{A}"))),
        field(
            "title",
            FieldValue::Literal(Literal::new("The Fullness of Times")),
        ),
        field("year", FieldValue::Integer(1995)),
    ];
    for value in values {
        entry
            .field(
                value.id().clone(),
                value.value().clone(),
                value.stage(),
                value.provenance().clone(),
            )
            .expect("field is unique");
    }
    let entry = entry.freeze();
    let id = entry.id().clone();
    let mut section = ProcessedSectionBuilder::new(SectionId::new(0));
    section.entry(entry).expect("unique entry");
    section
        .list(
            DataList::new(
                DataListId::new("custom/global//global/global/global").expect("valid list"),
                [id.clone()],
            )
            .expect("valid list"),
        )
        .expect("known entry");
    section
        .list(
            DataList::new(
                DataListId::new("shorthand/global//global/global/global").expect("valid list"),
                [id.clone()],
            )
            .expect("valid list")
            .with_kind(DataListKind::List),
        )
        .expect("known entry");
    let nty = DataList::new(
        DataListId::new("nty/global//global/global/global").expect("valid list"),
        [id.clone()],
    )
    .expect("valid list")
    .with_context_fields(
        &id,
        [
            field("sortinit", FieldValue::Literal(Literal::new("D"))),
            field(
                "sortinithash",
                FieldValue::Literal(Literal::new("6f385f66841fb5e82009dc833c761848")),
            ),
        ],
    )
    .expect("known contextual entry");
    section.list(nty).expect("known entry");
    section
        .alias(EntryId::new("F1a").expect("valid alias"), id)
        .expect("resolved alias");
    section
        .undefined_key(EntryId::new("C1").expect("valid key"))
        .expect("unique undefined key");

    let mut document = ProcessedBibliographyBuilder::new(
        BibConfigurationBuilder::new(COMPATIBILITY_VERSION).freeze(),
    );
    document.section(section.freeze()).expect("unique section");
    let actual = serialize(&document.freeze(), &request()).expect("serializes");
    assert_eq!(
        actual.bytes(),
        include_bytes!("../../../tests/corpus/bib/upstream-2.22/tdata/full-bbl.bbl")
    );
}

#[test]
fn serializes_typed_values_annotations_and_declared_order() {
    let mut entry = EntryBuilder::new(
        EntryId::new("typed").expect("valid entry"),
        EntryType::new("misc").expect("valid type"),
        source(),
    );
    let values = [
        field("enabled", FieldValue::Boolean(false)),
        field("raw", FieldValue::Verbatim(Verbatim::new("a%b"))),
        field(
            "tags",
            FieldValue::LiteralList(vec![Literal::new("one"), Literal::new("two")]),
        ),
        field(
            "links",
            FieldValue::UriList(vec![Uri::new("https://example.test/a").expect("valid URI")]),
        ),
        field(
            "pages",
            FieldValue::RangeList(vec![Range::new(
                RangeEndpoint::Integer(1),
                RangeEndpoint::Integer(4),
            )]),
        ),
        field(
            "date",
            FieldValue::Date(
                DateValue::new(2026, Some(7), Some(17))
                    .expect("valid date")
                    .with_uncertain(true),
            ),
        ),
    ];
    for value in values {
        entry
            .field(
                value.id().clone(),
                value.value().clone(),
                value.stage(),
                value.provenance().clone(),
            )
            .expect("unique field");
    }
    entry
        .annotation(
            bib_model::Annotation::new(FieldId::new("note").expect("valid field"), "reviewed")
                .expect("valid annotation"),
        )
        .expect("unique annotation");
    let bytes = serialize(&document_with_entry(entry.freeze()), &request())
        .expect("serializes")
        .bytes()
        .to_vec();
    let text = String::from_utf8(bytes).expect("UTF-8 output");
    for expected in [
        "\\false{enabled}",
        "\\verb{raw}\n      \\verb a%b\n      \\endverb",
        "\\list{tags}{2}{%",
        "\\list{links}{1}{%",
        "\\range{pages}{1}{%",
        "\\field{date}{2026-07-17?}",
        "\\annotation{note}{reviewed}",
    ] {
        assert!(text.contains(expected), "missing `{expected}` in {text}");
    }
}

#[test]
fn applies_newline_encoding_and_output_limits_with_typed_diagnostics() {
    let mut entry = EntryBuilder::new(
        EntryId::new("encoding").expect("valid entry"),
        EntryType::new("misc").expect("valid type"),
        source(),
    );
    entry
        .field(
            FieldId::new("title").expect("valid field"),
            FieldValue::Literal(Literal::new("Café")),
            FieldValueStage::Normalized,
            provenance(),
        )
        .expect("unique field");
    let document = document_with_entry(entry.freeze());
    let encoded = serialize(
        &document,
        &request()
            .with_encoding(LegacyEncoding::Latin1)
            .with_newline(OutputNewline::CrLf),
    )
    .expect("Latin-1 serializes");
    assert!(encoded.bytes().windows(2).any(|bytes| bytes == b"\r\n"));
    assert!(encoded.bytes().contains(&0xe9));

    let error = serialize(&document, &request().with_max_bytes(16)).expect_err("limit fails");
    assert_eq!(error.kind(), BblOutputFailureKind::Limit);
    assert_eq!(
        error
            .diagnostics()
            .next()
            .expect("diagnostic")
            .code()
            .as_str(),
        "BIB_OUTPUT_LIMIT"
    );

    let mut unrepresentable = EntryBuilder::new(
        EntryId::new("emoji").expect("valid entry"),
        EntryType::new("misc").expect("valid type"),
        source(),
    );
    unrepresentable
        .field(
            FieldId::new("title").expect("valid field"),
            FieldValue::Literal(Literal::new("🦀")),
            FieldValueStage::Normalized,
            provenance(),
        )
        .expect("unique field");
    let error = serialize(
        &document_with_entry(unrepresentable.freeze()),
        &request().with_encoding(LegacyEncoding::Latin1),
    )
    .expect_err("unrepresentable value fails");
    assert_eq!(error.kind(), BblOutputFailureKind::Unrepresentable);
}
