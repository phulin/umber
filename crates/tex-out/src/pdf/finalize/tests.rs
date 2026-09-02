use super::*;

#[test]
fn tounicode_cmap_matches_pdftex_generic_resource_shape() {
    // pdftex.web section 32e delegates to tounicode.c::write_tounicode.
    let mappings = [
        ToUnicodeMapping {
            code: b'A',
            mapping: ResolvedGlyphUnicode::Numeric(0x0041),
        },
        ToUnicodeMapping {
            code: b'B',
            mapping: ResolvedGlyphUnicode::String(vec![0x0066, 0x0066]),
        },
        ToUnicodeMapping {
            code: b'C',
            mapping: ResolvedGlyphUnicode::Numeric(0x0043),
        },
    ];
    let expected = b"%!PS-Adobe-3.0 Resource-CMap\n\
%%DocumentNeededResources: ProcSet (CIDInit)\n\
%%IncludeResource: ProcSet (CIDInit)\n\
%%BeginResource: CMap (TeX-cmr10-builtin-0)\n\
%%Title: (TeX-cmr10-builtin-0 TeX cmr10-builtin 0)\n\
%%Version: 1.000\n\
%%EndComments\n\
/CIDInit /ProcSet findresource begin\n\
12 dict begin\n\
begincmap\n\
/CIDSystemInfo\n\
<< /Registry (TeX)\n\
/Ordering (cmr10-builtin)\n\
/Supplement 0\n\
>> def\n\
/CMapName /TeX-cmr10-builtin-0 def\n\
/CMapType 2 def\n\
1 begincodespacerange\n\
<00> <FF>\n\
endcodespacerange\n\
0 beginbfrange\n\
endbfrange\n\
3 beginbfchar\n\
<41> <0041>\n\
<42> <00660066>\n\
<43> <0043>\n\
endbfchar\n\
endcmap\n\
CMapName currentdict /CMap defineresource pop\n\
end\n\
end\n\
%%EndResource\n\
%%EOF\n";

    let actual = build_to_unicode_cmap(b"cmr10-builtin", &mappings);
    assert_eq!(actual, expected);
    assert_eq!(actual.len(), 652);
}

#[test]
fn tounicode_ranges_exclude_strings_and_stop_before_utf16_low_byte_wrap() {
    // The string at 0x12 and the 0x00ff low byte independently prevent a
    // wider bfrange; neither surface-adjacent mapping may be folded into it.
    let mappings = [
        ToUnicodeMapping {
            code: 0x10,
            mapping: ResolvedGlyphUnicode::Numeric(0x00fe),
        },
        ToUnicodeMapping {
            code: 0x11,
            mapping: ResolvedGlyphUnicode::Numeric(0x00ff),
        },
        ToUnicodeMapping {
            code: 0x12,
            mapping: ResolvedGlyphUnicode::String(vec![0x0100]),
        },
        ToUnicodeMapping {
            code: 0x13,
            mapping: ResolvedGlyphUnicode::Numeric(0x0101),
        },
    ];

    let cmap =
        String::from_utf8(build_to_unicode_cmap(b"range-control", &mappings)).expect("ASCII CMap");
    assert!(cmap.contains("1 beginbfrange\n<10> <11> <00FE>\nendbfrange"));
    assert!(cmap.contains("2 beginbfchar\n<12> <0100>\n<13> <0101>\nendbfchar"));
    assert!(!cmap.contains("<10> <13>"));
}

#[test]
fn tounicode_glyph_resolution_strips_suffixes_and_composes_components() {
    let glyph_to_unicode = BTreeMap::from([
        (b"A".to_vec(), vec![0x0041]),
        (b"acute".to_vec(), vec![0x0301]),
    ]);

    assert_eq!(
        resolve_glyph_unicode(&glyph_to_unicode, false, b"A.alt_ignored"),
        Some(ResolvedGlyphUnicode::Numeric(0x0041))
    );
    assert_eq!(
        resolve_glyph_unicode(&glyph_to_unicode, false, b"A_acute.alt"),
        Some(ResolvedGlyphUnicode::String(vec![0x0041, 0x0301]))
    );
}

