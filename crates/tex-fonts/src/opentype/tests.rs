use sha2::{Digest, Sha256};

use super::*;

fn key() -> FontRequestKey {
    FontRequestKey::new(
        "cmu-serif-roman",
        0,
        VariationSelection::default(),
        FontFeaturePolicy::default(),
    )
    .expect("request key")
}

fn wasm_request() -> FontRequest {
    FontRequest {
        key: key(),
        accepted_containers: AcceptedFontContainers::WASM,
        purposes: FontPurposes::LAYOUT_AND_HTML,
    }
}

#[test]
fn woff2_and_decoded_ttf_have_one_program_identity_and_projection() {
    let woff2 = include_bytes!("../../../umber-wasm/assets/cmu-serif-500-roman.woff2").to_vec();
    let ttf = woff2_patched::convert_woff2_to_ttf(&mut woff2.as_slice()).expect("decode fixture");
    let web = OpenTypeFont::parse(
        &wasm_request(),
        ResolvedFont {
            request: key(),
            container: FontContainer::Woff2,
            declared_object_sha256: Some(FontObjectIdentity::for_bytes(&woff2)),
            declared_program_identity: None,
            provenance: Some("CMU Serif under the SIL OFL".to_owned()),
            bytes: woff2,
        },
        FontLimits::default(),
    )
    .expect("WOFF2 font");
    let native_request = FontRequest {
        key: key(),
        accepted_containers: AcceptedFontContainers::NATIVE,
        purposes: FontPurposes::LAYOUT,
    };
    let native = OpenTypeFont::parse(
        &native_request,
        ResolvedFont {
            request: key(),
            container: FontContainer::TrueType,
            declared_object_sha256: Some(FontObjectIdentity::from_bytes(
                Sha256::digest(&ttf).into(),
            )),
            declared_program_identity: Some(web.identity),
            provenance: None,
            bytes: ttf,
        },
        FontLimits::default(),
    )
    .expect("TTF font");
    assert_eq!(web.identity, native.identity);
    assert_ne!(web.object_identity, native.object_identity);
    assert_eq!(web.cmap, native.cmap);
    assert_eq!(web.metrics, native.metrics);
    assert_eq!(web.shaping, native.shaping);
    assert_eq!(web.math, native.math);

    let (scalar, glyph) = web
        .cmap
        .mappings()
        .iter()
        .find_map(|(&scalar, &glyph)| {
            (scalar > u32::from(u8::MAX))
                .then(|| char::from_u32(scalar).map(|ch| (ch, glyph)))
                .flatten()
        })
        .expect("fixture has a non-Latin-1 Unicode mapping");
    let size = tex_arith::Scaled::from_raw(655_360);
    let loaded = crate::LoadedFont::new(
        "cmu-serif",
        "cmu-serif.tfm",
        [0; 32],
        0,
        size,
        size,
        vec![tex_arith::Scaled::from_raw(0); 7],
        crate::FontMetrics::new(Vec::new(), Vec::new(), None, None, Vec::new()),
    )
    .with_opentype(crate::OpenTypeProgramSelection {
        font: web.clone(),
        variation: VariationSelection::default(),
        features: FontFeaturePolicy::default(),
        direction: WritingDirection::LeftToRight,
    });
    let advance = web.metrics.horizontal_advances[usize::from(glyph)];
    assert!(loaded.character_exists(scalar));
    assert_eq!(
        loaded.character_width(scalar),
        Some(tex_arith::Scaled::from_raw(
            web.metrics
                .units_to_sp(i32::from(advance), size.raw())
                .expect("fixture advance scales")
        ))
    );
    assert!(matches!(
        loaded.metrics_source(),
        crate::FontMetricsSource::OpenType(_)
    ));
}

