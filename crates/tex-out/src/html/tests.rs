use std::cell::Cell;

use tex_arith::Scaled;
use umber_hash::{AHash64, HashDomain};

use crate::{
    BoxNode, FontResource, GlueOrder, GlueSetRatio, GlueSign, JobInfo, MathGlyph,
    MathGlyphSelection, MathOutputEvent, MathRule, MathStart, OpenTypeFontResource, PageEffect,
    PageNode, UnvalidatedPageArtifact,
};

use super::incremental::{
    PatchLimits, PatchOp, RenderLimits, RenderSessionId, build_positioned_render_document,
    build_render_document, plan_patch,
};
use super::{
    AssetMode, HtmlError, HtmlFontAsset, HtmlFontAssets, HtmlFontKey, HtmlOptions,
    RenderedOutputId, validate_mapped_glyphs, write_html, write_positioned_html,
    write_render_document,
};

fn sp(raw: i32) -> Scaled {
    Scaled::from_raw(raw)
}

#[test]
fn rendered_output_identity_has_one_canonical_safe_encoding() {
    let identity = RenderedOutputId::from_bytes([0xab; 16]);
    assert_eq!(identity.to_string(), "abababababababababababababababab");
    assert_eq!(
        RenderedOutputId::parse_hex(&identity.to_string()),
        Some(identity)
    );
    assert_eq!(
        RenderedOutputId::parse_hex("ABABABABABABABABABABABABABABABAB"),
        None
    );
    assert_eq!(RenderedOutputId::parse_hex("éééééééééééééééé"), None);
}

#[test]
fn manifest_reuses_one_retained_object_and_program_derived_family() {
    let mut page = page();
    let bytes = include_bytes!("../../../umber-wasm/assets/cmu-serif-500-roman.woff2");
    let program = parsed_font("cmu", bytes).identity;
    page.testing_mut().fonts[0].opentype = Some(OpenTypeFontResource {
        program_identity: program,
        object_identity: tex_fonts::FontObjectIdentity::for_bytes(bytes),
        instance_identity: tex_fonts::FontInstanceIdentity::from_bytes([8; 8]),
        container: tex_fonts::FontContainer::Woff2,
        face_index: 0,
        variation: tex_fonts::VariationSelection::new(vec![tex_fonts::VariationCoordinate {
            tag: tex_fonts::OpenTypeTag::new(*b"wght"),
            value: 700 << 16,
        }])
        .expect("variation"),
        features: tex_fonts::FontFeaturePolicy::new(vec![tex_fonts::FeatureSetting {
            tag: tex_fonts::OpenTypeTag::new(*b"salt"),
            value: 2,
        }])
        .expect("features"),
        direction: tex_fonts::WritingDirection::RightToLeft,
        script: Some(tex_fonts::OpenTypeTag::new(*b"arab")),
        language: Some(tex_fonts::FontLanguage::new("ar").expect("language")),
        encoding_map_version: None,
        encoding_map_identity: None,
        fontdimen_synthesis_version: None,
    });
    let options = HtmlOptions {
        asset_mode: AssetMode::Manifest {
            relative_directory: "fonts".to_owned(),
        },
        ..HtmlOptions::default()
    };
    let resolver = Resolver { missing_b: false };
    let output = write_html(&[page.clone(), page], &resolver, &options).expect("manifest HTML");
    assert_eq!(output.assets.len(), 1);
    assert!(output.assets[0].path.starts_with("ahash64-v1-"));
    let html = String::from_utf8(output.html).expect("UTF-8 HTML");
    assert!(html.contains(&format!("umber-font-{}", super::hex(&program.bytes()))));
    assert!(html.contains("fonts/ahash64-v1-"));
    assert!(html.contains("font-variation-settings:'wght' 700"));
    assert!(html.contains("font-feature-settings:'salt' 2"));
    assert!(html.contains("direction=\"rtl\" lang=\"ar\""));
    assert!(html.contains("data-umber-script=\"arab\""));
    assert_eq!(html.matches("data-umber-revision=\"1\"").count(), 2);
}

fn parsed_font(name: &str, bytes: &[u8]) -> tex_fonts::OpenTypeFont {
    let key = tex_fonts::FontRequestKey::new(
        name,
        0,
        tex_fonts::VariationSelection::default(),
        tex_fonts::FontFeaturePolicy::default(),
    )
    .expect("fixture key");
    tex_fonts::OpenTypeFont::parse(
        &tex_fonts::FontRequest {
            key: key.clone(),
            accepted_containers: tex_fonts::AcceptedFontContainers::WASM,
            purposes: tex_fonts::FontPurposes::LAYOUT_AND_HTML,
        },
        tex_fonts::ResolvedFont {
            request: key,
            container: tex_fonts::FontContainer::Woff2,
            bytes: bytes.to_vec(),
            declared_object_ahash64: None,
            declared_program_identity: None,
            provenance: None,
            legacy_mapping: None,
        },
        tex_fonts::FontLimits::default(),
    )
    .expect("validated fixture font")
}

