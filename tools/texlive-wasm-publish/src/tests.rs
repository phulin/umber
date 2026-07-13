use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::Result;
use tempfile::TempDir;

use super::{FormatConfig, PublishConfig, RootConfig, publish, tree_sha256};

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
    write(&first, "fonts/tfm/public/cm/cmr10.tfm", b"tfm bytes")?;
    write(&second, "other/plain.tex", b"shadowed plain\n")?;
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
    assert!(manifest.files.contains_key("tex:other/plain.tex"));
    assert_eq!(plain.dependencies, ["tfm:cmr10.tfm"]);
    let format = manifest.formats.get("plain").expect("plain format");
    assert_eq!(format.engine, "umber");
    assert_eq!(format.format_schema, 4);
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
    write(&root_a, "z/shared.tex", b"root-a-z")?;
    write(&root_a, "a/shared.tex", b"root-a-a")?;
    write(&root_b, "0/shared.tex", b"root-b")?;
    let mut config = config(vec![root("a", &root_a)?, root("b", &root_b)?]);
    config.dependencies.clear();

    let manifest = publish(&config, &fixture.path().join("out"))?;
    assert_eq!(
        manifest.files["tex:shared.tex"].virtual_path,
        "/texlive/a/shared.tex"
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
    let mut config = config(vec![
        root("collision-a", &collision_a)?,
        root("collision-b", &collision_b)?,
    ]);
    config.dependencies.clear();
    let error = publish(&config, &fixture.path().join("collision-out"))
        .expect_err("case-fold collision must fail");
    assert!(error.to_string().contains("case-fold"));

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
    write(&root_path, "plain.tex", b"plain")?;
    let pinned = root("root", &root_path)?;
    write(&root_path, "plain.tex", b"changed")?;
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
