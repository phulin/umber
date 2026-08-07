#![allow(clippy::disallowed_methods)] // host-only hermetic fixture audit

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use sha2::{Digest, Sha256};

const SOURCE_SHA256: &str = "c62ab513ef167e93f71a23bd34f311e243210afd7c7a0f9b779614b71e398324";
const MODULE_COUNT: u64 = 1380;
const DEFAULT_GAP_BEAD: &str = "umber2-johp.218";
const DEFAULT_RATIONALE: &str =
    "Explicitly deferred to the full catalogue audit; scope is not inferred.";
const RESOLVED_DISPOSITIONS_SHA256: &str =
    "5daa9e3d125be82bf1ae3f07a0d857db59e096d936eb262a540cc57352090eeb";

#[derive(Clone, Debug, Eq, PartialEq)]
enum ModuleDisposition {
    Property {
        owner: String,
        property_ids: Vec<String>,
        gap_bead: Option<String>,
        rationale: String,
    },
    DeferredReview {
        gap_bead: String,
        rationale: String,
    },
    DefinitionOnly {
        owner: String,
        gap_bead: Option<String>,
        rationale: String,
    },
    ContextOnly {
        owner: String,
        gap_bead: Option<String>,
        rationale: String,
    },
    OutOfScope {
        owner: String,
        gap_bead: Option<String>,
        rationale: String,
    },
}

impl ModuleDisposition {
    fn deferred_review() -> Self {
        Self::DeferredReview {
            gap_bead: DEFAULT_GAP_BEAD.into(),
            rationale: DEFAULT_RATIONALE.into(),
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Self::Property { .. } => "property",
            Self::DeferredReview { .. } => "deferred_review",
            Self::DefinitionOnly { .. } => "definition_only",
            Self::ContextOnly { .. } => "context_only",
            Self::OutOfScope { .. } => "out_of_scope",
        }
    }

    fn owner(&self) -> Option<&str> {
        match self {
            Self::Property { owner, .. }
            | Self::DefinitionOnly { owner, .. }
            | Self::ContextOnly { owner, .. }
            | Self::OutOfScope { owner, .. } => Some(owner),
            Self::DeferredReview { .. } => None,
        }
    }

    fn property_ids(&self) -> &[String] {
        match self {
            Self::Property { property_ids, .. } => property_ids,
            _ => &[],
        }
    }

    fn gap_bead(&self) -> Option<&str> {
        match self {
            Self::DeferredReview { gap_bead, .. } => Some(gap_bead),
            Self::Property { gap_bead, .. }
            | Self::DefinitionOnly { gap_bead, .. }
            | Self::ContextOnly { gap_bead, .. }
            | Self::OutOfScope { gap_bead, .. } => gap_bead.as_deref(),
        }
    }

    fn rationale(&self) -> &str {
        match self {
            Self::Property { rationale, .. }
            | Self::DeferredReview { rationale, .. }
            | Self::DefinitionOnly { rationale, .. }
            | Self::ContextOnly { rationale, .. }
            | Self::OutOfScope { rationale, .. } => rationale,
        }
    }

    fn projection(&self, module: u64) -> Value {
        serde_json::json!([
            module,
            self.kind(),
            self.owner(),
            self.property_ids(),
            self.gap_bead(),
            self.rationale()
        ])
    }
}

#[derive(Debug)]
struct ValidatedCatalogue {
    census: CatalogueCensus,
    resolved: BTreeMap<u64, ModuleDisposition>,
}

#[derive(Debug, Eq, PartialEq)]
struct CatalogueCensus {
    reviewed: usize,
    deferred: usize,
    covered: usize,
    gap: usize,
}

impl CatalogueCensus {
    fn report(&self) -> String {
        format!(
            "{} reviewed, {} deferred; {} covered, {} gap",
            self.reviewed, self.deferred, self.covered, self.gap
        )
    }
}

fn root() -> PathBuf {
    test_support::repository_root()
}

fn json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).expect("catalogue file must be readable"))
        .expect("catalogue file must contain valid JSON")
}

fn text<'a>(value: &'a Value, field: &str) -> Result<&'a str, String> {
    value[field]
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("missing non-empty {field}"))
}

fn optional_text(value: &Value, field: &str) -> Result<Option<String>, String> {
    if value[field].is_null() {
        return Ok(None);
    }
    Ok(Some(text(value, field)?.to_owned()))
}

fn validate(repository: &Path) -> Result<ValidatedCatalogue, String> {
    validate_catalogue(&repository.join("tests/tex82-properties"), repository)
}

