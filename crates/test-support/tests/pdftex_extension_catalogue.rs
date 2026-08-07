#![allow(clippy::disallowed_methods)] // host-only hermetic fixture audit

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

const SCHEMA: &str = "pdftex-extension-properties-v1";
const SOURCE_COMMIT: &str = "1664cf0ab3f6ce3b80db649bc6723f54ab12016c";
const SOURCE_SHA256: &str = "5a105669acc1b49aedb7560d4d15cb2e23467cb16d895eb0031c8dd9fea32f04";
const INVENTORY_AUTHORITY: &str = "docs/pdftex_primitives.md";

// Reviewed directly against SOURCE_SHA256. This deliberately duplicates the
// catalogue's compact citations: changing an in-range section number or moving
// a real section to the wrong property must fail until its semantic identity is
// reviewed here too.
const CITATION_AUDIT: &[(&str, &[u64], &str)] = &[
    (
        "pdftex.extension.compatibility-controls",
        &[1151, 1264, 1655],
        "Section 1151 applies ignore_primitive_error to infinite-shrinkage recovery; section 1264 defines quitvmode's mode-sensitive paragraph entry; section 1655 registers ignoreprimitiveerror.",
    ),
    (
        "pdftex.extension.font-code-tables",
        &[703, 1429, 1430],
        "Section 703 defines the bounded font-code setters and no-ligature mutation; section 1429 dispatches assign_font_int globally; section 1430 registers the font-code primitives.",
    ),
    (
        "pdftex.extension.font-configuration",
        &[252, 254],
        "Section 252 places the font-configuration integers in eqtb; section 254 registers them as assign_int primitives, giving them ordinary scoped assignment behavior.",
    ),
    (
        "pdftex.extension.form-diagnostics",
        &[1546],
        "Section 1546 implements pdfxform, fetches the numbered box, and raises the void-box fatal error.",
    ),
    (
        "pdftex.extension.form-state",
        &[440, 448, 1546, 1547, 1621, 1635],
        "Sections 440 and 448 expose last-form enquiries; sections 1546 and 1547 create and reference forms with captured dimensions; section 1621 ships immediate forms; section 1635 records referenced forms for page resources.",
    ),
    (
        "pdftex.extension.form-traversal-diagnostics",
        &[725, 755, 758],
        "Section 725 routes pdfsave and pdfrestore through the graphics-state checker; sections 755 and 758 bracket each page or form content stream and its final balance check.",
    ),
    (
        "pdftex.extension.ignored-dimension-effects",
        &[853, 1062, 1063],
        "Section 853 compares prev_depth with pdfignoreddimen; section 1062 initializes the line overrides from that sentinel; section 1063 applies overrides only when they differ from it.",
    ),
    (
        "pdftex.extension.image-configuration",
        &[252, 254, 1550],
        "Section 252 places image and page-policy integers in eqtb; section 254 registers their assign_int primitives; section 1550 consumes the page-box, resolution, and inclusion policy while scanning an image.",
    ),
    (
        "pdftex.extension.metadata-configuration",
        &[252, 254],
        "Section 252 places the metadata-policy integers in eqtb; section 254 registers them as assign_int primitives, giving them ordinary scoped assignment behavior.",
    ),
    (
        "pdftex.extension.microtype-effects",
        &[703, 1055, 1061, 1064, 1217, 1533],
        "Section 703 inserts configured character-side kerns; sections 1055 and 1061 select protrusion nodes; section 1064 applies expansion while packing lines; section 1217 adjusts interword glue; section 1533 configures expanded fonts.",
    ),
    (
        "pdftex.extension.move-chars-warning",
        &[690],
        "Section 690 warns on a positive pdfmovechars value when a PDF font is first marked used and resets it to zero.",
    ),
    (
        "pdftex.extension.output-policy",
        &[252, 254, 670, 681],
        "Sections 252 and 254 define and register the output-policy eqtb integers; section 670 supplies PDF defaults; section 681 validates and recovers version and object-stream settings.",
    ),
    (
        "pdftex.extension.destination-lifecycle",
        &[792, 793, 794, 795, 796, 1562, 1635],
        "Sections 1562 and 1635 implement duplicate-destination warning and traversal; section 792 invokes final destination checks; sections 793--796 diagnose and repair missing ordinary and structure destinations.",
    ),
    (
        "pdftex.extension.destination-scanner",
        &[1563],
        "Section 1563 implements the complete pdfdest identifier, destination-kind, zoom, rectangle, and error scanner.",
    ),
    (
        "pdftex.extension.outline-scanner",
        &[440, 448, 1554, 1561],
        "Sections 440 and 448 expose the last-object enquiry; section 1554 scans every outline action form; section 1561 scans attributes, count, and title while constructing outline objects.",
    ),
    (
        "pdftex.extension.outline-tree",
        &[786, 787, 1561],
        "Section 1561 constructs the parent, sibling, and child links; sections 786 and 787 serialize the outline root and entries after pages are complete.",
    ),
    (
        "pdftex.extension.thread-graph",
        &[784, 788, 1598, 1635],
        "Section 1635 creates and links page beads; section 784 emits bead rectangles; sections 788 and 1598 serialize thread graphs and repair referenced threads with no beads.",
    ),
    (
        "pdftex.extension.thread-lifecycle",
        &[1566, 1567, 1635],
        "Sections 1566 and 1567 create running-thread boundary nodes; section 1635 enforces hlist, nesting, page, and end-thread lifecycle rules.",
    ),
    (
        "pdftex.extension.thread-scanner",
        &[1550, 1554, 1564, 1565, 1566],
        "Section 1550 scans reordered rule dimensions; section 1554 composes dimensions and attributes for thread nodes; section 1564 validates thread identifiers; sections 1565 and 1566 implement one-shot and running thread starts.",
    ),
    (
        "pdftex.extension.ximage-enquiries",
        &[440, 448, 1548, 1550, 1551, 1552],
        "Sections 440 and 448 expose the three image enquiries; section 1548 owns their state; section 1550 imports an image and updates all three values; sections 1551 and 1552 implement image creation and reference nodes.",
    ),
];

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
    let citation_audit = CITATION_AUDIT
        .iter()
        .map(|(id, sections, rationale)| (*id, (*sections, *rationale)))
        .collect::<BTreeMap<_, _>>();
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
        let (audited_sections, audited_rationale) = citation_audit
            .get(id)
            .ok_or_else(|| format!("property {id} lacks a reviewed source citation audit"))?;
        if sections != *audited_sections {
            return Err(format!(
                "property {id} citations do not match the reviewed pdftex.web modules: catalogue={sections:?}, audited={audited_sections:?}"
            ));
        }
        if required_text(property, "citation_rationale")? != *audited_rationale {
            return Err(format!(
                "property {id} citation rationale does not match the reviewed module identities"
            ));
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
    if ids.len() != citation_audit.len() {
        return Err(format!(
            "citation audit and catalogue differ in size: catalogue={}, audit={}",
            ids.len(),
            citation_audit.len()
        ));
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

#[test]
fn catalogue_rejects_semantically_drifted_but_in_range_citations() {
    let repository = root();
    let mut drifted = catalogue();
    drifted["properties"][0]["sections"] = serde_json::json!([1522, 1525]);
    assert!(
        validate(&repository, &drifted)
            .expect_err("in-range but unrelated sections must fail")
            .contains("do not match the reviewed pdftex.web modules")
    );
}
