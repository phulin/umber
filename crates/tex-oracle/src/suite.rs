use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::{CommittedFixture, Event, ObservationStream, SchemaVersion};

const REGENERATION_MANIFEST: &str = "tests/oracle-regeneration-manifest.txt";
const TEX82_FIXTURE_ROOT: &str = "tests/corpus/command/tex82";
const TEX82_SOURCE_MANIFEST: &str = "tests/tex82-oracle-manifest.txt";
const TEX82_GEOMETRY_SOURCE: &str = "tests/tex82-oracle/geometry.tex";
const TEX82_GEOMETRY_EVENTS: &str = "tests/tex82-oracle/geometry-expected.jsonl";

#[cfg(test)]
pub(crate) const COMMITTED_TEX82_COMMAND_TRACE_EVENT_COUNT: usize = 12_899;

/// A deterministic inventory of the offline TeX82 command-trace suite.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Tex82CommandTraceSuite {
    /// Total committed schema-v1 events across all locked fixtures.
    pub events: usize,
    /// Per-fixture inventory in selector order.
    pub fixtures: Vec<Tex82TraceFixture>,
}

/// One fully audited canonical TeX82 command fixture.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Tex82TraceFixture {
    /// Stable selector recorded in the regeneration contract.
    pub selector: String,
    /// Canonical profile recorded in the regeneration contract.
    pub profile: String,
    /// Schema-v1 oracle-manifest identity bound into the event stream.
    pub oracle_identity: String,
    /// Number of ordered events in the committed stream.
    pub events: usize,
    /// Audited `family/boundary` command seams in bytewise order.
    pub seams: Vec<String>,
}

/// The committed, geometry-only schema-v2 TeX82 microfixture.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Tex82GeometryTraceFixture {
    pub selector: String,
    pub source: Vec<u8>,
    pub stream: ObservationStream,
    pub identity: String,
}

/// Load the separately instrumented TeX82 geometry projection.
///
/// The projection remains outside the schema-v1 command-fixture manifest:
/// its all-zero manifest field is the reference builder's explicit detached
/// projection sentinel. Both files are nevertheless content-pinned by the
/// TeX82 source manifest, and the stream must be schema v2 and geometry-only.
pub fn validate_tex82_geometry_trace_fixture(
    repository: impl AsRef<Path>,
) -> Result<Tex82GeometryTraceFixture, String> {
    let repository = repository.as_ref();
    let source_manifest = read(&repository.join(TEX82_SOURCE_MANIFEST))?;
    let pins = repository_sha256_pins(&source_manifest)?;
    let source = pinned_file(repository, TEX82_GEOMETRY_SOURCE, &pins)?;
    let event_bytes = pinned_file(repository, TEX82_GEOMETRY_EVENTS, &pins)?;
    let stream = ObservationStream::from_canonical_json_lines(&event_bytes)
        .map_err(|error| format!("{TEX82_GEOMETRY_EVENTS} is invalid: {error}"))?;
    if stream.header.schema != SchemaVersion::V2.number() {
        return Err(format!(
            "{TEX82_GEOMETRY_EVENTS} uses schema {}, expected {}",
            stream.header.schema,
            SchemaVersion::V2.number()
        ));
    }
    if stream.events.is_empty()
        || stream
            .events
            .iter()
            .any(|event| !matches!(event.semantic, Event::Geometry(_)))
    {
        return Err(format!(
            "{TEX82_GEOMETRY_EVENTS} must be a nonempty geometry-only projection"
        ));
    }
    Ok(Tex82GeometryTraceFixture {
        selector: "tex82/geometry-v2".into(),
        source,
        stream,
        identity: sha256(&event_bytes),
    })
}

