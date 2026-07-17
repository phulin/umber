use sha2::{Digest, Sha256};
use tex_arith::Scaled;

use crate::{
    BoxNode, ContentHash, FontResource, GlueOrder, GlueSetRatio, GlueSign, JobInfo, MathGlyph,
    MathGlyphSelection, MathOutputEvent, MathRule, MathStart, OpenTypeFontResource, PageEffect,
    PageNode, UnvalidatedPageArtifact,
};

use super::{
    AssetMode, HtmlError, HtmlFontKey, HtmlFontResolver, HtmlOptions, RenderedOutputId, WebFont,
    write_html, write_positioned_html,
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
        instance_identity: tex_fonts::FontInstanceIdentity::from_bytes([8; 32]),
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
    let mut resolver = Resolver { missing_b: false };
    let output = write_html(&[page.clone(), page], &mut resolver, &options).expect("manifest HTML");
    assert_eq!(output.assets.len(), 1);
    assert!(output.assets[0].path.starts_with("sha256-"));
    let html = String::from_utf8(output.html).expect("UTF-8 HTML");
    assert!(html.contains(&format!(
        "umber-font-{}",
        &super::hex(&program.bytes())[..24]
    )));
    assert!(html.contains("fonts/sha256-"));
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
            declared_object_sha256: None,
            declared_program_identity: None,
            provenance: None,
        },
        tex_fonts::FontLimits::default(),
    )
    .expect("validated fixture font")
}

struct Resolver {
    missing_b: bool,
}

enum BrokenFont {
    Container,
    Cmap,
}

impl HtmlFontResolver for BrokenFont {
    fn resolve(&mut self, font: &FontResource) -> Result<WebFont, String> {
        let mut web = Resolver { missing_b: false }.resolve(font)?;
        match self {
            Self::Container => web.woff2 = b"wOF2not-a-font".to_vec(),
            Self::Cmap => web.encoding[usize::from(b'A')] = Some("\u{10ffff}".to_owned()),
        }
        web.sha256 = Sha256::digest(&web.woff2).into();
        Ok(web)
    }
}

impl HtmlFontResolver for Resolver {
    fn resolve(&mut self, font: &FontResource) -> Result<WebFont, String> {
        let bytes = include_bytes!("../../../umber-wasm/assets/cmu-serif-500-roman.woff2").to_vec();
        let mut encoding = vec![None; 256];
        encoding[usize::from(b'A')] = Some("A".to_owned());
        if !self.missing_b {
            encoding[usize::from(b'B')] = Some("<&B".to_owned());
        }
        Ok(WebFont {
            key: HtmlFontKey::from(font),
            sha256: Sha256::digest(&bytes).into(),
            woff2: bytes,
            encoding,
            provenance: "test fixture".to_owned(),
            embeddable: true,
        })
    }
}

struct SingleScalarResolver;

impl HtmlFontResolver for SingleScalarResolver {
    fn resolve(&mut self, font: &FontResource) -> Result<WebFont, String> {
        let mut web = Resolver { missing_b: false }.resolve(font)?;
        web.encoding[usize::from(b'B')] = Some("B".to_owned());
        Ok(web)
    }
}

struct MathResolver;

