#![allow(clippy::disallowed_methods)] // host-only hermetic fixture audit

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

const SOURCE_SHA256: &str = "c62ab513ef167e93f71a23bd34f311e243210afd7c7a0f9b779614b71e398324";

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
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

fn validate(repository: &Path) -> Result<(), String> {
    let base = repository.join("tests/tex82-properties");
    let inventory = json(&base.join("modules.json"));
    let dispositions = json(&base.join("dispositions.json"));
    if inventory["source_sha256"] != SOURCE_SHA256 || dispositions["source_sha256"] != SOURCE_SHA256
    {
        return Err("catalogue does not cite pinned tex.web".into());
    }
    let modules = inventory["modules"].as_array().ok_or("missing modules")?;
    if inventory["module_count"] != 1380 || modules.len() != 1380 {
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

    let mut by_module = BTreeMap::new();
    for record in dispositions["dispositions"]
        .as_array()
        .ok_or("missing dispositions")?
    {
        let number = record["module"]
            .as_u64()
            .ok_or("missing disposition module")?;
        if !numbers.contains(&number) || by_module.insert(number, record).is_some() {
            return Err(format!("invalid or duplicate disposition {number}"));
        }
        text(record, "rationale")?;
        match text(record, "disposition")? {
            "property" => {
                text(record, "owner")?;
                if record["property_ids"].as_array().is_none_or(Vec::is_empty) {
                    return Err(format!("property module {number} lacks property IDs"));
                }
            }
            "deferred_review" => {
                text(record, "gap_bead")?;
                if !record["property_ids"].as_array().is_some_and(Vec::is_empty) {
                    return Err(format!("deferred module {number} links properties"));
                }
            }
            "definition_only" | "context_only" | "out_of_scope" => {}
            other => return Err(format!("unknown disposition {other}")),
        }
    }
    if by_module.len() != 1380 {
        return Err("at least one module lacks an explicit disposition".into());
    }

    let mut properties = BTreeMap::new();
    let mut ids = BTreeSet::new();
    for entry in fs::read_dir(base.join("shards")).map_err(|error| error.to_string())? {
        let path = entry.map_err(|error| error.to_string())?.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let shard = json(&path);
        text(&shard, "domain")?;
        for property in shard["properties"].as_array().ok_or("missing properties")? {
            let id = text(property, "id")?.to_owned();
            if !ids.insert(id.clone()) {
                return Err(format!("duplicate property ID {id}"));
            }
            for field in ["claim", "semantic_owner", "test_level", "status"] {
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
            if sections.is_empty()
                || sections.iter().any(|section| {
                    section
                        .as_u64()
                        .is_none_or(|number| !numbers.contains(&number))
                })
            {
                return Err(format!("property {id} has invalid citations"));
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
            for link in coverage {
                validate_link(repository, &id, link)?;
            }
            properties.insert(
                id,
                sections
                    .iter()
                    .filter_map(Value::as_u64)
                    .collect::<BTreeSet<_>>(),
            );
        }
    }
    for (module, record) in by_module {
        if record["disposition"] != "property" {
            continue;
        }
        for id in record["property_ids"]
            .as_array()
            .expect("property disposition was validated")
        {
            let id = id.as_str().ok_or("property ID is not a string")?;
            let sections = properties
                .get(id)
                .ok_or_else(|| format!("module {module} links absent property {id}"))?;
            if !sections.contains(&module) {
                return Err(format!("property {id} does not cite module {module}"));
            }
        }
    }
    Ok(())
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

#[test]
fn committed_tex82_property_catalogue_is_complete_and_resolvable() {
    if let Err(error) = validate(&root()) {
        panic!("{error}");
    }
}