#[test]
fn stix_math_is_identical_from_woff2_and_native_sfnt() {
    let woff2 = include_bytes!("../../tests/fixtures/stix-two-math.woff2").to_vec();
    let ttf = woff2_patched::convert_woff2_to_ttf(&mut woff2.as_slice()).expect("decode fixture");
    let web = OpenTypeFont::parse(
        &wasm_request(),
        ResolvedFont {
            request: key(),
            container: FontContainer::Woff2,
            bytes: woff2,
            declared_object_sha256: None,
            declared_program_identity: None,
            provenance: Some("STIX Two Math under the SIL OFL".to_owned()),
        },
        FontLimits::default(),
    )
    .expect("STIX WOFF2");
    let native_request = FontRequest {
        key: key(),
        accepted_containers: AcceptedFontContainers::NATIVE,
        purposes: FontPurposes::LAYOUT,
    };
    let native = OpenTypeFont::parse(
        &native_request,
        ResolvedFont {
            request: key(),
            container: FontContainer::TrueType,
            bytes: ttf,
            declared_object_sha256: None,
            declared_program_identity: Some(web.identity),
            provenance: None,
        },
        FontLimits::default(),
    )
    .expect("STIX native sfnt");

    assert_eq!(web.identity, native.identity);
    assert_eq!(web.math, native.math);
    let size = tex_arith::Scaled::from_raw(10 * tex_arith::Scaled::UNITY);
    let web_loaded = crate::LoadedFont::new_opentype(
        "stix-web-math",
        "stix-two-math.woff2",
        size,
        size,
        crate::OpenTypeProgramSelection {
            font: web.clone(),
            variation: VariationSelection::default(),
            features: FontFeaturePolicy::default(),
            direction: WritingDirection::LeftToRight,
        },
    );
    let native_loaded = crate::LoadedFont::new_opentype(
        "stix-native-math",
        "stix-two-math.ttf",
        size,
        size,
        crate::OpenTypeProgramSelection {
            font: native.clone(),
            variation: VariationSelection::default(),
            features: FontFeaturePolicy::default(),
            direction: WritingDirection::LeftToRight,
        },
    );
    let crate::MathMetricsSource::OpenType(web_metrics) = web_loaded.math_metrics_source() else {
        panic!("web MATH metrics");
    };
    let crate::MathMetricsSource::OpenType(native_metrics) = native_loaded.math_metrics_source()
    else {
        panic!("native MATH metrics");
    };
    for ch in ['∑', '(', '⏞'] {
        let web_glyph = web_metrics
            .glyph(ch, 0)
            .expect("fixture construction glyph");
        let native_glyph = native_metrics
            .glyph(ch, 0)
            .expect("fixture construction glyph");
        assert_eq!(web_glyph, native_glyph);
        for direction in [
            crate::MathVariantDirection::Horizontal,
            crate::MathVariantDirection::Vertical,
        ] {
            assert_eq!(
                web_metrics.construction(web_glyph.glyph_id, direction),
                native_metrics.construction(native_glyph.glyph_id, direction)
            );
        }
    }
    let math = web.math.expect("fixture MATH table");
    assert!(!math.constants.values().is_empty());
    assert!(math.glyph_info.is_some());
    assert!(math.variants.is_some());
}

#[test]
fn stix_direct_math_metrics_cover_basic_layout_queries_and_classic_fallback() {
    let request = wasm_request();
    let font = OpenTypeFont::parse(
        &request,
        ResolvedFont {
            request: request.key.clone(),
            container: FontContainer::Woff2,
            bytes: include_bytes!("../../tests/fixtures/stix-two-math.woff2").to_vec(),
            declared_object_sha256: None,
            declared_program_identity: None,
            provenance: Some("STIX Two Math under the SIL OFL".to_owned()),
        },
        FontLimits::default(),
    )
    .expect("STIX fixture");
    let size = tex_arith::Scaled::from_raw(10 * tex_arith::Scaled::UNITY);
    let identity = font.identity;
    let loaded = crate::LoadedFont::new_opentype(
        "stix-two-math",
        "stix-two-math.woff2",
        size,
        size,
        crate::OpenTypeProgramSelection {
            font,
            variation: VariationSelection::default(),
            features: FontFeaturePolicy::default(),
            direction: WritingDirection::LeftToRight,
        },
    );
    let crate::MathMetricsSource::OpenType(math) = loaded.math_metrics_source() else {
        panic!("validated MATH table must be exposed directly")
    };
    assert_eq!(math.program_identity(), identity);
    assert!(math.constant(MathConstant::AxisHeight).raw() > 0);
    assert!(math.constant(MathConstant::FractionRuleThickness).raw() > 0);
    assert!(math.constant(MathConstant::SuperscriptShiftUp).raw() > 0);

    let italic = math.glyph('f', 0).expect("italic math glyph");
    assert_eq!(italic.metrics.italic_correction, italic.italic_correction);
    assert!(italic.italic_correction.raw() >= 0);
    let accent_base = math.glyph('A', 0).expect("accent base glyph");
    assert!(accent_base.top_accent_attachment.is_some());
    assert_eq!(math.glyph('A', 1), math.glyph('A', 1));
    assert_eq!(math.glyph('A', 2), math.glyph('A', 2));

    let math_tables = loaded
        .shaping_font()
        .expect("OpenType selection")
        .parts()
        .0
        .math
        .as_ref()
        .expect("MATH table");
    if let Some((&glyph, kerns)) = math_tables
        .glyph_info
        .as_ref()
        .and_then(|info| info.kern_info.iter().next())
    {
        let corner = if kerns.top_right.is_some() {
            crate::MathKernCorner::TopRight
        } else if kerns.top_left.is_some() {
            crate::MathKernCorner::TopLeft
        } else if kerns.bottom_right.is_some() {
            crate::MathKernCorner::BottomRight
        } else {
            crate::MathKernCorner::BottomLeft
        };
        assert_eq!(
            math.kern(glyph, corner, tex_arith::Scaled::from_raw(0)),
            math.kern(glyph, corner, tex_arith::Scaled::from_raw(0))
        );
    }
    assert!(loaded.supports_math());

    let classic = crate::LoadedFont::new(
        "classic",
        "classic.tfm",
        [0; 32],
        0,
        size,
        size,
        vec![],
        crate::FontMetrics::default(),
    );
    assert!(matches!(
        classic.math_metrics_source(),
        crate::MathMetricsSource::ClassicTfmExact
    ));
}

