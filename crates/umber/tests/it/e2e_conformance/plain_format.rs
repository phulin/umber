//! Pinned Plain construction, staged job resources, and provider reuse.

use super::*;

#[test]
#[allow(clippy::disallowed_methods)] // Builds one isolated host-side staged job.
fn plain_job_split_types_non_preload_resources() {
    let temp = tempfile::tempdir().expect("temporary staged job");
    fs::write(
        temp.path().join("texput.tex"),
        b"\\input plain.tex\n\\input support.tex\n",
    )
    .expect("root");
    fs::write(temp.path().join("plain.tex"), b"format-only").expect("plain");
    fs::write(temp.path().join("hyphen.tex"), b"format-only").expect("hyphen");
    fs::write(temp.path().join("cmr10.tfm"), b"preloaded").expect("preloaded tfm");
    fs::write(temp.path().join("extra.tfm"), b"job tfm").expect("job tfm");
    fs::write(temp.path().join("support.tex"), b"job input").expect("job input");
    let input = plain_job_input(&temp.path().join("texput.tex")).expect("typed Plain job");
    assert_eq!(input.source.as_slice(), b"\\input support.tex\n");
    assert_eq!(
        input.resources,
        vec![
            LoadedFormatResource::Tfm {
                logical_name: "extra.tfm".into(),
                bytes: b"job tfm".to_vec(),
            },
            LoadedFormatResource::Input {
                logical_name: "support.tex".into(),
                resolved_name: "./support.tex".into(),
                source_kind: RegisteredSourceKind::Generated,
                bytes: b"job input".to_vec(),
            },
        ]
    );
}

#[test]
fn plain_provider_reuses_one_verified_construction_with_fresh_jobs() {
    let repo_root = test_support::repository_root();
    let recipe = plain_format_recipe(&repo_root).expect("complete Plain recipe");
    let cache = tempfile::tempdir().expect("isolated persistent Plain cache");
    let launcher = super::umber_format_worker_launcher();
    let first_provider =
        PreparedFormatProvider::with_store(FormatCacheStore::new(cache.path()), launcher.clone());
    let first = first_provider
        .prepare(&recipe)
        .expect("cold Plain preparation");
    let second_provider =
        PreparedFormatProvider::with_store(FormatCacheStore::new(cache.path()), launcher);
    let second = second_provider
        .prepare(&recipe)
        .expect("independent warm Plain preparation");
    assert_eq!(
        recipe.identity().expect("Plain identity").key(),
        plain_format_recipe(&repo_root)
            .expect("same Plain route recipe")
            .identity()
            .expect("same Plain identity")
            .key()
    );
    assert_eq!(first.image(), second.image());
    assert_eq!(
        first.construction_evidence(),
        second.construction_evidence()
    );
    assert_eq!(
        fs::read_dir(cache.path().join("blobs-v2"))
            .expect("Plain provider namespace")
            .filter_map(Result::ok)
            .filter(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with("ahash64-v1-"))
            .count(),
        1,
        "all Plain routes must publish exactly one construction identity"
    );
    for (source, expected) in [
        (b"\\count0=41\\end\n".as_slice(), 41),
        (b"\\end\n".as_slice(), 1),
    ] {
        let mut observer = CapturedObservations::default();
        let run = second_provider
            .run(
                &second,
                PreparedFormatJob {
                    engine: EngineMode::Tex82,
                    engine_binary: tex_exec::EngineBinaryIdentity::Tex82,
                    backend: OutputCapability::Dvi,
                    clock: PLAIN_CLOCK,
                    interaction: tex_state::InteractionMode::Nonstop,
                    error_context_widths: recipe.construction_error_context_widths,
                    provenance_demand: ProvenanceDemand::DIAGNOSTICS,
                    guards: plain_guards(),
                    startup_line: "plain-provider-isolation.tex".into(),
                    source_name: "plain-provider-isolation.tex".into(),
                    source_kind: RegisteredSourceKind::Generated,
                    source: source.to_vec(),
                    resources: Vec::new(),
                    terminal_input: Vec::new(),
                    projection: LoadedFormatProjectionDemand {
                        count_registers: vec![0],
                        ..LoadedFormatProjectionDemand::default()
                    },
                    observer: &mut observer,
                },
            )
            .expect("fresh Plain loaded job");
        assert_eq!(run.projection.counts, [(0, expected)]);
        assert!(!observer.into_captured().is_empty());
    }
}
