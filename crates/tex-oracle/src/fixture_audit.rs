use std::collections::{BTreeMap, BTreeSet};

use crate::{CommittedFixture, FixtureError};

#[derive(Clone, Debug)]
struct SemanticRequirement<'a> {
    family: &'a str,
    boundary: &'a str,
    sources: &'a str,
    pattern: &'a str,
}

#[derive(Clone, Debug)]
struct AuditOwner<'a> {
    family: &'a str,
    citation_source: &'a str,
    citation_section: &'a str,
    outputs: &'a str,
}

impl CommittedFixture {
    /// Audit a committed fixture against its executable semantic matrix and
    /// repository-owned citation/output ownership matrix.
    ///
    /// Both inputs are line-oriented, `|`-separated UTF-8. The semantic
    /// matrix columns are `family|boundary|sources|seam|event fragment`; the
    /// audit columns are `family|citation source|citation section|outputs`.
    pub fn audit_matrices(
        &self,
        semantic_matrix: &[u8],
        audit_matrix: &[u8],
    ) -> Result<(), FixtureError> {
        let semantic = parse_semantic_matrix(semantic_matrix)?;
        let audit = parse_audit_matrix(audit_matrix)?;
        let event_bytes = self
            .stream
            .events
            .iter()
            .map(serde_json::to_vec)
            .collect::<Result<Vec<_>, _>>()
            .map_err(crate::EncodingError::from)
            .map_err(FixtureError::Encoding)?;

        let mut semantic_families = BTreeSet::new();
        let mut referenced_sources = BTreeSet::new();
        let mut requirement_keys = BTreeSet::new();
        for requirement in &semantic {
            semantic_families.insert(requirement.family);
            if !requirement_keys.insert((requirement.family, requirement.boundary)) {
                return invalid(format!(
                    "semantic matrix repeats {}/{}",
                    requirement.family, requirement.boundary
                ));
            }
            let pattern = requirement.pattern.as_bytes();
            let finder = memchr::memmem::Finder::new(pattern);
            if !event_bytes.iter().any(|event| finder.find(event).is_some()) {
                return invalid(format!(
                    "committed event stream is missing {}/{}",
                    requirement.family, requirement.boundary
                ));
            }
            for source in requirement.sources.split(" and ") {
                if !self.manifest.sources.contains_key(source) {
                    return invalid(format!(
                        "semantic matrix {}/{} cites undeclared source {source}",
                        requirement.family, requirement.boundary
                    ));
                }
                referenced_sources.insert(source);
            }
        }
        let declared_sources = self
            .manifest
            .sources
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        if referenced_sources != declared_sources {
            return invalid(format!(
                "semantic matrix sources differ from committed sources: matrix={referenced_sources:?}, manifest={declared_sources:?}"
            ));
        }

        let mut audit_families = BTreeSet::new();
        let mut owned_citations = BTreeSet::new();
        let mut owned_outputs = BTreeSet::new();
        for owner in &audit {
            audit_families.insert(owner.family);
            let citation = self.manifest.citations.iter().find(|citation| {
                citation.source == owner.citation_source
                    && citation.section == owner.citation_section
            });
            if citation.is_none() {
                return invalid(format!(
                    "audit family {} cites absent canonical authority {}/{}",
                    owner.family, owner.citation_source, owner.citation_section
                ));
            }
            owned_citations.insert((owner.citation_source, owner.citation_section));
            for output in owner.outputs.split(',').filter(|output| !output.is_empty()) {
                if !self.manifest.outputs.contains_key(output) {
                    return invalid(format!(
                        "audit family {} cites absent ordinary output {output}",
                        owner.family
                    ));
                }
                owned_outputs.insert(output);
            }
        }
        if semantic_families != audit_families {
            return invalid(format!(
                "semantic and audit behavior families differ: semantic={semantic_families:?}, audit={audit_families:?}"
            ));
        }

        let declared_citations = self
            .manifest
            .citations
            .iter()
            .map(|citation| (citation.source.as_str(), citation.section.as_str()))
            .collect::<BTreeSet<_>>();
        if owned_citations != declared_citations {
            return invalid(
                "fixture audit does not account for every canonical citation exactly by identity",
            );
        }
        let declared_outputs = self
            .manifest
            .outputs
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        if owned_outputs != declared_outputs {
            return invalid(format!(
                "fixture audit outputs differ from committed outputs: audit={owned_outputs:?}, manifest={declared_outputs:?}"
            ));
        }
        Ok(())
    }
}

fn parse_semantic_matrix(bytes: &[u8]) -> Result<Vec<SemanticRequirement<'_>>, FixtureError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| FixtureError::Invalid("semantic matrix is not UTF-8".into()))?;
    parse_rows(text, "semantic", 5, |fields| SemanticRequirement {
        family: fields[0],
        boundary: fields[1],
        sources: fields[2],
        pattern: fields[4],
    })
}

pub(crate) fn semantic_matrix_sources(bytes: &[u8]) -> Result<BTreeSet<&str>, FixtureError> {
    let requirements = parse_semantic_matrix(bytes)?;
    let mut sources = BTreeSet::new();
    for requirement in requirements {
        sources.extend(requirement.sources.split(" and "));
    }
    Ok(sources)
}

fn parse_audit_matrix(bytes: &[u8]) -> Result<Vec<AuditOwner<'_>>, FixtureError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| FixtureError::Invalid("fixture audit matrix is not UTF-8".into()))?;
    let rows = parse_rows(text, "fixture audit", 4, |fields| AuditOwner {
        family: fields[0],
        citation_source: fields[1],
        citation_section: fields[2],
        outputs: fields[3],
    })?;
    let mut identities = BTreeMap::new();
    for row in &rows {
        let key = (row.family, row.citation_source, row.citation_section);
        if identities.insert(key, ()).is_some() {
            return invalid(format!(
                "fixture audit repeats {}/{}/{}",
                row.family, row.citation_source, row.citation_section
            ));
        }
    }
    Ok(rows)
}

fn parse_rows<'a, T>(
    text: &'a str,
    kind: &str,
    columns: usize,
    make: impl Fn(&[&'a str]) -> T,
) -> Result<Vec<T>, FixtureError> {
    let mut rows = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields = line.split('|').collect::<Vec<_>>();
        if fields.len() != columns || fields.iter().any(|field| field.is_empty()) {
            return invalid(format!(
                "malformed {kind} matrix row {}: expected {columns} nonempty columns",
                index + 1
            ));
        }
        rows.push(make(&fields));
    }
    if rows.is_empty() {
        return invalid(format!("{kind} matrix contains no requirements"));
    }
    Ok(rows)
}

fn invalid<T>(message: impl Into<String>) -> Result<T, FixtureError> {
    Err(FixtureError::Invalid(message.into()))
}