fn repository_sha256_pins(bytes: &[u8]) -> Result<BTreeMap<String, String>, String> {
    let text =
        std::str::from_utf8(bytes).map_err(|_| format!("{TEX82_SOURCE_MANIFEST} is not UTF-8"))?;
    let mut pins = BTreeMap::new();
    for (index, line) in text.lines().enumerate() {
        let fields = line.split_ascii_whitespace().collect::<Vec<_>>();
        if fields.first() != Some(&"repository-sha256") {
            continue;
        }
        if fields.len() != 3 {
            return Err(format!(
                "{TEX82_SOURCE_MANIFEST} line {} has a malformed repository pin",
                index + 1
            ));
        }
        if pins.insert(fields[1].into(), fields[2].into()).is_some() {
            return Err(format!(
                "{TEX82_SOURCE_MANIFEST} repeats repository pin {}",
                fields[1]
            ));
        }
    }
    Ok(pins)
}

fn pinned_file(
    repository: &Path,
    logical_path: &str,
    pins: &BTreeMap<String, String>,
) -> Result<Vec<u8>, String> {
    let expected = pins
        .get(logical_path)
        .ok_or_else(|| format!("{TEX82_SOURCE_MANIFEST} does not pin {logical_path}"))?;
    let bytes = read(&repository.join(logical_path))?;
    let observed = sha256(&bytes);
    if &observed != expected {
        return Err(format!(
            "{logical_path} identity drifted: expected {expected}, observed {observed}"
        ));
    }
    Ok(bytes)
}

/// Load every committed TeX82 command fixture, validate its immutable
/// regeneration-contract identity and coverage matrices, and inventory the
/// seams covered by its ordered schema-v1 stream.
///
/// This is entirely offline: it only reads committed repository artifacts and
/// never invokes an engine, legacy Umber, or a network client.
pub fn validate_tex82_command_trace_suite(
    repository: impl AsRef<Path>,
) -> Result<Tex82CommandTraceSuite, String> {
    let repository = repository.as_ref();
    let contract_path = repository.join(REGENERATION_MANIFEST);
    let contract = read(&contract_path)?;
    let records = parse_contract(&contract)?;
    let fixture_root = repository.join(TEX82_FIXTURE_ROOT);
    let discovered = discover_fixture_selectors(&fixture_root)?;

    if records.keys().collect::<BTreeSet<_>>() != discovered.iter().collect() {
        return Err(format!(
            "TeX82 command fixture inventory differs from the regeneration contract: contract={:?}, committed={discovered:?}",
            records.keys().collect::<Vec<_>>()
        ));
    }

    let mut events = 0;
    let mut fixtures = Vec::with_capacity(records.len());
    for record in records.values() {
        let expected_manifest = format!("tests/corpus/command/{}/manifest.json", record.selector);
        if record.manifest != expected_manifest {
            return Err(format!(
                "{} pins manifest {} instead of {expected_manifest}",
                record.selector, record.manifest
            ));
        }
        let manifest_path = repository.join(&record.manifest);
        if sha256(&read(&manifest_path)?) != record.manifest_sha256 {
            return Err(format!(
                "{} manifest identity drifted from the regeneration contract",
                record.selector
            ));
        }
        let semantic_path = repository.join(&record.semantic_matrix);
        let semantic = read(&semantic_path)?;
        if sha256(&semantic) != record.semantic_sha256 {
            return Err(format!(
                "{} semantic matrix identity drifted from the regeneration contract",
                record.selector
            ));
        }
        let audit_path = repository.join(&record.audit_matrix);
        let audit = read(&audit_path)?;
        if sha256(&audit) != record.audit_sha256 {
            return Err(format!(
                "{} fixture audit matrix identity drifted from the regeneration contract",
                record.selector
            ));
        }

        let directory = manifest_path
            .parent()
            .ok_or_else(|| format!("{} has no fixture directory", record.manifest))?;
        let fixture = CommittedFixture::load(directory)
            .map_err(|error| format!("{} is invalid: {error}", record.selector))?;
        if fixture.manifest.name != record.selector {
            return Err(format!(
                "{} manifest selector is {}",
                record.selector, fixture.manifest.name
            ));
        }
        if fixture.manifest.oracle.engine.dialect != crate::EngineDialect::Tex82 {
            return Err(format!("{} is not a TeX82 fixture", record.selector));
        }
        if record.profile != "initex-eight-bit"
            || fixture.manifest.profile.invocation != "initex"
            || fixture.manifest.profile.characters != "eight_bit_exact"
        {
            return Err(format!(
                "{} has a noncanonical TeX82 profile",
                record.selector
            ));
        }
        fixture
            .audit_matrices(&semantic, &audit)
            .map_err(|error| format!("{} coverage audit failed: {error}", record.selector))?;

        let seams = semantic_seams(&semantic)?;
        let oracle_identity = fixture
            .manifest
            .oracle
            .identity()
            .map_err(|error| format!("{} has invalid oracle identity: {error}", record.selector))?
            .hex();
        events += fixture.stream.events.len();
        fixtures.push(Tex82TraceFixture {
            selector: record.selector.clone(),
            profile: record.profile.clone(),
            oracle_identity,
            events: fixture.stream.events.len(),
            seams,
        });
    }
    Ok(Tex82CommandTraceSuite { events, fixtures })
}

