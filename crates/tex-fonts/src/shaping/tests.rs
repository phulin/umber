use super::*;
use crate::{
    AcceptedFontContainers, FeatureSetting, FontContainer, FontFeaturePolicy, FontLanguage,
    FontLimits, FontMetrics, FontObjectIdentity, FontPurposes, FontRequest, FontRequestKey,
    LoadedFont, OpenTypeFont, ResolvedFont, VariationSelection, WritingDirection,
};

const CMU_SERIF: &[u8] = include_bytes!("../../../umber-wasm/assets/cmu-serif-500-roman.woff2");
const NOTO_SANS_ARABIC: &[u8] = include_bytes!("../../tests/fixtures/shaping/NotoSansArabic.ttf");
const NOTO_SANS_DEVANAGARI: &[u8] =
    include_bytes!("../../tests/fixtures/shaping/NotoSansDevanagari.ttf");

struct TestShapingContext {
    direction: WritingDirection,
    script: Option<OpenTypeTag>,
    language: Option<FontLanguage>,
}

impl Default for TestShapingContext {
    fn default() -> Self {
        Self {
            direction: WritingDirection::LeftToRight,
            script: None,
            language: None,
        }
    }
}

fn cmu_serif(features: FontFeaturePolicy) -> LoadedFont {
    loaded_font(
        "cmu-serif",
        CMU_SERIF,
        FontContainer::Woff2,
        AcceptedFontContainers::WASM,
        features,
        TestShapingContext::default(),
    )
}

fn loaded_font(
    name: &str,
    bytes: &[u8],
    container: FontContainer,
    accepted_containers: AcceptedFontContainers,
    features: FontFeaturePolicy,
    context: TestShapingContext,
) -> LoadedFont {
    let key = FontRequestKey::new(name, 0, VariationSelection::default(), features.clone())
        .expect("fixture request key")
        .with_shaping_context(context.direction, context.script, context.language)
        .expect("fixture shaping context");
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
            declared_object_ahash64: Some(FontObjectIdentity::for_bytes(bytes)),
            declared_program_identity: None,
            provenance: Some("committed SIL Open Font License 1.1 fixture".to_owned()),
            legacy_mapping: None,
            bytes: bytes.to_vec(),
        },
        FontLimits::default(),
    )
    .expect("validated fixture font");
    let size = Scaled::from_raw(10 * Scaled::UNITY);
    LoadedFont::new(
        name,
        name,
        [0; 8],
        0,
        size,
        size,
        vec![Scaled::from_raw(0); 7],
        FontMetrics::new(Vec::new(), Vec::new(), None, None, Vec::new()),
    )
    .with_opentype(font)
}

#[test]
fn script_detection_skips_common_prefixes() {
    assert_eq!(run_script("(Hello)"), Script::Latin);
    assert_eq!(run_script("123"), Script::Common);
    assert_eq!(run_script("(مرحبا)"), Script::Arabic);
    assert_eq!(text_direction("(Hello)"), WritingDirection::LeftToRight);
    assert_eq!(text_direction("123 مرحبا"), WritingDirection::RightToLeft);
}

#[test]
fn cmu_serif_ligatures_and_mark_attachment_match_fixture() {
    let features = FontFeaturePolicy::default();
    let font = cmu_serif(features.clone());
    let ligature = font
        .shape_run(ShapingRequest::new("office"))
        .expect("OpenType fixture");
    assert_eq!(
        ligature.glyphs,
        vec![
            glyph(82, 0, 327_680, 0),
            glyph(2236, 1, 545_915, 0),
            glyph(70, 4, 290_980, 0),
            glyph(72, 5, 290_980, 0),
        ]
    );

    let mark = font
        .shape_run(ShapingRequest::new("x\u{0301}"))
        .expect("OpenType fixture");
    assert_eq!(
        mark.glyphs,
        vec![glyph(91, 0, 345_375, 0), glyph(685, 0, 0, -45_220)]
    );
}

#[test]
fn callback_shaping_reuses_feature_and_rustybuzz_storage() {
    let font = cmu_serif(FontFeaturePolicy::default());
    let mut scratch = ShapingScratch::default();
    let mut first = Vec::new();
    let metadata = font
        .shape_run_with_scratch(
            ShapingRequest::with_breaks("office", &[2]),
            &mut scratch,
            |glyph| first.push(glyph),
        )
        .expect("OpenType fixture");
    assert_eq!(metadata.direction, WritingDirection::LeftToRight);
    assert_eq!(metadata.script, Script::Latin);
    assert!(!first.is_empty());
    assert!(scratch.input.as_ref().is_some_and(|input| input.is_empty()));
    let feature_capacity = scratch.features.capacity();

    let mut second_count = 0;
    font.shape_run_with_scratch(ShapingRequest::new("office"), &mut scratch, |_| {
        second_count += 1
    })
    .expect("OpenType fixture");
    assert_eq!(second_count, 4);
    assert_eq!(scratch.features.capacity(), feature_capacity);
    assert!(scratch.input.as_ref().is_some_and(|input| input.is_empty()));
}

