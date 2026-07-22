use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::Result;
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use umber_distribution::{FontRequestKey, LegacyMappingRequestKey};

use super::{
    FormatConfig, HtmlInventoryConfig, HtmlProfileConfig, InventoryConfig, PublicationProfile,
    PublishConfig, RootConfig, publish, shard_index, tree_sha256, verify_sharded_snapshot,
};

fn write(root: &Path, relative: &str, bytes: &[u8]) -> Result<()> {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().expect("fixture file has a parent"))?;
    fs::write(path, bytes)?;
    Ok(())
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn html_config(fixture: &TempDir) -> Result<PublishConfig> {
    let root_path = fixture.path().join("html-root");
    fs::create_dir_all(&root_path)?;
    write(&root_path, "tex/plain/base/plain.tex", b"plain runtime")?;
    write(&root_path, "tex/latex/base/article.cls", b"class runtime")?;
    let tfm_bytes = b"exact tfm";
    write(&root_path, "fonts/tfm/public/cm/cmr10.tfm", tfm_bytes)?;
    for (relative, bytes) in [
        ("fonts/afm/public/cm/cmr10.afm", b"afm".as_slice()),
        ("fonts/enc/dvips/cm/cm.enc", b"enc".as_slice()),
        ("fonts/map/pdftex/cm/cm.map", b"map".as_slice()),
        ("fonts/pk/modeless/cm/cmr10.pk", b"pk".as_slice()),
        ("fonts/type1/public/cm/cmr10.pfb", b"type1".as_slice()),
        ("fonts/truetype/public/cm/cmr10.ttf", b"ttf".as_slice()),
        ("fonts/opentype/public/cm/cmr10.otf", b"otf".as_slice()),
        ("fonts/vf/public/cm/cmr10.vf", b"vf".as_slice()),
    ] {
        write(&root_path, relative, bytes)?;
    }

    let format_bytes = [b"UMBRFMT\0".as_slice(), &10_u32.to_le_bytes()].concat();
    let format_path = fixture.path().join("plain.fmt");
    fs::write(&format_path, &format_bytes)?;
    let format_metadata = fixture.path().join("plain-format.json");
    fs::write(
        &format_metadata,
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema": 2,
            "name": "plain-html",
            "object": format!("sha256-{}", digest(&format_bytes)),
            "sha256": digest(&format_bytes),
            "bytes": format_bytes.len(),
            "engine": "umber",
            "engineVersion": "fixture",
            "formatSchema": 10,
            "sourceDistribution": "fixture",
            "sourceManifestSha256": "1".repeat(64),
            "sourceDateEpoch": 1,
            "inputClosure": {"schema": 1, "keys": ["tex:plain.tex", "tfm:cmr10.tfm"]}
        }))?,
    )?;

    let font_bytes = b"fixture woff2";
    let license_bytes = b"fixture license";
    let font_digest = digest(font_bytes);
    let license_digest = digest(license_bytes);
    let font_path = fixture.path().join("fixture.woff2");
    let license_path = fixture.path().join("LICENSE.txt");
    fs::write(&font_path, font_bytes)?;
    fs::write(&license_path, license_bytes)?;
    let font_key = FontRequestKey::new("fixture-serif")?
        .manifest_key()
        .to_string();
    let tfm_digest = digest(tfm_bytes);
    let mapping_key =
        LegacyMappingRequestKey::new(&tfm_digest, 1, "html-layout", Some("OT1".to_owned()))?
            .manifest_key()
            .to_string();
    let unicode_map = std::iter::once(serde_json::Value::String("A".to_owned()))
        .chain(std::iter::repeat_n(serde_json::Value::Null, 255))
        .collect::<Vec<_>>();
    let provenance = serde_json::json!({
        "identity": "2".repeat(64),
        "upstream": "Fixture Serif",
        "upstreamVersion": "1",
        "sourceUrl": "https://example.test/font",
        "conversionTool": "fixture",
        "conversionVersion": "1"
    });
    let license = serde_json::json!({
        "identity": "3".repeat(64),
        "object": format!("sha256-{license_digest}"),
        "sha256": license_digest,
        "bytes": license_bytes.len(),
        "spdx": "OFL-1.1",
        "embeddable": true,
        "redistributable": true
    });
    let object = serde_json::json!({
        "object": format!("sha256-{font_digest}"),
        "sha256": font_digest,
        "bytes": font_bytes.len(),
        "container": "woff2",
        "programIdentity": "4".repeat(64)
    });
    let catalog_path = fixture.path().join("html-catalog.json");
    fs::write(
        &catalog_path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema": 2,
            "distribution": "html-fixture-v1",
            "index": 0,
            "files": {},
            "fonts": {
                font_key.clone(): {
                    "schema": 1,
                    "object": object["object"], "sha256": object["sha256"], "bytes": object["bytes"],
                    "container": object["container"], "programIdentity": object["programIdentity"],
                    "featurePolicyVersion": 1,
                    "provenance": provenance.clone(), "license": license.clone()
                }
            },
            "legacyMappings": {
                mapping_key: {
                    "schema": 1, "tfmSha256": tfm_digest, "fontKey": font_key,
                    "object": object["object"], "sha256": object["sha256"], "bytes": object["bytes"],
                    "container": object["container"], "programIdentity": object["programIdentity"],
                    "unicodeMap": unicode_map, "mappingVersion": 1, "fontdimenVersion": 1,
                    "featurePolicyVersion": 1, "fallback": "classic-tfm-exact",
                    "provenance": provenance, "license": license
                }
            }
        }))?,
    )?;
    Ok(PublishConfig {
        schema: 4,
        distribution: "html-fixture-v1".to_owned(),
        objects_base_url: "https://cdn.example.test/html/objects/".to_owned(),
        shard_bits: 2,
        roots: vec![root("fixture", &root_path)?],
        dependencies: BTreeMap::new(),
        formats: vec![FormatConfig {
            path: format_path,
            metadata: format_metadata,
        }],
        package_database: None,
        inventory: None,
        profile: PublicationProfile::Html,
        html: Some(HtmlProfileConfig {
            runtime_file_keys: vec!["tex:article.cls".to_owned()],
            catalog: catalog_path,
            object_sources: BTreeMap::from([
                (font_digest, font_path),
                (license_digest, license_path),
            ]),
            inventory: HtmlInventoryConfig {
                maximum_logical_files: 3,
                maximum_objects: 16,
                maximum_bytes: 1_000_000,
                maximum_fonts: 1,
                maximum_legacy_mappings: 1,
                maximum_licenses: 1,
            },
        }),
    })
}

