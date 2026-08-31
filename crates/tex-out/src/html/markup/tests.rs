use sha2::{Digest, Sha256};
use tex_arith::Scaled;

use super::*;
use crate::html::incremental::{
    RENDER_SCHEMA_VERSION, RenderBox, RenderDigest, RenderDocument, RenderFont, RenderKey,
    RenderMathDrawing, RenderMathGlyph, RenderNode, RenderNodeValue, RenderPage, RenderResource,
    RenderRevision, RenderRule, RenderSessionId, RenderSpecial, RenderSpecialAction, RenderText,
};
use crate::html::{AssetMode, HtmlFontKey, HtmlOptions, RenderedOutputId, write_render_document};
use crate::positioned::{BoxKind, TextUnit};
use crate::{MathGlyph, MathGlyphSelection, MathRule, MathStart};

#[global_allocator]
static ALLOCATOR: tex_state_profiling_allocator::HotCoreAllocator =
    tex_state_profiling_allocator::HotCoreAllocator;

const ZERO_DIGEST: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn sp(raw: i32) -> Scaled {
    Scaled::from_raw(raw)
}

fn font_key() -> HtmlFontKey {
    HtmlFontKey {
        name: "mixed-font".to_owned(),
        tfm_content_hash: [0x22; 8],
        tfm_checksum: 0x1234_5678,
        design_size_raw: 655_360,
        at_size_raw: 655_360,
        opentype_program_identity: None,
        opentype_instance_identity: None,
    }
}

fn node(event_ordinal: u32, value: RenderNodeValue) -> RenderNode {
    let digest = RenderDigest::parse_hex(ZERO_DIGEST).expect("zero digest");
    RenderNode {
        key: RenderKey::ROOT,
        digest,
        match_digest: digest,
        event_ordinal,
        value,
    }
}

fn text_node(event_ordinal: u32, x: i32, line: Option<u32>, text: &str) -> RenderNode {
    node(
        event_ordinal,
        RenderNodeValue::Text(Box::new(RenderText {
            font_id: 7,
            face_index: None,
            mapped_encoding: true,
            exact_character_positions: true,
            x: sp(x),
            baseline: sp(-x),
            positions: vec![sp(x), sp(-x)],
            units: vec![TextUnit::Code(0), TextUnit::Code(u32::MAX), TextUnit::Space],
            text: text.to_owned(),
            font: font_key(),
            family: "umber-font-mixed".to_owned(),
            resource: [0x42; 8],
            direction: incremental::RenderDirection::LeftToRight,
            script: None,
            language: None,
            features: Vec::new(),
            variations: Vec::new(),
            color: None,
            link: Some("https://example.test/path?a=1&b=2".to_owned()),
            accessibility_line: line,
        })),
    )
}

