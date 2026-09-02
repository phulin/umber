use super::*;
use tex_arith::Scaled;

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
fn begin_text_restores_page_origin_after_an_origin_literal() {
    // pdftex.web §690: `pdf_begin_text` calls `pdf_set_origin` for the
    // page/form origin. The inverse `cm` is observable normalized PDF
    // evidence; retaining the literal's CTM changes consumer float rounding.
    let bytes = ordered_page_content(&[
        PdfContentOperation::Literal {
            mode: crate::PdfLiteralMode::Origin,
            x: 10.0,
            y: 20.0,
            bytes: b"ORIGIN".to_vec(),
        },
        PdfContentOperation::ImageXObject {
            x: 10.0,
            y: 20.0,
            width: 2.0,
            height: 3.0,
            name: b"Im1".to_vec(),
        },
        PdfContentOperation::Text(PdfContentTextRun {
            x: 30.0,
            raster: None,
            baseline: 40.0,
            font_name: b"F1".to_vec(),
            font_size: 10.0,
            horizontal_scale: 1.0,
            bytes: b"A".to_vec(),
            advance: None,
        }),
        PdfContentOperation::Rectangle(PdfContentRectangle {
            x: 50.0,
            y: 60.0,
            width: 7.0,
            height: 8.0,
        }),
        PdfContentOperation::FormXObject {
            x: 70.0,
            y: 80.0,
            name: b"Fm1".to_vec(),
        },
    ]);
    assert_eq!(
        String::from_utf8(bytes).expect("ASCII content"),
        concat!(
            "1 0 0 1 10 20 cm\n",
            "ORIGIN\n",
            "q\n",
            "2 0 0 3 0 0 cm\n",
            "/Im1 Do\n",
            "Q\n",
            "1 0 0 1 -10 -20 cm\n",
            "BT\n",
            "/F1 10 Tf\n",
            "30 40 Td\n",
            "(A) Tj\n",
            "ET\n",
            "q\n",
            "50 60 7 8 re\n",
            "f\n",
            "Q\n",
            "q\n",
            "1 0 0 1 70 80 cm\n",
            "/Fm1 Do\n",
            "Q",
        )
    );
}

