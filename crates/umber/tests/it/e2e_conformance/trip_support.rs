//! Shared trip support for the conformance families.

use super::*;

pub(super) fn target_dir(repo_root: &Path) -> PathBuf {
    env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .map_or_else(
            || repo_root.join("target"),
            |path| {
                if path.is_absolute() {
                    path
                } else {
                    repo_root.join(path)
                }
            },
        )
}

pub(super) struct InProcessRun {
    pub(super) dvi: Option<Vec<u8>>,
    pub(super) terminal: Vec<u8>,
    pub(super) log: Vec<u8>,
    pub(super) capture: PhaseCapture,
}

pub(super) enum PhaseCapture {
    Live(LiveCapture),
    Detached(tex_oracle::OracleBundle),
}

impl PhaseCapture {
    pub(super) fn command(&self, fixture_name: &str, phase: &str, oracle: &[u8]) -> Vec<u8> {
        command_stream_for_fixture_phase(fixture_name, phase, self.streams(oracle))
    }

    pub(super) fn streams(&self, oracle: &[u8]) -> LiveSessionStreams {
        match self {
            Self::Live(capture) => capture.streams(oracle),
            Self::Detached(evidence) => {
                let diagnostic =
                    tex_oracle::canonical_bundle_json_lines(&evidence.semantic, oracle)
                        .expect("construction semantic evidence encodes under oracle header");
                LiveSessionStreams {
                    diagnostic: diagnostic.clone(),
                    stable: diagnostic,
                }
            }
        }
    }

    pub(super) fn geometry(&self, oracle: &[u8]) -> Vec<u8> {
        match self {
            Self::Live(capture) => capture.geometry(oracle),
            Self::Detached(evidence) => {
                tex_oracle::canonical_bundle_json_lines(&evidence.geometry, oracle)
                    .expect("construction geometry evidence encodes under oracle header")
            }
        }
    }
}

pub(super) struct LiveCapture {
    pub(super) root: LiveSource,
    pub(super) observations: Vec<tex_command::CommandObservation>,
    pub(super) outcome: LiveSessionOutcome,
}

pub(super) fn trip_geometry_profile(schema: SchemaVersion) -> GeometryEvidenceProfile {
    if schema >= SchemaVersion::V3 {
        GeometryEvidenceProfile::Located
    } else {
        GeometryEvidenceProfile::Positionless
    }
}

pub(super) fn command_stream_for_fixture_phase(
    fixture_name: &str,
    phase: &str,
    streams: LiveSessionStreams,
) -> Vec<u8> {
    if fixture_name == "trip" && phase == "format-loaded" {
        streams.stable
    } else {
        streams.diagnostic
    }
}

impl LiveCapture {
    pub(super) fn streams(&self, oracle: &[u8]) -> LiveSessionStreams {
        let header = ObservationStream::from_canonical_json_lines(oracle)
            .expect("oracle stream validates")
            .header;
        let mut translator =
            LiveSessionTranslator::for_root(SchemaVersion::V1, "terminal", self.root.clone());
        translator.translate_captured(self.observations.clone());
        translator
            .finish(header, self.outcome.clone())
            .expect("live observations translate")
    }

    pub(super) fn geometry(&self, oracle: &[u8]) -> Vec<u8> {
        let header = ObservationStream::from_canonical_json_lines(oracle)
            .expect("oracle geometry stream validates")
            .header;
        let schema = SchemaVersion::try_from(header.schema).expect("supported geometry schema");
        let geometry_profile = trip_geometry_profile(schema);
        let mut translator = LiveSessionTranslator::for_root(schema, "terminal", self.root.clone());
        translator.translate_captured(self.observations.iter().cloned());
        tex_oracle::canonical_bundle_json_lines(
            &translator
                .finalize_profile(SemanticEvidenceProfile::Complete, geometry_profile)
                .geometry,
            oracle,
        )
        .expect("geometry observations translate")
    }
}

