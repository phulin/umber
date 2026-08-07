#![allow(clippy::disallowed_methods)] // host-only hermetic fixture audit

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use sha2::{Digest, Sha256};
use test_support::closed_case::FixtureCase;

const SCHEMA: &str = "pdftex-extension-properties-v1";
const SOURCE_COMMIT: &str = "1664cf0ab3f6ce3b80db649bc6723f54ab12016c";
const SOURCE_SHA256: &str = "5a105669acc1b49aedb7560d4d15cb2e23467cb16d895eb0031c8dd9fea32f04";
const INVENTORY_AUTHORITY: &str = "docs/pdftex_primitives.md";
const SOURCE_EVIDENCE_SCHEMA: &str = "# pdftex-extension-source-evidence-v1";
const SOURCE_EVIDENCE_SHA256: &str =
    "08752d2e05d122ef70f9aa2b044186df369534b23c79b2d16205227e5ee4581c";
const WEB_MODULE_COUNT: u64 = 1868;
const ACTIVE_RUNNER: &str = "crates/umber/src/pdftex/tests/retained_fixture_properties.rs::retained_pdftex_extension_fixtures_compare_oracle_projections";

struct SourceEvidence {
    modules: BTreeMap<u64, (String, String)>,
    bindings: BTreeMap<String, Vec<u64>>,
}

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

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn source_evidence(repository: &Path) -> Result<SourceEvidence, String> {
    let path = repository.join("tests/pdftex-properties/source-evidence.tsv");
    let text =
        fs::read_to_string(&path).map_err(|error| format!("read {}: {error}", path.display()))?;
    if format!("{:x}", Sha256::digest(text.as_bytes())) != SOURCE_EVIDENCE_SHA256 {
        return Err("pdfTeX source-evidence lock digest changed".into());
    }
    let mut lines = text.lines();
    if lines.next() != Some(SOURCE_EVIDENCE_SCHEMA) {
        return Err("invalid pdfTeX source-evidence schema".into());
    }
    let source = lines
        .next()
        .ok_or("source evidence lacks pinned source identity")?
        .split('\t')
        .collect::<Vec<_>>();
    if source
        != [
            "source",
            "pdfTeX 1.40.29",
            "pdftex.web",
            SOURCE_COMMIT,
            SOURCE_SHA256,
        ]
    {
        return Err("source evidence does not bind the pinned pdftex.web identity".into());
    }
    let module_count = lines
        .next()
        .ok_or("source evidence lacks WEB module count")?
        .split('\t')
        .collect::<Vec<_>>();
    if module_count != ["web_module_count", &WEB_MODULE_COUNT.to_string()] {
        return Err("source evidence has the wrong WEB module count".into());
    }

    let mut modules = BTreeMap::new();
    let mut bindings = BTreeMap::new();
    for (index, line) in lines.enumerate() {
        let fields = line.split('\t').collect::<Vec<_>>();
        match fields.first().copied() {
            Some("module") if fields.len() == 4 => {
                let number = fields[1]
                    .parse::<u64>()
                    .map_err(|_| format!("invalid module number on evidence line {}", index + 4))?;
                if !is_sha256(fields[2]) || fields[3].trim().is_empty() {
                    return Err(format!("module {number} lacks a locked body hash or title"));
                }
                if modules
                    .insert(number, (fields[2].to_owned(), fields[3].to_owned()))
                    .is_some()
                {
                    return Err(format!("duplicate source-evidence module {number}"));
                }
            }
            Some("binding") if fields.len() == 3 => {
                let cited = fields[2]
                    .split(',')
                    .map(|number| {
                        number.parse::<u64>().map_err(|_| {
                            format!("invalid binding module on evidence line {}", index + 4)
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                if cited.is_empty() || bindings.insert(fields[1].to_owned(), cited).is_some() {
                    return Err(format!("invalid or duplicate binding {}", fields[1]));
                }
            }
            _ => return Err(format!("invalid source-evidence line {}", index + 4)),
        }
    }
    if modules.is_empty() || bindings.is_empty() {
        return Err("source evidence lacks modules or property bindings".into());
    }
    Ok(SourceEvidence { modules, bindings })
}

fn required_text<'a>(value: &'a Value, field: &str) -> Result<&'a str, String> {
    value[field]
        .as_str()
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| format!("missing non-empty {field}"))
}

fn retained_pdftex_cases(repository: &Path) -> Result<BTreeSet<String>, String> {
    let mut cases = BTreeSet::new();
    for (relative, require_pdf_prefix) in [
        ("tests/corpus/tex_exec", true),
        ("tests/pdftex-properties/fixtures", false),
    ] {
        for entry in fs::read_dir(repository.join(relative)).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| format!("non-UTF-8 case in {relative}"))?;
            if (!require_pdf_prefix || name.starts_with("pdf_")) && !cases.insert(name.clone()) {
                return Err(format!(
                    "pdfTeX case {name} has both legacy and property-owned fixtures"
                ));
            }
        }
    }
    Ok(cases)
}