fn mixed_document() -> RenderDocument {
    let digest = RenderDigest::parse_hex(ZERO_DIGEST).expect("zero digest");
    let first = RenderPage {
        key: RenderKey::ROOT,
        digest,
        match_digest: digest,
        ordinal: 1,
        width: sp(17),
        height: sp(-17),
        origin_x: sp(-17),
        origin_y: sp(17),
        mag: 1_000,
        counts: [1, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        nodes: vec![
            node(
                0,
                RenderNodeValue::Box(RenderBox {
                    id: 1,
                    kind: BoxKind::Horizontal,
                    x: sp(-17),
                    y: sp(17),
                    width: sp(65_536),
                    height: sp(-1),
                    baseline: sp(123_456),
                }),
            ),
            node(
                1,
                RenderNodeValue::Rule(RenderRule {
                    x: sp(17),
                    y: sp(-17),
                    width: sp(32_768),
                    height: sp(1),
                    color: Some("#123abc".to_owned()),
                }),
            ),
            text_node(2, 17, Some(7), "A<&\" mapped"),
            node(
                3,
                RenderNodeValue::Special(RenderSpecial {
                    x: sp(-17),
                    y: sp(17),
                    class: "html&meta".to_owned(),
                    payload: vec![0, 0xab, 0xff],
                    action: RenderSpecialAction::Destination("umber-dest-mixed.1".to_owned()),
                }),
            ),
            node(
                4,
                RenderNodeValue::MathStart(MathStart {
                    id: 99,
                    x: sp(-17),
                    baseline: sp(17),
                    width: sp(65_536),
                    height: sp(32_768),
                    depth: sp(16_384),
                }),
            ),
            node(
                5,
                RenderNodeValue::MathGlyph(Box::new(RenderMathGlyph {
                    glyph: MathGlyph {
                        font_instance: tex_fonts::FontInstanceIdentity::from_bytes([0x33; 8]),
                        glyph_id: 42,
                        selection: MathGlyphSelection::Cmap {
                            scalar: u32::from('<'),
                        },
                        ssty: 2,
                        x: sp(17),
                        baseline: sp(-17),
                        width: sp(4),
                        height: sp(5),
                        depth: sp(6),
                    },
                    drawing: RenderMathDrawing::Text {
                        scalar: '<',
                        family: "umber-font-mixed".to_owned(),
                        font_size_raw: 655_360,
                        variations: vec![(*b"wght", 98_304)],
                    },
                })),
            ),
            node(
                6,
                RenderNodeValue::MathRule(MathRule {
                    x: sp(-17),
                    y: sp(17),
                    width: sp(8),
                    height: sp(9),
                }),
            ),
            node(7, RenderNodeValue::MathEnd),
        ],
    };
    let second = RenderPage {
        key: RenderKey::ROOT,
        digest,
        match_digest: digest,
        ordinal: 2,
        width: sp(9_472_806),
        height: sp(9_472_643),
        origin_x: sp(4_736_286),
        origin_y: sp(4_736_286),
        mag: 1_200,
        counts: [2, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        nodes: vec![
            node(
                0,
                RenderNodeValue::Text(Box::new(RenderText {
                    font_id: u32::MAX,
                    face_index: Some(12),
                    mapped_encoding: false,
                    exact_character_positions: false,
                    x: sp(-2_147_483_648),
                    baseline: sp(2_147_483_647),
                    positions: Vec::new(),
                    units: vec![TextUnit::Code(u32::from('&')), TextUnit::Space],
                    text: "RTL > & accessibility".to_owned(),
                    font: font_key(),
                    family: "umber-font-mixed".to_owned(),
                    resource: [0x42; 8],
                    direction: incremental::RenderDirection::RightToLeft,
                    script: Some(*b"arab"),
                    language: Some("ar".to_owned()),
                    features: vec![(*b"salt", u32::MAX)],
                    variations: vec![(*b"wght", -32_768)],
                    color: Some("blue".to_owned()),
                    link: None,
                    accessibility_line: None,
                })),
            ),
            node(
                1,
                RenderNodeValue::Special(RenderSpecial {
                    x: sp(0),
                    y: sp(0),
                    class: "diagnostic".to_owned(),
                    payload: b"<&".to_vec(),
                    action: RenderSpecialAction::Inert,
                }),
            ),
        ],
    };
    RenderDocument {
        revision: RenderRevision {
            schema_version: RENDER_SCHEMA_VERSION,
            session_id: RenderSessionId::from_bytes([0x31; 16]),
            revision: u64::MAX,
            title: "Mixed <&> \"golden\"".to_owned(),
            language: "en-US".to_owned(),
            pages: vec![first, second],
            resources: vec![RenderResource {
                identity: [0x42; 8],
                bytes: vec![0, 0xff, b'&'],
                family: "umber-font-mixed".to_owned(),
                provenance: "mixed golden".to_owned(),
            }],
            digest,
        },
        fonts: vec![RenderFont {
            key: font_key(),
            identity: [0x42; 8],
            digest_hex: "4242424242424242".to_owned(),
            family: "umber-font-mixed".to_owned(),
        }],
    }
}

#[test]
fn mixed_multi_page_markup_has_exact_embedded_and_manifest_goldens() {
    let document = mixed_document();
    let embedded_options = HtmlOptions {
        output_id: RenderedOutputId::from_bytes([0x31; 16]),
        revision: u64::MAX,
        ..HtmlOptions::default()
    };
    let embedded = write_render_document(&document, &embedded_options).expect("embedded golden");
    let repeated = write_render_document(&document, &embedded_options).expect("repeat golden");
    assert_eq!(embedded, repeated);
    assert!(embedded.assets.is_empty());
    let embedded_hash = hex(&Sha256::digest(&embedded.html));
    assert_eq!(
        embedded_hash,
        "dcaa6fc943cf217524fab69a4e943f774aa8ee5862ac01461c456a9e0e503ac2"
    );

    let html = std::str::from_utf8(&embedded.html).expect("golden is UTF-8");
    assert!(html.contains("width:0.00034457px;height:-0.00034457px"));
    assert!(html.contains("A&lt;&amp;\" mapped"));
    assert!(html.contains("data-umber-special-hex=\"00abff\""));
    assert!(html.contains("font-variation-settings:'wght' 1.5"));
    assert!(html.contains(">&lt;</text>"));
    assert!(html.contains("aria-label=\"Page 2\"><p class=\"umber-a11y-line\">"));

    let manifest_options = HtmlOptions {
        asset_mode: AssetMode::Manifest {
            relative_directory: "fonts/golden".to_owned(),
        },
        ..embedded_options.clone()
    };
    let manifest = write_render_document(&document, &manifest_options).expect("manifest golden");
    assert_eq!(manifest.assets.len(), 1);
    assert_eq!(manifest.assets[0].bytes, [0, 0xff, b'&']);
    assert_eq!(
        hex(&Sha256::digest(&manifest.html)),
        "b2414da713a1f2313f6df454d401ac1c6873a63a5a386f76b0d4afabb8cea518"
    );

    let mut html_limited = embedded_options.clone();
    html_limited.max_html_bytes = embedded.html.len() - 1;
    assert!(matches!(
        write_render_document(&document, &html_limited),
        Err(HtmlError::HtmlTooLarge { limit, .. }) if limit == html_limited.max_html_bytes
    ));
    let mut asset_limited = manifest_options;
    asset_limited.max_total_asset_bytes = 2;
    assert_eq!(
        write_render_document(&document, &asset_limited),
        Err(HtmlError::AssetsTooLarge { bytes: 3, limit: 2 })
    );
}

#[test]
fn warmed_markup_page_emission_allocates_zero_bytes() {
    let digest = RenderDigest::parse_hex(ZERO_DIGEST).expect("zero digest");
    let text = "direct accessibility <&> without a page-sized staging string";
    let page = RenderPage {
        key: RenderKey::ROOT,
        digest,
        match_digest: digest,
        ordinal: u32::MAX,
        width: sp(i32::MAX),
        height: sp(i32::MIN),
        origin_x: sp(-17),
        origin_y: sp(17),
        mag: i32::MAX,
        counts: [0; 10],
        nodes: (0..1_024)
            .map(|ordinal| {
                text_node(
                    ordinal,
                    i32::try_from(ordinal).expect("test ordinal fits in i32") - 512,
                    Some(ordinal / 32),
                    text,
                )
            })
            .collect(),
    };
    let options = HtmlOptions {
        revision: u64::MAX,
        output_id: RenderedOutputId::from_bytes([0xff; 16]),
        max_html_bytes: usize::MAX,
        ..HtmlOptions::default()
    };
    let mut out = String::with_capacity(2 * 1024 * 1024);
    let initial_capacity = out.capacity();

    const OWNER: usize = 0;
    let before = tex_state_profiling_allocator::thread_measurement(OWNER);
    {
        let _scope = tex_state_profiling_allocator::scope(OWNER);
        write_render_page(&mut out, &page, &options).expect("bounded page markup");
    }
    let after = tex_state_profiling_allocator::thread_measurement(OWNER);

    assert_eq!(out.capacity(), initial_capacity);
    assert_eq!(after.calls, before.calls);
    assert_eq!(after.requested_bytes, before.requested_bytes);
    assert_eq!(out.matches("<svg class=\"umber-run\"").count(), 1_024);
    assert_eq!(out.matches("<p class=\"umber-a11y-line\">").count(), 32);
    assert!(out.contains("data-umber-codes=\"0x0,0xffffffff,space\""));
}