#[test]
fn mapping_validation_uses_the_selected_collection_face() {
    let cmu_woff2 = include_bytes!("../../../umber-wasm/assets/cmu-serif-500-roman.woff2");
    let cmu =
        woff2_patched::convert_woff2_to_ttf(&mut cmu_woff2.as_slice()).expect("decode CMU fixture");
    let stix_woff2 = include_bytes!("../../../tex-fonts/tests/fixtures/stix-two-math.woff2");
    let stix = woff2_patched::convert_woff2_to_ttf(&mut stix_woff2.as_slice())
        .expect("decode STIX fixture");
    let collection = make_ttc([cmu.as_slice(), stix.as_slice()]);
    let first = ttf_parser::Face::parse(&collection, 0).expect("first collection face");
    let second = ttf_parser::Face::parse(&collection, 1).expect("second collection face");
    let scalar = (0..=0x10ffff).find_map(|value| {
        let scalar = char::from_u32(value)?;
        (first.glyph_index(scalar).is_none() && second.glyph_index(scalar).is_some())
            .then_some(scalar)
    });
    let scalar = scalar.expect("STIX fixture has a glyph absent from CMU Serif");
    let mut encoding = vec![None; 256];
    encoding[usize::from(b'A')] = Some(scalar.to_string());

    validate_mapped_glyphs(&collection, 1, &encoding, "collection")
        .expect("selected second face supplies the mapped glyph");
    assert!(matches!(
        validate_mapped_glyphs(&collection, 0, &encoding, "collection"),
        Err(HtmlError::MissingFontGlyph {
            code: b'A',
            ch,
            ..
        }) if ch == scalar
    ));
}

fn make_ttc(faces: [&[u8]; 2]) -> Vec<u8> {
    fn relocate(face: &[u8], base: usize) -> Vec<u8> {
        let mut face = face.to_vec();
        let table_count = usize::from(u16::from_be_bytes([face[4], face[5]]));
        for index in 0..table_count {
            let offset = 12 + index * 16 + 8;
            let old = u32::from_be_bytes(face[offset..offset + 4].try_into().expect("offset"));
            let new = old
                .checked_add(u32::try_from(base).expect("fixture size"))
                .expect("offset");
            face[offset..offset + 4].copy_from_slice(&new.to_be_bytes());
        }
        face
    }

    let first_offset = 20_usize;
    let second_offset = (first_offset + faces[0].len() + 3) & !3;
    let mut collection = Vec::with_capacity(second_offset + faces[1].len());
    collection.extend_from_slice(b"ttcf");
    collection.extend_from_slice(&0x0001_0000_u32.to_be_bytes());
    collection.extend_from_slice(&2_u32.to_be_bytes());
    collection.extend_from_slice(&(first_offset as u32).to_be_bytes());
    collection.extend_from_slice(&(second_offset as u32).to_be_bytes());
    collection.extend_from_slice(&relocate(faces[0], first_offset));
    collection.resize(second_offset, 0);
    collection.extend_from_slice(&relocate(faces[1], second_offset));
    collection
}

struct Resolver {
    missing_b: bool,
}

enum BrokenFont {
    Container,
    Cmap,
}

impl HtmlFontAssets for BrokenFont {
    fn font_asset(&self, font: &FontResource) -> Result<HtmlFontAsset, String> {
        let mut web = Resolver { missing_b: false }.font_asset(font)?;
        match self {
            Self::Container => web.woff2 = b"wOF2not-a-font".to_vec(),
            Self::Cmap => web.encoding[usize::from(b'A')] = Some("\u{10ffff}".to_owned()),
        }
        web.ahash64 = AHash64::for_bytes(HashDomain::HtmlResource, &web.woff2).to_le_bytes();
        Ok(web)
    }
}

impl HtmlFontAssets for Resolver {
    fn font_asset(&self, font: &FontResource) -> Result<HtmlFontAsset, String> {
        let bytes = include_bytes!("../../../umber-wasm/assets/cmu-serif-500-roman.woff2").to_vec();
        let mut encoding = vec![None; 256];
        encoding[usize::from(b'A')] = Some("A".to_owned());
        if !self.missing_b {
            encoding[usize::from(b'B')] = Some("<&B".to_owned());
        }
        Ok(HtmlFontAsset {
            key: HtmlFontKey::from(font),
            ahash64: AHash64::for_bytes(HashDomain::HtmlResource, &bytes).to_le_bytes(),
            woff2: bytes,
            encoding,
            provenance: "test fixture".to_owned(),
            embeddable: true,
        })
    }
}

struct SingleScalarResolver;

impl HtmlFontAssets for SingleScalarResolver {
    fn font_asset(&self, font: &FontResource) -> Result<HtmlFontAsset, String> {
        let mut web = Resolver { missing_b: false }.font_asset(font)?;
        web.encoding[usize::from(b'B')] = Some("B".to_owned());
        Ok(web)
    }
}

struct OrderedResolver;

impl HtmlFontAssets for OrderedResolver {
    fn font_asset(&self, font: &FontResource) -> Result<HtmlFontAsset, String> {
        let bytes = if font.name == "second" {
            include_bytes!("../../../tex-fonts/tests/fixtures/stix-two-math.woff2").to_vec()
        } else {
            include_bytes!("../../../umber-wasm/assets/cmu-serif-500-roman.woff2").to_vec()
        };
        let mut encoding = vec![None; 256];
        encoding[usize::from(b'A')] = Some("A".to_owned());
        encoding[usize::from(b'B')] = Some("B".to_owned());
        Ok(HtmlFontAsset {
            key: HtmlFontKey::from(font),
            ahash64: AHash64::for_bytes(HashDomain::HtmlResource, &bytes).to_le_bytes(),
            woff2: bytes,
            encoding,
            provenance: font.name.clone(),
            embeddable: true,
        })
    }
}

struct CountingResolver {
    calls: Cell<usize>,
}

impl HtmlFontAssets for CountingResolver {
    fn font_asset(&self, font: &FontResource) -> Result<HtmlFontAsset, String> {
        self.calls.set(self.calls.get() + 1);
        OrderedResolver.font_asset(font)
    }
}

struct MathResolver;

impl HtmlFontAssets for MathResolver {
    fn font_asset(&self, font: &FontResource) -> Result<HtmlFontAsset, String> {
        let bytes =
            include_bytes!("../../../tex-fonts/tests/fixtures/stix-two-math.woff2").to_vec();
        Ok(HtmlFontAsset {
            key: HtmlFontKey::from(font),
            ahash64: AHash64::for_bytes(HashDomain::HtmlResource, &bytes).to_le_bytes(),
            woff2: bytes,
            encoding: vec![None; 256],
            provenance: "STIX Two Math under the SIL OFL".to_owned(),
            embeddable: true,
        })
    }
}