fn root(name: &str, path: &Path) -> Result<RootConfig> {
    Ok(RootConfig {
        name: name.to_owned(),
        path: path.to_owned(),
        tree_sha256: tree_sha256(path)?,
    })
}

fn config(roots: Vec<RootConfig>) -> PublishConfig {
    PublishConfig {
        schema: 3,
        distribution: "texlive-fixture-2026".to_owned(),
        objects_base_url: "https://cdn.example.test/texlive/objects/".to_owned(),
        shard_bits: 3,
        roots,
        dependencies: BTreeMap::from([(
            "tex:plain.tex".to_owned(),
            vec!["tfm:cmr10.tfm".to_owned()],
        )]),
        formats: Vec::new(),
        package_database: None,
        inventory: None,
        profile: PublicationProfile::Full,
        html: None,
    }
}

#[test]
fn latex_wasm_script_emits_current_sharded_publisher_config() -> Result<()> {
    let fixture = TempDir::new()?;
    let output = fixture.path().join("publish.json");
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/write-latex-wasm-publish-config.sh");
    let status = Command::new("bash")
        .arg(script)
        .arg(&output)
        .arg("texlive-2026-03-01")
        .arg("https://cdn.example.test/latex/objects/")
        .arg("/fixture/texmf-dist")
        .arg("0".repeat(64))
        .arg("/fixture/latex.fmt")
        .arg("/fixture/latex-format.json")
        .status()?;
    assert!(status.success());

    let config: PublishConfig = serde_json::from_slice(&fs::read(output)?)?;
    assert_eq!(config.schema, super::sharded::ROOT_SCHEMA);
    assert_eq!(config.shard_bits, 8);
    assert_eq!(config.roots.len(), 1);
    assert_eq!(config.formats.len(), 1);
    Ok(())
}

