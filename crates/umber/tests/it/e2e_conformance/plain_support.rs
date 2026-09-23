//! Shared plain support for the conformance families.

use super::*;

pub(super) const PLAIN_CLOCK: JobClock = JobClock {
    time: 13 * 60 + 36,
    second: 0,
    day: 9,
    month: 7,
    year: 2026,
};

pub(super) fn plain_guards() -> FormatGenerationGuards {
    FormatGenerationGuards {
        command_fuel: tex_command::DEFAULT_COMMAND_FUEL_LIMIT,
        wall_time: Duration::from_secs(1_800),
        resident_bytes: 6 * 1024 * 1024 * 1024,
    }
}

#[allow(clippy::disallowed_methods)] // Reads only repository-pinned construction inputs.
pub(super) fn plain_format_recipe(repo_root: &Path) -> Result<FormatRecipe, String> {
    let read = |relative: &str| {
        fs::read(repo_root.join(relative))
            .map_err(|error| format!("read pinned Plain construction resource {relative}: {error}"))
    };
    let mut resources = vec![
        FormatResource::Input {
            logical_name: "plain.tex".into(),
            source_kind: RegisteredSourceKind::Generated,
            bytes: read("third_party/corpus/plain.tex")?,
        },
        FormatResource::Input {
            logical_name: "hyphen.tex".into(),
            source_kind: RegisteredSourceKind::Generated,
            bytes: read("third_party/hyphen/hyphen.tex")?,
        },
    ];
    for name in parity_harness::PLAIN_PRELOAD_FONTS {
        resources.push(FormatResource::Tfm {
            logical_name: format!("{name}.tfm"),
            bytes: read(&format!("third_party/fonts/{name}.tfm"))?,
        });
    }
    Ok(FormatRecipe {
        engine: EngineMode::Tex82,
        hyphenation_exception_capacity: 307,
        format_name: "repository-plain-tex82".into(),
        format_ident_name: "repository-plain-tex82".into(),
        construction_source_name: "repository-plain-tex82.ini".into(),
        construction_source: b"\\input plain.tex\n\\dump\n".to_vec(),
        resources,
        distribution_identity: b"repository-pinned-plain-tex82-v1".to_vec(),
        clock: PLAIN_CLOCK,
        construction_interaction: tex_state::InteractionMode::Nonstop,
        construction_error_context_widths: tex_state::print::ErrorContextWidths::new(64, 32)
            .expect("canonical Plain context widths"),
        guards: plain_guards(),
    })
}

pub(super) struct PlainJobInput {
    source_name: String,
    pub(super) source: Vec<u8>,
    pub(super) resources: Vec<LoadedFormatResource>,
}

