use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use umber_distribution::{
    FeatureSetting, FontRequestContext, FontRequestKey, LegacyMappingRequestKey, ManifestShard,
    VariationInstance, WritingDirection,
};

const CMR10_SHA256: &str = "87f2d8981927644cbecaf3d639e96e348ea4e7be49d8804468bd8ba9ff3f5244";
const CMR10_BYTES: usize = 1_296;
const CMU_SHA256: &str = "1b875e541dc5c517cd11d244710d8639addbe91a0bb1ba55e7c4593225c7a970";
const CMU_BYTES: usize = 222_840;
const CMU_PROGRAM: &str = "7f8f29c0b55f41195c211242ce71837776e485828423324e74bbc7f425ad78a4";
const CMU_LICENSE_SHA256: &str = "73273dffdefe2e5f1e138084d4a4b65b1c50df2ab0179f78484f31beefe30d84";
const CMU_LICENSE_BYTES: usize = 4_820;
const STIX_SHA256: &str = "cb1149b7c8b7b194eff7f42e20cf9e7a9706d342ffc2b14765624577d8be38e3";
const STIX_BYTES: usize = 558_764;
const STIX_PROGRAM: &str = "b48adabb239892c9c246da05fb77e6b9525ca18b3ed613116cbca45bb35213c5";
const STIX_LICENSE_SHA256: &str =
    "0c8825913b60d858aacdb33c4ca6660a7d64b0d6464702efbb19313f5765861a";
const STIX_LICENSE_BYTES: usize = 4_519;

pub fn write_html_mvp_catalog(
    distribution: &str,
    output: &Path,
    cmr10_tfm: &Path,
    cmu_woff2: &Path,
    cmu_license: &Path,
    stix_woff2: &Path,
    stix_license: &Path,
) -> Result<()> {
    verify_input(cmr10_tfm, CMR10_SHA256, CMR10_BYTES, "cmr10 TFM")?;
    verify_input(cmu_woff2, CMU_SHA256, CMU_BYTES, "CMU Serif WOFF2")?;
    verify_input(
        cmu_license,
        CMU_LICENSE_SHA256,
        CMU_LICENSE_BYTES,
        "CMU Serif license",
    )?;
    verify_input(stix_woff2, STIX_SHA256, STIX_BYTES, "STIX Two Math WOFF2")?;
    verify_input(
        stix_license,
        STIX_LICENSE_SHA256,
        STIX_LICENSE_BYTES,
        "STIX Two Math license",
    )?;

    let cmu_key = catalog_font_key("cmu-serif-roman")?;
    let stix_key = catalog_font_key("stix-two-math")?;
    let mapping_key =
        LegacyMappingRequestKey::new(CMR10_SHA256, 1, "html-layout", Some("OT1".to_owned()))?
            .manifest_key()
            .to_string();
    let cmu_provenance = provenance(
        "Computer Modern Unicode Serif Roman",
        "0.7.0 (computer-modern npm package 0.1.3)",
        "https://www.npmjs.com/package/computer-modern/v/0.1.3",
        "computer-modern npm package",
        "0.1.3",
    );
    let stix_provenance = provenance(
        "STIX Two Math Regular",
        "google/fonts 389b770410cc0b7c21c85673bfa2077420fe7f65",
        "https://github.com/google/fonts/blob/389b770410cc0b7c21c85673bfa2077420fe7f65/ofl/stixtwomath/STIXTwoMath-Regular.ttf",
        "fontTools",
        "4.63.0",
    );
    let cmu_license = license(CMU_LICENSE_SHA256, CMU_LICENSE_BYTES);
    let stix_license = license(STIX_LICENSE_SHA256, STIX_LICENSE_BYTES);
    let cmu_object = object(CMU_SHA256, CMU_BYTES, CMU_PROGRAM);
    let stix_object = object(STIX_SHA256, STIX_BYTES, STIX_PROGRAM);
    let catalog = json!({
        "schema": 2,
        "distribution": distribution,
        "index": 0,
        "files": {},
        "fonts": {
            cmu_key.clone(): font_record(&cmu_object, cmu_provenance.clone(), cmu_license.clone()),
            stix_key: font_record(&stix_object, stix_provenance, stix_license),
        },
        "legacyMappings": {
            mapping_key: mapping_record(&cmu_key, &cmu_object, cmu_provenance, cmu_license),
        },
    });
    let parsed = ManifestShard::parse(&serde_json::to_string(&catalog)?)
        .context("validate generated HTML MVP catalog")?;
    let canonical: Value = serde_json::from_str(&parsed.to_json())?;
    let mut formatted = serde_json::to_string_pretty(&canonical)?;
    formatted.push('\n');
    fs::write(output, formatted)
        .with_context(|| format!("write HTML MVP catalog {}", output.display()))?;
    Ok(())
}

