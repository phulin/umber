use super::*;
use tex_fonts::{
    AcceptedFontContainers, FeatureSetting, FontContainer, FontLimits, FontMetrics,
    FontObjectIdentity, FontPurposes, FontRequest, FontRequestKey, LoadedFont, OpenTypeFont,
    OpenTypeProgramSelection, ResolvedFont, VariationSelection, WritingDirection,
};

const CMU_SERIF: &[u8] = include_bytes!("../../umber-wasm/assets/cmu-serif-500-roman.woff2");
const NOTO_SANS_ARABIC: &[u8] = include_bytes!("../tests/fixtures/NotoSansArabic.ttf");
const NOTO_SANS_DEVANAGARI: &[u8] = include_bytes!("../tests/fixtures/NotoSansDevanagari.ttf");

fn cmu_serif(features: FontFeaturePolicy) -> LoadedFont {
    loaded_font(
        "cmu-serif",
        CMU_SERIF,
        FontContainer::Woff2,
        AcceptedFontContainers::WASM,
        features,
    )
}

fn loaded_font(
    name: &str,
    bytes: &[u8],
    container: FontContainer,
    accepted_containers: AcceptedFontContainers,
    features: FontFeaturePolicy,
) -> LoadedFont {
    let key = FontRequestKey::new(name, 0, VariationSelection::default(), features.clone())
        .expect("fixture request key");
    let request = FontRequest {
        key: key.clone(),
        accepted_containers,
        purposes: FontPurposes::LAYOUT,
    };
    let font = OpenTypeFont::parse(
        &request,
        ResolvedFont {
            request: key,
            container,
            declared_object_sha256: Some(FontObjectIdentity::for_bytes(bytes)),
            declared_program_identity: None,
            provenance: Some("committed SIL Open Font License 1.1 fixture".to_owned()),
            bytes: bytes.to_vec(),
        },
        FontLimits::default(),
    )
    .expect("validated fixture font");
    let size = Scaled::from_raw(10 * Scaled::UNITY);
    LoadedFont::new(
        name,
        name,
        [0; 32],
        0,
        size,
        size,
        vec![Scaled::from_raw(0); 7],
        FontMetrics::new(Vec::new(), Vec::new(), None, None, Vec::new()),
    )
    .with_opentype(OpenTypeProgramSelection {
        font,
        variation: VariationSelection::default(),
        features,
        direction: WritingDirection::LeftToRight,
    })
}

#[test]
fn script_detection_skips_common_prefixes() {
    assert_eq!(run_script("(Hello)"), Script::Latin);
    assert_eq!(run_script("123"), Script::Common);
    assert_eq!(run_script("(مرحبا)"), Script::Arabic);
    assert_eq!(Direction::from_text("(Hello)"), Direction::LeftToRight);
    assert_eq!(Direction::from_text("123 مرحبا"), Direction::RightToLeft);
}

#[test]
fn cmu_serif_ligatures_and_mark_attachment_match_fixture() {
    let features = FontFeaturePolicy::default();
    let font = cmu_serif(features.clone());
    let ligature = shape_run(
        font.shaping_font().expect("OpenType fixture"),
        "office",
        &features,
        Direction::LeftToRight,
    );
    assert_eq!(
        ligature.glyphs,
        vec![
            glyph(82, 0, 327_680, 0),
            glyph(2236, 1, 545_915, 0),
            glyph(70, 4, 290_980, 0),
            glyph(72, 5, 290_980, 0),
        ]
    );

    let mark = shape_run(
        font.shaping_font().expect("OpenType fixture"),
        "x\u{0301}",
        &features,
        Direction::LeftToRight,
    );
    assert_eq!(
        mark.glyphs,
        vec![glyph(91, 0, 345_375, 0), glyph(685, 0, 0, -45_220)]
    );
}

#[test]
fn complex_script_fixtures_match_glyph_and_position_snapshots() {
    let features = FontFeaturePolicy::default();
    for (name, bytes, text, direction) in [
        ("noto-arabic", NOTO_SANS_ARABIC, "لَا", Direction::RightToLeft),
        (
            "noto-devanagari",
            NOTO_SANS_DEVANAGARI,
            "क्षि",
            Direction::LeftToRight,
        ),
    ] {
        let font = loaded_font(
            name,
            bytes,
            FontContainer::TrueType,
            AcceptedFontContainers::NATIVE,
            features.clone(),
        );
        let shaped = shape_run(
            font.shaping_font().expect("OpenType fixture"),
            text,
            &features,
            direction,
        );
        let expected = match name {
            "noto-arabic" => vec![
                glyph_full(10, 4, 237_896, 0, 0),
                glyph_full(371, 0, 0, -74_711, 167_772),
                glyph_full(73, 0, 143_524, 0, 0),
            ],
            "noto-devanagari" => vec![
                glyph_full(551, 0, 169_738, 0, 0),
                glyph_full(90, 0, 469_893, 0, 0),
            ],
            _ => unreachable!("known fixture"),
        };
        assert_eq!(shaped.glyphs, expected, "{name}");
        assert!(
            shaped
                .glyphs
                .iter()
                .all(|glyph| text.is_char_boundary(glyph.cluster as usize)),
            "{name} clusters are source UTF-8 boundaries"
        );
    }
}

#[test]
fn feature_policy_can_disable_ligatures() {
    let features = FontFeaturePolicy::new(vec![FeatureSetting {
        tag: OpenTypeTag::new(*b"liga"),
        enabled: false,
    }])
    .expect("feature policy");
    let font = cmu_serif(features.clone());
    let shaped = shape_run(
        font.shaping_font().expect("OpenType fixture"),
        "office",
        &features,
        Direction::LeftToRight,
    );
    assert_eq!(
        shaped
            .glyphs
            .iter()
            .map(|glyph| glyph.cluster)
            .collect::<Vec<_>>(),
        vec![0, 1, 2, 3, 4, 5]
    );
}

fn glyph(glyph_id: u32, cluster: u32, x_advance: i32, x_offset: i32) -> ShapedGlyph {
    glyph_full(glyph_id, cluster, x_advance, x_offset, 0)
}

fn glyph_full(
    glyph_id: u32,
    cluster: u32,
    x_advance: i32,
    x_offset: i32,
    y_offset: i32,
) -> ShapedGlyph {
    ShapedGlyph {
        glyph_id,
        cluster,
        x_advance: Scaled::from_raw(x_advance),
        y_advance: Scaled::from_raw(0),
        x_offset: Scaled::from_raw(x_offset),
        y_offset: Scaled::from_raw(y_offset),
    }
}