impl HtmlFontResolver for MathResolver {
    fn resolve(&mut self, font: &FontResource) -> Result<WebFont, String> {
        let bytes =
            include_bytes!("../../../tex-fonts/tests/fixtures/stix-two-math.woff2").to_vec();
        Ok(WebFont {
            key: HtmlFontKey::from(font),
            sha256: Sha256::digest(&bytes).into(),
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
    let instance = tex_fonts::FontInstanceIdentity::from_bytes([0x5a; 32]);
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
    let outline_glyph = parsed
        .math
        .as_ref()
        .and_then(|math| math.variants.as_ref())
        .and_then(|variants| {
            variants
                .vertical
                .values()
                .chain(variants.horizontal.values())
                .find_map(|construction| {
                    construction
                        .assembly
                        .as_ref()
                        .and_then(|assembly| assembly.parts.first())
                        .map(|part| part.glyph_id)
                        .or_else(|| {
                            construction
                                .variants
                                .first()
                                .map(|variant| variant.glyph_id)
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
    let output = write_html(&[page], &mut MathResolver, &HtmlOptions::default())
        .expect("positioned math HTML");
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
    let instance = tex_fonts::FontInstanceIdentity::from_bytes([0x33; 32]);
    let mut page = page();
    let PageNode::HList(root) = &mut page.testing_mut().root else {
        unreachable!()
    };
    root.children.clear();
    page.testing_mut().fonts[0].name = "stix-two-math".to_owned();
    page.testing_mut().fonts[0].opentype = Some(OpenTypeFontResource {
        program_identity: tex_fonts::FontProgramIdentity::from_bytes([0xff; 32]),
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
        write_html(&[page.clone()], &mut MathResolver, &HtmlOptions::default()),
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
        write_html(&[page], &mut MathResolver, &HtmlOptions::default()),
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
    let mut first_resolver = Resolver { missing_b: false };
    let first =
        write_html(std::slice::from_ref(&page), &mut first_resolver, &options).expect("first HTML");
    let mut second_resolver = Resolver { missing_b: false };
    let second = write_html(&[page], &mut second_resolver, &options).expect("second HTML");
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
    let output = write_html(
        &[page()],
        &mut SingleScalarResolver,
        &HtmlOptions::default(),
    )
    .expect("positioned HTML");
    let html = String::from_utf8(output.html).expect("UTF-8 HTML");

    assert!(html.contains("x=\"0.00034457px 0.00095265px\""), "{html}");
}

#[test]
fn configured_physical_dimensions_build_the_page_box() {
    let mut page = page();
    page.testing_mut().job.page_width = sp(1_000);
    page.testing_mut().job.page_height = sp(2_000);
    let mut resolver = Resolver { missing_b: false };

    let output =
        write_html(&[page], &mut resolver, &HtmlOptions::default()).expect("physical page HTML");
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
    let mut resolver = Resolver { missing_b: false };

    let output =
        write_html(&[page], &mut resolver, &HtmlOptions::default()).expect("plain TeX page HTML");
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
    let mut resolver = Resolver { missing_b: true };
    let error =
        write_html(&[page()], &mut resolver, &HtmlOptions::default()).expect_err("mapping failure");
    assert_eq!(
        error,
        HtmlError::MissingTextMapping {
            font: "cmr10".to_owned(),
            code: b'B'
        }
    );
}

#[test]
fn invalid_woff2_and_uncovered_mappings_fail_before_serialization() {
    assert!(matches!(
        write_html(
            &[page()],
            &mut BrokenFont::Container,
            &HtmlOptions::default()
        ),
        Err(HtmlError::CorruptFontAsset { .. })
    ));
    assert!(matches!(
        write_html(&[page()], &mut BrokenFont::Cmap, &HtmlOptions::default()),
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
    let mut resolver = Resolver { missing_b: false };
    let output = write_html(&[page], &mut resolver, &HtmlOptions::default()).expect("special HTML");
    let html = String::from_utf8(output.html).expect("UTF-8");
    assert!(html.contains("<svg class=\"umber-run\""));
    assert!(
        html.contains(";color:red\"><rect class=\"umber-baseline\"")
            && html.contains("<a href=\"https://example.test/path?a=1&amp;b=2\""),
        "{html}"
    );
    assert!(html.contains("id=\"umber-dest-section.1\""));
}

#[test]
fn dangerous_link_special_fails_without_markup_injection() {
    let mut page = page();
    page.testing_mut().effects[0] = PageEffect::Special {
        class: "html".to_owned(),
        payload: b"link javascript:alert(1)".to_vec(),
    };
    let mut resolver = Resolver { missing_b: false };
    assert!(matches!(
        write_html(&[page], &mut resolver, &HtmlOptions::default()),
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
    let mut resolver = Resolver { missing_b: false };
    assert_eq!(
        write_positioned_html(
            &[positioned.clone(), positioned.clone()],
            &mut resolver,
            &options
        )
        .expect_err("page limit"),
        HtmlError::TooManyPages { count: 2, limit: 1 }
    );
    options.max_positioned_events = 0;
    let mut resolver = Resolver { missing_b: false };
    assert!(matches!(
        write_positioned_html(std::slice::from_ref(&positioned), &mut resolver, &options),
        Err(HtmlError::Positioned(
            crate::positioned::PositionedError::TooManyEvents { limit: 0 }
        ))
    ));
    options.max_positioned_events = usize::MAX;
    options.max_text_run_units = 1;
    let mut resolver = Resolver { missing_b: false };
    assert!(matches!(
        write_positioned_html(std::slice::from_ref(&positioned), &mut resolver, &options),
        Err(HtmlError::Positioned(
            crate::positioned::PositionedError::TextRunTooLong { limit: 1 }
        ))
    ));
    options.max_text_run_units = usize::MAX;
    options.max_total_asset_bytes = 3;
    let mut resolver = Resolver { missing_b: false };
    assert!(matches!(
        write_positioned_html(std::slice::from_ref(&positioned), &mut resolver, &options),
        Err(HtmlError::AssetsTooLarge { .. })
    ));
    options.max_total_asset_bytes = usize::MAX;
    options.max_html_bytes = 64;
    let mut resolver = Resolver { missing_b: false };
    assert!(matches!(
        write_positioned_html(&[positioned], &mut resolver, &options),
        Err(HtmlError::HtmlTooLarge { .. })
    ));
}

#[test]
fn unclosed_special_scope_is_rejected() {
    let mut page = page();
    page.testing_mut().effects[0] = PageEffect::Special {
        class: "html".to_owned(),
        payload: b"color push red".to_vec(),
    };
    let mut resolver = Resolver { missing_b: false };
    assert!(matches!(
        write_html(&[page], &mut resolver, &HtmlOptions::default()),
        Err(HtmlError::InvalidSpecial { .. })
    ));
}

fn page() -> crate::PageArtifact {
    let font = FontResource {
        font_id: 7,
        name: "cmr10".to_owned(),
        tfm_content_hash: ContentHash::from_bytes(b"cmr10"),
        tfm_checksum: 123,
        design_size: sp(655_360),
        at_size: sp(655_360),
        layout_policy: tex_fonts::FontLayoutPolicy::ClassicTfmExact,
        mapping_fallback: None,
        opentype: None,
        semantic_identity: tex_fonts::FontSourceIdentity::from_bytes([7; 32]),
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