fn catalog_font_key(logical_name: &str) -> Result<String> {
    Ok(FontRequestKey::new(logical_name)?
        .with_context(FontRequestContext {
            face_index: 0,
            variation_instance: VariationInstance::Default,
            variations: Vec::new(),
            features: vec![
                FeatureSetting {
                    tag: *b"kern",
                    value: 1,
                },
                FeatureSetting {
                    tag: *b"liga",
                    value: 1,
                },
            ],
            direction: WritingDirection::LeftToRight,
            script: None,
            language: None,
        })?
        .manifest_key()
        .to_string())
}

fn verify_input(
    path: &Path,
    expected_digest: &str,
    expected_bytes: usize,
    label: &str,
) -> Result<()> {
    let bytes = fs::read(path).with_context(|| format!("read {label} {}", path.display()))?;
    let digest = format!("{:x}", Sha256::digest(&bytes));
    if bytes.len() != expected_bytes || digest != expected_digest {
        bail!(
            "{label} differs from the pinned catalog input: expected {expected_bytes} bytes and {expected_digest}, got {} bytes and {digest}",
            bytes.len()
        );
    }
    Ok(())
}

fn object(digest: &str, bytes: usize, program: &str) -> Value {
    json!({
        "object": format!("sha256-{digest}"),
        "sha256": digest,
        "bytes": bytes,
        "container": "woff2",
        "programIdentity": program,
    })
}

fn provenance(
    upstream: &str,
    upstream_version: &str,
    source_url: &str,
    conversion_tool: &str,
    conversion_version: &str,
) -> Value {
    let receipt = format!(
        "umber-html-font-provenance-v1\0{upstream}\0{upstream_version}\0{source_url}\0{conversion_tool}\0{conversion_version}"
    );
    json!({
        "identity": format!("{:x}", Sha256::digest(receipt.as_bytes())),
        "upstream": upstream,
        "upstreamVersion": upstream_version,
        "sourceUrl": source_url,
        "conversionTool": conversion_tool,
        "conversionVersion": conversion_version,
    })
}

fn license(digest: &str, bytes: usize) -> Value {
    json!({
        "identity": digest,
        "object": format!("sha256-{digest}"),
        "sha256": digest,
        "bytes": bytes,
        "spdx": "OFL-1.1",
        "embeddable": true,
        "redistributable": true,
    })
}

fn font_record(object: &Value, provenance: Value, license: Value) -> Value {
    let mut record = object.clone();
    let Value::Object(fields) = &mut record else {
        unreachable!("object helper returns an object");
    };
    fields.insert("schema".to_owned(), json!(1));
    fields.insert("featurePolicyVersion".to_owned(), json!(1));
    fields.insert("provenance".to_owned(), provenance);
    fields.insert("license".to_owned(), license);
    record
}

fn mapping_record(font_key: &str, object: &Value, provenance: Value, license: Value) -> Value {
    let mut record = font_record(object, provenance, license);
    let Value::Object(fields) = &mut record else {
        unreachable!("font record helper returns an object");
    };
    fields.insert("tfmSha256".to_owned(), json!(CMR10_SHA256));
    fields.insert("fontKey".to_owned(), json!(font_key));
    fields.insert("unicodeMap".to_owned(), json!(ot1_unicode_map()));
    fields.insert("mappingVersion".to_owned(), json!(1));
    fields.insert("fontdimenVersion".to_owned(), json!(1));
    fields.insert("fallback".to_owned(), json!("classic-tfm-exact"));
    record
}

fn ot1_unicode_map() -> Vec<Option<String>> {
    let mut map = vec![None; 256];
    let low = [
        "Γ", "Δ", "Θ", "Λ", "Ξ", "Π", "Σ", "Υ", "Φ", "Ψ", "Ω", "ff", "fi", "fl", "ffi", "ffl", "ı",
        "ȷ", "`", "´", "ˇ", "˘", "¯", "˚", "¸", "ß", "æ", "œ", "ø", "Æ", "Œ", "Ø",
    ];
    for (slot, text) in low.into_iter().enumerate() {
        map[slot] = Some(text.to_owned());
    }
    let ascii = [
        " ", "!", "”", "#", "$", "%", "&", "’", "(", ")", "*", "+", ",", "-", ".", "/", "0", "1",
        "2", "3", "4", "5", "6", "7", "8", "9", ":", ";", "¡", "=", "¿", "?", "@", "A", "B", "C",
        "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R", "S", "T", "U",
        "V", "W", "X", "Y", "Z", "[", "“", "]", "^", "˙", "‘", "a", "b", "c", "d", "e", "f", "g",
        "h", "i", "j", "k", "l", "m", "n", "o", "p", "q", "r", "s", "t", "u", "v", "w", "x", "y",
        "z", "–", "—", "˝", "~", "¨",
    ];
    for (offset, text) in ascii.into_iter().enumerate() {
        map[32 + offset] = Some(text.to_owned());
    }
    map
}
