#![allow(
    clippy::disallowed_methods,
    reason = "fixture bootstrap is host-only live-reference tooling, not engine I/O"
)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::{CommittedFixture, FixtureArtifact, FixtureManifest, ManifestInput, ObservationStream};

/// Builds a complete candidate fixture from one pinned TeX82 oracle run.
///
/// The input manifest supplies only stable selection metadata such as citations
/// and the canonical profile. Source, output, tool, engine-change, and stream
/// identities are re-derived from the live oracle artifacts before the
/// candidate is validated. `output` must be an empty staging directory.
pub fn bootstrap_tex82_fixture(
    template: &Path,
    live_directory: &Path,
    event_stream: &Path,
    build_record: &Path,
    output: &Path,
) -> Result<(), String> {
    ensure_empty_directory(output)?;
    let mut manifest = read_template(template)?;
    let record = BuildRecord::read(build_record)?;
    derive_oracle_identity(&mut manifest, &record)?;

    let sources = stage_sources(live_directory, output)?;
    manifest.oracle.inputs = sources
        .iter()
        .map(|(name, artifact)| {
            (
                name.clone(),
                ManifestInput {
                    sha256: artifact.sha256.clone(),
                    bytes: artifact.bytes,
                },
            )
        })
        .collect();
    manifest.sources = sources;

    let outputs = stage_outputs(live_directory, output, &manifest.outputs)?;
    manifest.oracle.ordinary_output_sha256 = outputs
        .iter()
        .map(|(channel, artifact)| (channel.clone(), artifact.sha256.clone()))
        .collect();
    manifest.outputs = outputs;

    let identity = manifest
        .oracle
        .identity()
        .map_err(|error| format!("derived oracle manifest is invalid: {error}"))?
        .hex();
    let events = bind_unbound_stream(event_stream, &identity)?;
    write_artifact(output, &manifest.events.path, &events)?;
    manifest.events = artifact(&manifest.events.path, &events);

    let manifest_bytes = manifest
        .to_canonical_json()
        .map_err(|error| format!("derived fixture manifest is invalid: {error}"))?;
    fs::write(output.join("manifest.json"), manifest_bytes)
        .map_err(|error| format!("failed to write candidate manifest: {error}"))?;
    CommittedFixture::load(output)
        .map_err(|error| format!("derived fixture candidate is invalid: {error}"))?;
    Ok(())
}

fn read_template(template: &Path) -> Result<FixtureManifest, String> {
    let bytes = fs::read(template.join("manifest.json"))
        .map_err(|error| format!("failed to read fixture template: {error}"))?;
    FixtureManifest::from_canonical_json(&bytes)
        .map_err(|error| format!("fixture template is invalid: {error}"))
}

fn ensure_empty_directory(directory: &Path) -> Result<(), String> {
    fs::create_dir_all(directory).map_err(|error| {
        format!(
            "failed to create candidate directory {}: {error}",
            directory.display()
        )
    })?;
    let mut entries = fs::read_dir(directory).map_err(|error| {
        format!(
            "failed to inspect candidate directory {}: {error}",
            directory.display()
        )
    })?;
    if entries
        .next()
        .transpose()
        .map_err(|error| {
            format!(
                "failed to inspect candidate directory {}: {error}",
                directory.display()
            )
        })?
        .is_some()
    {
        return Err(format!(
            "candidate directory {} is not empty",
            directory.display()
        ));
    }
    Ok(())
}

fn derive_oracle_identity(
    manifest: &mut FixtureManifest,
    record: &BuildRecord,
) -> Result<(), String> {
    let (web_source, changes) = record
        .ordered_changes
        .split_first()
        .ok_or_else(|| "oracle build record has no ordered changes".to_owned())?;
    if web_source.0 != "texk/web2c/tex.web" {
        return Err("oracle build record does not start with tex.web".into());
    }
    manifest.oracle.engine.web_source_sha256 = web_source.1.clone();
    manifest.oracle.engine.upstream_change_sha256 =
        changes.iter().map(|(_, digest)| digest.clone()).collect();
    manifest.oracle.engine.instrumentation_change_sha256 = record.instrumentation_change.clone();
    manifest.oracle.environment.insert(
        "source_manifest_sha256".into(),
        record.source_manifest.clone(),
    );
    manifest.oracle.source_date_epoch = record.source_date_epoch;
    for tool in ["tie", "tangle", "web2c"] {
        let digest = record
            .tools
            .get(tool)
            .ok_or_else(|| format!("oracle build record has no {tool} identity"))?;
        let entry = manifest
            .tools
            .get_mut(tool)
            .ok_or_else(|| format!("fixture template has no {tool} tool identity"))?;
        entry.sha256 = digest.clone();
    }
    Ok(())
}