#[test]
fn opentype_only_font_synthesizes_versioned_text_fontdimens() {
    let request = wasm_request();
    let font = OpenTypeFont::parse(
        &request,
        ResolvedFont {
            request: request.key.clone(),
            container: FontContainer::Woff2,
            bytes: include_bytes!("../../../umber-wasm/assets/cmu-serif-500-roman.woff2").to_vec(),
            declared_object_sha256: None,
            declared_program_identity: None,
            provenance: None,
        },
        FontLimits::default(),
    )
    .expect("fixture font");
    let size = tex_arith::Scaled::from_raw(10 * tex_arith::Scaled::UNITY);
    let space_glyph = font.cmap.glyph(' ').expect("space glyph");
    let space = tex_arith::Scaled::from_raw(
        font.metrics
            .units_to_sp(
                i32::from(font.metrics.horizontal_advances[usize::from(space_glyph)]),
                size.raw(),
            )
            .expect("space advance scales"),
    );
    let x_height = font.metadata.x_height.map(|height| {
        tex_arith::Scaled::from_raw(
            font.metrics
                .units_to_sp(i32::from(height), size.raw())
                .expect("x-height scales"),
        )
    });
    let loaded = crate::LoadedFont::new_opentype(
        "cmu-serif-roman",
        "cmu-serif-roman",
        size,
        size,
        crate::OpenTypeProgramSelection {
            font,
            variation: VariationSelection::default(),
            features: FontFeaturePolicy::default(),
            direction: WritingDirection::LeftToRight,
        },
    );

    assert_eq!(crate::OPENTYPE_FONTDIMEN_SYNTHESIS_VERSION, 1);
    assert_eq!(loaded.parameters()[0], tex_arith::Scaled::from_raw(0));
    assert_eq!(loaded.parameters()[1], space);
    assert_eq!(loaded.parameters()[2].raw(), (space.raw() + 1) / 2);
    assert_eq!(loaded.parameters()[3].raw(), (space.raw() + 1) / 3);
    assert_eq!(
        loaded.parameters()[4],
        x_height.unwrap_or(tex_arith::Scaled::from_raw(0))
    );
    assert_eq!(loaded.parameters()[5], size);
    assert_eq!(loaded.parameters()[6], tex_arith::Scaled::from_raw(0));
    assert!(!loaded.supports_classic_math());
    assert!(loaded.character_exists('A'));
}