#[test]
fn direct_literal_preserves_text_state_but_page_literal_closes_it() {
    let text = |bytes: &[u8]| {
        PdfContentOperation::Text(PdfContentTextRun {
            x: 0.0,
            raster: None,
            baseline: 0.0,
            font_name: b"F1".to_vec(),
            font_size: 10.0,
            horizontal_scale: 1.0,
            bytes: bytes.to_vec(),
            advance: None,
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
    assert!(content.contains("(A) Tj\nDIRECT\n0 0 Td"), "{content}");
    assert_eq!(content.matches("/F1 10 Tf").count(), 1, "{content}");
    assert!(content.contains("(B) Tj\nET\nPAGE"), "{content}");
}

#[test]
fn mapped_text_keeps_pdftex_tj_position_across_direct_color_operations() {
    let text = |x, byte| {
        PdfContentOperation::Text(PdfContentTextRun {
            x,
            raster: None,
            baseline: 20.0,
            font_name: b"F1".to_vec(),
            font_size: 10.0,
            horizontal_scale: 1.0,
            bytes: vec![byte],
            advance: Some(5.0),
        })
    };
    let bytes = ordered_page_content(&[
        text(10.0, b'A'),
        PdfContentOperation::ColorStack {
            mode: crate::PdfLiteralMode::Direct,
            x: 0.0,
            y: 0.0,
            bytes: b"0 g".to_vec(),
        },
        text(17.0, b'B'),
    ]);
    assert_eq!(
        String::from_utf8(bytes).expect("ASCII content"),
        concat!(
            "BT\n",
            "/F1 10 Tf\n",
            "10 20 Td\n",
            "(A) Tj\n",
            "0 g\n",
            "[-200 (B)] TJ\n",
            "ET",
        )
    );
}

#[test]
fn mapped_text_uses_td_and_consolidates_adjacent_glyph_and_kern_runs() {
    let run = |x: f64, bytes: &[u8]| {
        PdfContentOperation::Text(PdfContentTextRun {
            x: x as f32,
            raster: Some(PdfContentTextRaster {
                serialized_x: x,
                position_x: x,
                font_size: 10.0,
                exact: None,
                glyphs: bytes
                    .iter()
                    .enumerate()
                    .map(|(index, _)| PdfContentGlyphRaster {
                        position_x: x + index as f64 * 5.0,
                        advance: 5.0,
                        position_raw: 0,
                        width_raw: 0,
                    })
                    .collect(),
            }),
            baseline: 20.0,
            font_name: b"F42".to_vec(),
            font_size: 17.215,
            horizontal_scale: 1.0,
            bytes: bytes.to_vec(),
            advance: Some(bytes.len() as f64 * 5.0),
        })
    };
    let bytes = ordered_page_content(&[run(10.0, b"Global"), run(43.02, b"exp")]);
    assert_eq!(
        String::from_utf8(bytes).expect("ASCII content"),
        concat!(
            "BT\n",
            "/F42 17.215 Tf\n",
            "10 20 Td\n",
            "[(Global) -302 (exp)] TJ\n",
            "ET",
        )
    );
}

#[test]
fn mapped_text_starts_its_raster_at_the_serialized_position() {
    // pdftex.web §690: `pdf_set_text_pos` assigns `pdf_h` from the rounded
    // position written to the PDF, while the next `pdf_begin_string` compares
    // the unrounded TeX anchor against that raster. Keeping the unrounded
    // initial anchor instead changes this boundary adjustment from 772 to 773.
    let font_size = 8.9664;
    let advance = 525.0 * font_size / 1000.0;
    let text = |x, serialized_x, position_x, byte| {
        PdfContentOperation::Text(PdfContentTextRun {
            x,
            raster: Some(PdfContentTextRaster {
                serialized_x,
                position_x,
                font_size,
                exact: None,
                glyphs: Vec::new(),
            }),
            baseline: 20.0,
            font_name: b"F1".to_vec(),
            font_size: font_size as f32,
            horizontal_scale: 1.0,
            bytes: vec![byte],
            advance: Some(advance),
        })
    };
    let bytes = ordered_page_content(&[
        text(67.649, 67.649, 67.6485, b'#'),
        text(79.282, 79.282, 79.282_455_68, b'D'),
    ]);
    assert!(
        String::from_utf8(bytes)
            .expect("ASCII content")
            .contains("[-772 (D)] TJ"),
        "the retained cursor must start at the rounded serialized position"
    );
}

#[test]
fn mapped_text_corrects_width_raster_inside_a_contiguous_string() {
    // pdftex.web §690: `output_one_char` calls `pdf_begin_string` before every
    // character. A rounded `/Widths` advance can therefore require a TJ
    // correction even when adjacent character nodes have no intervening kern.
    let bytes = ordered_page_content(&[PdfContentOperation::Text(PdfContentTextRun {
        x: 0.0,
        raster: Some(PdfContentTextRaster {
            serialized_x: 0.0,
            position_x: 0.0,
            font_size: 10.0,
            exact: Some(PdfContentTextExactRaster {
                serialized_h: 0,
                font_size: 10_000,
                expansion_ratio: 0,
            }),
            glyphs: vec![
                PdfContentGlyphRaster {
                    position_x: 0.0,
                    advance: 5.0,
                    position_raw: 0,
                    width_raw: 5_000,
                },
                PdfContentGlyphRaster {
                    position_x: 5.0,
                    advance: 5.01,
                    position_raw: 5_000,
                    width_raw: 5_005,
                },
                PdfContentGlyphRaster {
                    position_x: 10.0,
                    advance: 5.0,
                    position_raw: 10_000,
                    width_raw: 5_000,
                },
            ],
        }),
        baseline: 20.0,
        font_name: b"F1".to_vec(),
        font_size: 10.0,
        horizontal_scale: 1.0,
        bytes: b"red".to_vec(),
        advance: Some(15.01),
    })]);
    assert!(
        String::from_utf8(bytes)
            .expect("ASCII content")
            .contains("[(re) 1 (d)] TJ"),
        "the third character must return to its exact TeX anchor"
    );
}

#[test]
fn text_strings_escape_every_byte_without_changing_the_decoded_payload() {
    let payload = (u8::MIN..=u8::MAX).collect::<Vec<_>>();
    let bytes = ordered_page_content(&[PdfContentOperation::Text(PdfContentTextRun {
        x: 0.0,
        raster: None,
        baseline: 0.0,
        font_name: b"F1".to_vec(),
        font_size: 10.0,
        horizontal_scale: 1.0,
        bytes: payload.clone(),
        advance: None,
    })]);

    let mut encoded = Vec::with_capacity(514);
    encoded.push(b'<');
    for byte in &payload {
        encoded.extend_from_slice(format!("{byte:02X}").as_bytes());
    }
    encoded.extend_from_slice(b"> Tj");
    assert!(
        bytes.windows(encoded.len()).any(|window| window == encoded),
        "all binary text bytes use one exact PDF hex string"
    );
}

#[test]
fn auto_expanded_font_uses_its_pdftex_horizontal_text_matrix_scale() {
    // pdftex.web §690: auto-expanded fonts share the base PDF font resource,
    // and `pdf_set_text_pos` writes `(1000 + ratio) / 1000` as Tm's a value.
    let construction = crate::FontResourceConstruction::Expanded {
        source_font_id: 1,
        source_identity: tex_fonts::FontSourceIdentity::from_bytes([7; 8]),
        ratio: -20,
    };
    assert_eq!(super::finalize::font_horizontal_scale(&construction), 0.98);

    let bytes = ordered_page_content(&[PdfContentOperation::Text(PdfContentTextRun {
        x: 12.0,
        raster: None,
        baseline: 34.0,
        font_name: b"F1".to_vec(),
        font_size: 10.0,
        horizontal_scale: super::finalize::font_horizontal_scale(&construction),
        bytes: b"proprietary,".to_vec(),
        advance: None,
    })]);
    assert!(
        String::from_utf8(bytes)
            .expect("ASCII content")
            .contains("0.98 0 0 1 12 34 Tm"),
        "expanded text must carry the canonical horizontal scale"
    );
}

#[test]
fn pdftex_scalable_width_keeps_its_one_decimal_raster() {
    // pdftex.web §690: `adv_char_width` and `/Widths` share a 1/10000
    // font-size raster, serialized as one decimal place in text-space units.
    let font_size = Scaled::from_raw(10 * Scaled::UNITY);
    let one_third_em = Scaled::from_raw(218_453);
    assert_eq!(
        super::finalize::pdftex_scalable_width_tenths(one_third_em, font_size),
        Some(3333)
    );
}

#[test]
fn expanded_glyph_end_exposes_internal_font_kerns() {
    // pdftex.web §690: `adv_char_width` advances on the expanded glyph raster,
    // while a following font kern remains a distinct `pdf_begin_string`
    // movement. The PDF segment builder must therefore break at 6031, rather
    // than absorbing that kern into the adjustment before the next word.
    let construction = crate::FontResourceConstruction::Expanded {
        source_font_id: 1,
        source_identity: tex_fonts::FontSourceIdentity::from_bytes([7; 8]),
        ratio: 20,
    };
    let glyph_end = super::finalize::positioned_char_end(
        Scaled::from_raw(5000),
        Scaled::from_raw(1000),
        &construction,
    )
    .expect("expanded glyph end fits");
    assert_eq!(glyph_end, Scaled::from_raw(6020));
    assert_ne!(glyph_end, Scaled::from_raw(6031));
}

#[test]
fn pdftex_font_size_uses_four_places_for_tf_and_cursor_advances() {
    // pdftex.web §690: `pdf_set_font` emits the font size with four
    // decimal places, and `pdf_use_font` retains that raster for
    // `adv_char_width`, independently of `\pdfdecimaldigits`.
    let font_size = super::finalize::pdftex_font_size(Scaled::from_raw(9 * 65_536));
    assert_eq!(font_size, 8.9664);

    let bytes = ordered_page_content(&[PdfContentOperation::Text(PdfContentTextRun {
        x: 0.0,
        raster: None,
        baseline: 0.0,
        font_name: b"F1".to_vec(),
        font_size,
        horizontal_scale: 1.0,
        bytes: b"A".to_vec(),
        advance: Some(4.5),
    })]);
    assert!(
        String::from_utf8(bytes)
            .expect("ASCII content")
            .contains("/F1 8.9664 Tf"),
        "the serialized Tf operand must preserve pdfTeX's four-place raster"
    );
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

fn outlined_input() -> UnvalidatedPdfDocument {
    let sample = sample_document(&[1, 2, 3, 4, 5]);
    let mut objects = sample.objects().cloned().collect::<Vec<_>>();
    let PdfObject::Value(PdfValue::Dictionary(catalog)) = &mut objects[0].object else {
        panic!("catalog dictionary")
    };
    catalog
        .insert("Outlines", PdfValue::Reference(id(6)))
        .expect("outline root key");
    objects.extend([
        PdfIndirectObject {
            id: id(6),
            object: PdfObject::Outline(PdfOutlineObject {
                first: id(7),
                last: id(8),
                visible_count: 2,
            }),
        },
        PdfIndirectObject {
            id: id(7),
            object: PdfObject::OutlineItem(PdfOutlineItemObject {
                title: id(5),
                action: id(4),
                parent: id(6),
                previous: None,
                next: Some(id(8)),
                first: None,
                last: None,
                count: None,
                raw_entries: Vec::new(),
            }),
        },
        PdfIndirectObject {
            id: id(8),
            object: PdfObject::OutlineItem(PdfOutlineItemObject {
                title: id(5),
                action: id(4),
                parent: id(6),
                previous: Some(id(7)),
                next: None,
                first: None,
                last: None,
                count: None,
                raw_entries: Vec::new(),
            }),
        },
    ]);
    UnvalidatedPdfDocument {
        version: sample.version(),
        catalog: sample.catalog(),
        objects,
        trailer: Default::default(),
    }
}

#[test]
fn malformed_outline_parent_sibling_cycle_and_count_are_exact_retry_stable() {
    type MalformedOutlineCase = (PdfObjectId, fn(&mut PdfOutlineItemObject), PdfModelError);
    let cases: [MalformedOutlineCase; 4] = [
        (
            id(7),
            |item| item.parent = id(8),
            PdfModelError::OutlineParentInvalid(id(7)),
        ),
        (
            id(8),
            |item| item.previous = None,
            PdfModelError::OutlineSiblingInvalid(id(8)),
        ),
        (
            id(8),
            |item| item.next = Some(id(7)),
            PdfModelError::OutlineCycle(id(7)),
        ),
        (
            id(7),
            |item| item.count = Some(1),
            PdfModelError::OutlineCountInvalid(id(7)),
        ),
    ];
    for (target, mutate, expected) in cases {
        let mut input = outlined_input();
        let PdfObject::OutlineItem(item) = &mut input
            .objects
            .iter_mut()
            .find(|object| object.id == target)
            .expect("target outline item")
            .object
        else {
            panic!("typed outline item")
        };
        mutate(item);
        let first = input.clone().validate().expect_err("malformed outline");
        let second = input.validate().expect_err("retry rejects same outline");
        assert_eq!(first, expected);
        assert_eq!(second, first);
        assert_eq!(second.to_string(), first.to_string());
    }
}

#[test]
fn destination_missing_page_and_outline_missing_action_fail_before_serialization() {
    let mut destination = sample_document(&[1, 2, 3, 4, 5])
        .objects()
        .cloned()
        .collect::<Vec<_>>();
    destination.push(PdfIndirectObject {
        id: id(6),
        object: PdfObject::Destination(PdfExplicitDestination {
            page: id(99),
            view: PdfDestinationView::Fit,
        }),
    });
    assert_eq!(
        UnvalidatedPdfDocument {
            version: PdfVersion::new(1, 4).expect("version"),
            catalog: id(1),
            objects: destination,
            trailer: Default::default(),
        }
        .validate(),
        Err(PdfModelError::MissingObject(id(99)))
    );

    let mut outline = outlined_input();
    let PdfObject::OutlineItem(item) = &mut outline.objects[6].object else {
        panic!("first outline item")
    };
    item.action = id(99);
    assert_eq!(
        outline.validate(),
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
fn article_threads_use_typed_catalog_page_thread_and_bead_writers() {
    let sample = sample_document(&[1, 2, 3, 4, 5]);
    let mut objects = sample.objects().cloned().collect::<Vec<_>>();
    let PdfObject::Value(PdfValue::Dictionary(catalog)) = &mut objects[0].object else {
        panic!("catalog")
    };
    catalog
        .insert("Threads", PdfValue::Reference(id(6)))
        .expect("thread list key");
    let PdfObject::Value(PdfValue::Dictionary(page)) = &mut objects[2].object else {
        panic!("page")
    };
    page.insert("B", PdfValue::Array(vec![PdfValue::Reference(id(8))]))
        .expect("page beads key");
    objects.extend([
        PdfIndirectObject {
            id: id(6),
            object: PdfObject::ThreadList(vec![id(7)]),
        },
        PdfIndirectObject {
            id: id(7),
            object: PdfObject::Thread(PdfThreadObject {
                first_bead: id(8),
                default_title: Some(b"(chapter)".to_vec()),
                raw_entries: Vec::new(),
            }),
        },
        PdfIndirectObject {
            id: id(8),
            object: PdfObject::Bead(PdfBeadObject {
                thread: Some(id(7)),
                previous: id(8),
                next: id(8),
                page: id(3),
                rectangle: id(9),
            }),
        },
        indirect(
            9,
            PdfValue::Array(vec![
                PdfValue::Integer(1),
                PdfValue::Integer(2),
                PdfValue::Integer(3),
                PdfValue::Integer(4),
            ]),
        ),
    ]);
    let document = UnvalidatedPdfDocument {
        version: sample.version(),
        catalog: sample.catalog(),
        objects,
        trailer: Default::default(),
    }
    .validate()
    .expect("valid thread graph");
    let bytes = document
        .to_pdf_bytes_with_options(PdfSerializationOptions::default())
        .expect("typed thread graph serializes");
    for needle in [
        b"/Threads 6 0 R".as_slice(),
        b"/B[8 0 R]".as_slice(),
        b"/F 8 0 R".as_slice(),
        b"/T 7 0 R".as_slice(),
        b"/V 8 0 R".as_slice(),
        b"/N 8 0 R".as_slice(),
        b"/P 3 0 R".as_slice(),
        b"/R 9 0 R".as_slice(),
    ] {
        assert!(
            bytes.windows(needle.len()).any(|window| window == needle),
            "missing {}",
            String::from_utf8_lossy(needle)
        );
    }
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
