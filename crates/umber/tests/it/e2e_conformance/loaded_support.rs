//! Shared loaded support for the conformance families.

use super::*;

pub(super) fn run_focused_loaded_trip_through(last_source_line: usize) -> String {
    let trip: Arc<[u8]> = Arc::from(
        test_support::read_repository_asset("third_party/trip/trip.tex").expect("read TRIP source"),
    );
    let text = std::str::from_utf8(&trip).expect("TRIP source is UTF-8");
    let lines = text.lines().collect::<Vec<_>>();
    let source: Arc<[u8]> =
        Arc::from(format!("{}\n\\end\n", lines[92..last_source_line].join("\n")).into_bytes());
    run_loaded_trip_source(source)
}

pub(super) fn run_loaded_trip_source(source: Arc<[u8]>) -> String {
    run_loaded_trip_source_observed(source).0
}

pub(super) fn positionless_geometry(observer: TripObservers, oracle: &[u8]) -> Vec<u8> {
    let mut translator = LiveSessionTranslator::new("terminal", SchemaVersion::V2);
    translator.translate_captured(observer.into_captured());
    let evidence = translator.finalize_profile(
        SemanticEvidenceProfile::Complete,
        GeometryEvidenceProfile::Positionless,
    );
    tex_oracle::canonical_bundle_json_lines(&evidence.geometry, oracle)
        .expect("focused geometry stream")
}

pub(super) fn run_loaded_trip_source_observed(source: Arc<[u8]>) -> (String, TripObservers) {
    let trip =
        test_support::read_repository_asset("third_party/trip/trip.tex").expect("read TRIP source");
    let tripos = test_support::read_repository_asset("third_party/trip/tripos.tex")
        .expect("read TRIP terminal input");
    let tfm = test_support::read_repository_asset("third_party/trip/trip.tfm")
        .expect("read TRIP font metrics");
    let recipe = trip_format_recipe(
        TripEngineProfile::Tex82,
        "trip",
        "trip.tex",
        trip,
        tripos.clone(),
        tfm.clone(),
    );
    let provider = PreparedFormatProvider::from_environment(super::umber_format_worker_launcher())
        .expect("focused TRIP format provider");
    let prepared = provider.prepare(&recipe).expect("focused TRIP format");
    let mut observer = TripObservers::default();
    let loaded = provider
        .run(
            &prepared,
            PreparedFormatJob {
                engine: EngineMode::Tex82,
                engine_binary: EngineMode::Tex82.binary_identity(),
                backend: OutputCapability::Dvi,
                clock: recipe.clock,
                interaction: tex_state::InteractionMode::Nonstop,
                error_context_widths: recipe.construction_error_context_widths,
                provenance_demand: ProvenanceDemand::DIAGNOSTICS,
                guards: recipe.guards,
                startup_line: "&trip focused.tex".into(),
                source_name: "focused.tex".into(),
                source_kind: RegisteredSourceKind::Generated,
                source: source.to_vec(),
                resources: vec![
                    LoadedFormatResource::Input {
                        logical_name: "tripos.tex".into(),
                        resolved_name: "./tripos.tex".into(),
                        source_kind: RegisteredSourceKind::Generated,
                        bytes: tripos,
                    },
                    LoadedFormatResource::Tfm {
                        logical_name: "trip.tfm".into(),
                        bytes: tfm,
                    },
                ],
                terminal_input: Vec::new(),
                projection: LoadedFormatProjectionDemand {
                    channels: true,
                    ..LoadedFormatProjectionDemand::default()
                },
                observer: &mut observer,
            },
        )
        .expect("focused loaded TRIP run");
    let channels = loaded
        .projection
        .channels
        .expect("focused TRIP channel projection");
    let (_, log) =
        append_transcript_suffix(channels.terminal, channels.log, &channels.pending_effects);
    (String::from_utf8(log).expect("TRIP log is UTF-8"), observer)
}