fn fixture_directory(repository: &Path, property: &Value, case: &str) -> Result<PathBuf, String> {
    let property_owned = required_text(property, "active_test")? == ACTIVE_RUNNER;
    let relative = if property_owned {
        format!("tests/pdftex-properties/fixtures/{case}")
    } else {
        format!("tests/corpus/tex_exec/{case}")
    };
    let alternate = if property_owned {
        format!("tests/corpus/tex_exec/{case}")
    } else {
        format!("tests/pdftex-properties/fixtures/{case}")
    };
    if repository.join(&alternate).exists() {
        return Err(format!(
            "pdfTeX case {case} has a shadow fixture at {alternate}"
        ));
    }
    FixtureCase::discover_tracked_at(
        repository,
        &relative,
        format!("{case}.tex"),
        "pdftex-extension-properties",
    )
    .map_err(|error| format!("{relative} is not a closed fixture: {error:#}"))?;
    Ok(repository.join(relative))
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
    let evidence = source_evidence(repository)?;
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
    let mut cited_modules = BTreeSet::new();
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
        let sections = sections
            .iter()
            .map(|section| {
                section
                    .as_u64()
                    .ok_or_else(|| format!("property {id} has a nonnumeric pdftex.web section"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let authenticated_sections = evidence
            .bindings
            .get(id)
            .ok_or_else(|| format!("property {id} lacks authenticated source evidence"))?;
        if sections != *authenticated_sections {
            return Err(format!(
                "property {id} citations do not match authenticated pdftex.web modules: catalogue={sections:?}, evidence={authenticated_sections:?}"
            ));
        }
        required_text(property, "citation_rationale")?;
        for section in &sections {
            if !evidence.modules.contains_key(section) {
                return Err(format!(
                    "property {id} cites module {section} without a locked title and body hash"
                ));
            }
            cited_modules.insert(*section);
        }
        validate_test_link(repository, required_text(property, "active_test")?)?;

        let expected_success = property["expected_success"]
            .as_bool()
            .ok_or_else(|| format!("property {id} lacks expected_success"))?;
        let reference_path = fixture_directory(repository, property, case)?.join("expected.ref");
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
                    if !observation["bug"].is_null()
                        || !observation["actual"].is_null()
                        || !observation["actual_normalized_sha256"].is_null()
                    {
                        return Err(format!(
                            "property {id} pass observation carries xfail metadata"
                        ));
                    }
                }
                "xfail" => {
                    let bug = required_text(observation, "bug")?;
                    if !bug.starts_with("umber2-") {
                        return Err(format!("property {id} xfail has invalid bug {bug}"));
                    }
                    match channel {
                        "status" => {
                            let actual = required_text(observation, "actual")?;
                            if normalized(actual) == normalized(projection) {
                                return Err(format!(
                                    "property {id} status xfail actual matches its oracle projection"
                                ));
                            }
                            if !observation["actual_normalized_sha256"].is_null() {
                                return Err(format!(
                                    "property {id} status xfail must use an exact actual status"
                                ));
                            }
                        }
                        "terminal" | "log" => {
                            let digest = required_text(observation, "actual_normalized_sha256")?;
                            if !is_sha256(digest) || !observation["actual"].is_null() {
                                return Err(format!(
                                    "property {id} {channel} xfail must use one normalized SHA-256 fingerprint"
                                ));
                            }
                        }
                        _ => unreachable!(),
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
    if ids != evidence.bindings.keys().cloned().collect() {
        return Err(format!(
            "source-evidence bindings and catalogue differ: catalogue={}, evidence={}",
            ids.len(),
            evidence.bindings.len()
        ));
    }
    if cited_modules != evidence.modules.keys().copied().collect() {
        return Err("source evidence contains an unreferenced or unauthenticated module".into());
    }
    Ok(())
}

#[test]
fn retained_pdftex_extension_properties_have_complete_unique_active_ownership() {
    validate(&root(), &catalogue()).expect("valid pdfTeX extension property catalogue");
}

#[test]
fn catalogue_accepts_a_fully_resolved_zero_xfail_state() {
    let catalogue = catalogue();
    assert!(
        catalogue["properties"]
            .as_array()
            .expect("catalogue properties")
            .iter()
            .flat_map(|property| property["observations"].as_array().expect("observations"))
            .all(|observation| observation["disposition"] == "pass"),
        "the resolved catalogue must not acquire a live xfail"
    );
    validate(&root(), &catalogue).expect("a fully resolved catalogue must remain valid");
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

#[test]
fn catalogue_rejects_citation_drift_and_weak_xfail_fingerprints() {
    let repository = root();
    let mut drifted = catalogue();
    drifted["properties"][0]["sections"] = serde_json::json!([1522, 1525]);
    assert!(
        validate(&repository, &drifted)
            .expect_err("in-range but unrelated sections must fail")
            .contains("do not match authenticated pdftex.web modules")
    );

    for (fingerprint, expected_error) in [
        ("", "missing non-empty actual_normalized_sha256"),
        (
            "not-a-sha256",
            "terminal xfail must use one normalized SHA-256 fingerprint",
        ),
    ] {
        let mut weak_xfail = catalogue();
        let observation = weak_xfail["properties"]
            .as_array_mut()
            .expect("properties")
            .iter_mut()
            .flat_map(|property| {
                property["observations"]
                    .as_array_mut()
                    .expect("observations")
            })
            .find(|observation| {
                observation["disposition"] == "pass" && observation["channel"] == "terminal"
            })
            .expect("passing terminal observation");
        observation["disposition"] = serde_json::json!("xfail");
        observation["bug"] = serde_json::json!("umber2-synthetic-negative-control");
        observation["actual_normalized_sha256"] = serde_json::json!(fingerprint);

        assert!(
            validate(&repository, &weak_xfail)
                .expect_err("synthetic weak xfail fingerprint must fail")
                .contains(expected_error)
        );
    }
}
