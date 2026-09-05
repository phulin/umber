use super::*;

fn identity() -> PrefetchIdentity {
    PrefetchIdentity::new("pdftex", "latex", "pdf=true", "root-a", "v2").expect("identity")
}

#[test]
fn readiness_prioritizes_absence_then_admission() {
    assert_eq!(
        Readiness::from_evidence(true, true, true),
        Readiness::Absent
    );
    assert_eq!(
        Readiness::from_evidence(true, true, false),
        Readiness::Ready
    );
    assert_eq!(
        Readiness::from_evidence(true, false, false),
        Readiness::ExistsNotReady
    );
}

#[test]
fn accepted_manifest_round_trips_and_promotes_roles() {
    let resolved = ResolvedIdentity::new(
        "tex:foo",
        Some("/texlive/foo.tex".to_owned()),
        "ahash64-v1-0123456789abcdef",
        "0123456789abcdef",
        7,
    )
    .expect("resolved identity");
    let record = |role| {
        LookupRecord::new(
            "foo",
            "tex:foo",
            "tex",
            "user-before-distribution",
            role,
            LookupOutcome::Resolved(resolved.clone()),
        )
        .expect("record")
    };
    let mut manifest = LookupManifest::new(identity());
    manifest.record(record(LookupRole::Hint)).expect("hint");
    manifest
        .record(record(LookupRole::Required))
        .expect("promotion");
    manifest
        .record(
            LookupRecord::new(
                "missing",
                "tex:missing",
                "tex",
                "distribution",
                LookupRole::Probe,
                LookupOutcome::Absent(NegativeScope::Distribution {
                    root: "root-a".to_owned(),
                }),
            )
            .expect("negative"),
        )
        .expect("record negative");
    let decoded = LookupManifest::decode(&manifest.encode()).expect("round trip");
    assert_eq!(decoded, manifest);
    assert_eq!(decoded.resolved_records().count(), 1);
    assert_eq!(decoded.records()[0].role, LookupRole::Required);
}
