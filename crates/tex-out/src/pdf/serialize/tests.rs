use super::*;
use crate::pdf::{
    PdfIndirectObject, PdfModelError, PdfObject, PdfObjectId, PdfTrailer, PdfValue, PdfVersion,
    UnvalidatedPdfDocument,
};
use std::collections::BTreeMap;
use test_support::pdf_probe::{PdfProbe, ProbeDictionary, ProbeLimits, ProbeObjectId, ProbeValue};

fn probe(bytes: &[u8]) -> PdfProbe {
    PdfProbe::new(bytes, ProbeLimits::default()).expect("Hayro probe parses serialized PDF")
}

fn probe_id(raw: i32) -> ProbeObjectId {
    ProbeObjectId::new(raw, 0)
}

fn probe_dictionary<'a>(value: &'a ProbeValue, context: &str) -> &'a ProbeDictionary {
    value
        .as_dictionary()
        .unwrap_or_else(|| panic!("{context} is a dictionary"))
}

fn probe_stream<'a>(
    value: &'a ProbeValue,
    context: &str,
) -> &'a test_support::pdf_probe::ProbeStream {
    match value.resolved() {
        ProbeValue::Stream(stream) => stream,
        _ => panic!("{context} is a stream"),
    }
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn classic_xref_offsets_match(bytes: &[u8], object_ids: &[u32]) -> bool {
    let Some(startxref) = bytes
        .windows(b"startxref\n".len())
        .rposition(|window| window == b"startxref\n")
    else {
        return false;
    };
    let offset_start = startxref + b"startxref\n".len();
    let Some(offset_end) = bytes[offset_start..].iter().position(|byte| *byte == b'\n') else {
        return false;
    };
    let Ok(xref_offset) = std::str::from_utf8(&bytes[offset_start..offset_start + offset_end])
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .ok_or(())
    else {
        return false;
    };
    if bytes.get(xref_offset..xref_offset + b"xref\n".len()) != Some(b"xref\n") {
        return false;
    }
    let xref = &bytes[xref_offset..startxref];
    object_ids.iter().all(|object_id| {
        let header = format!("{object_id} 0 obj\n");
        let Some(offset) = find_bytes(bytes, header.as_bytes()) else {
            return false;
        };
        let entry = format!("{offset:010} 00000 n\r\n");
        find_bytes(xref, entry.as_bytes()).is_some()
    })
}

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
        trailer: Default::default(),
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

    assert!(classic_xref_offsets_match(&first_bytes, &[1, 2, 3, 4, 5]));
    let mut corrupted_xref = first_bytes.clone();
    let object_four = find_bytes(&first_bytes, b"4 0 obj\n").expect("object four offset");
    let entry = format!("{object_four:010} 00000 n\r\n");
    let entry_offset =
        find_bytes(&corrupted_xref, entry.as_bytes()).expect("object four xref entry");
    corrupted_xref[entry_offset] = if corrupted_xref[entry_offset] == b'9' {
        b'8'
    } else {
        b'9'
    };
    assert!(!classic_xref_offsets_match(
        &corrupted_xref,
        &[1, 2, 3, 4, 5]
    ));

    let parsed = probe(&first_bytes);
    assert_eq!(parsed.version(), (1, 4));
    assert_eq!(parsed.pages().expect("ordered pages").len(), 1);
    assert_eq!(parsed.root_id(), probe_id(1));
    let trailer = parsed
        .trailer()
        .expect("project trailer")
        .expect("classic trailer");
    assert_eq!(
        trailer.get(b"Root").and_then(ProbeValue::referenced_id),
        Some(probe_id(1))
    );
    let content = parsed.object(probe_id(4)).expect("content object");
    assert_eq!(
        probe_stream(&content, "content object").raw,
        b"q\n10 20 30 40 re\nS\nQ\n"
    );
}