#[allow(clippy::disallowed_methods)] // Acquires one isolated staged job into typed values.
pub(super) fn plain_job_input(path: &Path) -> Result<PlainJobInput, String> {
    let path = path
        .canonicalize()
        .map_err(|error| format!("resolve {}: {error}", path.display()))?;
    let parent = path
        .parent()
        .ok_or_else(|| format!("input has no parent: {}", path.display()))?;
    let source_name = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .ok_or_else(|| format!("input name is not UTF-8: {}", path.display()))?
        .to_owned();
    let source = fs::read(&path).map_err(|error| format!("read {}: {error}", path.display()))?;
    let source = source
        .strip_prefix(b"\\input plain.tex\n")
        .unwrap_or(&source)
        .to_vec();
    let mut entries = fs::read_dir(parent)
        .map_err(|error| format!("read staged directory {}: {error}", parent.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("read staged directory entry: {error}"))?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    let mut resources = Vec::new();
    for entry in entries {
        if !entry
            .file_type()
            .map_err(|error| format!("inspect {}: {error}", entry.path().display()))?
            .is_file()
            || entry.path() == path
        {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == "plain.tex" || name == "hyphen.tex" {
            continue;
        }
        let bytes = fs::read(entry.path())
            .map_err(|error| format!("read {}: {error}", entry.path().display()))?;
        if let Some(font) = name.strip_suffix(".tfm") {
            if !parity_harness::PLAIN_PRELOAD_FONTS.contains(&font) {
                resources.push(LoadedFormatResource::Tfm {
                    logical_name: name,
                    bytes,
                });
            }
        } else {
            resources.push(LoadedFormatResource::Input {
                logical_name: name.clone(),
                resolved_name: format!("./{name}"),
                source_kind: RegisteredSourceKind::Generated,
                bytes,
            });
        }
    }
    Ok(PlainJobInput {
        source_name,
        source,
        resources,
    })
}

pub(super) fn run_file_with_plain_format(path: &Path) -> Result<InProcessRun, String> {
    let repo_root = test_support::repository_root();
    let recipe = plain_format_recipe(&repo_root)?;
    let provider = PreparedFormatProvider::from_environment(super::umber_format_worker_launcher())
        .map_err(|error| format!("Plain persistent format provider failed: {error}"))?;
    let prepared = provider
        .prepare(&recipe)
        .map_err(|error| format!("Plain format preparation failed: {error}"))?;
    let input = plain_job_input(path)?;
    let source_name = input.source_name.clone();
    let source = input.source.clone();
    let mut observers = CapturedObservations::default();
    let loaded = provider
        .run(
            &prepared,
            PreparedFormatJob {
                engine: EngineMode::Tex82,
                engine_binary: tex_exec::EngineBinaryIdentity::Tex82,
                backend: OutputCapability::Dvi,
                clock: PLAIN_CLOCK,
                interaction: tex_state::InteractionMode::Nonstop,
                error_context_widths: tex_state::print::ErrorContextWidths::new(64, 32)
                    .expect("canonical Plain context widths"),
                provenance_demand: ProvenanceDemand::DIAGNOSTICS_AND_RENDERED_SOURCE,
                guards: plain_guards(),
                startup_line: source_name.clone(),
                source_name: source_name.clone(),
                source_kind: RegisteredSourceKind::Generated,
                source: source.clone(),
                resources: input.resources,
                terminal_input: Vec::new(),
                projection: LoadedFormatProjectionDemand {
                    channels: true,
                    ..LoadedFormatProjectionDemand::default()
                },
                observer: &mut observers,
            },
        )
        .map_err(|error| format!("Plain loaded job failed: {error}"))?;
    let dvi = (!loaded.result.dvi_pages.is_empty())
        .then(|| dvi_from_page_plans(&loaded.result.dvi_pages))
        .transpose()
        .map_err(|error| error.to_string())?;
    let channels = loaded
        .projection
        .channels
        .expect("Plain job channel projection");
    Ok(InProcessRun {
        dvi,
        terminal: channels.terminal,
        log: channels.log,
        capture: PhaseCapture::Live(LiveCapture {
            root: LiveSource {
                name: source_name,
                source: tex_state::SourceId::new(0),
                bytes: Arc::<[u8]>::from(source).into(),
            },
            observations: observers.into_captured(),
            outcome: LiveSessionOutcome::Completed,
        }),
    })
}

#[allow(clippy::disallowed_methods)] // Acquires one isolated staged raw TeX82 job.
pub(super) fn run_file_with_raw_tex82_format(path: &Path) -> Result<InProcessRun, String> {
    let path = path
        .canonicalize()
        .map_err(|error| format!("resolve {}: {error}", path.display()))?;
    let parent = path
        .parent()
        .ok_or_else(|| format!("input has no parent: {}", path.display()))?;
    let source_name = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .ok_or_else(|| format!("input name is not UTF-8: {}", path.display()))?
        .to_owned();
    let source = fs::read(&path).map_err(|error| format!("read {}: {error}", path.display()))?;
    let mut entries = fs::read_dir(parent)
        .map_err(|error| format!("read staged directory {}: {error}", parent.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("read staged directory entry: {error}"))?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    let mut resources = Vec::new();
    for entry in entries {
        if !entry
            .file_type()
            .map_err(|error| format!("inspect {}: {error}", entry.path().display()))?
            .is_file()
            || entry.path() == path
        {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let bytes = fs::read(entry.path())
            .map_err(|error| format!("read {}: {error}", entry.path().display()))?;
        if name.ends_with(".tfm") {
            resources.push(LoadedFormatResource::Tfm {
                logical_name: name,
                bytes,
            });
        } else if name.ends_with(".tex") || name.ends_with(".inc") {
            resources.push(LoadedFormatResource::Input {
                logical_name: name.clone(),
                resolved_name: format!("./{name}"),
                source_kind: RegisteredSourceKind::Generated,
                bytes,
            });
        }
    }
    let recipe = FormatRecipe::raw_tex82();
    let provider = PreparedFormatProvider::from_environment(super::umber_format_worker_launcher())
        .map_err(|error| format!("raw TeX82 persistent format provider failed: {error}"))?;
    let prepared = provider
        .prepare(&recipe)
        .map_err(|error| format!("raw TeX82 format preparation failed: {error}"))?;
    let mut observers = CapturedObservations::default();
    let loaded = provider
        .run(
            &prepared,
            PreparedFormatJob {
                engine: EngineMode::Tex82,
                engine_binary: tex_exec::EngineBinaryIdentity::Tex82,
                backend: OutputCapability::Dvi,
                clock: PLAIN_CLOCK,
                interaction: tex_state::InteractionMode::Nonstop,
                error_context_widths: tex_state::print::ErrorContextWidths::new(64, 32)
                    .expect("canonical raw TeX82 context widths"),
                provenance_demand: ProvenanceDemand::DIAGNOSTICS_AND_RENDERED_SOURCE,
                guards: plain_guards(),
                startup_line: source_name.clone(),
                source_name: source_name.clone(),
                source_kind: RegisteredSourceKind::Generated,
                source: source.clone(),
                resources,
                terminal_input: Vec::new(),
                projection: LoadedFormatProjectionDemand {
                    channels: true,
                    ..LoadedFormatProjectionDemand::default()
                },
                observer: &mut observers,
            },
        )
        .map_err(|error| format!("raw TeX82 loaded job failed: {error}"))?;
    let dvi = (!loaded.result.dvi_pages.is_empty())
        .then(|| dvi_from_page_plans(&loaded.result.dvi_pages))
        .transpose()
        .map_err(|error| error.to_string())?;
    let channels = loaded
        .projection
        .channels
        .expect("raw TeX82 job channel projection");
    Ok(InProcessRun {
        dvi,
        terminal: channels.terminal,
        log: channels.log,
        capture: PhaseCapture::Live(LiveCapture {
            root: LiveSource {
                name: source_name,
                source: tex_state::SourceId::new(0),
                bytes: Arc::<[u8]>::from(source).into(),
            },
            observations: observers.into_captured(),
            outcome: LiveSessionOutcome::Completed,
        }),
    })
}