#[test]
fn fixture_publication_is_byte_stable_and_content_addressed() -> Result<()> {
    let fixture = TempDir::new()?;
    let first = fixture.path().join("first-root");
    let second = fixture.path().join("second-root");
    fs::create_dir_all(&first)?;
    fs::create_dir_all(&second)?;
    write(&first, "tex/plain/base/plain.tex", b"first plain\n")?;
    write(&first, "tex/latex/base/article.cls", b"class bytes\n")?;
    write(&first, "fonts/tfm/public/cm/cmr10.tfm", b"tfm bytes")?;
    write(&second, "tex/other/plain.tex", b"shadowed plain\n")?;
    write(&second, "tex/extra.tex", b"extra\n")?;

    let mut config = config(vec![root("first", &first)?, root("second", &second)?]);
    let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/umber-wasm/assets");
    config.formats.push(FormatConfig {
        path: assets.join("plain.fmt"),
        metadata: assets.join("plain-format.json"),
    });
    let output_a = fixture.path().join("out-a");
    let output_b = fixture.path().join("out-b");
    let manifest = publish(&config, &output_a)?;
    publish(&config, &output_b)?;

    assert_eq!(
        fs::read(output_a.join("manifest.json"))?,
        fs::read(output_b.join("manifest.json"))?
    );
    let objects_a = directory_bytes(&output_a.join("objects"))?;
    let objects_b = directory_bytes(&output_b.join("objects"))?;
    assert_eq!(objects_a, objects_b);
    let plain = manifest.files.get("tex:plain.tex").expect("plain winner");
    assert_eq!(plain.virtual_path, "/texlive/tex/plain/base/plain.tex");
    assert_eq!(
        objects_a.get(&plain.object).map(Vec::as_slice),
        Some(b"first plain\n".as_slice())
    );
    assert!(manifest.files.contains_key("tex:tex/other/plain.tex"));
    assert!(manifest.files.contains_key("tex:article.cls"));
    assert_eq!(plain.dependencies, ["tfm:cmr10.tfm"]);
    let shard = &manifest.shards[shard_index("tex:plain.tex", config.shard_bits)];
    let inline = &shard.files["tex:plain.tex"].dependencies[0];
    assert_eq!(inline.key, "tfm:cmr10.tfm");
    assert_eq!(
        inline.virtual_path,
        manifest.files["tfm:cmr10.tfm"].virtual_path
    );
    assert_eq!(inline.sha256, manifest.files["tfm:cmr10.tfm"].sha256);
    let format = manifest.formats.get("plain").expect("plain format");
    assert_eq!(format.engine, "umber");
    assert_eq!(format.format_schema, 10);
    assert_eq!(
        objects_a.get(&format.object).map(Vec::len),
        Some(format.bytes as usize)
    );
    Ok(())
}