#[test]
fn document_info_is_registered_in_the_pdf_writer_trailer() {
    let mut input = sample_input(&[1, 2, 3, 4, 5]);
    let info_id = id(6);
    input.trailer.info = Some(info_id);
    input.objects.push(indirect(
        6,
        PdfValue::Dictionary(dictionary([
            ("Creator", PdfValue::String(b"TeX".to_vec())),
            ("Trapped", PdfValue::Name("False".into())),
        ])),
    ));
    let document = input.validate().expect("document info dictionary is valid");
    assert_eq!(document.info(), Some(info_id));
    assert!(matches!(
        &document
            .objects()
            .find(|object| object.id == info_id)
            .expect("typed info object")
            .object,
        PdfObject::Value(PdfValue::Dictionary(_))
    ));
    let bytes = document.to_pdf_bytes().expect("serialize info dictionary");
    let parsed = probe(&bytes);
    let trailer = parsed
        .trailer()
        .expect("project trailer")
        .expect("classic trailer");
    assert_eq!(
        trailer.get(b"Info").and_then(ProbeValue::referenced_id),
        Some(probe_id(6))
    );
    let info = parsed.dictionary(probe_id(6)).expect("Info dictionary");
    assert_eq!(
        info.get(b"Creator").map(ProbeValue::resolved),
        Some(&ProbeValue::String(b"TeX".to_vec()))
    );
    assert_eq!(
        info.get(b"Trapped").map(ProbeValue::resolved),
        Some(&ProbeValue::Name(b"False".to_vec()))
    );
}

#[test]
fn raw_page_entries_are_hashed_validated_and_serialized_verbatim() {
    let mut input = sample_input(&[1, 2, 3, 4, 5]);
    let page = input
        .objects
        .iter_mut()
        .find(|object| object.id == id(3))
        .expect("page object");
    let PdfObject::Value(PdfValue::Dictionary(page)) = &mut page.object else {
        panic!("page dictionary");
    };
    let mut raw_page = dictionary([
        ("Type", PdfValue::Name("Page".into())),
        ("Parent", PdfValue::Reference(id(2))),
        ("Resources", PdfValue::Reference(id(5))),
        ("Contents", PdfValue::Reference(id(4))),
    ]);
    raw_page.set_raw_entries(b"/MediaBox [1 2 300 400] /Rotate 90".to_vec());
    *page = raw_page;

    let document = input.validate().expect("raw MediaBox satisfies page graph");
    let with_raw_hash = document.semantic_hash();
    let typed_page = document
        .objects()
        .find(|object| object.id == id(3))
        .expect("typed page object");
    let PdfObject::Value(PdfValue::Dictionary(typed_page)) = &typed_page.object else {
        panic!("typed page dictionary");
    };
    assert!(typed_page.raw_entries_contain(b"/Rotate 90"));
    let bytes = document.to_pdf_bytes().expect("serialize raw entries");
    assert!(
        bytes
            .windows(b"/MediaBox [1 2 300 400] /Rotate 90".len())
            .any(|window| window == b"/MediaBox [1 2 300 400] /Rotate 90")
    );
    let parsed = probe(&bytes);
    let pages = parsed.pages().expect("project pages");
    assert_eq!(pages[0].id, probe_id(3));
    assert_eq!(
        pages[0].dictionary.get(b"Rotate").map(ProbeValue::resolved),
        Some(&ProbeValue::Number(90.0))
    );

    let mut changed = sample_input(&[1, 2, 3, 4, 5]);
    let page = changed
        .objects
        .iter_mut()
        .find(|object| object.id == id(3))
        .expect("page object");
    let PdfObject::Value(PdfValue::Dictionary(page)) = &mut page.object else {
        panic!("page dictionary");
    };
    page.set_raw_entries(b"/Rotate 90".to_vec());
    assert_ne!(
        with_raw_hash,
        changed
            .validate()
            .expect("valid changed sample")
            .semantic_hash()
    );
}