#[derive(Debug)]
struct FixtureRecord {
    selector: String,
    profile: String,
    manifest: String,
    manifest_sha256: String,
    semantic_matrix: String,
    semantic_sha256: String,
    audit_matrix: String,
    audit_sha256: String,
}

fn parse_contract(contract: &[u8]) -> Result<BTreeMap<String, FixtureRecord>, String> {
    let contract = std::str::from_utf8(contract)
        .map_err(|_| "oracle regeneration contract is not UTF-8".to_owned())?;
    let mut fixtures = BTreeMap::new();
    let mut audits = BTreeMap::new();
    for (line_number, line) in contract.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields = line.split_ascii_whitespace().collect::<Vec<_>>();
        match fields.first().copied() {
            Some("fixture") if fields.get(2) == Some(&"tex82") => {
                if fields.len() != 6 {
                    return Err(format!(
                        "contract line {} has malformed TeX82 fixture",
                        line_number + 1
                    ));
                }
                let selector = checked_selector(fields[1])?;
                if fixtures
                    .insert(selector.clone(), (fields[3], fields[4], fields[5]))
                    .is_some()
                {
                    return Err(format!("contract repeats TeX82 fixture {selector}"));
                }
            }
            Some("fixture-audit")
                if fields
                    .get(1)
                    .is_some_and(|selector| selector.starts_with("tex82/")) =>
            {
                if fields.len() != 6 {
                    return Err(format!(
                        "contract line {} has malformed fixture audit",
                        line_number + 1
                    ));
                }
                let selector = checked_selector(fields[1])?;
                if audits
                    .insert(
                        selector.clone(),
                        (fields[2], fields[3], fields[4], fields[5]),
                    )
                    .is_some()
                {
                    return Err(format!("contract repeats fixture audit {selector}"));
                }
            }
            _ => {}
        }
    }
    if fixtures.is_empty() {
        return Err("regeneration contract declares no TeX82 command fixtures".into());
    }
    let mut records = BTreeMap::new();
    for (selector, (profile, manifest, manifest_sha256)) in fixtures {
        let Some((semantic_matrix, semantic_sha256, audit_matrix, audit_sha256)) =
            audits.remove(&selector)
        else {
            return Err(format!("TeX82 fixture {selector} has no coverage audit"));
        };
        records.insert(
            selector.clone(),
            FixtureRecord {
                selector,
                profile: profile.into(),
                manifest: manifest.into(),
                manifest_sha256: manifest_sha256.into(),
                semantic_matrix: semantic_matrix.into(),
                semantic_sha256: semantic_sha256.into(),
                audit_matrix: audit_matrix.into(),
                audit_sha256: audit_sha256.into(),
            },
        );
    }
    if let Some(selector) = audits.keys().next() {
        return Err(format!("fixture audit {selector} has no TeX82 fixture"));
    }
    Ok(records)
}

