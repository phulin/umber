use super::*;

const MANIFEST_FIXTURE: &str = include_str!("../../../tests/corpus/distribution/manifest.json");
const SELECTION_FIXTURE: &str = include_str!("../../../tests/corpus/distribution/selection.case");

#[test]
fn shared_fixture_round_trips_and_selects_expected_jobs_and_misses() {
    let manifest = Manifest::parse(MANIFEST_FIXTURE).expect("parse manifest fixture");
    let encoded = manifest.to_json_pretty();
    assert_eq!(Manifest::parse(&encoded), Ok(manifest.clone()));

    let mut requests = Vec::new();
    let mut expected_jobs = Vec::new();
    let mut expected_misses = Vec::new();
    for line in SELECTION_FIXTURE
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
    {
        let fields = line.split('\t').collect::<Vec<_>>();
        match fields.as_slice() {
            ["request", "file", kind, name] => requests.push(ManifestRequest::File(
                FileRequestKey::new(
                    FileKind::from_manifest_name(kind).expect("fixture kind"),
                    *name,
                )
                .expect("fixture file request"),
            )),
            ["request", "font", name] => requests.push(ManifestRequest::Font(
                FontRequestKey::new(*name).expect("fixture font request"),
            )),
            ["job", requirement, kind, key, digest] => {
                expected_jobs.push(format!("{requirement}\t{kind}\t{key}\t{digest}"))
            }
            ["miss", kind, key] => expected_misses.push(format!("{kind}\t{key}")),
            _ => panic!("invalid selection fixture line: {line}"),
        }
    }

    let selection = select(&manifest, &requests);
    let jobs = selection
        .jobs
        .iter()
        .map(|job| {
            let requirement = match job.requirement {
                JobRequirement::Required => "required",
                JobRequirement::DependencyHint => "hint",
            };
            let kind = match job.request {
                ManifestRequest::File(_) => "file",
                ManifestRequest::Font(_) => "font",
            };
            format!(
                "{requirement}\t{kind}\t{}\t{}",
                job.manifest_key, job.object.sha256
            )
        })
        .collect::<Vec<_>>();
    let misses = selection
        .misses
        .iter()
        .map(|miss| match miss {
            ManifestMiss::File(key) => format!("file\t{}", key.manifest_key()),
            ManifestMiss::Font(key) => format!("font\t{}", key.manifest_key()),
        })
        .collect::<Vec<_>>();
    assert_eq!(jobs, expected_jobs);
    assert_eq!(misses, expected_misses);
}

#[test]
fn strict_parser_rejects_unknown_duplicate_and_unsafe_fields() {
    let unknown = MANIFEST_FIXTURE.replacen("\"schema\": 1,", "\"schema\": 1, \"extra\": true,", 1);
    assert!(Manifest::parse(&unknown).is_err());
    let duplicate = MANIFEST_FIXTURE.replacen("\"schema\": 1,", "\"schema\": 1, \"schema\": 1,", 1);
    assert!(Manifest::parse(&duplicate).is_err());
    let traversal =
        MANIFEST_FIXTURE.replacen("tex/latex/base/article.cls", "tex/../article.cls", 1);
    assert!(Manifest::parse(&traversal).is_err());
    let absent_dependency = MANIFEST_FIXTURE.replacen("tex:latex.ltx\"]", "tex:absent.sty\"]", 1);
    assert!(Manifest::parse(&absent_dependency).is_err());
}

#[test]
fn request_key_encoding_is_canonical() {
    let file = FileRequestKey::new(FileKind::Tex, "latex/base/article.cls").expect("valid key");
    assert_eq!(file.manifest_key().as_str(), "tex:latex/base/article.cls");
    assert_eq!(
        FileRequestKey::from_manifest_key(file.manifest_key().as_str()),
        Ok(file)
    );
    assert!(FileRequestKey::new(FileKind::Tex, "../article.cls").is_err());
    assert!(FontRequestKey::new("bad\0font").is_err());
}
