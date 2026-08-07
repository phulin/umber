#![allow(clippy::disallowed_methods)] // host-only hermetic fixture audit

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

const SCHEMA: &str = "pdftex-extension-properties-v1";
const SOURCE_COMMIT: &str = "1664cf0ab3f6ce3b80db649bc6723f54ab12016c";
const SOURCE_SHA256: &str = "5a105669acc1b49aedb7560d4d15cb2e23467cb16d895eb0031c8dd9fea32f04";
const INVENTORY_AUTHORITY: &str = "docs/pdftex_primitives.md";

fn root() -> PathBuf {
    test_support::repository_root()
}

fn catalogue() -> Value {
    serde_json::from_str(
        &fs::read_to_string(root().join("tests/pdftex-properties/catalogue.json"))
            .expect("read pdfTeX extension property catalogue"),
    )
    .expect("parse pdfTeX extension property catalogue")
}

fn required_text<'a>(value: &'a Value, field: &str) -> Result<&'a str, String> {
    value[field]
        .as_str()
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| format!("missing non-empty {field}"))
}

fn retained_pdftex_cases(repository: &Path) -> Result<BTreeSet<String>, String> {
    fs::read_dir(repository.join("tests/corpus/tex_exec"))
        .map_err(|error| error.to_string())?
        .filter_map(|entry| {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => return Some(Err(error.to_string())),
            };
            let name = match entry.file_name().into_string() {
                Ok(name) => name,
                Err(_) => return Some(Err("non-UTF-8 tex_exec case".into())),
            };
            name.starts_with("pdf_").then_some(Ok(name))
        })
        .collect()
}

fn normalized(text: &str) -> String {
    text.chars()
        .filter(|character| !character.is_whitespace())
        .collect()
}

fn reference_channels<'a>(
    case: &str,
    reference: &'a str,
) -> Result<(bool, &'a str, &'a str), String> {
    let success = reference
        .strip_prefix("success: ")
        .and_then(|rest| rest.lines().next())
        .ok_or_else(|| format!("{case} reference lacks success status"))?;
    let success = match success {
        "true" => true,
        "false" => false,
        other => return Err(format!("{case} has invalid success status {other:?}")),
    };
    let (_, channels) = reference
        .split_once("\nstdout:\n")
        .ok_or_else(|| format!("{case} reference lacks stdout channel"))?;
    let (terminal, log) = channels
        .split_once("log:\n")
        .ok_or_else(|| format!("{case} reference lacks log channel"))?;
    Ok((success, terminal, log))
}

fn validate_test_link(repository: &Path, link: &str) -> Result<(), String> {
    let (path, function) = link
        .rsplit_once("::")
        .ok_or_else(|| format!("invalid active_test link {link}"))?;
    let source = fs::read_to_string(repository.join(path))
        .map_err(|error| format!("read active_test source {path}: {error}"))?;
    let needle = format!("fn {function}(");
    let matches = source.match_indices(&needle).collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(format!(
            "active_test {link} resolves {} times, expected exactly one",
            matches.len()
        ));
    }
    let prefix = &source[..matches[0].0];
    if prefix
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .map(str::trim)
        != Some("#[test]")
    {
        return Err(format!(
            "active_test {link} is not immediately marked #[test]"
        ));
    }
    Ok(())
}

