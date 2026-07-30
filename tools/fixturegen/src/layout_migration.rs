use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FamilySpec {
    pub area: &'static str,
    pub shared_inputs: &'static [&'static str],
    pub named_inputs: &'static [(&'static str, &'static str)],
}

pub const EXECUTION_FAMILIES: &[FamilySpec] = &[
    FamilySpec {
        area: "align",
        shared_inputs: &[],
        named_inputs: &[],
    },
    FamilySpec {
        area: "etex_exec",
        shared_inputs: &[],
        named_inputs: &[("expansion_virtual_input", "expansion_virtual_input.txt")],
    },
    FamilySpec {
        area: "exec",
        shared_inputs: &[],
        named_inputs: &[],
    },
    FamilySpec {
        area: "expand",
        shared_inputs: &[],
        named_inputs: &[("input_main", "input_secondary.inc")],
    },
    FamilySpec {
        area: "math",
        shared_inputs: &["math_preamble.inc"],
        named_inputs: &[],
    },
    FamilySpec {
        area: "tex_exec",
        shared_inputs: &[],
        named_inputs: &[],
    },
    FamilySpec {
        area: "tex_exec_io",
        shared_inputs: &[],
        named_inputs: &[],
    },
    FamilySpec {
        area: "typeset",
        shared_inputs: &[],
        named_inputs: &[],
    },
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    Plan,
    Apply,
}

pub fn run(corpus: &Path, specs: &[FamilySpec], mode: Mode) -> Result<String> {
    let mut report = String::new();
    for spec in specs {
        validate_component(spec.area)?;
        let area = corpus.join(spec.area);
        let cases = discover_cases(&area)?;
        if cases.is_empty() {
            bail!("{} has no flat or directory cases", area.display());
        }
        for (case, layout) in cases {
            validate_component(&case)?;
            let inventory = inventory_for_case(&area, spec, &case, layout)?;
            report.push_str(&format!(
                "{}/{case}: {} files {} bytes sha256={}\n",
                spec.area,
                inventory.len(),
                inventory.values().map(Vec::len).sum::<usize>(),
                inventory_digest(&inventory)
            ));
            if mode == Mode::Apply && layout == Layout::Flat {
                apply_case(&area, &case, &inventory)?;
            }
        }
        if mode == Mode::Apply {
            remove_consumed_flat_files(&area, spec)?;
            let post = discover_cases(&area)?;
            if post.values().any(|layout| *layout != Layout::Directory) {
                bail!("{} retained a flat case after migration", area.display());
            }
        }
    }
    Ok(report)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Layout {
    Flat,
    Directory,
}

fn discover_cases(area: &Path) -> Result<BTreeMap<String, Layout>> {
    let mut cases = BTreeMap::new();
    for entry in fs::read_dir(area).with_context(|| format!("read {}", area.display()))? {
        let entry = entry.context("read area entry")?;
        let path = entry.path();
        let kind = entry.file_type().context("read area entry type")?;
        if kind.is_symlink() {
            bail!("symlink is forbidden: {}", path.display());
        }
        if kind.is_dir() {
            let case = file_name(&path)?;
            if cases.insert(case.clone(), Layout::Directory).is_some() {
                bail!("duplicate case {case}");
            }
        } else if kind.is_file() && path.extension().and_then(|value| value.to_str()) == Some("tex")
        {
            let case = path
                .file_stem()
                .and_then(|value| value.to_str())
                .context("invalid source name")?
                .to_owned();
            if cases.insert(case.clone(), Layout::Flat).is_some() {
                bail!("flat/directory collision for case {case}");
            }
        }
    }
    Ok(cases)
}

fn inventory_for_case(
    area: &Path,
    spec: &FamilySpec,
    case: &str,
    layout: Layout,
) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut inventory = BTreeMap::new();
    match layout {
        Layout::Directory => {
            let directory = area.join(case);
            for entry in
                fs::read_dir(&directory).with_context(|| format!("read {}", directory.display()))?
            {
                let entry = entry.context("read case entry")?;
                let kind = entry.file_type().context("read case entry type")?;
                if !kind.is_file() || kind.is_symlink() {
                    bail!("non-regular case entry {}", entry.path().display());
                }
                inventory.insert(file_name(&entry.path())?, fs::read(entry.path())?);
            }
        }
        Layout::Flat => {
            add_file(
                &mut inventory,
                format!("{case}.tex"),
                area.join(format!("{case}.tex")),
            )?;
            let prefix = format!("{case}.expected.");
            for entry in fs::read_dir(area)? {
                let entry = entry?;
                let name = file_name(&entry.path())?;
                if let Some(channel) = name.strip_prefix(&prefix) {
                    add_file(&mut inventory, format!("expected.{channel}"), entry.path())?;
                }
            }
            for input in spec.shared_inputs {
                add_file(&mut inventory, (*input).to_owned(), area.join(input))?;
            }
            for (owner, input) in spec.named_inputs {
                if *owner == case {
                    add_file(&mut inventory, (*input).to_owned(), area.join(input))?;
                }
            }
        }
    }
    if !inventory.contains_key(&format!("{case}.tex")) {
        bail!("{}/{case} lacks its exact source", spec.area);
    }
    Ok(inventory)
}

