use super::*;

#[test]
fn ordered_graphics_content_uses_typed_state_and_preserves_literal_bytes() {
    let bytes = ordered_page_content(&[
        PdfContentOperation::Save { x: 10.0, y: 20.0 },
        PdfContentOperation::SetMatrix {
            x: 10.0,
            y: 20.0,
            matrix: [1.0, 0.25, -0.5, 1.0],
        },
        PdfContentOperation::Literal {
            mode: crate::PdfLiteralMode::Direct,
            x: 99.0,
            y: 99.0,
            bytes: b"0.1 g 1 2 m".to_vec(),
        },
        PdfContentOperation::Restore { x: 10.0, y: 20.0 },
    ]);
    let text = String::from_utf8(bytes).expect("ASCII content");
    assert_eq!(
        text,
        "1 0 0 1 10 20 cm\nq\n1 0.25 -0.5 1 0 0 cm\n0.1 g 1 2 m\nQ"
    );
}

#[test]
fn origin_literal_moves_but_page_and_direct_literals_do_not() {
    let bytes = ordered_page_content(&[
        PdfContentOperation::Literal {
            mode: crate::PdfLiteralMode::Page,
            x: 10.0,
            y: 20.0,
            bytes: b"PAGE".to_vec(),
        },
        PdfContentOperation::Literal {
            mode: crate::PdfLiteralMode::Origin,
            x: 10.0,
            y: 20.0,
            bytes: b"ORIGIN".to_vec(),
        },
        PdfContentOperation::Literal {
            mode: crate::PdfLiteralMode::Direct,
            x: 30.0,
            y: 40.0,
            bytes: b"DIRECT".to_vec(),
        },
    ]);
    assert_eq!(
        String::from_utf8(bytes).expect("ASCII content"),
        "PAGE\n1 0 0 1 10 20 cm\nORIGIN\nDIRECT"
    );
}

#[test]
fn direct_literal_preserves_text_state_but_page_literal_closes_it() {
    let text = |bytes: &[u8]| {
        PdfContentOperation::Text(PdfContentTextRun {
            x: 0.0,
            baseline: 0.0,
            font_name: b"F1".to_vec(),
            font_size: 10.0,
            bytes: bytes.to_vec(),
        })
    };
    let bytes = ordered_page_content(&[
        text(b"A"),
        PdfContentOperation::Literal {
            mode: crate::PdfLiteralMode::Direct,
            x: 0.0,
            y: 0.0,
            bytes: b"DIRECT".to_vec(),
        },
        text(b"B"),
        PdfContentOperation::Literal {
            mode: crate::PdfLiteralMode::Page,
            x: 0.0,
            y: 0.0,
            bytes: b"PAGE".to_vec(),
        },
    ]);
    let content = String::from_utf8(bytes).expect("ASCII content");
    assert_eq!(content.matches("BT").count(), 1);
    assert!(content.contains("(A) Tj\nDIRECT\n/F1"), "{content}");
    assert!(content.contains("(B) Tj\nET\nPAGE"), "{content}");
}

#[test]
fn color_stack_bytes_use_the_writer_owned_path_and_literal_modes() {
    let bytes = ordered_page_content(&[
        PdfContentOperation::ColorStack {
            mode: crate::PdfLiteralMode::Page,
            x: 99.0,
            y: 99.0,
            bytes: b"0 0 1 rg".to_vec(),
        },
        PdfContentOperation::ColorStack {
            mode: crate::PdfLiteralMode::Origin,
            x: 10.0,
            y: 20.0,
            bytes: b"1 0 0 rg".to_vec(),
        },
        PdfContentOperation::ColorStack {
            mode: crate::PdfLiteralMode::Direct,
            x: 30.0,
            y: 40.0,
            bytes: b"0 g".to_vec(),
        },
    ]);
    assert_eq!(
        String::from_utf8(bytes).expect("ASCII content"),
        "0 0 1 rg\n1 0 0 1 10 20 cm\n1 0 0 rg\n0 g"
    );
}

fn id(raw: u32) -> PdfObjectId {
    PdfObjectId::new(raw).expect("nonzero test object id")
}

fn dictionary(entries: impl IntoIterator<Item = (&'static str, PdfValue)>) -> PdfDictionary {
    let mut dictionary = PdfDictionary::new();
    for (key, value) in entries {
        dictionary.insert(key, value).expect("unique test key");
    }
    dictionary
}

fn indirect(id: u32, value: PdfValue) -> PdfIndirectObject {
    PdfIndirectObject {
        id: self::id(id),
        object: PdfObject::Value(value),
    }
}

fn sample_document(order: &[u32]) -> PdfDocument {
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
                        PdfValue::Integer(612),
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
                data: b"q\nQ\n".to_vec(),
            },
        },
        indirect(
            5,
            PdfValue::Dictionary(dictionary([(
                "ProcSet",
                PdfValue::Array(vec![PdfValue::Name("PDF".into())]),
            )])),
        ),
    ];
    let mut by_id = objects
        .into_iter()
        .map(|object| (object.id.get(), object))
        .collect::<BTreeMap<_, _>>();
    UnvalidatedPdfDocument {
        version: PdfVersion::new(1, 4).expect("supported version"),
        catalog: id(1),
        objects: order
            .iter()
            .map(|id| by_id.remove(id).expect("test object exists"))
            .collect(),
        trailer: Default::default(),
    }
    .validate()
    .expect("valid sample PDF graph")
}