#[test]
fn format_input_closures_are_canonical_and_verified() -> Result<()> {
    let fixture = TempDir::new()?;
    let root_path = fixture.path().join("root");
    fs::create_dir_all(&root_path)?;
    write(&root_path, "tex/plain.tex", b"plain")?;
    write(&root_path, "fonts/tfm/public/cm/cmr10.tfm", b"tfm")?;
    let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/umber-wasm/assets");
    let mut metadata: serde_json::Value =
        serde_json::from_slice(&fs::read(assets.join("plain-format.json"))?)?;
    metadata["schema"] = 2.into();
    metadata["inputClosure"] = serde_json::json!({
        "schema": 1,
        "keys": ["tfm:cmr10.tfm", "tex:plain.tex"]
    });
    let metadata_path = fixture.path().join("plain-format.json");
    fs::write(&metadata_path, serde_json::to_vec_pretty(&metadata)?)?;
    let mut config = config(vec![root("runtime", &root_path)?]);
    config.dependencies.clear();
    config.formats.push(FormatConfig {
        path: assets.join("plain.fmt"),
        metadata: metadata_path,
    });

    let output = fixture.path().join("out");
    let publication = publish(&config, &output)?;
    assert_eq!(
        publication.formats["plain"]
            .input_closure
            .as_ref()
            .expect("input closure")
            .keys,
        ["tex:plain.tex", "tfm:cmr10.tfm"]
    );

    metadata["inputClosure"]["keys"] = serde_json::json!(["tex:plain.tex", "tfm:cmr10.tfm"]);
    let sorted_metadata_path = fixture.path().join("plain-format-sorted.json");
    fs::write(&sorted_metadata_path, serde_json::to_vec(&metadata)?)?;
    let mut sorted_config = config.clone();
    sorted_config.formats[0].metadata = sorted_metadata_path;
    let sorted_output = fixture.path().join("out-sorted");
    publish(&sorted_config, &sorted_output)?;
    assert_eq!(
        fs::read(output.join("manifest.json"))?,
        fs::read(sorted_output.join("manifest.json"))?
    );
    assert_eq!(
        directory_bytes(&output.join("objects"))?,
        directory_bytes(&sorted_output.join("objects"))?
    );

    let mut corrupt_root: super::RootManifest =
        serde_json::from_slice(&fs::read(output.join("manifest.json"))?)?;
    corrupt_root
        .formats
        .get_mut("plain")
        .expect("plain format")
        .input_closure
        .as_mut()
        .expect("plain closure")
        .keys
        .insert(0, "tex:missing.tex".to_owned());
    let mut bytes = serde_json::to_vec(&corrupt_root)?;
    bytes.push(b'\n');
    fs::write(output.join("manifest.json"), bytes)?;
    let error = verify_sharded_snapshot(&output).expect_err("absent closure key must fail");
    assert!(error.to_string().contains("is absent"));
    Ok(())
}

#[test]
fn rejects_duplicate_and_oversized_format_input_closures() -> Result<()> {
    let fixture = TempDir::new()?;
    let root_path = fixture.path().join("root");
    fs::create_dir_all(&root_path)?;
    write(&root_path, "tex/plain.tex", b"plain")?;
    let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/umber-wasm/assets");
    let base: serde_json::Value =
        serde_json::from_slice(&fs::read(assets.join("plain-format.json"))?)?;
    for (label, keys) in [
        ("duplicate", vec!["tex:plain.tex".to_owned(); 2]),
        (
            "oversized",
            (0..=umber_distribution::MAX_FORMAT_INPUTS)
                .map(|index| format!("tex:{index}.tex"))
                .collect(),
        ),
    ] {
        let mut metadata = base.clone();
        metadata["schema"] = 2.into();
        metadata["inputClosure"] = serde_json::json!({"schema": 1, "keys": keys});
        let metadata_path = fixture.path().join(format!("{label}.json"));
        fs::write(&metadata_path, serde_json::to_vec(&metadata)?)?;
        let mut config = config(vec![root("runtime", &root_path)?]);
        config.dependencies.clear();
        config.formats.push(FormatConfig {
            path: assets.join("plain.fmt"),
            metadata: metadata_path,
        });
        assert!(publish(&config, &fixture.path().join(format!("out-{label}"))).is_err());
    }
    Ok(())
}