#[test]
fn mapped_tfm_identity_records_policy_map_and_classic_math_authority() {
    let request = wasm_request();
    let font = OpenTypeFont::parse(
        &request,
        ResolvedFont {
            request: request.key.clone(),
            container: FontContainer::Woff2,
            bytes: include_bytes!("../../../umber-wasm/assets/cmu-serif-500-roman.woff2").to_vec(),
            declared_object_sha256: None,
            declared_program_identity: None,
            provenance: None,
        },
        FontLimits::default(),
    )
    .expect("fixture font");
    let selection = || crate::OpenTypeProgramSelection {
        font: font.clone(),
        variation: VariationSelection::default(),
        features: FontFeaturePolicy::default(),
        direction: WritingDirection::LeftToRight,
    };
    let mut first_entries = vec![None; 256];
    first_entries[65] = Some("A".to_owned());
    let mut second_entries = first_entries.clone();
    second_entries[65] = Some("B".to_owned());
    let size = tex_arith::Scaled::from_raw(10 * tex_arith::Scaled::UNITY);
    let make = |map| {
        crate::LoadedFont::new(
            "cmr10",
            "cmr10.tfm",
            [7; 32],
            0,
            size,
            size,
            vec![],
            crate::FontMetrics::default(),
        )
        .with_mapped_opentype(
            selection(),
            crate::LegacyEncodingMap::new(map).expect("map"),
        )
    };
    let first = make(first_entries);
    let second = make(second_entries);
    assert_eq!(
        first.layout_policy(),
        crate::FontLayoutPolicy::OpenTypePreferred
    );
    assert!(first.character_exists('A'));
    assert_ne!(first.source_identity(), second.source_identity());
    assert!(matches!(
        first.math_metrics_source(),
        crate::MathMetricsSource::ClassicTfmExact
    ));
    assert_eq!(first.encoding_map().expect("map").version(), 1);
    assert_eq!(crate::FONT_LAYOUT_POLICY_VERSION, 1);
}

#[test]
fn opentype_unit_scaling_uses_shared_boundary_rounding() {
    let metrics = OpenTypeMetrics {
        units_per_em: 2,
        ascender: 0,
        descender: 0,
        line_gap: 0,
        global_bounds: None,
        horizontal_advances: Vec::new(),
        glyph_bounds: Vec::new(),
    };
    assert_eq!(metrics.units_to_sp(1, 5), Ok(3));
    assert_eq!(metrics.units_to_sp(-1, 5), Ok(-3));
    assert_eq!(
        metrics.units_to_sp(i32::MAX, i32::MAX),
        Err(FontParseError::ArithmeticOverflow)
    );
}

#[test]
fn request_selection_is_canonical_and_rejects_unsafe_duplicates() {
    let kern = FeatureSetting {
        tag: OpenTypeTag::new(*b"kern"),
        enabled: true,
    };
    assert_eq!(
        FontFeaturePolicy::new(vec![
            FeatureSetting {
                tag: OpenTypeTag::new(*b"liga"),
                enabled: true
            },
            kern,
        ])
        .expect("features"),
        FontFeaturePolicy::new(vec![
            kern,
            FeatureSetting {
                tag: OpenTypeTag::new(*b"liga"),
                enabled: true
            },
        ])
        .expect("features"),
    );
    assert_eq!(
        FontFeaturePolicy::new(vec![kern, kern]),
        Err(FontSelectionError::DuplicateFeature)
    );
}

#[test]
fn canonical_request_and_binary_response_encodings_round_trip() {
    let request = wasm_request();
    let encoded = request.to_wire_bytes();
    assert_eq!(&encoded[..5], b"UFRQ\x01");
    assert_eq!(FontRequest::from_wire_bytes(&encoded), Ok(request));

    let response = ResolvedFont {
        request: key(),
        container: FontContainer::Woff2,
        bytes: vec![0, 1, 2, 255],
        declared_object_sha256: Some(FontObjectIdentity::from_bytes([3; 32])),
        declared_program_identity: Some(FontProgramIdentity::from_bytes([4; 32])),
        provenance: Some("fixture".to_owned()),
    };
    let encoded = response.to_wire_bytes();
    assert_eq!(&encoded[..5], b"UFRS\x01");
    assert_eq!(ResolvedFont::from_wire_bytes(&encoded), Ok(response));
    assert_eq!(
        FontRequest::from_wire_bytes(b"UFRQ\x02"),
        Err(FontWireError::UnsupportedVersion)
    );
}

#[test]
fn mismatches_and_malformed_containers_fail_before_publication() {
    let request = wasm_request();
    let mismatch = ResolvedFont {
        request: key(),
        container: FontContainer::Woff2,
        bytes: b"wOF2".to_vec(),
        declared_object_sha256: Some(FontObjectIdentity::from_bytes([7; 32])),
        declared_program_identity: None,
        provenance: None,
    };
    assert_eq!(
        OpenTypeFont::parse(&request, mismatch, FontLimits::default()),
        Err(FontParseError::ObjectIdentityMismatch)
    );
}