#[test]
fn configured_version_and_pretty_policy_are_deterministic() {
    let mut input = sample_input(&[1, 2, 3, 4, 5]);
    input.version = PdfVersion::new(1, 7).expect("supported version");
    let document = input.validate().expect("valid sample");
    let options = PdfSerializationOptions {
        pretty: true,
        stream_compression: PdfStreamCompression::None,
        object_compression: PdfObjectCompression::None,
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
        object_compression: PdfObjectCompression::None,
    };
    let first = document
        .to_pdf_bytes_with_options(options)
        .expect("compressed PDF");
    let second = document
        .to_pdf_bytes_with_options(options)
        .expect("repeat compressed PDF");
    assert_eq!(first, second);

    let parsed = probe(&first);
    let content = parsed.object(probe_id(4)).expect("content object");
    let content = probe_stream(&content, "content object");
    assert_eq!(
        content.dictionary.get(b"Filter").map(ProbeValue::resolved),
        Some(&ProbeValue::Name(b"FlateDecode".to_vec()))
    );
    assert_ne!(content.raw, content.decoded);
    assert_eq!(content.decoded, b"q\n10 20 30 40 re\nS\nQ\n");
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
        trailer: Default::default(),
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
        trailer: Default::default(),
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
            object_compression: PdfObjectCompression::None,
        }),
        Err(PdfSerializeError::InvalidCompressionLevel(10))
    );
    assert_eq!(
        sample.to_pdf_bytes_with_options(PdfSerializationOptions {
            pretty: false,
            stream_compression: PdfStreamCompression::None,
            object_compression: PdfObjectCompression::ObjectStreams { level: 4 },
        }),
        Err(PdfSerializeError::InvalidObjectCompressionLevel(4))
    );
    assert_eq!(
        sample.to_pdf_bytes_with_options(PdfSerializationOptions {
            pretty: false,
            stream_compression: PdfStreamCompression::None,
            object_compression: PdfObjectCompression::ObjectStreams { level: 1 },
        }),
        Err(PdfSerializeError::ObjectStreamsRequirePdf15)
    );
}

#[test]
fn raw_objects_and_trailer_extensions_keep_pdf_writer_framing() {
    let sample = sample_document(&[1, 2, 3, 4, 5]);
    let mut objects = sample.objects().cloned().collect::<Vec<_>>();
    objects.push(PdfIndirectObject {
        id: id(6),
        object: PdfObject::Raw(b"<< /Extension true >>".to_vec()),
    });
    let document = UnvalidatedPdfDocument {
        version: sample.version(),
        catalog: sample.catalog(),
        objects,
        trailer: PdfTrailer {
            info: None,
            file_id: Some((vec![1; 16], vec![2; 16])),
            raw_entries: b"/Custom 7".to_vec(),
        },
    }
    .validate()
    .expect("raw extension document validates");
    assert_eq!(document.trailer().file_id, Some((vec![1; 16], vec![2; 16])));
    assert_eq!(document.trailer().raw_entries, b"/Custom 7");
    let bytes = document.to_pdf_bytes().expect("raw extension serializes");

    assert!(
        bytes
            .windows(b"6 0 obj\n<< /Extension true >>\nendobj".len())
            .any(|window| window == b"6 0 obj\n<< /Extension true >>\nendobj")
    );
    let custom = bytes
        .windows(b"/Custom 7".len())
        .position(|window| window == b"/Custom 7")
        .expect("custom trailer entry");
    let id_entry = bytes
        .windows(b"/ID[".len())
        .position(|window| window == b"/ID[")
        .expect("typed ID entry");
    assert!(custom < id_entry, "raw trailer entries precede the file ID");
    assert!(classic_xref_offsets_match(&bytes, &[1, 2, 3, 4, 5, 6]));
    let parsed = probe(&bytes);
    let trailer = parsed
        .trailer()
        .expect("project trailer")
        .expect("classic trailer");
    assert_eq!(
        trailer.get(b"Custom").map(ProbeValue::resolved),
        Some(&ProbeValue::Number(7.0))
    );
    let file_id = trailer
        .get(b"ID")
        .and_then(ProbeValue::as_array)
        .expect("file ID array");
    assert_eq!(
        file_id,
        [
            ProbeValue::String(vec![1; 16]),
            ProbeValue::String(vec![2; 16])
        ]
    );
    let raw = parsed.object(probe_id(6)).expect("raw object");
    assert_eq!(
        probe_dictionary(&raw, "raw object")
            .get(b"Extension")
            .map(ProbeValue::resolved),
        Some(&ProbeValue::Boolean(true))
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
            object_compression: PdfObjectCompression::None,
        }),
        Err(PdfSerializeError::CompressionFilterConflict(id(4)))
    );
}