#[test]
fn partitions_by_leading_sha256_bits_and_proves_authoritative_absence() -> Result<()> {
    let fixture = TempDir::new()?;
    let root_path = fixture.path().join("root");
    fs::create_dir_all(&root_path)?;
    for name in ["alpha.tex", "beta.tex", "gamma.tex", "delta.tex"] {
        write(&root_path, &format!("tex/{name}"), name.as_bytes())?;
    }
    let mut config = config(vec![root("runtime", &root_path)?]);
    config.dependencies.clear();
    config.shard_bits = 4;
    let output = fixture.path().join("out");
    let publication = publish(&config, &output)?;
    assert_eq!(publication.root.shard_count, 16);
    assert_eq!(publication.root.shards.len(), 16);
    for (index, shard) in publication.shards.iter().enumerate() {
        for key in shard.files.keys() {
            assert_eq!(shard_index(key, config.shard_bits), index);
        }
    }
    let absent = "tex:absent.tex";
    assert!(
        !publication.shards[shard_index(absent, config.shard_bits)]
            .files
            .contains_key(absent)
    );
    verify_sharded_snapshot(&output)?;
    Ok(())
}

#[test]
fn verifier_rejects_noncanonical_and_tampered_shards() -> Result<()> {
    let fixture = TempDir::new()?;
    let root_path = fixture.path().join("root");
    fs::create_dir_all(&root_path)?;
    write(&root_path, "tex/plain.tex", b"plain")?;
    let mut config = config(vec![root("runtime", &root_path)?]);
    config.dependencies.clear();
    config.shard_bits = 0;
    let output = fixture.path().join("out");
    let publication = publish(&config, &output)?;
    let shard_path = output
        .join("objects")
        .join(format!("sha256-{}", publication.root.shards[0]));
    let canonical = fs::read(&shard_path)?;

    let mut noncanonical = canonical.clone();
    noncanonical.push(b' ');
    fs::write(&shard_path, &noncanonical)?;
    let error = verify_sharded_snapshot(&output).expect_err("changed shard must fail its root pin");
    assert!(error.to_string().contains("declared digest"));

    fs::write(&shard_path, canonical)?;
    let object = &publication.files["tex:plain.tex"].object;
    fs::write(output.join("objects").join(object), b"corrupt")?;
    let error = verify_sharded_snapshot(&output).expect_err("changed content object must fail");
    assert!(error.to_string().contains("digest and length"));
    Ok(())
}

#[test]
fn ordered_roots_and_paths_define_duplicate_basename_precedence() -> Result<()> {
    let fixture = TempDir::new()?;
    let root_a = fixture.path().join("a");
    let root_b = fixture.path().join("b");
    fs::create_dir_all(&root_a)?;
    fs::create_dir_all(&root_b)?;
    write(&root_a, "tex/z/shared.tex", b"root-a-z")?;
    write(&root_a, "tex/a/shared.tex", b"root-a-a")?;
    write(&root_b, "tex/0/shared.tex", b"root-b")?;
    let mut config = config(vec![root("a", &root_a)?, root("b", &root_b)?]);
    config.dependencies.clear();

    let manifest = publish(&config, &fixture.path().join("out"))?;
    assert_eq!(
        manifest.files["tex:shared.tex"].virtual_path,
        "/texlive/tex/a/shared.tex"
    );
    Ok(())
}

#[test]
fn rejects_case_fold_collisions_and_invalid_paths() -> Result<()> {
    let fixture = TempDir::new()?;
    let collision_a = fixture.path().join("collision-a");
    let collision_b = fixture.path().join("collision-b");
    fs::create_dir_all(&collision_a)?;
    fs::create_dir_all(&collision_b)?;
    write(&collision_a, "tex/Foo.tex", b"one")?;
    write(&collision_b, "tex/foo.tex", b"two")?;
    let mut collision_config = config(vec![
        root("collision-a", &collision_a)?,
        root("collision-b", &collision_b)?,
    ]);
    collision_config.dependencies.clear();
    let error = publish(&collision_config, &fixture.path().join("collision-out"))
        .expect_err("case-fold collision must fail");
    assert!(error.to_string().contains("case-fold"));

    let distinct = fixture.path().join("case-distinct");
    fs::create_dir_all(&distinct)?;
    write(&distinct, "fonts/tfm/a/Cherokee.tfm", b"upper")?;
    write(&distinct, "fonts/tfm/b/cherokee.tfm", b"lower")?;
    let mut config = config(vec![root("case-distinct", &distinct)?]);
    config.dependencies.clear();
    let manifest = publish(&config, &fixture.path().join("case-distinct-out"))?;
    assert!(manifest.files.contains_key("tfm:Cherokee.tfm"));
    assert!(manifest.files.contains_key("tfm:cherokee.tfm"));

    let invalid = fixture.path().join("invalid");
    fs::create_dir_all(&invalid)?;
    write(&invalid, "tex/bad\\name.tex", b"bad")?;
    let error = tree_sha256(&invalid).expect_err("backslash path must fail");
    assert!(error.to_string().contains("invalid TEXMF path"));
    Ok(())
}