#[test]
fn positioned_math_uses_ssty_text_rules_and_validated_outline_paths() {
    let bytes = include_bytes!("../../../tex-fonts/tests/fixtures/stix-two-math.woff2");
    let parsed = parsed_font("stix-two-math", bytes);
    let instance = tex_fonts::FontInstanceIdentity::from_bytes([0x5a; 8]);
    let mut page = page();
    let PageNode::HList(root) = &mut page.testing_mut().root else {
        unreachable!()
    };
    root.children.clear();
    page.testing_mut().fonts[0].name = "stix-two-math".to_owned();
    page.testing_mut().fonts[0].opentype = Some(OpenTypeFontResource {
        program_identity: parsed.identity,
        object_identity: parsed.object_identity,
        instance_identity: instance,
        container: tex_fonts::FontContainer::Woff2,
        face_index: 0,
        variation: tex_fonts::VariationSelection::default(),
        features: tex_fonts::FontFeaturePolicy::default(),
        direction: tex_fonts::WritingDirection::LeftToRight,
        script: None,
        language: None,
        encoding_map_version: None,
        encoding_map_identity: None,
        fontdimen_synthesis_version: None,
    });
    let scalar = 'A';
    let text_glyph = selected_fixture_glyph(bytes, scalar, 2);
    let glyph_count = parsed.metadata.glyph_count;
    let size = Scaled::from_raw(10 * Scaled::UNITY);
    let loaded = tex_fonts::LoadedFont::new_opentype(
        "stix-two-math",
        "stix-two-math.woff2",
        size,
        size,
        parsed.clone(),
    );
    let tex_fonts::MathMetricsSource::OpenType(math) = loaded.math_metrics_source() else {
        panic!("STIX MATH metrics")
    };
    let outline_glyph = (0..glyph_count)
        .find_map(|glyph| {
            [
                tex_fonts::MathVariantDirection::Vertical,
                tex_fonts::MathVariantDirection::Horizontal,
            ]
            .into_iter()
            .find_map(|direction| math.construction(glyph, direction))
            .and_then(|construction| {
                construction
                    .assembly
                    .and_then(|assembly| assembly.parts.first().map(|part| part.glyph.glyph_id))
                    .or_else(|| {
                        construction
                            .variants
                            .first()
                            .map(|variant| variant.glyph.glyph_id)
                    })
            })
        })
        .expect("STIX has a variant or assembly outline");
    page.testing_mut().math_events = vec![
        MathOutputEvent::Start(MathStart {
            id: 91,
            x: sp(-20),
            baseline: sp(300),
            width: sp(800),
            height: sp(240),
            depth: sp(60),
        }),
        MathOutputEvent::Glyph(MathGlyph {
            font_instance: instance,
            glyph_id: text_glyph,
            selection: MathGlyphSelection::Cmap {
                scalar: scalar as u32,
            },
            ssty: 2,
            x: sp(10),
            baseline: sp(200),
            width: sp(100),
            height: sp(120),
            depth: sp(20),
        }),
        MathOutputEvent::Rule(MathRule {
            x: sp(120),
            y: sp(150),
            width: sp(300),
            height: sp(12),
        }),
        MathOutputEvent::Glyph(MathGlyph {
            font_instance: instance,
            glyph_id: outline_glyph,
            selection: MathGlyphSelection::OutlineFallback,
            ssty: 0,
            x: sp(450),
            baseline: sp(220),
            width: sp(140),
            height: sp(180),
            depth: sp(30),
        }),
        MathOutputEvent::End,
    ];
    let output =
        write_html(&[page], &MathResolver, &HtmlOptions::default()).expect("positioned math HTML");
    let html = String::from_utf8(output.html).expect("UTF-8 HTML");
    assert!(html.contains("class=\"umber-math\""));
    assert!(html.contains("data-umber-math=\"91\" data-umber-x-sp=\"-20\""));
    assert!(html.contains("font-feature-settings:'ssty' 2"));
    assert!(html.contains(">A</text>"));
    assert!(html.contains("class=\"umber-math-rule\""));
    assert!(html.contains("class=\"umber-math-outline\" d=\"M"));
    assert!(html.contains("transform=\"translate("));
}

#[test]
fn positioned_math_rejects_unpublished_programs_and_unreproducible_cmap_glyphs() {
    let bytes = include_bytes!("../../../tex-fonts/tests/fixtures/stix-two-math.woff2");
    let parsed = parsed_font("stix-two-math", bytes);
    let instance = tex_fonts::FontInstanceIdentity::from_bytes([0x33; 8]);
    let mut page = page();
    let PageNode::HList(root) = &mut page.testing_mut().root else {
        unreachable!()
    };
    root.children.clear();
    page.testing_mut().fonts[0].name = "stix-two-math".to_owned();
    page.testing_mut().fonts[0].opentype = Some(OpenTypeFontResource {
        program_identity: tex_fonts::FontProgramIdentity::from_bytes([0xff; 8]),
        object_identity: parsed.object_identity,
        instance_identity: instance,
        container: tex_fonts::FontContainer::Woff2,
        face_index: 0,
        variation: tex_fonts::VariationSelection::default(),
        features: tex_fonts::FontFeaturePolicy::default(),
        direction: tex_fonts::WritingDirection::LeftToRight,
        script: None,
        language: None,
        encoding_map_version: None,
        encoding_map_identity: None,
        fontdimen_synthesis_version: None,
    });
    assert!(matches!(
        write_html(&[page.clone()], &MathResolver, &HtmlOptions::default()),
        Err(HtmlError::CorruptFontAsset { .. })
    ));

    page.testing_mut().fonts[0]
        .opentype
        .as_mut()
        .expect("test font has OpenType identity")
        .program_identity = parsed.identity;
    page.testing_mut().math_events = vec![
        MathOutputEvent::Start(MathStart {
            id: 1,
            x: sp(0),
            baseline: sp(0),
            width: sp(1),
            height: sp(1),
            depth: sp(0),
        }),
        MathOutputEvent::Glyph(MathGlyph {
            font_instance: instance,
            glyph_id: u16::MAX,
            selection: MathGlyphSelection::Cmap { scalar: 'A' as u32 },
            ssty: 0,
            x: sp(0),
            baseline: sp(0),
            width: sp(1),
            height: sp(1),
            depth: sp(0),
        }),
        MathOutputEvent::End,
    ];
    assert!(matches!(
        write_html(&[page], &MathResolver, &HtmlOptions::default()),
        Err(HtmlError::MathGlyphMismatch { .. })
    ));
}