fn validate_catalogue(base: &Path, source_root: &Path) -> Result<ValidatedCatalogue, String> {
    let inventory = json(&base.join("modules.json"));
    if inventory["source_sha256"] != SOURCE_SHA256 {
        return Err("catalogue does not cite pinned tex.web".into());
    }
    let modules = inventory["modules"].as_array().ok_or("missing modules")?;
    if inventory["module_count"] != MODULE_COUNT || modules.len() != MODULE_COUNT as usize {
        return Err("inventory does not contain exactly 1380 modules".into());
    }
    let mut numbers = BTreeSet::new();
    for (index, module) in modules.iter().enumerate() {
        let number = module["module"].as_u64().ok_or("missing module number")?;
        if number != index as u64 + 1 || !numbers.insert(number) {
            return Err(format!("invalid or duplicate module {number}"));
        }
        text(module, "heading")?;
        text(module, "sha256")?;
    }

    let default_disposition = ModuleDisposition::deferred_review();
    let mut resolved = (1..=MODULE_COUNT)
        .map(|number| (number, default_disposition.clone()))
        .collect::<BTreeMap<_, _>>();

    let mut shard_paths = fs::read_dir(base.join("shards"))
        .map_err(|error| error.to_string())?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    shard_paths.retain(|path| path.extension().and_then(|value| value.to_str()) == Some("json"));
    shard_paths.sort();

    let mut override_owner = BTreeMap::<u64, String>::new();
    let mut properties = BTreeMap::<String, (String, String, BTreeSet<u64>)>::new();
    let mut property_statuses = Vec::<String>::new();
    let mut section_claims = BTreeMap::<u64, String>::new();
    for path in shard_paths {
        let shard = json(&path);
        let domain = text(&shard, "domain")?.to_owned();
        let shard_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or("invalid shard filename")?
            .to_owned();
        for override_record in shard["module_dispositions"]
            .as_array()
            .ok_or_else(|| format!("{shard_name} has no module_dispositions"))?
        {
            let range = &override_record["modules"];
            let first = range["first"].as_u64().ok_or("missing first module")?;
            let last = range["last"].as_u64().ok_or("missing last module")?;
            if first > last || !numbers.contains(&first) || !numbers.contains(&last) {
                return Err(format!(
                    "{shard_name} has invalid module range {first}..={last}"
                ));
            }
            let disposition = parse_disposition(override_record, first)?;
            for number in first..=last {
                if let Some(previous) = override_owner.insert(number, shard_name.clone()) {
                    return Err(format!(
                        "module {number} disposition claimed by both {previous} and {shard_name}"
                    ));
                }
                resolved.insert(number, disposition.clone());
            }
        }
        for property in shard["properties"]
            .as_array()
            .ok_or_else(|| format!("{shard_name} has no properties"))?
        {
            let id = text(property, "id")?.to_owned();
            let semantic_owner = text(property, "semantic_owner")?.to_owned();
            for field in ["claim", "test_level", "status"] {
                text(property, field)?;
            }
            for field in [
                "preconditions",
                "stimulus",
                "expected_observations",
                "postconditions",
                "equivalence_cases",
                "recovery_cases",
            ] {
                if property[field].as_array().is_none_or(Vec::is_empty) {
                    return Err(format!("property {id} lacks {field}"));
                }
            }
            let sections = property["sections"].as_array().ok_or("missing sections")?;
            if sections.is_empty() {
                return Err(format!("property {id} has no canonical citations"));
            }
            let mut section_set = BTreeSet::new();
            for section in sections {
                let number = section
                    .as_u64()
                    .filter(|number| numbers.contains(number))
                    .ok_or_else(|| format!("property {id} has invalid canonical citations"))?;
                if !section_set.insert(number) {
                    return Err(format!("property {id} cites section {number} twice"));
                }
                if let Some(previous) = section_claims.insert(number, id.clone()) {
                    return Err(format!(
                        "section {number} claimed by both properties {previous} and {id}"
                    ));
                }
            }
            let coverage = property["coverage"].as_array().ok_or("missing coverage")?;
            match text(property, "status")? {
                "covered" if coverage.is_empty() => {
                    return Err(format!("covered property {id} lacks tests"));
                }
                "covered" => {}
                "gap" => {
                    text(property, "gap_bead")?;
                }
                status => return Err(format!("invalid property status {status}")),
            }
            property_statuses.push(text(property, "status")?.to_owned());
            for link in coverage {
                validate_link(source_root, &id, link)?;
            }
            if let Some((previous_domain, _, _)) =
                properties.insert(id.clone(), (domain.clone(), semantic_owner, section_set))
            {
                return Err(format!(
                    "property {id} owned by both domains {previous_domain} and {domain}"
                ));
            }
        }
    }

    for (module, disposition) in &resolved {
        if disposition.kind() != "property" {
            continue;
        }
        let owner = disposition.owner().expect("property has an owner");
        for id in disposition.property_ids() {
            let (_, property_owner, sections) = properties
                .get(id)
                .ok_or_else(|| format!("module {module} links absent property {id}"))?;
            if property_owner != owner {
                return Err(format!(
                    "property {id} owner {property_owner} conflicts with module {module} owner {owner}"
                ));
            }
            if !sections.contains(module) {
                return Err(format!("property {id} does not cite module {module}"));
            }
        }
    }
    for (id, (_, _, sections)) in &properties {
        for section in sections {
            let disposition = resolved
                .get(section)
                .ok_or_else(|| format!("section {section} is unclassified"))?;
            if disposition.kind() != "property"
                || !disposition.property_ids().iter().any(|value| value == id)
            {
                return Err(format!(
                    "property {id} cites section {section} without owning its disposition"
                ));
            }
        }
    }
    let census = catalogue_census(
        resolved.values().map(ModuleDisposition::kind),
        property_statuses.iter().map(String::as_str),
    );
    Ok(ValidatedCatalogue { census, resolved })
}

