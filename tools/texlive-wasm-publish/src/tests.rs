use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::Result;
use tempfile::TempDir;

use super::{FormatConfig, InventoryConfig, PublishConfig, RootConfig, publish, tree_sha256};

fn write(root: &Path, relative: &str, bytes: &[u8]) -> Result<()> {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().expect("fixture file has a parent"))?;
    fs::write(path, bytes)?;
    Ok(())
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
        schema: 1,
        distribution: "texlive-fixture-2026".to_owned(),
        objects_base_url: "https://cdn.example.test/texlive/objects/".to_owned(),
        roots,
        dependencies: BTreeMap::from([(
            "tex:plain.tex".to_owned(),
            vec!["tfm:cmr10.tfm".to_owned()],
        )]),
        formats: Vec::new(),
        package_database: None,
        inventory: None,
    }
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
    let format = manifest.formats.get("plain").expect("plain format");
    assert_eq!(format.engine, "umber");
    assert_eq!(format.format_schema, 8);
    assert_eq!(
        objects_a.get(&format.object).map(Vec::len),
        Some(format.bytes as usize)
    );
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