#[test]
fn tounicode_uses_unused_slots_from_the_original_builtin_encoding() {
    let header = b"%!PS\n/Encoding 256 array\n\
dup 65 /A put\n\
dup 90 /Z put\nreadonly def\n";
    let mut pfb = vec![0x80, 1];
    pfb.extend_from_slice(&(header.len() as u32).to_le_bytes());
    pfb.extend_from_slice(header);
    pfb.extend_from_slice(&[0x80, 2, 1, 0, 0, 0, 0, 0x80, 3]);
    let type1 = tex_fonts::PdfType1Program::from_pfb(&pfb).expect("valid synthetic Type-1 program");
    let glyph_to_unicode =
        BTreeMap::from([(b"A".to_vec(), vec![0x0041]), (b"Z".to_vec(), vec![0x005A])]);

    assert_eq!(
        to_unicode_mappings(&glyph_to_unicode, false, None, Some(&type1)),
        vec![
            ToUnicodeMapping {
                code: b'A',
                mapping: ResolvedGlyphUnicode::Numeric(0x0041),
            },
            ToUnicodeMapping {
                code: b'Z',
                mapping: ResolvedGlyphUnicode::Numeric(0x005A),
            },
        ]
    );
}

#[test]
fn font_dictionary_name_is_type3_only() {
    for subtype in ["Type1", "TrueType"] {
        let dictionary = font_dictionary_header(PdfFontDictionaryHeader::Scalable {
            subtype,
            base_font: b"ABCDEF+CanonicalFont",
        })
        .expect("valid scalable font header");
        assert!(dictionary.get(b"Name").is_none());
        assert_eq!(
            dictionary.get(b"BaseFont"),
            Some(&PdfValue::Name(PdfName::new(b"ABCDEF+CanonicalFont")))
        );
    }

    let dictionary = font_dictionary_header(PdfFontDictionaryHeader::Type3 {
        resource_name: b"F31",
    })
    .expect("valid Type-3 font header");
    assert_eq!(
        dictionary.get(b"Name"),
        Some(&PdfValue::Name(PdfName::new(b"F31")))
    );
    assert!(dictionary.get(b"BaseFont").is_none());
}

#[test]
fn type1_descriptor_fallback_uses_named_tfm_characters_instead_of_table_extrema() {
    let mut widths = [Scaled::from_raw(0); 256];
    let mut heights = [Scaled::from_raw(0); 256];
    let mut depths = [Scaled::from_raw(0); 256];
    widths[usize::from(b'.')] = Scaled::from_raw(156_000);
    widths[usize::from(b',')] = Scaled::from_raw(9 * Scaled::UNITY);
    heights[usize::from(b'h')] = Scaled::from_raw(7 * Scaled::UNITY);
    heights[usize::from(b'H')] = Scaled::from_raw(6 * Scaled::UNITY);
    heights[usize::from(b'A')] = Scaled::from_raw(9 * Scaled::UNITY);
    depths[usize::from(b'y')] = Scaled::from_raw(2 * Scaled::UNITY);
    depths[usize::from(b'g')] = Scaled::from_raw(3 * Scaled::UNITY);
    let metrics = PdfFontMetricsInput {
        widths,
        heights,
        depths,
        x_height: Scaled::from_raw(4 * Scaled::UNITY),
    };

    assert_eq!(
        type1_fallback_descriptor_metrics(&metrics, Scaled::from_raw(10 * Scaled::UNITY),),
        [700, -200, 600, 79, 400],
    );
}

#[test]
fn type1_std_vw_overrides_the_period_width_fallback() {
    fn type1_program(header: &[u8]) -> tex_fonts::PdfType1Program {
        let mut pfb = vec![0x80, 1];
        pfb.extend_from_slice(&(header.len() as u32).to_le_bytes());
        pfb.extend_from_slice(header);
        pfb.extend_from_slice(&[0x80, 2, 1, 0, 0, 0, 0, 0x80, 3]);
        tex_fonts::PdfType1Program::from_pfb(&pfb).expect("valid synthetic Type-1 program")
    }

    let explicit = type1_program(b"%!PS\n/StdVW [71] def\n");
    let absent = type1_program(b"%!PS\n/ItalicAngle 0 def\n");

    assert_eq!(type1_descriptor_stem_v(&explicit, 79), 71);
    assert_eq!(type1_descriptor_stem_v(&absent, 79), 79);
}