fn selected_fixture_glyph(mut bytes: &[u8], scalar: char, ssty: u8) -> u16 {
    let sfnt = woff2_patched::convert_woff2_to_ttf(&mut bytes).expect("decode STIX");
    let face = rustybuzz::Face::from_slice(&sfnt, 0).expect("shape STIX");
    let mut buffer = rustybuzz::UnicodeBuffer::new();
    let mut encoded = [0; 4];
    buffer.push_str(scalar.encode_utf8(&mut encoded));
    let feature = rustybuzz::Feature::new(
        rustybuzz::ttf_parser::Tag::from_bytes(b"ssty"),
        u32::from(ssty),
        ..,
    );
    u16::try_from(rustybuzz::shape(&face, &[feature], buffer).glyph_infos()[0].glyph_id)
        .expect("fixture glyph id")
}

#[test]
fn serialization_is_deterministic_exact_and_escaped() {
    let page = page();
    let options = HtmlOptions {
        revision: 42,
        ..HtmlOptions::default()
    };
    let first_resolver = Resolver { missing_b: false };
    let first =
        write_html(std::slice::from_ref(&page), &first_resolver, &options).expect("first HTML");
    let second_resolver = Resolver { missing_b: false };
    let second = write_html(&[page], &second_resolver, &options).expect("second HTML");
    assert_eq!(first, second);
    let html = String::from_utf8(first.html).expect("UTF-8 HTML");
    assert!(html.contains("data-umber-page=\"1\" data-umber-revision=\"42\""));
    assert!(html.contains("data-umber-output=\"00000000000000000000000000000000\""));
    assert!(html.contains("data-umber-x-sp=\"17\""));
    assert!(html.contains("data-umber-baseline-sp=\"53\""));
    assert!(html.contains("A&lt;&amp;B"));
    assert!(!html.contains("<script>alert(1)</script>"));
    assert!(
        html.contains(
            "data-umber-special-hex=\"3c7363726970743e616c6572742831293c2f7363726970743e\""
        )
    );
}

#[test]
fn single_scalar_runs_use_exact_tex_character_positions() {
    let output = write_html(&[page()], &SingleScalarResolver, &HtmlOptions::default())
        .expect("positioned HTML");
    let html = String::from_utf8(output.html).expect("UTF-8 HTML");

    assert!(html.contains("x=\"0.00034457px 0.00095265px\""), "{html}");
}

#[test]
fn configured_physical_dimensions_build_the_page_box() {
    let mut page = page();
    page.testing_mut().job.page_width = sp(1_000);
    page.testing_mut().job.page_height = sp(2_000);
    let resolver = Resolver { missing_b: false };

    let output =
        write_html(&[page], &resolver, &HtmlOptions::default()).expect("physical page HTML");
    let html = String::from_utf8(output.html).expect("UTF-8 HTML");

    assert!(html.contains("data-umber-width-sp=\"1000\""));
    assert!(html.contains("data-umber-height-sp=\"2000\""));
    assert!(html.contains("style=\"width:0.02026904px;height:0.04053809px\""));
}

#[test]
fn plain_tex_fallback_surrounds_content_with_the_dvi_origin() {
    let mut page = page();
    page.testing_mut().job.page_origin_x = sp(4_736_286);
    page.testing_mut().job.page_origin_y = sp(4_736_286);
    let resolver = Resolver { missing_b: false };

    let output =
        write_html(&[page], &resolver, &HtmlOptions::default()).expect("plain TeX page HTML");
    let html = String::from_utf8(output.html).expect("UTF-8 HTML");

    assert!(html.contains("data-umber-width-sp=\"9472806\""));
    assert!(html.contains("data-umber-height-sp=\"9472643\""));
    assert!(html.contains("data-umber-origin-x-sp=\"4736286\""));
    assert!(
        html.contains(
            "class=\"umber-page-content\" style=\"left:95.99998541px;top:95.99998541px\""
        )
    );
}

#[test]
fn unavailable_text_mapping_is_actionable() {
    let resolver = Resolver { missing_b: true };
    let error =
        write_html(&[page()], &resolver, &HtmlOptions::default()).expect_err("mapping failure");
    assert_eq!(
        error,
        HtmlError::MissingTextMapping {
            font: "cmr10".to_owned(),
            code: u32::from(b'B')
        }
    );
}

#[test]
fn invalid_woff2_and_uncovered_mappings_fail_before_serialization() {
    assert!(matches!(
        write_html(&[page()], &BrokenFont::Container, &HtmlOptions::default()),
        Err(HtmlError::CorruptFontAsset { .. })
    ));
    assert!(matches!(
        write_html(&[page()], &BrokenFont::Cmap, &HtmlOptions::default()),
        Err(HtmlError::MissingFontGlyph {
            code: b'A',
            ch: '\u{10ffff}',
            ..
        })
    ));
}