fn stage_sources(
    live_directory: &Path,
    output: &Path,
) -> Result<BTreeMap<String, FixtureArtifact>, String> {
    let mut sources = BTreeMap::new();
    for entry in read_directory(live_directory)? {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".tex") || !entry.file_type().map_err(io_error)?.is_file() {
            continue;
        }
        let bytes = fs::read(entry.path()).map_err(io_error)?;
        let path = format!("sources/{name}");
        write_artifact(output, &path, &bytes)?;
        sources.insert(name, artifact(&path, &bytes));
    }
    if sources.is_empty() {
        return Err(format!(
            "oracle run {} did not stage focused TeX sources",
            live_directory.display()
        ));
    }
    Ok(sources)
}

fn stage_outputs(
    live_directory: &Path,
    output: &Path,
    template: &BTreeMap<String, FixtureArtifact>,
) -> Result<BTreeMap<String, FixtureArtifact>, String> {
    let mut outputs = BTreeMap::new();
    for (channel, template_artifact) in template {
        let file_name = Path::new(&template_artifact.path)
            .file_name()
            .ok_or_else(|| format!("output {channel} has no file name"))?;
        let live_path = live_directory.join(file_name);
        let bytes = fs::read(&live_path).map_err(|error| {
            format!(
                "failed to read oracle output {} for {channel}: {error}",
                live_path.display()
            )
        })?;
        write_artifact(output, &template_artifact.path, &bytes)?;
        outputs.insert(channel.clone(), artifact(&template_artifact.path, &bytes));
    }
    Ok(outputs)
}

fn bind_unbound_stream(event_stream: &Path, identity: &str) -> Result<Vec<u8>, String> {
    let bytes = fs::read(event_stream).map_err(|error| {
        format!(
            "failed to read live semantic stream {}: {error}",
            event_stream.display()
        )
    })?;
    let stream = ObservationStream::from_canonical_json_lines(&bytes)
        .map_err(|error| format!("live semantic stream is invalid: {error}"))?;
    if stream.header.manifest != "0".repeat(64) {
        return Err(
            "live semantic stream is not marked with the unbound zero manifest sentinel".into(),
        );
    }
    let newline = bytes
        .iter()
        .position(|byte| *byte == b'\n')
        .ok_or_else(|| "live semantic stream has no header line".to_owned())?;
    let mut bound = format!("{{\"schema\":1,\"manifest\":\"{identity}\"}}\n").into_bytes();
    bound.extend_from_slice(&bytes[newline + 1..]);
    ObservationStream::from_canonical_json_lines(&bound)
        .map_err(|error| format!("bound semantic stream is invalid: {error}"))?;
    Ok(bound)
}

fn write_artifact(output: &Path, path: &str, bytes: &[u8]) -> Result<(), String> {
    let destination = output.join(path);
    let parent = destination
        .parent()
        .ok_or_else(|| format!("artifact {path} has no parent directory"))?;
    fs::create_dir_all(parent).map_err(io_error)?;
    fs::write(destination, bytes).map_err(io_error)
}

fn artifact(path: &str, bytes: &[u8]) -> FixtureArtifact {
    FixtureArtifact {
        path: path.into(),
        bytes: bytes.len() as u64,
        sha256: hex_hash(bytes),
    }
}