#[test]
fn complex_script_fixtures_match_glyph_and_position_snapshots() {
    let features = FontFeaturePolicy::default();
    for (name, bytes, text, direction) in [
        (
            "noto-arabic",
            NOTO_SANS_ARABIC,
            "لَا",
            WritingDirection::RightToLeft,
        ),
        (
            "noto-devanagari",
            NOTO_SANS_DEVANAGARI,
            "क्षि",
            WritingDirection::LeftToRight,
        ),
    ] {
        let font = loaded_font(
            name,
            bytes,
            FontContainer::TrueType,
            AcceptedFontContainers::NATIVE,
            features.clone(),
            TestShapingContext {
                direction,
                ..TestShapingContext::default()
            },
        );
        let shaped = font
            .shape_run(ShapingRequest::new(text))
            .expect("OpenType fixture");
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
fn explicit_script_language_and_mark_policy_reach_rustybuzz() {
    let enabled = FontFeaturePolicy::new(vec![
        FeatureSetting {
            tag: OpenTypeTag::new(*b"mark"),
            value: 1,
        },
        FeatureSetting {
            tag: OpenTypeTag::new(*b"mkmk"),
            value: 1,
        },
    ])
    .expect("enabled marks");
    let disabled = FontFeaturePolicy::new(vec![
        FeatureSetting {
            tag: OpenTypeTag::new(*b"mark"),
            value: 0,
        },
        FeatureSetting {
            tag: OpenTypeTag::new(*b"mkmk"),
            value: 0,
        },
    ])
    .expect("disabled marks");
    let font = loaded_font(
        "noto-arabic",
        NOTO_SANS_ARABIC,
        FontContainer::TrueType,
        AcceptedFontContainers::NATIVE,
        enabled.clone(),
        TestShapingContext {
            direction: WritingDirection::RightToLeft,
            script: Some(OpenTypeTag::new(*b"arab")),
            language: Some(FontLanguage::new("ar").expect("language")),
        },
    );
    let positioned = font
        .shape_run(ShapingRequest::new("لَا"))
        .expect("OpenType fixture");
    let unpositioned = loaded_font(
        "noto-arabic",
        NOTO_SANS_ARABIC,
        FontContainer::TrueType,
        AcceptedFontContainers::NATIVE,
        disabled,
        TestShapingContext {
            direction: WritingDirection::RightToLeft,
            script: Some(OpenTypeTag::new(*b"arab")),
            language: Some(FontLanguage::new("ar").expect("language")),
        },
    )
    .shape_run(ShapingRequest::new("لَا"))
    .expect("OpenType fixture");
    assert_eq!(positioned.glyphs[1].x_offset.raw(), -74_711);
    assert_eq!(unpositioned.glyphs[1].x_offset.raw(), 0);
    assert_eq!(positioned.glyphs[1].y_offset.raw(), 167_772);
    assert_eq!(unpositioned.glyphs[1].y_offset.raw(), 0);
}

#[test]
fn feature_policy_can_disable_ligatures() {
    let features = FontFeaturePolicy::new(vec![FeatureSetting {
        tag: OpenTypeTag::new(*b"liga"),
        value: 0,
    }])
    .expect("feature policy");
    let font = cmu_serif(features.clone());
    let shaped = font
        .shape_run(ShapingRequest::new("office"))
        .expect("OpenType fixture");
    assert_eq!(
        shaped
            .glyphs
            .iter()
            .map(|glyph| glyph.cluster)
            .collect::<Vec<_>>(),
        vec![0, 1, 2, 3, 4, 5]
    );
}

#[test]
fn candidate_break_suppresses_only_the_ligature_crossing_it() {
    let features = FontFeaturePolicy::default();
    let font = cmu_serif(features.clone());
    let unbroken = font
        .shape_run(ShapingRequest::new("office"))
        .expect("OpenType fixture");
    let candidate = font
        .shape_run(ShapingRequest::with_breaks("office", &[2]))
        .expect("OpenType fixture");

    assert!(!unbroken.glyphs.iter().any(|glyph| glyph.cluster == 2));
    assert!(
        candidate.glyphs.iter().any(|glyph| glyph.cluster == 2),
        "{:?}",
        candidate.glyphs
    );
    assert!(candidate.glyphs.len() > unbroken.glyphs.len());
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