#[test]
fn allowlisted_color_link_and_destination_are_typed_and_escaped() {
    let mut page = page();
    page.testing_mut().effects = vec![
        PageEffect::Special {
            class: "html".to_owned(),
            payload: b"color push red".to_vec(),
        },
        PageEffect::Special {
            class: "html".to_owned(),
            payload: b"link https://example.test/path?a=1&b=2".to_vec(),
        },
        PageEffect::Special {
            class: "html".to_owned(),
            payload: b"endlink".to_vec(),
        },
        PageEffect::Special {
            class: "html".to_owned(),
            payload: b"dest section.1".to_vec(),
        },
        PageEffect::Special {
            class: "html".to_owned(),
            payload: b"color pop".to_vec(),
        },
    ];
    let PageNode::HList(root) = &mut page.testing_mut().root else {
        unreachable!()
    };
    root.children = vec![
        PageNode::WhatsitAnchor { effect_index: 0 },
        PageNode::WhatsitAnchor { effect_index: 1 },
        PageNode::Char {
            font_id: 7,
            ch: b'A' as u32,
            width: sp(30),
        },
        PageNode::WhatsitAnchor { effect_index: 2 },
        PageNode::WhatsitAnchor { effect_index: 3 },
        PageNode::WhatsitAnchor { effect_index: 4 },
    ];
    let resolver = Resolver { missing_b: false };
    let output = write_html(&[page], &resolver, &HtmlOptions::default()).expect("special HTML");
    let html = String::from_utf8(output.html).expect("UTF-8");
    assert!(html.contains("<svg class=\"umber-run\""));
    assert!(
        html.contains(";color:red\"><rect class=\"umber-baseline\"")
            && html.contains("<a href=\"https://example.test/path?a=1&amp;b=2\""),
        "{html}"
    );
    assert!(html.contains("id=\"umber-dest-section.1\""));
    assert!(html.contains(
        "class=\"umber-a11y\" role=\"group\" aria-label=\"Page 1\"><p class=\"umber-a11y-line\"><a href=\"https://example.test/path?a=1&amp;b=2\" rel=\"noreferrer noopener\">A</a></p>"
    ));
}

#[test]
fn accessibility_tree_separates_pages_and_lines_without_moving_geometry() {
    let mut page = page();
    page.testing_mut().effects = vec![
        PageEffect::Special {
            class: "html".to_owned(),
            payload: b"link https://example.test/target".to_vec(),
        },
        PageEffect::Special {
            class: "html".to_owned(),
            payload: b"endlink".to_vec(),
        },
    ];
    let line = |children| {
        PageNode::HList(BoxNode {
            width: sp(200),
            height: sp(40),
            depth: sp(5),
            shift: sp(0),
            glue_set: GlueSetRatio::ZERO,
            glue_sign: GlueSign::Normal,
            glue_order: GlueOrder::Normal,
            children,
        })
    };
    page.testing_mut().root = PageNode::VList(BoxNode {
        width: sp(200),
        height: sp(90),
        depth: sp(0),
        shift: sp(0),
        glue_set: GlueSetRatio::ZERO,
        glue_sign: GlueSign::Normal,
        glue_order: GlueOrder::Normal,
        children: vec![
            line(vec![
                PageNode::WhatsitAnchor { effect_index: 0 },
                PageNode::Char {
                    font_id: 7,
                    ch: b'A' as u32,
                    width: sp(30),
                },
                PageNode::WhatsitAnchor { effect_index: 1 },
            ]),
            line(vec![PageNode::Char {
                font_id: 7,
                ch: b'B' as u32,
                width: sp(30),
            }]),
        ],
    });
    let resolver = SingleScalarResolver;
    let output = write_html(&[page.clone(), page], &resolver, &HtmlOptions::default())
        .expect("accessible positioned HTML");
    let html = String::from_utf8(output.html).expect("UTF-8");

    assert_eq!(html.matches("role=\"group\" aria-label=\"Page ").count(), 2);
    assert!(html.contains("role=\"group\" aria-label=\"Page 1\""));
    assert!(html.contains("role=\"group\" aria-label=\"Page 2\""));
    assert_eq!(html.matches("<p class=\"umber-a11y-line\">").count(), 4);
    assert_eq!(
        html.matches("<a href=\"https://example.test/target\" rel=\"noreferrer noopener\">A</a>")
            .count(),
        2
    );
    assert_eq!(
        html.matches("<p class=\"umber-a11y-line\">B</p>").count(),
        2
    );

    // The semantic tree is geometry-free; exact positioned events retain the
    // same integer anchors and remain independently hidden from accessibility.
    assert_eq!(
        html.matches("class=\"umber-run\" aria-hidden=\"true\"")
            .count(),
        4
    );
    assert_eq!(
        html.matches("class=\"umber-run\" aria-hidden=\"true\" data-umber-event=\"3\" data-umber-x-sp=\"17\" data-umber-baseline-sp=\"53\"")
            .count(),
        2
    );
    assert_eq!(
        html.matches("class=\"umber-run\" aria-hidden=\"true\" data-umber-event=\"7\" data-umber-x-sp=\"17\" data-umber-baseline-sp=\"98\"")
            .count(),
        2
    );
}

#[test]
fn dangerous_link_special_fails_without_markup_injection() {
    let mut page = page();
    page.testing_mut().effects[0] = PageEffect::Special {
        class: "html".to_owned(),
        payload: b"link javascript:alert(1)".to_vec(),
    };
    let resolver = Resolver { missing_b: false };
    assert!(matches!(
        write_html(&[page], &resolver, &HtmlOptions::default()),
        Err(HtmlError::InvalidSpecial { .. })
    ));
}