#[test]
fn validation_canonicalizes_object_and_dictionary_order_for_hashing() {
    let ascending = sample_document(&[1, 2, 3, 4, 5]);
    let shuffled = sample_document(&[5, 3, 1, 4, 2]);

    assert_eq!(ascending, shuffled);
    assert_eq!(ascending.semantic_hash(), shuffled.semantic_hash());
    assert_eq!(
        shuffled
            .objects()
            .map(|object| object.id.get())
            .collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 5]
    );
}

#[test]
fn normalized_numbers_have_one_semantic_identity() {
    assert_eq!(
        PdfNumber::new(12_300, 3).expect("number"),
        PdfNumber::new(123, 1).expect("number")
    );
    assert_eq!(
        PdfNumber::new(1, 10),
        Err(PdfModelError::NumberPrecisionTooLarge(10))
    );
}

#[test]
fn duplicate_and_dangling_object_identities_are_rejected() {
    let sample = sample_document(&[1, 2, 3, 4, 5]);
    let mut objects = sample.objects().cloned().collect::<Vec<_>>();
    objects.push(objects[0].clone());
    assert_eq!(
        UnvalidatedPdfDocument {
            version: sample.version(),
            catalog: sample.catalog(),
            objects,
            trailer: Default::default(),
        }
        .validate(),
        Err(PdfModelError::DuplicateObject(id(1)))
    );

    let mut objects = sample.objects().cloned().collect::<Vec<_>>();
    let PdfObject::Value(PdfValue::Dictionary(catalog)) = &mut objects[0].object else {
        panic!("catalog dictionary")
    };
    catalog
        .insert("Dangling", PdfValue::Reference(id(99)))
        .expect("new key");
    assert_eq!(
        UnvalidatedPdfDocument {
            version: sample.version(),
            catalog: sample.catalog(),
            objects,
            trailer: Default::default(),
        }
        .validate(),
        Err(PdfModelError::MissingObject(id(99)))
    );
}

#[test]
fn info_reference_must_name_a_dictionary() {
    let sample = sample_document(&[1, 2, 3, 4, 5]);
    let mut objects = sample.objects().cloned().collect::<Vec<_>>();
    objects.push(indirect(6, PdfValue::Integer(7)));
    assert_eq!(
        UnvalidatedPdfDocument {
            version: sample.version(),
            catalog: sample.catalog(),
            objects,
            trailer: PdfTrailer {
                info: Some(id(6)),
                ..PdfTrailer::default()
            },
        }
        .validate(),
        Err(PdfModelError::InfoNotDictionary(id(6)))
    );
}

#[test]
fn page_resources_contents_and_parent_are_structurally_validated() {
    for (key, value, expected) in [
        (
            "Parent",
            PdfValue::Reference(id(5)),
            PdfModelError::PageParentInvalid(id(3)),
        ),
        (
            "Resources",
            PdfValue::Reference(id(4)),
            PdfModelError::PageResourcesInvalid(id(3)),
        ),
        (
            "Contents",
            PdfValue::Reference(id(5)),
            PdfModelError::PageContentsInvalid(id(3)),
        ),
    ] {
        let sample = sample_document(&[1, 2, 3, 4, 5]);
        let mut objects = sample.objects().cloned().collect::<Vec<_>>();
        let PdfObject::Value(PdfValue::Dictionary(page)) = &mut objects[2].object else {
            panic!("page dictionary")
        };
        page.entries.insert(key.into(), value);
        assert_eq!(
            UnvalidatedPdfDocument {
                version: sample.version(),
                catalog: sample.catalog(),
                objects,
                trailer: Default::default(),
            }
            .validate(),
            Err(expected)
        );
    }
}

#[test]
fn stream_bytes_and_page_order_affect_semantic_identity() {
    let first = sample_document(&[1, 2, 3, 4, 5]);
    let mut objects = first.objects().cloned().collect::<Vec<_>>();
    let PdfObject::Stream { data, .. } = &mut objects[3].object else {
        panic!("content stream")
    };
    data.push(b' ');
    let second = UnvalidatedPdfDocument {
        version: first.version(),
        catalog: first.catalog(),
        objects,
        trailer: Default::default(),
    }
    .validate()
    .expect("changed stream remains valid");
    assert_ne!(first.semantic_hash(), second.semantic_hash());
}

#[test]
fn limits_and_writer_owned_stream_length_are_enforced() {
    let sample = sample_document(&[1, 2, 3, 4, 5]);
    let input = UnvalidatedPdfDocument {
        version: sample.version(),
        catalog: sample.catalog(),
        objects: sample.objects().cloned().collect(),
        trailer: Default::default(),
    };
    assert_eq!(
        input.clone().validate_with_limits(PdfModelLimits {
            max_objects: 4,
            ..PdfModelLimits::default()
        }),
        Err(PdfModelError::TooManyObjects {
            actual: 5,
            limit: 4
        })
    );

    let mut objects = input.objects;
    let PdfObject::Stream { dictionary, .. } = &mut objects[3].object else {
        panic!("content stream")
    };
    dictionary
        .insert("Length", PdfValue::Integer(4))
        .expect("new key");
    assert_eq!(
        UnvalidatedPdfDocument {
            version: input.version,
            catalog: input.catalog,
            objects,
            trailer: input.trailer,
        }
        .validate(),
        Err(PdfModelError::ReservedStreamLength(id(4)))
    );
}