#[test]
fn rejects_changed_pinned_root_and_unknown_dependency() -> Result<()> {
    let fixture = TempDir::new()?;
    let root_path = fixture.path().join("root");
    fs::create_dir_all(&root_path)?;
    write(&root_path, "tex/plain.tex", b"plain")?;
    let pinned = root("root", &root_path)?;
    write(&root_path, "tex/plain.tex", b"changed")?;
    let mut changed = config(vec![pinned]);
    changed.dependencies.clear();
    let error =
        publish(&changed, &fixture.path().join("changed-out")).expect_err("changed root must fail");
    assert!(error.to_string().contains("digest mismatch"));

    changed.roots[0].tree_sha256 = tree_sha256(&root_path)?;
    changed.dependencies.insert(
        "tex:plain.tex".to_owned(),
        vec!["tfm:missing.tfm".to_owned()],
    );
    let error = publish(&changed, &fixture.path().join("dependency-out"))
        .expect_err("unknown dependency must fail");
    assert!(error.to_string().contains("not published"));
    Ok(())
}

#[test]
fn publishes_runtime_trees_but_excludes_documentation_and_sources() -> Result<()> {
    let fixture = TempDir::new()?;
    let root_path = fixture.path().join("root");
    fs::create_dir_all(&root_path)?;
    write(&root_path, "tex/latex/pkg/pkg.sty", b"style")?;
    write(&root_path, "tex/context/pkg/runtime.lua", b"lua")?;
    write(&root_path, "fonts/tfm/public/pkg/font.tfm", b"tfm")?;
    write(&root_path, "fonts/map/dvips/pkg/font.map", b"map")?;
    write(&root_path, "fonts/enc/dvips/pkg/font.enc", b"enc")?;
    write(&root_path, "fonts/afm/public/pkg/font.afm", b"afm")?;
    write(
        &root_path,
        "fonts/opentype/public/pkg/font.otf",
        b"opentype",
    )?;
    write(&root_path, "fonts/pk/modeless/pkg/font.pk", b"pk")?;
    write(&root_path, "fonts/type1/public/pkg/font.pfb", b"type1")?;
    write(
        &root_path,
        "fonts/truetype/public/pkg/font.ttf",
        b"truetype",
    )?;
    write(&root_path, "fonts/vf/public/pkg/font.vf", b"vf")?;
    write(&root_path, "doc/latex/pkg/manual.tex", b"documentation")?;
    write(&root_path, "source/latex/pkg/pkg.dtx", b"source")?;
    let mut config = config(vec![root("runtime", &root_path)?]);
    config.dependencies.clear();

    let manifest = publish(&config, &fixture.path().join("out"))?;
    for key in [
        "tex:pkg.sty",
        "tex:runtime.lua",
        "tfm:font.tfm",
        "tex:font.map",
        "tex:font.enc",
        "tex:font.afm",
        "tex:font.otf",
        "tex:font.pk",
        "tex:font.pfb",
        "tex:font.ttf",
        "tex:font.vf",
    ] {
        assert!(manifest.files.contains_key(key), "missing {key}");
    }
    assert!(!manifest.files.contains_key("tex:manual.tex"));
    assert!(!manifest.files.contains_key("tex:pkg.dtx"));
    Ok(())
}