#[test]
fn encoded_streams_preserve_their_filter_and_bytes_under_automatic_compression() {
    let mut input = sample_input(&[1, 2, 3, 4, 5]);
    let encoded = b"already encoded image bytes".to_vec();
    input.objects.push(PdfIndirectObject {
        id: id(6),
        object: PdfObject::EncodedStream {
            dictionary: dictionary([("Filter", PdfValue::Name("DCTDecode".into()))]),
            data: encoded.clone(),
        },
    });
    let document = input.validate().expect("encoded stream document validates");
    assert!(matches!(
        &document
            .objects()
            .find(|object| object.id == id(6))
            .expect("typed encoded stream")
            .object,
        PdfObject::EncodedStream { dictionary, data }
            if dictionary.get(b"Filter") == Some(&PdfValue::Name("DCTDecode".into()))
                && data == &encoded
    ));
    let bytes = document
        .to_pdf_bytes_with_options(PdfSerializationOptions {
            pretty: false,
            stream_compression: PdfStreamCompression::Flate { level: 9 },
            object_compression: PdfObjectCompression::None,
        })
        .expect("encoded stream serializes");
    let parsed = probe(&bytes);
    assert_eq!(parsed.root_id(), probe_id(1));
    assert_eq!(parsed.pages().expect("ordered pages").len(), 1);
    assert!(find_bytes(&bytes, b"/Filter/DCTDecode").is_some());
    let stream_payload = [b"stream\n".as_slice(), encoded.as_slice(), b"\nendstream"].concat();
    assert!(find_bytes(&bytes, &stream_payload).is_some());
}

#[test]
fn model_version_validation_remains_typed_before_serialization() {
    assert!(PdfVersion::new(2, 0).is_ok());
    assert_eq!(
        PdfVersion::new(0, 0),
        Err(PdfModelError::UnsupportedVersion { major: 0, minor: 0 })
    );
}

#[test]
fn adapter_emits_real_object_streams_for_levels_one_through_three() {
    let mut input = sample_input(&[1, 2, 3, 4, 5]);
    input.version = PdfVersion::new(1, 5).expect("object stream PDF version");
    let document = input.validate().expect("valid object stream document");

    for level in 1..=3 {
        let options = PdfSerializationOptions {
            pretty: false,
            stream_compression: PdfStreamCompression::Flate { level: 6 },
            object_compression: PdfObjectCompression::ObjectStreams { level },
        };
        let first = document
            .to_pdf_bytes_with_options(options)
            .expect("object-stream PDF");
        assert_eq!(
            first,
            document
                .to_pdf_bytes_with_options(options)
                .expect("repeat object-stream PDF")
        );
        assert!(first.windows(12).any(|window| window == b"/Type/ObjStm"));
        assert!(first.windows(10).any(|window| window == b"/Type/XRef"));

        let parsed = probe(&first);
        assert_eq!(parsed.pages().expect("ordered pages").len(), 1);
        let pages = parsed
            .dictionary(probe_id(2))
            .expect("compressed pages object");
        assert_eq!(pages.id, Some(probe_id(2)));
        assert_eq!(
            pages.get(b"Type").map(ProbeValue::resolved),
            Some(&ProbeValue::Name(b"Pages".to_vec()))
        );
        let trailer = parsed
            .trailer()
            .expect("project xref stream")
            .expect("xref stream dictionary");
        assert_eq!(
            trailer.get(b"Type").map(ProbeValue::resolved),
            Some(&ProbeValue::Name(b"XRef".to_vec()))
        );
        let content = parsed.object(probe_id(4)).expect("ordinary content stream");
        let content = probe_stream(&content, "ordinary content stream");
        assert_eq!(content.decoded, b"q\n10 20 30 40 re\nS\nQ\n");
    }
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

        assert!(first.windows(12).any(|window| window == b"/Type/ObjStm"));
        assert!(first.windows(10).any(|window| window == b"/Type/XRef"));
        let document = probe(&first);
        let pages = document
            .dictionary(probe_id(2))
            .expect("type-2 xref resolves pages");
        assert_eq!(pages.id, Some(probe_id(2)));
        assert_eq!(
            pages.get(b"Type").map(ProbeValue::resolved),
            Some(&ProbeValue::Name(b"Pages".to_vec()))
        );
        let marker = document
            .dictionary(probe_id(3))
            .expect("second compressed object resolves");
        assert_eq!(marker.id, Some(probe_id(3)));
        assert_eq!(
            marker.get(b"Marker").map(ProbeValue::resolved),
            Some(&ProbeValue::String(b"compressed object".to_vec()))
        );
        let ordinary = document
            .object(probe_id(4))
            .expect("ordinary stream resolves");
        let ordinary = probe_stream(&ordinary, "ordinary stream object");
        assert_eq!(ordinary.raw, b"ordinary stream");
        assert!(ordinary.dictionary.get(b"Filter").is_none());
    }
}