fn validate(repository: &Path, catalogue: &Value) -> Result<(), String> {
    if catalogue["schema"] != SCHEMA {
        return Err("invalid pdfTeX extension property schema".into());
    }
    let source = &catalogue["source"];
    if source["engine"] != "pdfTeX 1.40.29"
        || source["file"] != "pdftex.web"
        || source["texlive_source_commit"] != SOURCE_COMMIT
        || source["sha256"] != SOURCE_SHA256
    {
        return Err("catalogue does not cite the pinned pdfTeX 1.40.29 source".into());
    }
    if catalogue["primitive_inventory_authority"] != INVENTORY_AUTHORITY
        || !catalogue["primitive_inventory"].is_null()
    {
        return Err(
            "catalogue must delegate primitive inventory authority to docs/pdftex_primitives.md"
                .into(),
        );
    }

    let properties = catalogue["properties"]
        .as_array()
        .ok_or("catalogue lacks properties")?;
    let mut ids = BTreeSet::new();
    let mut cases = BTreeSet::new();
    let mut owners = BTreeMap::new();
    for property in properties {
        let id = required_text(property, "id")?;
        if !id.starts_with("pdftex.extension.") || !ids.insert(id.to_owned()) {
            return Err(format!("invalid or duplicate property id {id}"));
        }
        let case = required_text(property, "case")?;
        if !cases.insert(case.to_owned()) {
            return Err(format!(
                "retained case {case} has overlapping property ownership"
            ));
        }
        owners.insert(case.to_owned(), id.to_owned());
        for field in ["claim", "semantic_owner"] {
            required_text(property, field)?;
        }
        let sections = property["sections"]
            .as_array()
            .filter(|sections| !sections.is_empty())
            .ok_or_else(|| format!("property {id} lacks pdftex.web sections"))?;
        if sections.iter().any(|section| {
            section
                .as_u64()
                .is_none_or(|section| section == 0 || section > 2000)
        }) {
            return Err(format!("property {id} has an invalid pdftex.web section"));
        }
        validate_test_link(repository, required_text(property, "active_test")?)?;

        let expected_success = property["expected_success"]
            .as_bool()
            .ok_or_else(|| format!("property {id} lacks expected_success"))?;
        let reference_path = repository.join(format!("tests/corpus/tex_exec/{case}/expected.ref"));
        let reference = fs::read_to_string(&reference_path)
            .map_err(|error| format!("read {}: {error}", reference_path.display()))?;
        let (reference_success, terminal, log) = reference_channels(case, &reference)?;
        if expected_success != reference_success {
            return Err(format!("property {id} status disagrees with expected.ref"));
        }

        let observations = property["observations"]
            .as_array()
            .filter(|observations| !observations.is_empty())
            .ok_or_else(|| format!("property {id} lacks observations"))?;
        let mut channels = BTreeSet::new();
        for observation in observations {
            let channel = required_text(observation, "channel")?;
            if !matches!(channel, "status" | "terminal" | "log") {
                return Err(format!("property {id} has invalid channel {channel}"));
            }
            channels.insert(channel);
            let projection = required_text(observation, "projection")?;
            if channel != "status" {
                let oracle = if channel == "terminal" { terminal } else { log };
                if !normalized(oracle).contains(&normalized(projection)) {
                    return Err(format!(
                        "property {id} {channel} projection {projection:?} is absent from expected.ref"
                    ));
                }
            }
            match required_text(observation, "disposition")? {
                "pass" => {
                    if !observation["bug"].is_null() {
                        return Err(format!("property {id} pass observation names a bug"));
                    }
                }
                "xfail" => {
                    let bug = required_text(observation, "bug")?;
                    if !bug.starts_with("umber2-") {
                        return Err(format!("property {id} xfail has invalid bug {bug}"));
                    }
                }
                disposition => {
                    return Err(format!(
                        "property {id} has invalid disposition {disposition}"
                    ));
                }
            }
        }
        if channels != BTreeSet::from(["status", "terminal", "log"]) {
            return Err(format!(
                "property {id} must disposition status, terminal, and log"
            ));
        }
    }

    let inventory = retained_pdftex_cases(repository)?;
    if cases != inventory {
        return Err(format!(
            "catalogue case ownership is incomplete: catalogue={cases:?}, retained={inventory:?}"
        ));
    }
    if owners.len() != inventory.len() {
        return Err("resolved case ownership is not one-to-one".into());
    }
    Ok(())
}

#[test]
fn retained_pdftex_extension_properties_have_complete_unique_active_ownership() {
    validate(&root(), &catalogue()).expect("valid pdfTeX extension property catalogue");
}

#[test]
fn catalogue_rejects_overlapping_case_ownership_and_missing_channels() {
    let repository = root();
    let mut duplicate = catalogue();
    let properties = duplicate["properties"]
        .as_array_mut()
        .expect("catalogue properties");
    properties[1]["case"] = properties[0]["case"].clone();
    assert!(
        validate(&repository, &duplicate)
            .expect_err("duplicate case must fail")
            .contains("overlapping property ownership")
    );

    let mut incomplete = catalogue();
    incomplete["properties"][0]["observations"]
        .as_array_mut()
        .expect("catalogue observations")
        .retain(|observation| observation["channel"] != "log");
    assert!(
        validate(&repository, &incomplete)
            .expect_err("missing channel must fail")
            .contains("must disposition status, terminal, and log")
    );
}