#[test]
fn rejects_seed_sized_publication_when_inventory_floor_is_configured() -> Result<()> {
    let fixture = TempDir::new()?;
    let root_path = fixture.path().join("root");
    fs::create_dir_all(&root_path)?;
    write(&root_path, "tex/plain.tex", b"plain")?;
    let mut config = config(vec![root("seed", &root_path)?]);
    config.dependencies.clear();
    config.inventory = Some(InventoryConfig {
        minimum_logical_files: 10_000,
        minimum_objects: 5_000,
        minimum_bytes: 100_000_000,
    });

    let error = publish(&config, &fixture.path().join("out"))
        .expect_err("seed bundle must fail the production inventory floor");
    assert!(error.to_string().contains("inventory is incomplete"));
    Ok(())
}

#[test]
fn derives_bounded_cross_package_and_package_peer_hints_from_tlpdb() -> Result<()> {
    let fixture = TempDir::new()?;
    let root_path = fixture.path().join("root");
    fs::create_dir_all(&root_path)?;
    write(&root_path, "tex/latex/alpha/alpha.sty", b"alpha")?;
    write(&root_path, "tex/latex/beta/beta.sty", b"beta")?;
    write(&root_path, "tex/latex/beta/beta.cfg", b"config")?;
    let database = fixture.path().join("texlive.tlpdb");
    fs::write(
        &database,
        "name alpha\ndepend beta\nrunfiles size=1\n texmf-dist/tex/latex/alpha/alpha.sty\n\nname beta\nrunfiles size=2\n texmf-dist/tex/latex/beta/beta.cfg\n texmf-dist/tex/latex/beta/beta.sty\n",
    )?;
    let mut config = config(vec![root("runtime", &root_path)?]);
    config.dependencies.clear();
    config.package_database = Some(database);

    let manifest = publish(&config, &fixture.path().join("out"))?;
    assert_eq!(
        manifest.files["tex:alpha.sty"].dependencies,
        ["tex:beta.cfg", "tex:beta.sty"]
    );
    assert_eq!(
        manifest.files["tex:beta.sty"].dependencies,
        ["tex:beta.cfg"]
    );
    Ok(())
}

#[test]
fn large_package_peer_hints_rotate_with_bounded_deterministic_output() -> Result<()> {
    const FILE_COUNT: usize = 24;
    const PEER_BUDGET: usize = 16;

    let fixture = TempDir::new()?;
    let root_path = fixture.path().join("root");
    fs::create_dir_all(&root_path)?;
    let mut database_contents = format!("name large\nrunfiles size={FILE_COUNT}\n");
    for index in 0..FILE_COUNT {
        let relative = format!("tex/latex/large/file-{index:02}.sty");
        write(&root_path, &relative, format!("file {index}\n").as_bytes())?;
        database_contents.push_str(&format!(" texmf-dist/{relative}\n"));
    }
    let database = fixture.path().join("texlive.tlpdb");
    fs::write(&database, database_contents)?;
    let mut config = config(vec![root("runtime", &root_path)?]);
    config.dependencies.clear();
    config.package_database = Some(database);

    let output_a = fixture.path().join("out-a");
    let output_b = fixture.path().join("out-b");
    let manifest = publish(&config, &output_a)?;
    publish(&config, &output_b)?;

    let package_keys = (0..FILE_COUNT)
        .map(|index| format!("tex:file-{index:02}.sty"))
        .collect::<std::collections::BTreeSet<_>>();
    let mut covered = std::collections::BTreeSet::new();
    let mut peer_sets = std::collections::BTreeSet::new();
    for key in &package_keys {
        let dependencies = &manifest.files[key].dependencies;
        assert_eq!(dependencies.len(), PEER_BUDGET);
        assert!(!dependencies.contains(key));
        assert!(
            dependencies
                .iter()
                .all(|dependency| package_keys.contains(dependency))
        );
        covered.extend(dependencies.iter().cloned());
        peer_sets.insert(dependencies.clone());
    }
    assert_eq!(covered, package_keys);
    assert!(
        peer_sets.len() > 1,
        "peer windows must rotate between owners"
    );
    assert_eq!(
        fs::read(output_a.join("manifest.json"))?,
        fs::read(output_b.join("manifest.json"))?
    );
    assert_eq!(
        directory_bytes(&output_a.join("objects"))?,
        directory_bytes(&output_b.join("objects"))?
    );
    verify_sharded_snapshot(&output_a)?;
    verify_sharded_snapshot(&output_b)?;
    Ok(())
}