#[test]
fn positioned_entry_point_and_embedded_assets_obey_caller_limits() {
    let page = page();
    let positioned = crate::positioned::lower_page(&page, 1).expect("position page");
    let mut options = HtmlOptions {
        max_pages: 1,
        ..HtmlOptions::default()
    };
    let resolver = Resolver { missing_b: false };
    assert_eq!(
        write_positioned_html(
            &[positioned.clone(), positioned.clone()],
            &resolver,
            &options
        )
        .expect_err("page limit"),
        HtmlError::TooManyPages { count: 2, limit: 1 }
    );
    options.max_positioned_events = 0;
    let resolver = Resolver { missing_b: false };
    assert!(matches!(
        write_positioned_html(std::slice::from_ref(&positioned), &resolver, &options),
        Err(HtmlError::Positioned(
            crate::positioned::PositionedError::TooManyEvents { limit: 0 }
        ))
    ));
    options.max_positioned_events = usize::MAX;
    options.max_text_run_units = 1;
    let resolver = Resolver { missing_b: false };
    assert!(matches!(
        write_positioned_html(std::slice::from_ref(&positioned), &resolver, &options),
        Err(HtmlError::Positioned(
            crate::positioned::PositionedError::TextRunTooLong { limit: 1 }
        ))
    ));
    options.max_text_run_units = usize::MAX;
    options.max_total_asset_bytes = 3;
    let resolver = Resolver { missing_b: false };
    assert!(matches!(
        write_positioned_html(std::slice::from_ref(&positioned), &resolver, &options),
        Err(HtmlError::AssetsTooLarge { .. })
    ));
    options.max_total_asset_bytes = usize::MAX;
    options.max_html_bytes = 64;
    let resolver = Resolver { missing_b: false };
    assert!(matches!(
        write_positioned_html(&[positioned], &resolver, &options),
        Err(HtmlError::HtmlTooLarge { .. })
    ));
}

#[test]
fn shared_render_document_matches_public_bytes_assets_and_incremental_identity() {
    let resolver = OrderedResolver;
    let options = HtmlOptions {
        asset_mode: AssetMode::Manifest {
            relative_directory: "fonts".to_owned(),
        },
        output_id: RenderedOutputId::from_bytes([0x31; 16]),
        revision: 7,
        ..HtmlOptions::default()
    };
    let mut second = page();
    second.testing_mut().fonts[0].name = "second".to_owned();
    let artifacts = [page(), second];
    let positioned = artifacts
        .iter()
        .enumerate()
        .map(|(index, page)| {
            crate::positioned::lower_page(page, (index + 1) as u32).expect("positioned page")
        })
        .collect::<Vec<_>>();
    let public =
        write_positioned_html(&positioned, &resolver, &options).expect("public standalone HTML");
    let artifact = write_html(&artifacts, &resolver, &options).expect("artifact standalone HTML");
    assert_eq!(artifact, public);
    let document = build_positioned_render_document(
        &positioned,
        &resolver,
        &options,
        RenderSessionId::from_bytes(options.output_id.as_bytes()),
        options.revision,
        None,
        RenderLimits::default(),
    )
    .expect("shared render document");
    let detached = write_render_document(&document, &options).expect("detached standalone HTML");
    assert_eq!(detached, public);

    let incremental = build_render_document(
        &artifacts,
        &resolver,
        &options,
        RenderSessionId::from_bytes(options.output_id.as_bytes()),
        options.revision,
        None,
        RenderLimits::default(),
    )
    .expect("incremental render document")
    .revision;
    assert_eq!(document.revision, incremental);
    for (positioned, rendered) in positioned.iter().zip(&document.revision.pages) {
        let expected = positioned
            .events
            .iter()
            .enumerate()
            .filter_map(|(ordinal, event)| match event {
                crate::positioned::PositionedEvent::Box(_)
                | crate::positioned::PositionedEvent::Rule(_)
                | crate::positioned::PositionedEvent::TextRun(_)
                | crate::positioned::PositionedEvent::Special(_) => Some(ordinal as u32),
                _ => None,
            })
            .chain((0..positioned.math_events.len()).map(|ordinal| ordinal as u32))
            .collect::<Vec<_>>();
        assert_eq!(
            rendered
                .nodes
                .iter()
                .map(|node| node.event_ordinal)
                .collect::<Vec<_>>(),
            expected
        );
    }
    assert_eq!(
        detached
            .assets
            .iter()
            .map(|asset| asset.ahash64)
            .collect::<Vec<_>>(),
        document
            .revision
            .resources
            .iter()
            .map(|resource| resource.identity)
            .collect::<Vec<_>>()
    );
    assert_eq!(detached.assets.len(), 2);
}

#[test]
fn detached_serialization_does_not_resolve_fonts_again() {
    let resolver = CountingResolver {
        calls: Cell::new(0),
    };
    let options = HtmlOptions::default();
    let document = build_render_document(
        &[page()],
        &resolver,
        &options,
        RenderSessionId::from_bytes(options.output_id.as_bytes()),
        options.revision,
        None,
        RenderLimits::default(),
    )
    .expect("render document");
    assert_eq!(resolver.calls.get(), 1);

    write_render_document(&document, &options).expect("standalone serialization");
    assert_eq!(resolver.calls.get(), 1);
    let target = build_render_document(
        &[page()],
        &resolver,
        &options,
        document.revision.session_id,
        document.revision.revision + 1,
        Some(&document.revision),
        RenderLimits::default(),
    )
    .expect("next render document");
    assert_eq!(resolver.calls.get(), 2);
    plan_patch(&document.revision, &target.revision, PatchLimits::default()).expect("patch plan");
    assert_eq!(resolver.calls.get(), 2);
}