fn catalogue_census<'a, 'b>(
    dispositions: impl IntoIterator<Item = &'a str>,
    property_statuses: impl IntoIterator<Item = &'b str>,
) -> CatalogueCensus {
    let mut census = CatalogueCensus {
        reviewed: 0,
        deferred: 0,
        covered: 0,
        gap: 0,
    };
    for disposition in dispositions {
        if disposition == "deferred_review" {
            census.deferred += 1;
        } else {
            census.reviewed += 1;
        }
    }
    for status in property_statuses {
        if status == "covered" {
            census.covered += 1;
        } else {
            census.gap += 1;
        }
    }
    census
}

fn parse_disposition(record: &Value, module: u64) -> Result<ModuleDisposition, String> {
    let rationale = text(record, "rationale")?.to_owned();
    let property_ids = record["property_ids"]
        .as_array()
        .ok_or_else(|| format!("module {module} lacks property IDs"))?
        .iter()
        .map(|id| {
            id.as_str()
                .map(str::to_owned)
                .ok_or_else(|| "property ID is not a string".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    match text(record, "disposition")? {
        "property" => {
            if property_ids.is_empty() {
                return Err(format!("property module {module} lacks property IDs"));
            }
            Ok(ModuleDisposition::Property {
                owner: text(record, "owner")?.to_owned(),
                property_ids,
                gap_bead: optional_text(record, "gap_bead")?,
                rationale,
            })
        }
        "deferred_review" => {
            if !property_ids.is_empty() {
                return Err(format!("deferred module {module} links properties"));
            }
            Ok(ModuleDisposition::DeferredReview {
                gap_bead: text(record, "gap_bead")?.to_owned(),
                rationale,
            })
        }
        kind @ ("definition_only" | "context_only" | "out_of_scope") => {
            if !property_ids.is_empty() {
                return Err(format!("non-property module {module} links properties"));
            }
            let owner = text(record, "owner")?.to_owned();
            let gap_bead = optional_text(record, "gap_bead")?;
            Ok(match kind {
                "definition_only" => ModuleDisposition::DefinitionOnly {
                    owner,
                    gap_bead,
                    rationale,
                },
                "context_only" => ModuleDisposition::ContextOnly {
                    owner,
                    gap_bead,
                    rationale,
                },
                "out_of_scope" => ModuleDisposition::OutOfScope {
                    owner,
                    gap_bead,
                    rationale,
                },
                _ => unreachable!(),
            })
        }
        other => Err(format!("unknown disposition {other}")),
    }
}

fn validate_link(repository: &Path, property: &str, link: &Value) -> Result<(), String> {
    let relative = text(link, "path")?;
    let test = text(link, "test")?;
    let source = fs::read_to_string(repository.join(relative))
        .map_err(|error| format!("property {property} test path {relative}: {error}"))?;
    let signature = format!("fn {test}(");
    let offset = source
        .find(&signature)
        .ok_or_else(|| format!("false test link {relative}::{test}"))?;
    let previous = source[..offset]
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default()
        .trim();
    if !previous.starts_with("#[test") {
        return Err(format!("{relative}::{test} is not a #[test]"));
    }
    Ok(())
}

fn staged_catalogue() -> tempfile::TempDir {
    let temporary = tempfile::tempdir().expect("temporary catalogue directory");
    let target = temporary.path().join("catalogue");
    fs::create_dir_all(target.join("shards")).expect("create staged shard directory");
    let source = root().join("tests/tex82-properties");
    fs::copy(source.join("modules.json"), target.join("modules.json"))
        .expect("copy catalogue inventory");
    fs::copy(
        source.join("shards/input-tokenization.json"),
        target.join("shards/input-tokenization.json"),
    )
    .expect("copy representative shard");
    temporary
}

#[test]
fn committed_tex82_property_catalogue_is_complete_and_resolvable() {
    let catalogue = validate(&root()).unwrap_or_else(|error| panic!("{error}"));
    println!(
        "tex82-property-catalogue: CENSUS: {}",
        catalogue.census.report()
    );
    assert_eq!(
        catalogue.census,
        CatalogueCensus {
            reviewed: 946,
            deferred: 434,
            covered: 112,
            gap: 39,
        }
    );
    let modules = catalogue.resolved.keys().copied().collect::<Vec<_>>();
    assert_eq!(modules, (1..=MODULE_COUNT).collect::<Vec<_>>());
    let projection = catalogue
        .resolved
        .iter()
        .map(|(&module, disposition)| disposition.projection(module))
        .collect::<Vec<_>>();
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&projection).expect("serialize resolved disposition projection")
            )
        ),
        RESOLVED_DISPOSITIONS_SHA256,
        "implicit defaults and shard overrides changed the reviewed resolved map"
    );
}