pub(super) fn transcript_channels<G>(
    stores: &tex_state::Universe<G>,
    effects: &[EffectRecord],
) -> (Vec<u8>, Vec<u8>) {
    // A shipout commits and drains the live effect prefix into the memory
    // backend. `RunResult::effects` consequently contains only the suffix
    // after the last commit. TeX82 §§61, 536, 638, and 1333 still define one
    // ordered terminal/log episode, so parity evidence must join the already
    // committed prefix to that pending suffix rather than treating the suffix
    // as the whole transcript.
    let terminal = stores
        .world()
        .memory_terminal_output()
        .expect("prepared-format jobs use a memory World")
        .to_vec();
    let log = stores
        .world()
        .memory_log_output()
        .expect("prepared-format jobs use a memory World")
        .to_vec();
    append_transcript_suffix(terminal, log, effects)
}

pub(super) fn append_transcript_suffix(
    mut terminal: Vec<u8>,
    mut log: Vec<u8>,
    effects: &[EffectRecord],
) -> (Vec<u8>, Vec<u8>) {
    for effect in effects {
        let EffectRecord::StreamWrite { sink, text } = effect else {
            continue;
        };
        match sink {
            PrintSink::Terminal => terminal.extend_from_slice(text.as_bytes()),
            PrintSink::Log => log.extend_from_slice(text.as_bytes()),
            PrintSink::TerminalAndLog => {
                terminal.extend_from_slice(text.as_bytes());
                log.extend_from_slice(text.as_bytes());
            }
            PrintSink::Stream(_) => {}
        }
    }
    (terminal, log)
}

pub(super) fn startup_input_name(canonical_source_name: &str) -> String {
    format!("./{canonical_source_name}")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TripEngineProfile {
    Tex82,
    ETex,
}

impl TripEngineProfile {
    pub(super) fn recipe(self) -> FormatRecipe {
        match self {
            Self::Tex82 => FormatRecipe::raw_tex82(),
            Self::ETex => FormatRecipe::raw_etex26(),
        }
    }

    pub(super) fn format_name(self) -> &'static str {
        match self {
            Self::Tex82 => "umber-tex82-oracle",
            Self::ETex => "umber-etex26-extended-oracle-clean",
        }
    }
}

pub(super) fn trip_format_recipe(
    profile: TripEngineProfile,
    fixture_name: &str,
    source_name: &str,
    source: Vec<u8>,
    tripos: Vec<u8>,
    tfm: Vec<u8>,
) -> FormatRecipe {
    let mut recipe = profile.recipe();
    // Knuth's TRIP build deliberately selects this non-production `hyph_size`.
    recipe.hyphenation_exception_capacity = 659;
    recipe.format_name = profile.format_name().into();
    // TeX82 §1328 persists the dump job name independently of web2c §61's
    // selected `dump_name` used by the terminal banner.
    recipe.format_ident_name = fixture_name.to_owned();
    recipe.construction_source_name = source_name.to_owned();
    recipe.construction_source = source;
    recipe.resources = vec![
        FormatResource::Input {
            logical_name: "tripos.tex".into(),
            source_kind: RegisteredSourceKind::Generated,
            bytes: tripos,
        },
        FormatResource::Tfm {
            logical_name: format!("{fixture_name}.tfm"),
            bytes: tfm,
        },
    ];
    recipe.distribution_identity = b"pinned-trip-public-format-boundary-v2".to_vec();
    recipe.clock = JobClock {
        time: 13 * 60 + 36,
        second: 0,
        day: 9,
        month: 7,
        year: 2026,
    };
    recipe.construction_interaction = tex_state::InteractionMode::Nonstop;
    recipe.construction_error_context_widths = tex_state::print::ErrorContextWidths::new(64, 32)
        .and_then(|widths| widths.with_max_print_line(72))
        .expect("canonical TRIP print widths");
    recipe.guards = FormatGenerationGuards {
        command_fuel: tex_command::DEFAULT_COMMAND_FUEL_LIMIT,
        wall_time: Duration::from_secs(1_800),
        resident_bytes: 6 * 1024 * 1024 * 1024,
    };
    recipe
}