#[test]
fn render_resource_limit_counts_content_addressed_objects_once() {
    let resolver = Resolver { missing_b: false };
    let options = HtmlOptions::default();
    let mut second = page();
    second.testing_mut().fonts[0].name = "second-binding".to_owned();
    let positioned = [page(), second]
        .iter()
        .enumerate()
        .map(|(index, page)| {
            crate::positioned::lower_page(page, (index + 1) as u32).expect("positioned page")
        })
        .collect::<Vec<_>>();
    let resource_bytes =
        include_bytes!("../../../umber-wasm/assets/cmu-serif-500-roman.woff2").len();
    let document = build_positioned_render_document(
        &positioned,
        &resolver,
        &options,
        RenderSessionId::from_bytes([0x32; 16]),
        1,
        None,
        RenderLimits {
            max_resource_bytes: resource_bytes,
            ..RenderLimits::default()
        },
    )
    .expect("two bindings share one resident resource");
    assert_eq!(document.fonts.len(), 2);
    assert_eq!(document.revision.resources.len(), 1);
}

#[test]
fn unclosed_special_scope_is_rejected() {
    let mut page = page();
    page.testing_mut().effects[0] = PageEffect::Special {
        class: "html".to_owned(),
        payload: b"color push red".to_vec(),
    };
    let resolver = Resolver { missing_b: false };
    assert!(matches!(
        write_html(&[page], &resolver, &HtmlOptions::default()),
        Err(HtmlError::InvalidSpecial { .. })
    ));
}

#[test]
fn canonical_render_revision_is_deterministic_and_reuses_unchanged_keys() {
    let resolver = Resolver { missing_b: false };
    let options = HtmlOptions::default();
    let session = RenderSessionId::from_bytes([9; 16]);
    let first = build_render_document(
        &[page()],
        &resolver,
        &options,
        session,
        1,
        None,
        RenderLimits::default(),
    )
    .expect("first render document")
    .revision;
    let repeated = build_render_document(
        &[page()],
        &resolver,
        &options,
        session,
        2,
        Some(&first),
        RenderLimits::default(),
    )
    .expect("repeated render document")
    .revision;

    assert_eq!(first.pages[0].key, repeated.pages[0].key);
    assert_eq!(first.pages[0].nodes, repeated.pages[0].nodes);
    assert_eq!(first.digest, repeated.digest);
}

#[test]
fn prefix_page_insertion_retains_suffix_page_and_node_identity() {
    let resolver = Resolver { missing_b: false };
    let options = HtmlOptions::default();
    let session = RenderSessionId::from_bytes([10; 16]);
    let first_page = page();
    let mut second_page = page();
    second_page.testing_mut().counts[0] = 2;
    let PageNode::HList(root) = &mut second_page.testing_mut().root else {
        unreachable!()
    };
    let PageNode::Char { ch, .. } = &mut root.children[0] else {
        unreachable!()
    };
    *ch = b'B' as u32;
    let first = build_render_document(
        &[first_page.clone(), second_page.clone()],
        &resolver,
        &options,
        session,
        1,
        None,
        RenderLimits::default(),
    )
    .expect("first render document")
    .revision;
    let mut inserted = page();
    inserted.testing_mut().counts[0] = 99;
    let PageNode::HList(root) = &mut inserted.testing_mut().root else {
        unreachable!()
    };
    root.width = sp(333);
    let next = build_render_document(
        &[inserted, first_page, second_page],
        &resolver,
        &options,
        session,
        2,
        Some(&first),
        RenderLimits::default(),
    )
    .expect("prefixed render document")
    .revision;

    for (old, new) in first.pages.iter().zip(&next.pages[1..]) {
        assert_eq!(old.key, new.key);
        assert_eq!(
            old.nodes.iter().map(|node| node.key).collect::<Vec<_>>(),
            new.nodes.iter().map(|node| node.key).collect::<Vec<_>>()
        );
    }
}

#[test]
fn deterministic_patch_plan_identifies_changed_node() {
    let resolver = Resolver { missing_b: false };
    let options = HtmlOptions::default();
    let session = RenderSessionId::from_bytes([11; 16]);
    let base_page = page();
    let base = build_render_document(
        std::slice::from_ref(&base_page),
        &resolver,
        &options,
        session,
        1,
        None,
        RenderLimits::default(),
    )
    .expect("base render document")
    .revision;
    let mut changed = base_page;
    changed.testing_mut().counts[0] = 7;
    let PageNode::HList(root) = &mut changed.testing_mut().root else {
        unreachable!()
    };
    let PageNode::Char { ch, .. } = &mut root.children[0] else {
        unreachable!()
    };
    *ch = b'B' as u32;
    let target = build_render_document(
        &[changed],
        &resolver,
        &options,
        session,
        2,
        Some(&base),
        RenderLimits::default(),
    )
    .expect("target render document")
    .revision;
    let first = plan_patch(&base, &target, PatchLimits::default()).expect("patch plan");
    let second = plan_patch(&base, &target, PatchLimits::default()).expect("repeat patch plan");

    assert_eq!(first, second);
    assert!(
        first
            .operations
            .iter()
            .any(|operation| matches!(operation, PatchOp::UpdateNode { .. }))
    );
}

#[test]
fn no_op_patch_is_empty() {
    let resolver = Resolver { missing_b: false };
    let options = HtmlOptions::default();
    let session = RenderSessionId::from_bytes([12; 16]);
    let base = build_render_document(
        &[page()],
        &resolver,
        &options,
        session,
        1,
        None,
        RenderLimits::default(),
    )
    .expect("base render document")
    .revision;
    let target = build_render_document(
        &[page()],
        &resolver,
        &options,
        session,
        2,
        Some(&base),
        RenderLimits::default(),
    )
    .expect("target render document")
    .revision;
    let empty = plan_patch(&base, &target, PatchLimits::default()).expect("empty patch");
    assert!(empty.operations.is_empty());
    assert!(empty.resource_additions.is_empty());
    assert!(empty.resource_releases.is_empty());
}

