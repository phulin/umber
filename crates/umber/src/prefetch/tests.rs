use super::*;

fn planner() -> PrefetchPlanner {
    PrefetchPlanner::new(
        PrefetchIdentity::new("pdftex", "latex", "dvi", "root", PREFETCH_POLICY_VERSION)
            .expect("identity"),
        PrefetchBudget::default(),
    )
}

#[test]
fn startup_policy_combines_prior_manifest_and_literal_hints() {
    let identity = PrefetchIdentity::new("pdftex", "latex", "dvi", "root", PREFETCH_POLICY_VERSION)
        .expect("identity");
    let mut prior = LookupManifest::new(identity.clone());
    let resolved = ResolvedIdentity::new(
        "tex:prior.sty",
        Some("/texlive/prior.sty".to_owned()),
        "object",
        "0123456789abcdef",
        10,
    )
    .expect("resolved");
    prior
        .record(
            LookupRecord::new(
                "prior.sty",
                "tex:prior.sty",
                "tex",
                "distribution",
                LookupRole::Required,
                LookupOutcome::Resolved(resolved),
            )
            .expect("record"),
        )
        .expect("manifest");
    let mut planner = PrefetchPlanner::with_prior(identity, PrefetchBudget::default(), Some(prior));
    let hints = planner.startup_hints("\\usepackage{newpkg}");
    let names = hints
        .iter()
        .map(|request| match request {
            ResourceRequest::File(request) => request.key().name(),
            _ => "other",
        })
        .collect::<Vec<_>>();
    assert_eq!(names, ["prior.sty", "newpkg"]);
    assert_eq!(planner.metrics().startup_candidates, 1);
}

#[test]
fn source_edits_do_not_change_identity_or_publish_predictions() {
    let mut planner = planner();
    let first = planner.startup_hints("\\input{first}");
    let second = planner.startup_hints("\\input{second}");
    assert_eq!(first.len(), 1);
    assert_eq!(second.len(), 1);
    assert!(planner.into_manifest().records().is_empty());
}