#[test]
fn catalogue_census_counts_review_and_property_statuses() {
    let census = catalogue_census(
        [
            "property",
            "definition_only",
            "deferred_review",
            "context_only",
        ],
        ["covered", "gap", "covered"],
    );
    assert_eq!(
        census,
        CatalogueCensus {
            reviewed: 3,
            deferred: 1,
            covered: 2,
            gap: 1
        }
    );
}

#[test]
fn shard_merge_rejects_duplicate_disposition_ownership() {
    let temporary = staged_catalogue();
    let catalogue = temporary.path().join("catalogue");
    fs::copy(
        catalogue.join("shards/input-tokenization.json"),
        catalogue.join("shards/second-domain.json"),
    )
    .expect("duplicate shard");
    let error = validate_catalogue(&catalogue, &root()).expect_err("overlap must fail");
    assert!(error.contains("disposition claimed by both"), "{error}");
}

#[test]
fn shard_merge_rejects_overlapping_property_sections() {
    let temporary = staged_catalogue();
    let catalogue = temporary.path().join("catalogue");
    let path = catalogue.join("shards/input-tokenization.json");
    let mut shard = json(&path);
    let mut duplicate = shard["properties"][0].clone();
    duplicate["id"] = Value::String("tex82.input.duplicate".into());
    shard["properties"]
        .as_array_mut()
        .expect("staged JSON array")
        .push(duplicate);
    fs::write(
        path,
        serde_json::to_vec(&shard).expect("serialize staged shard"),
    )
    .expect("write staged shard");
    let error = validate_catalogue(&catalogue, &root()).expect_err("overlap must fail");
    assert!(error.contains("claimed by both properties"), "{error}");
}

#[test]
fn shard_merge_rejects_conflicting_property_owner() {
    let temporary = staged_catalogue();
    let catalogue = temporary.path().join("catalogue");
    let path = catalogue.join("shards/input-tokenization.json");
    let mut shard = json(&path);
    shard["properties"][0]["semantic_owner"] = Value::String("tex-exec".into());
    fs::write(
        path,
        serde_json::to_vec(&shard).expect("serialize staged shard"),
    )
    .expect("write staged shard");
    let error = validate_catalogue(&catalogue, &root()).expect_err("owner conflict must fail");
    assert!(error.contains("conflicts with module"), "{error}");
}

#[test]
fn inventory_rejects_an_incomplete_module_set() {
    let temporary = staged_catalogue();
    let catalogue = temporary.path().join("catalogue");
    let path = catalogue.join("modules.json");
    let mut inventory = json(&path);
    inventory["modules"]
        .as_array_mut()
        .expect("staged JSON array")
        .pop();
    fs::write(
        path,
        serde_json::to_vec(&inventory).expect("serialize staged inventory"),
    )
    .expect("write staged inventory");
    let error = validate_catalogue(&catalogue, &root()).expect_err("missing module must fail");
    assert!(error.contains("exactly 1380 modules"), "{error}");
}