fn read_directory(directory: &Path) -> Result<Vec<fs::DirEntry>, String> {
    let mut entries = fs::read_dir(directory)
        .map_err(|error| {
            format!(
                "failed to read oracle directory {}: {error}",
                directory.display()
            )
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(io_error)?;
    entries.sort_by_key(|entry| entry.file_name());
    Ok(entries)
}

fn hex_hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn io_error(error: std::io::Error) -> String {
    error.to_string()
}

struct BuildRecord {
    source_manifest: String,
    source_date_epoch: i64,
    instrumentation_change: String,
    ordered_changes: Vec<(String, String)>,
    tools: BTreeMap<String, String>,
}

impl BuildRecord {
    fn read(path: &Path) -> Result<Self, String> {
        let content = fs::read_to_string(path).map_err(|error| {
            format!(
                "failed to read oracle build record {}: {error}",
                path.display()
            )
        })?;
        let mut source_manifest = None;
        let mut source_date_epoch = None;
        let mut instrumentation_change = None;
        let mut ordered_changes = Vec::new();
        let mut tools = BTreeMap::new();
        for line in content.lines() {
            let fields = line.split_ascii_whitespace().collect::<Vec<_>>();
            match fields.as_slice() {
                ["manifest-sha256", digest] => source_manifest = Some((*digest).into()),
                ["source-date-epoch", epoch] => {
                    source_date_epoch = Some(epoch.parse().map_err(|_| {
                        format!("oracle build record has invalid source-date-epoch {epoch}")
                    })?)
                }
                ["instrumentation-change-sha256", digest] => {
                    instrumentation_change = Some((*digest).into())
                }
                ["ordered-change-sha256", path, digest] => {
                    ordered_changes.push(((*path).into(), (*digest).into()))
                }
                ["tool-sha256", tool, digest] => {
                    tools.insert((*tool).into(), (*digest).into());
                }
                _ => {}
            }
        }
        Ok(Self {
            source_manifest: source_manifest
                .ok_or_else(|| "oracle build record has no manifest-sha256".to_owned())?,
            source_date_epoch: source_date_epoch
                .ok_or_else(|| "oracle build record has no source-date-epoch".to_owned())?,
            instrumentation_change: instrumentation_change.ok_or_else(|| {
                "oracle build record has no instrumentation-change-sha256".to_owned()
            })?,
            ordered_changes,
            tools,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::{
        CanonicalCitation, EngineDialect, EngineIdentity, FixtureArtifact, FixtureManifest,
        FixtureProfile, JsonLinesObserver, Manifest, ToolIdentity,
    };

    use super::bootstrap_tex82_fixture;

    const HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    static TEMPORARY_SEQUENCE: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn bootstrap_derives_a_complete_manifest_bound_candidate() {
        let temporary = std::env::temp_dir().join(format!(
            "umber-tex-oracle-bootstrap-{}-{}",
            std::process::id(),
            TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let template = temporary.join("template");
        let live = temporary.join("live");
        let output = temporary.join("candidate");
        fs::create_dir_all(&template).expect("template directory");
        fs::create_dir_all(&live).expect("live directory");
        fs::write(live.join("transitions.tex"), b"\\bye\n").expect("source");
        fs::write(live.join("status.txt"), b"0\n").expect("status");

        let mut oracle = Manifest::new(EngineIdentity {
            dialect: EngineDialect::Tex82,
            banner: "TeX, Version 3.141592653".into(),
            web_source_sha256: HASH.into(),
            upstream_change_sha256: vec![HASH.into()],
            instrumentation_change_sha256: HASH.into(),
        });
        oracle.clock = "2026-07-09T13:36:00Z".into();
        oracle.distribution_sha256 = HASH.into();
        oracle.environment.insert("locale".into(), "C".into());
        oracle.inputs.insert(
            "old.tex".into(),
            crate::ManifestInput {
                bytes: 0,
                sha256: HASH.into(),
            },
        );
        oracle
            .ordinary_output_sha256
            .insert("status".into(), HASH.into());
        let manifest = FixtureManifest {
            contract: 1,
            name: "tex82/synthetic".into(),
            profile: FixtureProfile {
                invocation: "initex".into(),
                characters: "eight_bit_exact".into(),
            },
            oracle,
            tools: BTreeMap::from([
                ("tie".into(), tool()),
                ("tangle".into(), tool()),
                ("web2c".into(), tool()),
            ]),
            citations: vec![CanonicalCitation {
                source: "tex.web".into(),
                section: "get_next".into(),
                description: "delivery".into(),
            }],
            sources: BTreeMap::from([("old.tex".into(), artifact("sources/old.tex"))]),
            events: artifact("events.jsonl"),
            outputs: BTreeMap::from([("status".into(), artifact("outputs/status.txt"))]),
        };
        fs::write(
            template.join("manifest.json"),
            manifest.to_canonical_json().expect("template manifest"),
        )
        .expect("write template");

        let identity = manifest.oracle.identity().expect("identity");
        let observer = JsonLinesObserver::new(Vec::new(), identity).expect("observer");
        let (events, _) = observer.finish().expect("stream");
        let unbound = String::from_utf8(events).expect("utf8 stream").replacen(
            &identity.hex(),
            &"0".repeat(64),
            1,
        );
        let events_path = temporary.join("live-events.jsonl");
        fs::write(&events_path, unbound).expect("live events");
        let record = temporary.join("build-record.txt");
        fs::write(
            &record,
            format!(
                "manifest-sha256 {HASH}\nsource-date-epoch 1783604160\ninstrumentation-change-sha256 {HASH}\nordered-change-sha256 texk/web2c/tex.web {HASH}\nordered-change-sha256 texk/web2c/tex.ch {HASH}\ntool-sha256 tie {HASH}\ntool-sha256 tangle {HASH}\ntool-sha256 web2c {HASH}\n"
            ),
        )
        .expect("record");

        bootstrap_tex82_fixture(&template, &live, &events_path, &record, &output)
            .expect("candidate");
        let candidate = crate::CommittedFixture::load(&output).expect("validated candidate");
        assert_eq!(candidate.manifest.sources.len(), 1);
        assert_eq!(candidate.manifest.outputs["status"].bytes, 2);
        assert_ne!(candidate.stream.header.manifest, "0".repeat(64));
        fs::remove_dir_all(temporary).expect("remove temporary directory");
    }

    fn tool() -> ToolIdentity {
        ToolIdentity {
            version: "pinned".into(),
            sha256: HASH.into(),
        }
    }

    fn artifact(path: &str) -> FixtureArtifact {
        FixtureArtifact {
            path: path.into(),
            bytes: 0,
            sha256: HASH.into(),
        }
    }
}