fn checked_selector(selector: &str) -> Result<String, String> {
    let Some(name) = selector.strip_prefix("tex82/") else {
        return Err(format!("fixture selector {selector} is not TeX82"));
    };
    if name.is_empty() || name.contains('/') || name.contains('\0') || name == "." || name == ".." {
        return Err(format!("fixture selector {selector} is unsafe"));
    }
    Ok(selector.into())
}

fn discover_fixture_selectors(root: &Path) -> Result<BTreeSet<String>, String> {
    let entries = fs::read_dir(root)
        .map_err(|error| format!("failed to read {}: {error}", root.display()))?;
    let mut selectors = BTreeSet::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("failed to read {}: {error}", root.display()))?;
        let kind = entry
            .file_type()
            .map_err(|error| format!("failed to inspect {}: {error}", entry.path().display()))?;
        if !kind.is_dir() || !entry.path().join("manifest.json").is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        selectors.insert(checked_selector(&format!("tex82/{name}"))?);
    }
    Ok(selectors)
}

fn semantic_seams(matrix: &[u8]) -> Result<Vec<String>, String> {
    let matrix =
        std::str::from_utf8(matrix).map_err(|_| "semantic matrix is not UTF-8".to_owned())?;
    let mut seams = BTreeSet::new();
    for (line_number, line) in matrix.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields = line.split('|').collect::<Vec<_>>();
        if fields.len() != 5 || fields[0].is_empty() || fields[1].is_empty() {
            return Err(format!(
                "semantic matrix line {} is malformed",
                line_number + 1
            ));
        }
        seams.insert(format!("{}/{}", fields[0], fields[1]));
    }
    if seams.is_empty() {
        return Err("semantic matrix contains no command seams".into());
    }
    Ok(seams.into_iter().collect())
}

#[allow(
    clippy::disallowed_methods,
    reason = "this host-only validator audits detached committed fixture files, not engine state"
)]
fn read(path: &Path) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|error| format!("failed to read {}: {error}", path.display()))
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{COMMITTED_TEX82_COMMAND_TRACE_EVENT_COUNT, validate_tex82_command_trace_suite};
    use std::path::PathBuf;

    #[test]
    fn committed_tex82_command_trace_suite_is_complete_and_offline() {
        let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let suite = validate_tex82_command_trace_suite(repository).expect("committed suite");
        // `umber2-alfh.2` split the former single 14-source `command-transitions-v1`
        // fixture into one minifixture per independent behavior (`fixtures`
        // sorts by selector, so alphabetically-first `alignment-delivery-v1`
        // is index 0); the input-stack/EOF-recovery seams that are inherently
        // about nested files stayed on the trimmed `command-transitions-v1`.
        assert_eq!(suite.fixtures.len(), 6);
        assert_eq!(suite.events, COMMITTED_TEX82_COMMAND_TRACE_EVENT_COUNT);
        assert_eq!(suite.fixtures[0].selector, "tex82/alignment-delivery-v1");
        assert!(
            suite.fixtures[0]
                .seams
                .iter()
                .any(|seam| seam == "alignment/u template push")
        );
        assert!(
            suite
                .fixtures
                .iter()
                .find(|fixture| fixture.selector == "tex82/command-transitions-v1")
                .expect("command-transitions-v1 fixture")
                .seams
                .iter()
                .any(|seam| seam == "input/source push")
        );
    }

    #[test]
    fn contract_requires_one_tex82_audit_per_fixture() {
        let error = super::parse_contract(
            b"fixture tex82/example tex82 initex-eight-bit tests/corpus/command/tex82/example/manifest.json a\n",
        )
        .expect_err("audit is required");
        assert_eq!(error, "TeX82 fixture tex82/example has no coverage audit");
    }
}
