use super::*;

const MANIFEST_FIXTURE: &str = include_str!("../../../tests/corpus/distribution/manifest.json");
const SELECTION_FIXTURE: &str = include_str!("../../../tests/corpus/distribution/selection.case");
const HTML_ROOT_FIXTURE: &str =
    include_str!("../../../tests/corpus/distribution/html-font-root.json");
const HTML_SHARD_TEMPLATE: &str =
    include_str!("../../../tests/corpus/distribution/html-font-shard.template.json");

fn html_shard_fixture() -> String {
    let unicode_map = std::iter::once(r#""A""#)
        .chain(std::iter::repeat_n("null", 255))
        .collect::<Vec<_>>()
        .join(",");
    HTML_SHARD_TEMPLATE.replace(r#""__UNICODE_MAP__""#, &unicode_map)
}

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
                ManifestRequest::LegacyMapping(_) => "legacy-mapping",
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
            ManifestMiss::Font(key) => format!("font\t{}", key.logical_name()),
            ManifestMiss::LegacyMapping(key) => {
                format!("legacy-mapping\t{}", key.manifest_key())
            }
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

#[test]
fn complete_font_and_exact_legacy_keys_round_trip_without_aliases() {
    let base = FontRequestKey::new("cmu-serif-roman")
        .expect("font key")
        .with_context(FontRequestContext {
            face_index: 0,
            variation_instance: VariationInstance::Default,
            variations: Vec::new(),
            features: vec![
                FeatureSetting {
                    tag: *b"liga",
                    value: 1,
                },
                FeatureSetting {
                    tag: *b"kern",
                    value: 1,
                },
            ],
            direction: WritingDirection::LeftToRight,
            script: Some(*b"latn"),
            language: Some("EN".to_owned()),
        })
        .expect("complete font key");
    let encoded = base.manifest_key();
    assert_eq!(
        FontRequestKey::from_manifest_key(encoded.as_str()),
        Ok(base.clone())
    );
    for changed in [
        base.clone().with_context(FontRequestContext {
            face_index: 1,
            variation_instance: VariationInstance::Default,
            variations: Vec::new(),
            features: base.features.clone(),
            direction: base.direction,
            script: base.script,
            language: base.language.clone(),
        }),
        base.clone().with_context(FontRequestContext {
            face_index: 0,
            variation_instance: VariationInstance::Default,
            variations: Vec::new(),
            features: vec![FeatureSetting {
                tag: *b"liga",
                value: 0,
            }],
            direction: base.direction,
            script: base.script,
            language: base.language.clone(),
        }),
    ] {
        assert_ne!(changed.expect("alternate key").manifest_key(), encoded);
    }

    let mapping =
        LegacyMappingRequestKey::new("c".repeat(64), 1, "html-layout", Some("OT1".to_owned()))
            .expect("mapping key");
    assert_eq!(
        LegacyMappingRequestKey::from_manifest_key(mapping.manifest_key().as_str()),
        Ok(mapping)
    );
}

#[test]
fn html_font_shard_parses_selects_and_serializes_canonically() {
    let root = ShardedManifestRoot::parse(HTML_ROOT_FIXTURE).expect("HTML root");
    let fixture = html_shard_fixture();
    let shard = ManifestShard::parse(&fixture).expect("HTML shard");
    shard
        .validate_identity(&root, 0)
        .expect("paired HTML shard");
    assert_eq!(ManifestShard::parse(&shard.to_json()), Ok(shard.clone()));

    let font = shard.fonts.values().next().expect("font").request.clone();
    let mapping = shard
        .legacy_mappings
        .values()
        .next()
        .expect("mapping")
        .request
        .clone();
    let absent = FontRequestKey::new("absent").expect("absent font key");
    assert_eq!(shard_index(&font.manifest_key(), 8), Ok(107));
    assert_eq!(shard_index(&mapping.manifest_key(), 8), Ok(220));
    let selection = select_shard(
        &shard,
        &[
            ManifestRequest::Font(font),
            ManifestRequest::LegacyMapping(mapping),
            ManifestRequest::Font(absent.clone()),
        ],
    );
    assert_eq!(selection.jobs.len(), 2);
    assert_eq!(selection.misses, [ManifestMiss::Font(absent)]);
}

#[test]
fn html_font_shard_rejects_identity_policy_mapping_and_license_failures() {
    let fixture = html_shard_fixture();
    let digest = "c".repeat(64);
    let cases = [
        fixture.replacen(
            &format!(r#""tfmSha256": "{digest}""#),
            &format!(r#""tfmSha256": "{}""#, "a".repeat(64)),
            1,
        ),
        fixture.replacen(r#""mappingVersion": 1"#, r#""mappingVersion": 2"#, 1),
        fixture.replacen(r#""unicodeMap": ["A",null"#, r#""unicodeMap": ["A""#, 1),
        fixture.replacen(r#""license": {"#, r#""missingLicense": {"#, 1),
        fixture.replacen(r#""embeddable": true"#, r#""embeddable": false"#, 1),
        fixture.replacen(
            "6b65726e=00000001,6c696761=00000001",
            "6b65726e=00000001,6b65726e=00000001",
            1,
        ),
        fixture.replacen(
            "\"schema\": 1,\n      \"object\"",
            "\"schema\": 2,\n      \"object\"",
            1,
        ),
    ];
    for invalid in cases {
        assert!(ManifestShard::parse(&invalid).is_err());
    }

    let conflict = fixture
        .replacen(
            &format!("sha256-{}", "e".repeat(64)),
            &format!("sha256-{}", "d".repeat(64)),
            1,
        )
        .replacen(
            &format!(r#""sha256": "{}""#, "e".repeat(64)),
            &format!(r#""sha256": "{}""#, "d".repeat(64)),
            1,
        );
    assert!(ManifestShard::parse(&conflict).is_err());
}

#[test]
fn classic_resource_kinds_use_stable_distribution_keys() {
    let cases = [
        (FileKind::BibAux, "main.aux", "bib-aux:main.aux"),
        (FileKind::ClassicBib, "refs.bib", "classic-bib:refs.bib"),
        (FileKind::BibStyle, "plain.bst", "bst:plain.bst"),
    ];
    for (kind, name, expected) in cases {
        let key = FileRequestKey::new(kind, name).expect("valid classic request");
        assert_eq!(key.manifest_key().as_str(), expected);
        assert_eq!(FileRequestKey::from_manifest_key(expected), Ok(key));
    }
}

#[test]
fn parses_sharded_root_and_full_inline_dependency_metadata() {
    let root = ShardedManifestRoot::parse(
        r#"{"schema":2,"distribution":"test","objectsBaseUrl":"https://example.test/objects/","shardBits":0,"shardCount":1,"shards":["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"]}"#,
    )
    .expect("root manifest");
    assert_eq!(
        root.shard_digest(0),
        Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    );

    let shard = ManifestShard::parse(
        r#"{"schema":1,"distribution":"test","index":0,"files":{"tex:plain.tex":{"virtualPath":"/texlive/tex/plain.tex","object":"sha256-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","sha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","bytes":10,"dependencies":[{"key":"tfm:cmr10.tfm","virtualPath":"/texlive/fonts/cmr10.tfm","object":"sha256-cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","sha256":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","bytes":20}]}}}"#,
    )
    .expect("index shard");
    shard.validate_identity(&root, 0).expect("shard identity");
    let dependency = &shard.files["tex:plain.tex"].dependencies[0];
    assert_eq!(dependency.key, "tfm:cmr10.tfm");
    assert_eq!(dependency.object_entry().bytes, 20);
}

#[test]
fn rejects_inconsistent_roots_and_mismatched_shard_identity() {
    let inconsistent = r#"{"schema":2,"distribution":"test","objectsBaseUrl":"https://example.test/objects/","shardBits":1,"shardCount":1,"shards":["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"]}"#;
    assert!(ShardedManifestRoot::parse(inconsistent).is_err());
    let root = ShardedManifestRoot::parse(
        r#"{"schema":2,"distribution":"test","objectsBaseUrl":"https://example.test/objects/","shardBits":0,"shardCount":1,"shards":["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"]}"#,
    )
    .expect("root manifest");
    let shard = ManifestShard::parse(r#"{"schema":1,"distribution":"other","index":0,"files":{}}"#)
        .expect("structurally valid shard");
    assert!(shard.validate_identity(&root, 0).is_err());
}

#[test]
fn parses_versioned_bounded_format_input_closures() {
    let root = ShardedManifestRoot::parse(
        r#"{"schema":3,"distribution":"test","objectsBaseUrl":"https://example.test/objects/","shardBits":0,"shardCount":1,"shards":["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"],"formats":{"latex":{"object":"sha256-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","sha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","bytes":10,"engine":"umber","engineVersion":"0.1.0","formatSchema":10,"sourceDistribution":"test","sourceManifestSha256":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","sourceDateEpoch":0,"inputClosure":{"schema":1,"keys":["tex:latex.ltx","tfm:cmr10.tfm"]}}}}"#,
    )
    .expect("root manifest with input closure");
    let closure = root.formats["latex"]
        .input_closure
        .as_ref()
        .expect("format input closure");
    assert_eq!(closure.schema, FORMAT_INPUT_CLOSURE_SCHEMA);
    assert_eq!(closure.keys, ["tex:latex.ltx", "tfm:cmr10.tfm"]);
    let schema_two = r#"{"schema":2,"distribution":"test","objectsBaseUrl":"https://example.test/objects/","shardBits":0,"shardCount":1,"shards":["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"],"formats":{"latex":{"object":"sha256-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","sha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","bytes":10,"engine":"umber","engineVersion":"0.1.0","formatSchema":10,"sourceDistribution":"test","sourceManifestSha256":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","sourceDateEpoch":0,"inputClosure":{"schema":1,"keys":["tex:latex.ltx"]}}}}"#;
    assert!(ShardedManifestRoot::parse(schema_two).is_err());
}

#[test]
fn rejects_corrupt_duplicate_and_oversized_format_input_closures() {
    let prefix = r#"{"schema":3,"distribution":"test","objectsBaseUrl":"https://example.test/objects/","shardBits":0,"shardCount":1,"shards":["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"],"formats":{"latex":{"object":"sha256-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","sha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","bytes":10,"engine":"umber","engineVersion":"0.1.0","formatSchema":10,"sourceDistribution":"test","sourceManifestSha256":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","sourceDateEpoch":0,"inputClosure":{"schema":1,"keys":["#;
    let suffix = r#"]}}}}"#;
    for keys in [
        r#"tex:latex.ltx","tex:latex.ltx"#,
        r#"tfm:cmr10.tfm","tex:latex.ltx"#,
        r#"invalid"#,
    ] {
        assert!(ShardedManifestRoot::parse(&format!("{prefix}{keys}{suffix}")).is_err());
    }
    let long_key = format!("tex:{}", "a".repeat(MAX_REQUEST_KEY_BYTES));
    assert!(ShardedManifestRoot::parse(&format!("{prefix}{long_key}\"{suffix}")).is_err());
    let too_many = (0..=MAX_FORMAT_INPUTS)
        .map(|index| format!(r#"tex:{index:03}.tex"#))
        .collect::<Vec<_>>()
        .join("\",\"");
    assert!(ShardedManifestRoot::parse(&format!("{prefix}{too_many}\"{suffix}")).is_err());
}