#[test]
fn html_profile_is_reproducible_bounded_and_contains_only_html_resources() -> Result<()> {
    let fixture = TempDir::new()?;
    let config = html_config(&fixture)?;
    let output_a = fixture.path().join("html-a");
    let output_b = fixture.path().join("html-b");
    fs::create_dir_all(output_a.join("objects"))?;
    fs::write(output_a.join("objects/stale-object"), b"stale")?;

    let publication = publish(&config, &output_a)?;
    publish(&config, &output_b)?;
    assert_eq!(publication.root.schema, 4);
    assert_eq!(publication.files.len(), 3);
    assert!(publication.files.contains_key("tex:plain.tex"));
    assert!(publication.files.contains_key("tex:article.cls"));
    assert!(publication.files.contains_key("tfm:cmr10.tfm"));
    assert_eq!(publication.fonts.len(), 1);
    assert_eq!(publication.legacy_mappings.len(), 1);
    for forbidden in [".afm", ".enc", ".map", ".pk", ".pfb", ".ttf", ".otf", ".vf"] {
        assert!(
            publication
                .files
                .values()
                .all(|file| !file.virtual_path.ends_with(forbidden)),
            "published forbidden HTML class {forbidden}"
        );
    }
    assert!(!output_a.join("objects/stale-object").exists());
    assert_eq!(
        fs::read(output_a.join("manifest.json"))?,
        fs::read(output_b.join("manifest.json"))?
    );
    assert_eq!(
        directory_bytes(&output_a.join("objects"))?,
        directory_bytes(&output_b.join("objects"))?
    );

    let license = &publication
        .fonts
        .values()
        .next()
        .expect("font")
        .license
        .object;
    fs::write(output_a.join("objects").join(&license.object), b"corrupt")?;
    let error = verify_sharded_snapshot(&output_a).expect_err("corrupt license must fail");
    assert!(error.to_string().contains("license"));
    Ok(())
}

#[test]
fn html_profile_rejects_every_pdf_and_dvi_only_class_and_inventory_overflow() -> Result<()> {
    let fixture = TempDir::new()?;
    let base = html_config(&fixture)?;
    for key in [
        "tex:cmr10.afm",
        "tex:cm.enc",
        "tex:cm.map",
        "tex:cmr10.pk",
        "tex:cmr10.pfb",
        "tex:cmr10.ttf",
        "tex:cmr10.otf",
        "tex:cmr10.vf",
    ] {
        let mut config = base.clone();
        config
            .html
            .as_mut()
            .expect("HTML config")
            .runtime_file_keys
            .push(key.to_owned());
        let error = publish(
            &config,
            &fixture
                .path()
                .join(format!("reject-{}", key.replace(':', "-"))),
        )
        .expect_err("forbidden HTML class must fail");
        assert!(
            error.to_string().contains("PDF/DVI-only"),
            "{key}: {error:#}"
        );
    }

    let mut bounded = base;
    bounded
        .html
        .as_mut()
        .expect("HTML config")
        .inventory
        .maximum_bytes = 1;
    let error = publish(&bounded, &fixture.path().join("over-budget"))
        .expect_err("HTML byte ceiling must fail");
    assert!(error.to_string().contains("exceeds ceiling"));
    Ok(())
}

fn directory_bytes(directory: &Path) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut entries = BTreeMap::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        entries.insert(
            entry.file_name().to_string_lossy().into_owned(),
            fs::read(entry.path())?,
        );
    }
    Ok(entries)
}