#[test]
fn generated_artifact_edit_sequences_equal_fresh_canonical_renders() {
    let resolver = Resolver { missing_b: false };
    let options = HtmlOptions::default();
    let session = RenderSessionId::from_bytes([14; 16]);
    let mut source_pages = vec![generated_page(1), generated_page(2), generated_page(3)];
    let mut mounted = build_render_document(
        &source_pages,
        &resolver,
        &options,
        session,
        1,
        None,
        RenderLimits::default(),
    )
    .expect("initial generated document")
    .revision;
    let mut random = 0x4d59_5df4_d0f3_3173_u64;

    for revision in 2..=41 {
        random = random
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let choice = (random >> 32) as usize;
        match choice % 5 {
            0 if source_pages.len() < 8 => {
                let index = choice % (source_pages.len() + 1);
                source_pages.insert(index, generated_page((random >> 8) as i32));
            }
            1 if source_pages.len() > 1 => {
                source_pages.remove(choice % source_pages.len());
            }
            2 if source_pages.len() > 1 => {
                let from = choice % source_pages.len();
                let page = source_pages.remove(from);
                let to = ((random >> 16) as usize) % (source_pages.len() + 1);
                source_pages.insert(to, page);
            }
            3 => {
                let index = choice % source_pages.len();
                source_pages[index] = generated_page((random >> 8) as i32);
            }
            _ if source_pages.len() < 8 => {
                let index = choice % source_pages.len();
                let duplicate = source_pages[index].clone();
                source_pages.insert(index, duplicate);
            }
            _ => source_pages.rotate_left(1),
        }

        let target = build_render_document(
            &source_pages,
            &resolver,
            &options,
            session,
            revision,
            Some(&mounted),
            RenderLimits::default(),
        )
        .expect("generated target document")
        .revision;
        let fresh = build_render_document(
            &source_pages,
            &resolver,
            &options,
            session,
            revision,
            None,
            RenderLimits::default(),
        )
        .expect("fresh generated document")
        .revision;
        assert_render_semantics(&target, &fresh, revision);

        plan_patch(&mounted, &target, PatchLimits::default()).expect("generated patch plan");
        mounted = target;
    }
}

fn assert_render_semantics(
    retained: &super::incremental::RenderRevision,
    fresh: &super::incremental::RenderRevision,
    revision: u64,
) {
    assert_eq!(retained.title, fresh.title, "revision {revision}");
    assert_eq!(retained.language, fresh.language, "revision {revision}");
    assert_eq!(retained.resources, fresh.resources, "revision {revision}");
    assert_eq!(
        retained.pages.len(),
        fresh.pages.len(),
        "revision {revision}"
    );
    for (left, right) in retained.pages.iter().zip(&fresh.pages) {
        assert_eq!(left.ordinal, right.ordinal, "revision {revision}");
        assert_eq!(left.width, right.width, "revision {revision}");
        assert_eq!(left.height, right.height, "revision {revision}");
        assert_eq!(left.origin_x, right.origin_x, "revision {revision}");
        assert_eq!(left.origin_y, right.origin_y, "revision {revision}");
        assert_eq!(left.mag, right.mag, "revision {revision}");
        assert_eq!(left.counts, right.counts, "revision {revision}");
        assert_eq!(left.nodes.len(), right.nodes.len(), "revision {revision}");
        for (left, right) in left.nodes.iter().zip(&right.nodes) {
            assert_eq!(left.value, right.value, "revision {revision}");
        }
    }
}

fn generated_page(identity: i32) -> crate::PageArtifact {
    let mut value = page();
    value.testing_mut().counts[0] = identity;
    let PageNode::HList(root) = &mut value.testing_mut().root else {
        unreachable!()
    };
    root.width = sp(200_i32.saturating_add(identity.rem_euclid(1_000)));
    let PageNode::Char { ch, .. } = &mut root.children[0] else {
        unreachable!()
    };
    *ch = u32::from(b'A') + identity.unsigned_abs() % 2;
    value
}

fn page() -> crate::PageArtifact {
    let font = FontResource {
        font_id: 7,
        name: "cmr10".to_owned(),
        tfm_content_hash: tex_fonts::font_content_hash(b"cmr10"),
        tfm_checksum: 123,
        design_size: sp(655_360),
        at_size: sp(655_360),
        layout_policy: tex_fonts::FontLayoutPolicy::ClassicTfmExact,
        mapping_fallback: None,
        opentype: None,
        semantic_identity: tex_fonts::FontSourceIdentity::from_bytes([7; 8]),
        construction: crate::FontResourceConstruction::Loaded,
    };
    UnvalidatedPageArtifact {
        job: JobInfo {
            mag: 1000,
            banner: "test".to_owned(),
            h_offset: sp(17),
            v_offset: sp(13),
            page_origin_x: sp(0),
            page_origin_y: sp(0),
            page_width: sp(0),
            page_height: sp(0),
        },
        fonts: vec![font],
        counts: [0; 10],
        root: PageNode::HList(BoxNode {
            width: sp(200),
            height: sp(40),
            depth: sp(5),
            shift: sp(0),
            glue_set: GlueSetRatio::ZERO,
            glue_sign: GlueSign::Normal,
            glue_order: GlueOrder::Normal,
            children: vec![
                PageNode::Char {
                    font_id: 7,
                    ch: b'A' as u32,
                    width: sp(30),
                },
                PageNode::Char {
                    font_id: 7,
                    ch: b'B' as u32,
                    width: sp(30),
                },
                PageNode::WhatsitAnchor { effect_index: 0 },
            ],
        }),
        effects: vec![PageEffect::Special {
            class: "dvi".to_owned(),
            payload: b"<script>alert(1)</script>".to_vec(),
        }],
        math_events: Vec::new(),
    }
    .validate()
    .expect("valid page")
}
