use super::*;
use crate::pdf::{
    PdfIndirectObject, PdfModelError, PdfObject, PdfObjectId, PdfValue, PdfVersion,
    UnvalidatedPdfDocument,
};
use std::collections::BTreeMap;

fn id(raw: u32) -> PdfObjectId {
    PdfObjectId::new(raw).expect("nonzero test id")
}

fn dictionary(entries: impl IntoIterator<Item = (&'static str, PdfValue)>) -> PdfDictionary {
    let mut dictionary = PdfDictionary::new();
    for (key, value) in entries {
        dictionary.insert(key, value).expect("unique test key");
    }
    dictionary
}

fn indirect(raw: u32, value: PdfValue) -> PdfIndirectObject {
    PdfIndirectObject {
        id: id(raw),
        object: PdfObject::Value(value),
    }
}

fn sample_input(order: &[u32]) -> UnvalidatedPdfDocument {
    let objects = vec![
        indirect(
            1,
            PdfValue::Dictionary(dictionary([
                ("Type", PdfValue::Name("Catalog".into())),
                ("Pages", PdfValue::Reference(id(2))),
            ])),
        ),
        indirect(
            2,
            PdfValue::Dictionary(dictionary([
                ("Type", PdfValue::Name("Pages".into())),
                ("Count", PdfValue::Integer(1)),
                ("Kids", PdfValue::Array(vec![PdfValue::Reference(id(3))])),
            ])),
        ),
        indirect(
            3,
            PdfValue::Dictionary(dictionary([
                ("Type", PdfValue::Name("Page".into())),
                ("Parent", PdfValue::Reference(id(2))),
                (
                    "MediaBox",
                    PdfValue::Array(vec![
                        PdfValue::Integer(0),
                        PdfValue::Integer(0),
                        PdfValue::Number(PdfNumber::new(6_125, 1).expect("number")),
                        PdfValue::Integer(792),
                    ]),
                ),
                ("Resources", PdfValue::Reference(id(5))),
                ("Contents", PdfValue::Reference(id(4))),
            ])),
        ),
        PdfIndirectObject {
            id: id(4),
            object: PdfObject::Stream {
                dictionary: PdfDictionary::new(),
                data: b"q\n10 20 30 40 re\nS\nQ\n".to_vec(),
            },
        },
        indirect(5, PdfValue::Dictionary(PdfDictionary::new())),
    ];
    let mut objects = objects
        .into_iter()
        .map(|object| (object.id.get(), object))
        .collect::<BTreeMap<_, _>>();
    UnvalidatedPdfDocument {
        version: PdfVersion::new(1, 4).expect("supported version"),
        catalog: id(1),
        objects: order
            .iter()
            .map(|raw| objects.remove(raw).expect("test object"))
            .collect(),
    }
}

fn sample_document(order: &[u32]) -> PdfDocument {
    sample_input(order).validate().expect("valid sample")
}

#[test]
fn compact_serialization_is_deterministic_and_independently_parseable() {
    let first = sample_document(&[1, 2, 3, 4, 5]);
    let reordered = sample_document(&[5, 3, 1, 4, 2]);

    let first_bytes = first.to_pdf_bytes().expect("serialize first");
    let second_bytes = first.to_pdf_bytes().expect("serialize again");
    let reordered_bytes = reordered.to_pdf_bytes().expect("serialize reordered");
    assert_eq!(first_bytes, second_bytes);
    assert_eq!(first_bytes, reordered_bytes);
    assert!(first_bytes.starts_with(b"%PDF-1.4\n"));
    assert!(first_bytes.ends_with(b"%%EOF"));

    let parsed = lopdf::Document::load_mem(&first_bytes).expect("lopdf parses output");
    assert_eq!(parsed.version, "1.4");
    assert_eq!(parsed.get_pages().len(), 1);
    assert_eq!(
        parsed
            .trailer
            .get(b"Root")
            .expect("root")
            .as_reference()
            .expect("root reference"),
        (1, 0)
    );
    let content = parsed
        .get_object((4, 0))
        .expect("content object")
        .as_stream()
        .expect("content stream");
    assert_eq!(content.content, b"q\n10 20 30 40 re\nS\nQ\n");
}

#[test]
fn configured_version_and_pretty_policy_are_deterministic() {
    let mut input = sample_input(&[1, 2, 3, 4, 5]);
    input.version = PdfVersion::new(1, 7).expect("supported version");
    let document = input.validate().expect("valid sample");
    let options = PdfSerializationOptions {
        pretty: true,
        stream_compression: PdfStreamCompression::None,
    };
    let first = document
        .to_pdf_bytes_with_options(options)
        .expect("pretty serialize");
    let second = document
        .to_pdf_bytes_with_options(options)
        .expect("pretty serialize again");
    assert_eq!(first, second);
    assert!(first.starts_with(b"%PDF-1.7\n"));
    assert_ne!(first, document.to_pdf_bytes().expect("compact serialize"));
}

#[test]
fn deterministic_flate_streams_are_declared_and_decode_exactly() {
    let document = sample_document(&[1, 2, 3, 4, 5]);
    let options = PdfSerializationOptions {
        pretty: false,
        stream_compression: PdfStreamCompression::Flate { level: 9 },
    };
    let first = document
        .to_pdf_bytes_with_options(options)
        .expect("compressed PDF");
    let second = document
        .to_pdf_bytes_with_options(options)
        .expect("repeat compressed PDF");
    assert_eq!(first, second);

    let parsed = lopdf::Document::load_mem(&first).expect("lopdf parses compressed output");
    let content = parsed
        .get_object((4, 0))
        .expect("content object")
        .as_stream()
        .expect("content stream");
    assert_eq!(
        content
            .dict
            .get(b"Filter")
            .expect("filter")
            .as_name()
            .expect("filter name"),
        b"FlateDecode"
    );
    assert_eq!(
        content.decompressed_content().expect("flate decodes"),
        b"q\n10 20 30 40 re\nS\nQ\n"
    );
}

#[test]
fn adapter_range_and_compression_errors_are_typed() {
    let sample = sample_document(&[1, 2, 3, 4, 5]);

    let mut objects = sample.objects().cloned().collect::<Vec<_>>();
    objects.push(indirect(u32::MAX, PdfValue::Null));
    let high_id = UnvalidatedPdfDocument {
        version: sample.version(),
        catalog: sample.catalog(),
        objects,
    }
    .validate()
    .expect("high unreferenced id is structurally valid");
    assert_eq!(
        high_id.to_pdf_bytes(),
        Err(PdfSerializeError::ObjectIdOutOfRange(id(u32::MAX)))
    );

    let mut objects = sample.objects().cloned().collect::<Vec<_>>();
    objects.push(indirect(6, PdfValue::Integer(i64::MAX)));
    let high_integer = UnvalidatedPdfDocument {
        version: sample.version(),
        catalog: sample.catalog(),
        objects,
    }
    .validate()
    .expect("high integer is structurally valid");
    assert_eq!(
        high_integer.to_pdf_bytes(),
        Err(PdfSerializeError::IntegerOutOfRange(i64::MAX))
    );

    assert_eq!(
        sample.to_pdf_bytes_with_options(PdfSerializationOptions {
            pretty: false,
            stream_compression: PdfStreamCompression::Flate { level: 10 },
        }),
        Err(PdfSerializeError::InvalidCompressionLevel(10))
    );
}

#[test]
fn automatic_compression_rejects_existing_filter_policy() {
    let mut input = sample_input(&[1, 2, 3, 4, 5]);
    let PdfObject::Stream { dictionary, .. } = &mut input.objects[3].object else {
        panic!("content stream")
    };
    dictionary
        .insert("Filter", PdfValue::Name("ASCIIHexDecode".into()))
        .expect("new filter");
    let document = input
        .validate()
        .expect("filtered stream is structurally valid");
    assert_eq!(
        document.to_pdf_bytes_with_options(PdfSerializationOptions {
            pretty: false,
            stream_compression: PdfStreamCompression::Flate { level: 6 },
        }),
        Err(PdfSerializeError::CompressionFilterConflict(id(4)))
    );
}

#[test]
fn model_version_validation_remains_typed_before_serialization() {
    assert_eq!(
        PdfVersion::new(2, 0),
        Err(PdfModelError::UnsupportedVersion { major: 2, minor: 0 })
    );
}

#[test]
fn pdf_writer_object_streams_parse_deterministically_at_levels_one_through_three() {
    fn serialize(level: u8) -> Vec<u8> {
        let mut pdf = Pdf::with_settings(Settings { pretty: false });
        pdf.set_version(1, 5);
        pdf.catalog(Ref::new(1)).pages(Ref::new(2));
        pdf.stream(Ref::new(4), b"ordinary stream");

        let mut objects = pdf.object_stream(Ref::new(6));
        objects
            .object(Ref::new(2))
            .dict()
            .pair(Name(b"Type"), Name(b"Pages"))
            .pair(Name(b"Count"), 0)
            .insert(Name(b"Kids"))
            .array();
        objects
            .object(Ref::new(3))
            .dict()
            .pair(Name(b"Marker"), Str(b"compressed object"));
        objects.finish_with_filter(Filter::FlateDecode, |data| {
            deflate(data, level).expect("in-memory compression")
        });

        pdf.finish_with_xref_stream(Ref::new(7))
    }

    for level in 1..=3 {
        let first = serialize(level);
        assert_eq!(first, serialize(level));

        let document = lopdf::Document::load_mem(&first).expect("lopdf parses object stream");
        let pages = document
            .get_object((2, 0))
            .expect("type-2 xref resolves pages")
            .as_dict()
            .expect("pages dictionary");
        assert_eq!(
            pages
                .get(b"Type")
                .expect("pages type")
                .as_name()
                .expect("name"),
            b"Pages"
        );
        let marker = document
            .get_object((3, 0))
            .expect("second compressed object resolves")
            .as_dict()
            .expect("marker dictionary");
        assert_eq!(
            marker
                .get(b"Marker")
                .expect("marker")
                .as_str()
                .expect("byte string"),
            b"compressed object"
        );
        let ordinary = document
            .get_object((4, 0))
            .expect("ordinary stream resolves")
            .as_stream()
            .expect("ordinary stream object");
        assert_eq!(ordinary.content, b"ordinary stream");
        assert!(ordinary.dict.get(b"Filter").is_err());
    }
}
