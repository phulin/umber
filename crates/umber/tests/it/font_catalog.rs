use sha2::{Digest, Sha256};
use tex_fonts::{
    AcceptedFontContainers, FontContainer, FontFeaturePolicy, FontLimits, FontObjectIdentity,
    FontPurposes, FontRequest, FontRequestKey, OpenTypeFont, ResolvedFont, VariationSelection,
};
use umber_distribution::ManifestShard;

const CMR10: &str = "87f2d8981927644cbecaf3d639e96e348ea4e7be49d8804468bd8ba9ff3f5244";

#[test]
fn html_mvp_catalog_binds_exact_programs_mapping_and_licenses() {
    let catalog = ManifestShard::parse(include_str!(
        "../../../../tools/texlive-wasm-publish/catalog/html-mvp-v1.json"
    ))
    .expect("parse catalog");
    assert!(catalog.files.is_empty());
    assert_eq!(catalog.fonts.len(), 2);
    assert_eq!(catalog.legacy_mappings.len(), 1);

    let tfm = include_bytes!("../../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
    assert_eq!(digest(tfm), CMR10);
    let mapping = catalog
        .legacy_mappings
        .values()
        .next()
        .expect("one mapping");
    assert_eq!(mapping.request.tfm_sha256(), CMR10);
    assert_eq!(mapping.unicode_map.len(), 256);
    assert!(mapping.unicode_map[..128].iter().all(Option::is_some));
    assert!(mapping.unicode_map[128..].iter().all(Option::is_none));
    for code in b"Hello from Umber." {
        assert!(mapping.unicode_map[usize::from(*code)].is_some());
    }

    let cmu = parse_font(
        "cmu-serif-roman",
        include_bytes!("../../../umber-wasm/assets/cmu-serif-500-roman.woff2"),
    );
    let cmu_record = catalog
        .fonts
        .get(&mapping.font_request.manifest_key().to_string())
        .expect("mapped CMU font record");
    assert_eq!(
        hex(&cmu.identity.bytes()),
        cmu_record
            .declared_program_identity
            .as_deref()
            .expect("CMU program identity")
    );
    assert_eq!(cmu_record.object.sha256, digest(&cmu.transport_bytes));
    assert_eq!(mapping.object, cmu_record.object);
    assert_eq!(mapping.license, cmu_record.license);
    for (code, text) in mapping.unicode_map.iter().enumerate() {
        if let Some(text) = text {
            assert!(
                text.chars().all(|scalar| cmu.cmap.glyph(scalar).is_some()),
                "CMU cmap omits mapped code {code:02x}"
            );
        }
    }

    let stix = parse_font(
        "stix-two-math",
        include_bytes!("../../../tex-fonts/tests/fixtures/stix-two-math.woff2"),
    );
    let stix_record = catalog
        .fonts
        .values()
        .find(|record| record.request.logical_name() == "stix-two-math")
        .expect("STIX font record");
    assert_eq!(
        hex(&stix.identity.bytes()),
        stix_record
            .declared_program_identity
            .as_deref()
            .expect("STIX program identity")
    );
    assert!(stix.math.is_some());

    for record in catalog.fonts.values() {
        let license: &[u8] = if record.request.logical_name() == "cmu-serif-roman" {
            include_bytes!("../../../umber-wasm/assets/CMU-OFL.txt")
        } else {
            include_bytes!("../../../tex-fonts/tests/fixtures/stix-two-math.LICENSE.txt")
        };
        assert_eq!(digest(license), record.license.object.sha256);
        assert!(record.license.embeddable && record.license.redistributable);
    }
}

fn parse_font(logical_name: &str, bytes: &[u8]) -> OpenTypeFont {
    let key = FontRequestKey::new(
        logical_name,
        0,
        VariationSelection::default(),
        FontFeaturePolicy::default(),
    )
    .expect("font key");
    let request = FontRequest {
        key: key.clone(),
        accepted_containers: AcceptedFontContainers::WASM,
        purposes: FontPurposes::LAYOUT_AND_HTML,
    };
    let bytes = bytes.to_vec();
    OpenTypeFont::parse(
        &request,
        ResolvedFont {
            request: key,
            container: FontContainer::Woff2,
            declared_object_sha256: Some(FontObjectIdentity::for_bytes(&bytes)),
            declared_program_identity: None,
            provenance: None,
            legacy_mapping: None,
            bytes,
        },
        FontLimits::default(),
    )
    .expect("parse font")
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