fn add_file(inventory: &mut BTreeMap<String, Vec<u8>>, name: String, path: PathBuf) -> Result<()> {
    validate_component(&name)?;
    let bytes = fs::read(&path).with_context(|| format!("read {}", path.display()))?;
    if inventory.insert(name.clone(), bytes).is_some() {
        bail!("duplicate destination {name}");
    }
    Ok(())
}

fn apply_case(area: &Path, case: &str, inventory: &BTreeMap<String, Vec<u8>>) -> Result<()> {
    let target = area.join(case);
    if target.exists() {
        bail!("refuse to overwrite collision {}", target.display());
    }
    let staged = area.join(format!(
        ".fixture-layout-stage-{}.{}",
        std::process::id(),
        case
    ));
    if staged.exists() {
        bail!("staging collision {}", staged.display());
    }
    fs::create_dir(&staged)?;
    let result = (|| {
        for (name, bytes) in inventory {
            fs::write(staged.join(name), bytes)?;
        }
        let staged_inventory = read_regular_inventory(&staged)?;
        if inventory != &staged_inventory {
            bail!("staged byte inventory changed for {}", target.display());
        }
        fs::rename(&staged, &target)
            .with_context(|| format!("atomically install {}", target.display()))
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staged);
    }
    result
}

fn remove_consumed_flat_files(area: &Path, spec: &FamilySpec) -> Result<()> {
    let mut directory_cases = BTreeSet::new();
    for entry in fs::read_dir(area)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            directory_cases.insert(file_name(&entry.path())?);
        }
    }
    for entry in fs::read_dir(area)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = file_name(&entry.path())?;
        let owned = name
            .strip_suffix(".tex")
            .is_some_and(|case| directory_cases.contains(case))
            || directory_cases
                .iter()
                .any(|case| name.starts_with(&format!("{case}.expected.")))
            || spec.shared_inputs.contains(&name.as_str())
            || spec.named_inputs.iter().any(|(_, input)| *input == name);
        if owned {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

fn read_regular_inventory(directory: &Path) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut inventory = BTreeMap::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            bail!("staged non-regular entry {}", entry.path().display());
        }
        inventory.insert(file_name(&entry.path())?, fs::read(entry.path())?);
    }
    Ok(inventory)
}

fn inventory_digest(inventory: &BTreeMap<String, Vec<u8>>) -> String {
    let mut digest = Sha256::new();
    for (name, bytes) in inventory {
        digest.update((name.len() as u64).to_be_bytes());
        digest.update(name.as_bytes());
        digest.update((bytes.len() as u64).to_be_bytes());
        digest.update(bytes);
    }
    format!("{:x}", digest.finalize())
}

fn validate_component(value: &str) -> Result<()> {
    let path = Path::new(value);
    if value.is_empty()
        || path.components().count() != 1
        || !matches!(path.components().next(), Some(Component::Normal(_)))
    {
        bail!("unsafe path component {value:?}");
    }
    Ok(())
}

fn file_name(path: &Path) -> Result<String> {
    path.file_name()
        .and_then(|value| value.to_str())
        .map(str::to_owned)
        .with_context(|| format!("invalid UTF-8 file name {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::{FamilySpec, Mode, run};

    const SPEC: &[FamilySpec] = &[FamilySpec {
        area: "sample",
        shared_inputs: &["shared.inc"],
        named_inputs: &[("multi", "child.txt")],
    }];

    #[test]
    fn plan_apply_and_repeat_preserve_exact_inventory() {
        let temp = tempfile::tempdir().expect("temp");
        let area = temp.path().join("sample");
        std::fs::create_dir(&area).expect("area");
        std::fs::write(area.join("one.tex"), b"one").expect("source");
        std::fs::write(area.join("one.expected.log"), b"log").expect("expected");
        std::fs::write(area.join("multi.tex"), b"multi").expect("source");
        std::fs::write(area.join("child.txt"), b"child").expect("child");
        std::fs::write(area.join("shared.inc"), b"shared").expect("shared");

        let plan = run(temp.path(), SPEC, Mode::Plan).expect("plan");
        assert!(plan.contains("sample/multi"));
        assert!(area.join("one.tex").is_file());
        let applied = run(temp.path(), SPEC, Mode::Apply).expect("apply");
        assert_eq!(plan, applied);
        assert_eq!(
            std::fs::read(area.join("one/expected.log")).expect("output"),
            b"log"
        );
        assert_eq!(
            std::fs::read(area.join("multi/child.txt")).expect("child"),
            b"child"
        );
        assert_eq!(
            run(temp.path(), SPEC, Mode::Apply).expect("repeat"),
            applied
        );
    }

    #[test]
    fn collision_fails_without_consuming_flat_authority() {
        let temp = tempfile::tempdir().expect("temp");
        let area = temp.path().join("sample");
        std::fs::create_dir_all(area.join("one")).expect("case");
        std::fs::write(area.join("one.tex"), b"flat").expect("flat");
        std::fs::write(area.join("one/one.tex"), b"directory").expect("directory");
        let error = run(temp.path(), SPEC, Mode::Apply).expect_err("collision");
        assert!(format!("{error:#}").contains("collision"));
        assert_eq!(
            std::fs::read(area.join("one.tex")).expect("flat survives"),
            b"flat"
        );
    }
}
