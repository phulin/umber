use super::*;
use std::collections::BTreeSet;
use std::sync::OnceLock;

use test_support::closed_case::FixtureCase;

#[global_allocator]
static ALLOCATOR: tex_state_profiling_allocator::HotCoreAllocator =
    tex_state_profiling_allocator::HotCoreAllocator;

const CASE_PATH: &str = "tests/corpus/distribution/cross-frontend-v1";

struct Fixture {
    manifest: String,
    html_root: String,
    html_shard_template: String,
}

fn fixture() -> &'static Fixture {
    static FIXTURE: OnceLock<Fixture> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        let case = FixtureCase::discover(CASE_PATH, "manifest.json", "cross-frontend-v1")
            .expect("validate typed distribution fixture case");
        Fixture {
            manifest: case.read_to_string("manifest.json").expect("manifest"),
            html_root: case
                .read_to_string("html-font-root.json")
                .expect("HTML root"),
            html_shard_template: case
                .read_to_string("html-font-shard.template.json")
                .expect("HTML shard template"),
        }
    })
}

fn html_shard_fixture() -> String {
    let unicode_map = std::iter::once(r#""A""#)
        .chain(std::iter::repeat_n("null", 255))
        .collect::<Vec<_>>()
        .join(",");
    fixture()
        .html_shard_template
        .replace(r#""__UNICODE_MAP__""#, &unicode_map)
}

#[test]
fn shared_fixture_strictly_parses() {
    Manifest::parse(&fixture().manifest).expect("parse manifest fixture");
}

#[test]
fn strict_parser_rejects_unknown_duplicate_and_unsafe_fields() {
    let unknown =
        fixture()
            .manifest
            .replacen("\"schema\": 2,", "\"schema\": 2, \"extra\": true,", 1);
    assert!(Manifest::parse(&unknown).is_err());
    let duplicate =
        fixture()
            .manifest
            .replacen("\"schema\": 2,", "\"schema\": 2, \"schema\": 2,", 1);
    assert!(Manifest::parse(&duplicate).is_err());
    let traversal =
        fixture()
            .manifest
            .replacen("tex/latex/base/article.cls", "tex/../article.cls", 1);
    assert!(Manifest::parse(&traversal).is_err());
    let absent_dependency = fixture()
        .manifest
        .replacen("tex:latex.ltx\"]", "tex:absent.sty\"]", 1);
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
        LegacyMappingRequestKey::new("a".repeat(16), 1, "html-layout", Some("OT1".to_owned()))
            .expect("mapping key");
    assert_eq!(
        LegacyMappingRequestKey::from_manifest_key(mapping.manifest_key().as_str()),
        Ok(mapping)
    );
}

#[test]
fn html_font_shard_parses_selects_and_serializes_canonically() {
    let root = ShardedManifestRoot::parse(&fixture().html_root).expect("HTML root");
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
    assert_eq!(shard_index(&font.manifest_key(), 8), Ok(251));
    assert_eq!(shard_index(&mapping.manifest_key(), 8), Ok(204));
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
    let digest = "c".repeat(16);
    let cases = [
        fixture.replacen(
            &format!(r#""tfmAhash64": "{digest}""#),
            &format!(r#""tfmAhash64": "{}""#, "b".repeat(16)),
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
            "\"schema\": 2,\n      \"object\"",
            "\"schema\": 3,\n      \"object\"",
            1,
        ),
    ];
    for (index, invalid) in cases.into_iter().enumerate() {
        assert!(
            ManifestShard::parse(&invalid).is_err(),
            "invalid case {index} was accepted"
        );
    }

    let conflict = fixture
        .replacen(
            &format!("ahash64-v1-{}", "d".repeat(16)),
            &format!("ahash64-v1-{}", "a".repeat(16)),
            1,
        )
        .replacen(
            &format!(r#""ahash64": "{}""#, "b".repeat(16)),
            &format!(r#""ahash64": "{}""#, "a".repeat(16)),
            1,
        );
    assert!(ManifestShard::parse(&conflict).is_err());
}

#[test]
fn mixed_html_catalog_extensions_preserve_v2_identity_and_shared_objects() {
    let original = ManifestShard::parse(&html_shard_fixture()).expect("MVP shard");
    let mut extended = original.clone();
    let base_font = original.fonts.values().next().expect("base font").clone();
    let advanced_font_request = FontRequestKey::new(base_font.request.logical_name())
        .expect("advanced font name")
        .with_context(FontRequestContext {
            face_index: 0,
            variation_instance: VariationInstance::Named(300),
            variations: Vec::new(),
            features: vec![FeatureSetting {
                tag: *b"liga",
                value: 0,
            }],
            direction: WritingDirection::LeftToRight,
            script: Some(*b"latn"),
            language: Some("en".to_owned()),
        })
        .expect("advanced instance");
    let mut advanced_font = base_font.clone();
    advanced_font.request = advanced_font_request.clone();
    extended.fonts.insert(
        advanced_font_request.manifest_key().to_string(),
        advanced_font,
    );

    let math_font_request = FontRequestKey::new("future-math-family")
        .expect("additional family")
        .with_context(FontRequestContext {
            face_index: 0,
            variation_instance: VariationInstance::Default,
            variations: Vec::new(),
            features: Vec::new(),
            direction: WritingDirection::LeftToRight,
            script: Some(*b"math"),
            language: None,
        })
        .expect("math family request");
    let mut math_font = base_font.clone();
    math_font.request = math_font_request.clone();
    extended
        .fonts
        .insert(math_font_request.manifest_key().to_string(), math_font);

    let base_mapping = original
        .legacy_mappings
        .values()
        .next()
        .expect("base mapping")
        .clone();
    let second_mapping_request =
        LegacyMappingRequestKey::new("b".repeat(16), 1, "html-layout", Some("T1".to_owned()))
            .expect("additional encoding mapping");
    let mut second_mapping = base_mapping.clone();
    second_mapping.request = second_mapping_request.clone();
    second_mapping.font_request = advanced_font_request.clone();
    second_mapping.unicode_map[0] = Some("Γ".to_owned());
    extended.legacy_mappings.insert(
        second_mapping_request.manifest_key().to_string(),
        second_mapping,
    );

    let reparsed = ManifestShard::parse(&extended.to_json()).expect("mixed v1 catalog");
    assert_eq!(reparsed, extended);
    assert_eq!(
        ManifestShard::parse(&original.to_json()),
        Ok(original.clone())
    );
    assert_eq!(reparsed.fonts.len(), 3);
    assert_eq!(reparsed.legacy_mappings.len(), 2);

    let program_identities = reparsed
        .fonts
        .values()
        .map(|record| record.declared_program_identity.as_deref())
        .collect::<BTreeSet<_>>();
    let font_objects = reparsed
        .fonts
        .values()
        .map(|record| (record.object.ahash64.clone(), record.object.bytes))
        .collect::<BTreeSet<_>>();
    let mapping_objects = reparsed
        .legacy_mappings
        .values()
        .map(|record| (record.object.ahash64.clone(), record.object.bytes))
        .collect::<BTreeSet<_>>();
    assert_eq!(program_identities.len(), 1, "instances share one program");
    assert_eq!(font_objects.len(), 1, "font records share one WOFF2 object");
    assert_eq!(
        mapping_objects, font_objects,
        "TFM mappings reuse that object"
    );
    assert_ne!(base_font.request, advanced_font_request);
    assert_ne!(base_mapping.request, second_mapping_request);
    assert_eq!(second_mapping_request.tfm_ahash64(), "b".repeat(16));
    assert!(
        second_mapping_request
            .manifest_key()
            .to_string()
            .ends_with("5431")
    );

    let basename_only =
        LegacyMappingRequestKey::new("c".repeat(16), 1, "html-layout", Some("T1".to_owned()))
            .expect("unmapped exact identity");
    let selection = select_shard(
        &reparsed,
        &[
            ManifestRequest::Font(advanced_font_request),
            ManifestRequest::LegacyMapping(second_mapping_request),
            ManifestRequest::LegacyMapping(basename_only.clone()),
        ],
    );
    assert_eq!(selection.jobs.len(), 2);
    assert_eq!(
        selection.misses,
        [ManifestMiss::LegacyMapping(basename_only)]
    );
}

#[test]
fn v2_reader_accepts_current_records_and_rejects_future_record_versions() {
    let original = html_shard_fixture();
    assert!(ManifestShard::parse(&original).is_ok());
    for unsupported in [
        original.replacen(
            "\"schema\": 2,\n      \"object\"",
            "\"schema\": 3,\n      \"object\"",
            1,
        ),
        original.replacen(
            "\"schema\": 2,\n      \"tfmAhash64\"",
            "\"schema\": 3,\n      \"tfmAhash64\"",
            1,
        ),
    ] {
        assert!(ManifestShard::parse(&unsupported).is_err());
    }
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
        r#"{"schema":8,"distribution":"test","objectsBaseUrl":"https://example.test/objects/","shardBits":0,"shardCount":1,"shards":["aaaaaaaaaaaaaaaa"]}"#,
    )
    .expect("root manifest");
    assert_eq!(root.shard_digest(0), Some("aaaaaaaaaaaaaaaa"));

    let shard = ManifestShard::parse(
        r#"{"schema":3,"distribution":"test","index":0,"files":{"tex:plain.tex":{"virtualPath":"/texlive/tex/plain.tex","object":"ahash64-v1-bbbbbbbbbbbbbbbb","ahash64":"bbbbbbbbbbbbbbbb","bytes":10,"dependencies":[{"key":"tfm:cmr10.tfm","virtualPath":"/texlive/fonts/cmr10.tfm","object":"ahash64-v1-cccccccccccccccc","ahash64":"cccccccccccccccc","bytes":20}]}}}"#,
    )
    .expect("index shard");
    shard.validate_identity(&root, 0).expect("shard identity");
    let dependency = &shard.files["tex:plain.tex"].dependencies[0];
    assert_eq!(dependency.key, "tfm:cmr10.tfm");
    assert_eq!(dependency.object_entry().bytes, 20);
    let request = ManifestRequest::File(
        FileRequestKey::from_manifest_key("tex:plain.tex").expect("request key"),
    );
    let selection = select_shard(&shard, &[request]);
    assert_eq!(selection.jobs.len(), 2);
    assert_eq!(selection.jobs[0].requirement, JobRequirement::Required);
    assert_eq!(
        selection.jobs[1].requirement,
        JobRequirement::DependencyHint
    );
    assert_eq!(selection.jobs[1].manifest_key.as_str(), "tfm:cmr10.tfm");
}

#[test]
fn root_serialization_and_sharding_are_canonical_catalog_operations() {
    let root = ShardedManifestRoot::parse(
        r#"{"schema":8,"distribution":"test","objectsBaseUrl":"https://example.test/objects/","shardBits":0,"shardCount":1,"shards":["aaaaaaaaaaaaaaaa"]}"#,
    )
    .expect("root manifest");
    assert_eq!(
        root.to_json(),
        concat!(
            r#"{"schema":8,"distribution":"test","objectsBaseUrl":"https://example.test/objects/","shardBits":0,"shardCount":1,"shards":["aaaaaaaaaaaaaaaa"]}"#,
            "\n"
        )
    );

    let manifest = Manifest::parse(&fixture().manifest).expect("monolithic fixture");
    let catalog = shard_manifest(&manifest, 2).expect("canonical sharding");
    assert_eq!(catalog.root.schema, SHARDED_ROOT_SCHEMA);
    assert_eq!(catalog.shards.len(), 4);
    for shard in &catalog.shards {
        for key in shard.files.keys() {
            assert_eq!(
                shard_index_for_key(key, catalog.root.shard_bits),
                Ok(shard.index)
            );
        }
        assert_eq!(ManifestShard::parse(&shard.to_json()), Ok(shard.clone()));
    }
}

#[test]
fn assembled_catalog_rejects_cross_shard_and_stale_dependency_semantics() {
    let root = ShardedManifestRoot::parse(
        r#"{"schema":8,"distribution":"test","objectsBaseUrl":"https://example.test/objects/","shardBits":0,"shardCount":1,"shards":["aaaaaaaaaaaaaaaa"]}"#,
    )
    .expect("root manifest");
    let shard = ManifestShard::parse(
        r#"{"schema":3,"distribution":"test","index":0,"files":{"tex:plain.tex":{"virtualPath":"/texlive/tex/plain.tex","object":"ahash64-v1-bbbbbbbbbbbbbbbb","ahash64":"bbbbbbbbbbbbbbbb","bytes":10,"dependencies":[{"key":"tfm:absent.tfm","virtualPath":"/texlive/fonts/absent.tfm","object":"ahash64-v1-cccccccccccccccc","ahash64":"cccccccccccccccc","bytes":20}]}}}"#,
    )
    .expect("structurally valid shard");
    let error = assemble_sharded_catalog(root, vec![shard]).expect_err("absent dependency");
    assert!(error.to_string().contains("is absent"));
}

#[test]
fn rejects_inconsistent_roots_and_mismatched_shard_identity() {
    let inconsistent = r#"{"schema":8,"distribution":"test","objectsBaseUrl":"https://example.test/objects/","shardBits":1,"shardCount":1,"shards":["aaaaaaaaaaaaaaaa"]}"#;
    assert!(ShardedManifestRoot::parse(inconsistent).is_err());
    let root = ShardedManifestRoot::parse(
        r#"{"schema":8,"distribution":"test","objectsBaseUrl":"https://example.test/objects/","shardBits":0,"shardCount":1,"shards":["aaaaaaaaaaaaaaaa"]}"#,
    )
    .expect("root manifest");
    let shard = ManifestShard::parse(r#"{"schema":3,"distribution":"other","index":0,"files":{}}"#)
        .expect("structurally valid shard");
    assert!(shard.validate_identity(&root, 0).is_err());
}

#[test]
fn parses_versioned_bounded_format_input_closures() {
    let root = ShardedManifestRoot::parse(
        r#"{"schema":8,"distribution":"test","objectsBaseUrl":"https://example.test/objects/","shardBits":0,"shardCount":1,"shards":["aaaaaaaaaaaaaaaa"],"formats":{"latex":{"object":"ahash64-v1-bbbbbbbbbbbbbbbb","ahash64":"bbbbbbbbbbbbbbbb","bytes":10,"engine":"umber","engineVersion":"0.1.0","formatSchema":10,"sourceDistribution":"test","sourceManifestAhash64":"cccccccccccccccc","sourceDateEpoch":0,"inputClosure":{"schema":1,"keys":["tex:latex.ltx","tfm:cmr10.tfm"]}}}}"#,
    )
    .expect("root manifest with input closure");
    let closure = root.formats["latex"]
        .input_closure
        .as_ref()
        .expect("format input closure");
    assert_eq!(closure.schema, FORMAT_INPUT_CLOSURE_SCHEMA);
    assert_eq!(closure.keys, ["tex:latex.ltx", "tfm:cmr10.tfm"]);
    let schema_two = r#"{"schema":5,"distribution":"test","objectsBaseUrl":"https://example.test/objects/","shardBits":0,"shardCount":1,"shards":["aaaaaaaaaaaaaaaa"],"formats":{"latex":{"object":"ahash64-v1-bbbbbbbbbbbbbbbb","ahash64":"bbbbbbbbbbbbbbbb","bytes":10,"engine":"umber","engineVersion":"0.1.0","formatSchema":10,"sourceDistribution":"test","sourceManifestAhash64":"cccccccccccccccc","sourceDateEpoch":0,"inputClosure":{"schema":1,"keys":["tex:latex.ltx"]}}}}"#;
    assert!(ShardedManifestRoot::parse(schema_two).is_err());
}

#[test]
fn rejects_corrupt_duplicate_and_oversized_format_input_closures() {
    let prefix = r#"{"schema":8,"distribution":"test","objectsBaseUrl":"https://example.test/objects/","shardBits":0,"shardCount":1,"shards":["aaaaaaaaaaaaaaaa"],"formats":{"latex":{"object":"ahash64-v1-bbbbbbbbbbbbbbbb","ahash64":"bbbbbbbbbbbbbbbb","bytes":10,"engine":"umber","engineVersion":"0.1.0","formatSchema":10,"sourceDistribution":"test","sourceManifestAhash64":"cccccccccccccccc","sourceDateEpoch":0,"inputClosure":{"schema":1,"keys":["#;
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

#[test]
fn named_format_envelope_canonicalizes_closure_once() {
    let text = r#"{
        "schema":4,
        "name":"latex",
        "object":"ahash64-v1-bbbbbbbbbbbbbbbb",
        "ahash64":"bbbbbbbbbbbbbbbb",
        "bytes":10,
        "engine":"umber",
        "engineVersion":"0.1.0",
        "formatSchema":11,
        "sourceDistribution":"test",
        "sourceManifestAhash64":"cccccccccccccccc",
        "sourceDateEpoch":0,
        "inputClosure":{"schema":1,"keys":["tfm:cmr10.tfm","tex:latex.ltx"]}
    }"#;
    let named = NamedFormat::parse(text).expect("publisher format envelope");
    assert_eq!(named.name, "latex");
    assert_eq!(
        named.format.input_closure.expect("closure").keys,
        ["tex:latex.ltx", "tfm:cmr10.tfm"]
    );
    let duplicate = text.replace(
        r#"["tfm:cmr10.tfm","tex:latex.ltx"]"#,
        r#"["tex:latex.ltx","tex:latex.ltx"]"#,
    );
    assert!(NamedFormat::parse(&duplicate).is_err());
}

#[test]
fn verified_batch_owns_exact_shards_and_required_before_hint_order() {
    let manifest = Manifest::parse(&fixture().manifest).expect("monolithic fixture");
    let catalog = shard_manifest(&manifest, 2).expect("sharded catalogue");
    let requests = vec![
        ManifestRequest::File(
            FileRequestKey::from_manifest_key("tex:article.cls").expect("article request"),
        ),
        ManifestRequest::File(
            FileRequestKey::from_manifest_key("tfm:missing.tfm").expect("missing request"),
        ),
    ];
    let (_, indexes) = prepare_batch(&catalog.root.to_json(), &requests).expect("prepare batch");
    let raw = indexes
        .iter()
        .map(|index| {
            (
                *index,
                pack_shard(&catalog.shards[*index as usize]).expect("packed shard"),
            )
        })
        .collect::<Vec<_>>();
    let borrowed = raw
        .iter()
        .map(|(index, bytes)| (*index, bytes.as_slice()))
        .collect::<Vec<_>>();
    let plan = verify_batch(&catalog.root.to_json(), &borrowed, &requests).expect("verified plan");
    assert_eq!(plan.selection.misses.len(), 1);
    assert_eq!(plan.selection.jobs[0].requirement, JobRequirement::Required);
    assert!(
        plan.selection.jobs[1..]
            .iter()
            .all(|job| job.requirement == JobRequirement::DependencyHint)
    );

    let mut tampered = raw;
    tampered[0].1.push(b' ');
    let tampered = tampered
        .iter()
        .map(|(index, bytes)| (*index, bytes.as_slice()))
        .collect::<Vec<_>>();
    assert!(verify_batch(&catalog.root.to_json(), &tampered, &requests).is_err());
}

#[test]
fn packed_shards_are_deterministic_roundtrip_and_probe_exact_keys() {
    let manifest = Manifest::parse(&fixture().manifest).expect("monolithic fixture");
    let catalog = shard_manifest(&manifest, 2).expect("sharded catalogue");
    for (index, shard) in catalog.shards.iter().enumerate() {
        let first = pack_shard(shard).expect("packed shard");
        let second = pack_shard(shard).expect("repeat packed shard");
        assert_eq!(first, second);
        let validated = ValidatedPackedShard::new(first, &catalog.root, index as u32)
            .expect("validated packed shard");
        assert_eq!(unpack_shard(&validated), Ok(shard.clone()));
        for key in shard
            .files
            .keys()
            .chain(shard.fonts.keys())
            .chain(shard.legacy_mappings.keys())
        {
            let record = validated.lookup(key).expect("exact packed lookup");
            assert_eq!(record.key(), key);
        }
        assert!(validated.lookup("tex:authoritative-absence.sty").is_none());
    }
}

#[test]
fn packed_schema_two_canonicalizes_tables_and_retains_schema_one_compatibility() {
    let manifest = Manifest::parse(&fixture().manifest).expect("monolithic fixture");
    let catalog = shard_manifest(&manifest, 0).expect("one packed shard");
    let bytes = pack_shard(&catalog.shards[0]).expect("packed shard");
    assert_eq!(&bytes[..8], b"UMBRPKS2");
    assert_eq!(
        u16::from_le_bytes(bytes[8..10].try_into().expect("packed schema")),
        2
    );

    let object_count = u32::from_le_bytes(bytes[36..40].try_into().expect("object count"));
    let objects_offset =
        u32::from_le_bytes(bytes[56..60].try_into().expect("objects offset")) as usize;
    let objects = (0..object_count)
        .map(|index| {
            let offset = objects_offset + index as usize * 16;
            u64::from_le_bytes(bytes[offset..offset + 8].try_into().expect("object digest"))
        })
        .collect::<Vec<_>>();
    assert!(objects.windows(2).all(|pair| pair[0] < pair[1]));

    let path_count = u32::from_le_bytes(bytes[40..44].try_into().expect("path count"));
    let paths_offset = u32::from_le_bytes(bytes[60..64].try_into().expect("paths offset")) as usize;
    let strings_offset =
        u32::from_le_bytes(bytes[72..76].try_into().expect("strings offset")) as usize;
    let paths = (0..path_count)
        .map(|index| {
            let offset = paths_offset + index as usize * 8;
            let start =
                u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("path offset"))
                    as usize;
            let len = u32::from_le_bytes(
                bytes[offset + 4..offset + 8]
                    .try_into()
                    .expect("path length"),
            ) as usize;
            std::str::from_utf8(&bytes[strings_offset + start..strings_offset + start + len])
                .expect("UTF-8 path")
        })
        .collect::<Vec<_>>();
    assert!(paths.windows(2).all(|pair| pair[0] < pair[1]));

    let mut legacy = bytes.clone();
    legacy[..8].copy_from_slice(b"UMBRPKS1");
    legacy[8..10].copy_from_slice(&LEGACY_PACKED_SHARD_SCHEMA.to_le_bytes());
    ValidatedPackedShard::new(legacy, &catalog.root, 0).expect("legacy packed shard");

    let mut mismatched = bytes;
    mismatched[..8].copy_from_slice(b"UMBRPKS1");
    assert!(ValidatedPackedShard::new(mismatched, &catalog.root, 0).is_err());
}

fn synthetic_packed_header(
    packed_schema: u16,
    total_len: u32,
    counts: [u32; 5],
    offsets: [u32; 7],
) -> Vec<u8> {
    let mut bytes = vec![0; total_len as usize];
    bytes[..8].copy_from_slice(if packed_schema == LEGACY_PACKED_SHARD_SCHEMA {
        b"UMBRPKS1"
    } else {
        b"UMBRPKS2"
    });
    bytes[8..10].copy_from_slice(&packed_schema.to_le_bytes());
    bytes[12..16].copy_from_slice(&INDEX_SHARD_SCHEMA.to_le_bytes());
    for (offset, value) in [28, 32, 36, 40, 44].into_iter().zip(counts) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    for (offset, value) in [48, 52, 56, 60, 64, 68, 72].into_iter().zip(offsets) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes[76..80].copy_from_slice(&total_len.to_le_bytes());
    bytes
}

fn empty_packed_root() -> ShardedManifestRoot {
    ShardedManifestRoot {
        schema: SHARDED_ROOT_SCHEMA,
        distribution: String::new(),
        objects_base_url: "https://example.invalid/objects/".to_owned(),
        shard_bits: 0,
        shard_count: 1,
        shards: vec!["0".repeat(16)],
        formats: Default::default(),
    }
}

#[test]
fn packed_validator_rejects_wrapped_sections_before_allocating_tables() {
    let root = empty_packed_root();

    // The object section ends at 112 + u32::MAX * 16. Narrowing that end to
    // u32 used to wrap it to 96 and admit a roughly 64-GiB legacy reserve.
    let legacy = synthetic_packed_header(
        LEGACY_PACKED_SHARD_SCHEMA,
        112,
        [2, 0, u32::MAX, 0, 0],
        [80, 112, 112, 96, 96, 96, 96],
    );
    let error = ValidatedPackedShard::new(legacy, &root, 0).expect_err("wrapped legacy section");
    assert_eq!(error.to_string(), "packed shard section is out of bounds");

    // Both bucket and record section sizes are multiples of 2^32. Their old
    // narrowed ends returned to offset 80 before a multi-gigabyte hash reserve.
    let canonical = synthetic_packed_header(
        PACKED_SHARD_SCHEMA,
        80,
        [1 << 31, 1 << 30, 0, 0, 0],
        [80, 80, 80, 80, 80, 80, 80],
    );
    let error =
        ValidatedPackedShard::new(canonical, &root, 0).expect_err("wrapped canonical sections");
    assert_eq!(error.to_string(), "packed shard section is out of bounds");
}

#[test]
fn packed_validator_uses_exact_table_load_arithmetic() {
    let saturated_load = synthetic_packed_header(
        PACKED_SHARD_SCHEMA,
        80,
        [1 << 31, u32::MAX, 0, 0, 0],
        [80, 80, 48, 48, 48, 48, 48],
    );
    let error = ValidatedPackedShard::new(saturated_load, &empty_packed_root(), 0)
        .expect_err("saturated packed table load");
    assert_eq!(error.to_string(), "invalid packed shard table size");
}

#[test]
fn warmed_packed_lookup_allocates_zero_bytes() {
    let manifest = Manifest::parse(&fixture().manifest).expect("monolithic fixture");
    let catalog = shard_manifest(&manifest, 0).expect("one packed shard");
    let packed = pack_shard(&catalog.shards[0]).expect("packed shard");
    let validated =
        ValidatedPackedShard::new(packed, &catalog.root, 0).expect("validated packed shard");
    let key = catalog.shards[0].files.keys().next().expect("file key");
    assert!(validated.lookup(key).is_some());
    assert!(validated.lookup("tex:authoritative-absence.sty").is_none());

    const OWNER: usize = 0;
    let before = tex_state_profiling_allocator::thread_measurement(OWNER);
    {
        let _scope = tex_state_profiling_allocator::scope(OWNER);
        for _ in 0..10_000 {
            std::hint::black_box(validated.lookup(std::hint::black_box(key)));
            std::hint::black_box(
                validated.lookup(std::hint::black_box("tex:authoritative-absence.sty")),
            );
        }
    }
    let after = tex_state_profiling_allocator::thread_measurement(OWNER);
    assert_eq!(after.calls, before.calls);
    assert_eq!(after.requested_bytes, before.requested_bytes);
}

#[test]
fn packed_validator_rejects_offsets_tables_duplicates_and_wrong_identity() {
    let manifest = Manifest::parse(&fixture().manifest).expect("monolithic fixture");
    let catalog = shard_manifest(&manifest, 0).expect("one packed shard");
    let bytes = pack_shard(&catalog.shards[0]).expect("packed shard");

    let mut bad_offset = bytes.clone();
    bad_offset[52..56].copy_from_slice(&0_u32.to_le_bytes());
    assert!(ValidatedPackedShard::new(bad_offset, &catalog.root, 0).is_err());

    let records_offset = u32::from_le_bytes(bytes[52..56].try_into().expect("record offset"));
    let mut bad_record_span = bytes.clone();
    bad_record_span[records_offset as usize + 24..records_offset as usize + 28]
        .copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(ValidatedPackedShard::new(bad_record_span, &catalog.root, 0).is_err());

    let bucket_count = u32::from_le_bytes(bytes[28..32].try_into().expect("bucket count"));
    let bucket_offset = u32::from_le_bytes(bytes[48..52].try_into().expect("bucket offset"));
    let occupied = (0..bucket_count)
        .find(|bucket| {
            let offset = bucket_offset as usize + *bucket as usize * 16 + 8;
            u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("bucket")) != u32::MAX
        })
        .expect("occupied bucket");
    let empty = (0..bucket_count)
        .find(|bucket| {
            let offset = bucket_offset as usize + *bucket as usize * 16 + 8;
            u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("bucket")) == u32::MAX
        })
        .expect("empty bucket");
    let occupied_offset = bucket_offset as usize + occupied as usize * 16;
    let empty_offset = bucket_offset as usize + empty as usize * 16;
    let mut duplicate = bytes.clone();
    let bucket = duplicate[occupied_offset..occupied_offset + 16].to_vec();
    duplicate[empty_offset..empty_offset + 16].copy_from_slice(&bucket);
    assert!(ValidatedPackedShard::new(duplicate, &catalog.root, 0).is_err());

    let next_empty = (1..=bucket_count)
        .map(|distance| (occupied + distance) & (bucket_count - 1))
        .find(|bucket| {
            let offset = bucket_offset as usize + *bucket as usize * 16 + 8;
            u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("bucket")) == u32::MAX
        })
        .expect("empty bucket after occupied bucket");
    let next_empty_offset = bucket_offset as usize + next_empty as usize * 16;
    let mut unreachable = bytes.clone();
    let occupied_bytes = unreachable[occupied_offset..occupied_offset + 16].to_vec();
    let empty_bytes = unreachable[next_empty_offset..next_empty_offset + 16].to_vec();
    unreachable[occupied_offset..occupied_offset + 16].copy_from_slice(&empty_bytes);
    unreachable[next_empty_offset..next_empty_offset + 16].copy_from_slice(&occupied_bytes);
    assert!(ValidatedPackedShard::new(unreachable, &catalog.root, 0).is_err());

    let mut duplicate_key = bytes.clone();
    let first_key_span =
        duplicate_key[records_offset as usize..records_offset as usize + 6].to_vec();
    duplicate_key[records_offset as usize + 32..records_offset as usize + 38]
        .copy_from_slice(&first_key_span);
    assert!(ValidatedPackedShard::new(duplicate_key, &catalog.root, 0).is_err());

    let objects_offset = u32::from_le_bytes(bytes[56..60].try_into().expect("object offset"));
    let object_count = u32::from_le_bytes(bytes[36..40].try_into().expect("object count"));
    assert!(object_count >= 2);
    let mut unordered_objects = bytes.clone();
    let first_object =
        unordered_objects[objects_offset as usize..objects_offset as usize + 16].to_vec();
    let second_object =
        unordered_objects[objects_offset as usize + 16..objects_offset as usize + 32].to_vec();
    unordered_objects[objects_offset as usize..objects_offset as usize + 16]
        .copy_from_slice(&second_object);
    unordered_objects[objects_offset as usize + 16..objects_offset as usize + 32]
        .copy_from_slice(&first_object);
    assert!(ValidatedPackedShard::new(unordered_objects, &catalog.root, 0).is_err());

    let mut duplicate_object = bytes.clone();
    let first_object =
        duplicate_object[objects_offset as usize..objects_offset as usize + 16].to_vec();
    duplicate_object[objects_offset as usize + 16..objects_offset as usize + 32]
        .copy_from_slice(&first_object);
    assert!(ValidatedPackedShard::new(duplicate_object, &catalog.root, 0).is_err());

    let mut conflicting_object = bytes.clone();
    let first_digest =
        conflicting_object[objects_offset as usize..objects_offset as usize + 8].to_vec();
    conflicting_object[objects_offset as usize + 16..objects_offset as usize + 24]
        .copy_from_slice(&first_digest);
    assert!(ValidatedPackedShard::new(conflicting_object, &catalog.root, 0).is_err());

    let mut oversized_object = bytes.clone();
    oversized_object[objects_offset as usize + 8..objects_offset as usize + 16]
        .copy_from_slice(&(128_u64 * 1024 * 1024 + 1).to_le_bytes());
    assert!(ValidatedPackedShard::new(oversized_object, &catalog.root, 0).is_err());

    let paths_offset = u32::from_le_bytes(bytes[60..64].try_into().expect("path offset"));
    let path_count = u32::from_le_bytes(bytes[40..44].try_into().expect("path count"));
    assert!(path_count >= 2);
    let mut unordered_paths = bytes.clone();
    let first_path = unordered_paths[paths_offset as usize..paths_offset as usize + 8].to_vec();
    let second_path =
        unordered_paths[paths_offset as usize + 8..paths_offset as usize + 16].to_vec();
    unordered_paths[paths_offset as usize..paths_offset as usize + 8].copy_from_slice(&second_path);
    unordered_paths[paths_offset as usize + 8..paths_offset as usize + 16]
        .copy_from_slice(&first_path);
    assert!(ValidatedPackedShard::new(unordered_paths, &catalog.root, 0).is_err());

    let mut duplicate_path = bytes.clone();
    let first_path = duplicate_path[paths_offset as usize..paths_offset as usize + 8].to_vec();
    duplicate_path[paths_offset as usize + 8..paths_offset as usize + 16]
        .copy_from_slice(&first_path);
    assert!(ValidatedPackedShard::new(duplicate_path, &catalog.root, 0).is_err());

    let strings_offset = u32::from_le_bytes(bytes[72..76].try_into().expect("strings offset"));
    let first_path_offset = u32::from_le_bytes(
        bytes[paths_offset as usize..paths_offset as usize + 4]
            .try_into()
            .expect("path span"),
    );
    let mut invalid_path = bytes.clone();
    invalid_path[strings_offset as usize + first_path_offset as usize] = b'x';
    assert!(ValidatedPackedShard::new(invalid_path, &catalog.root, 0).is_err());

    let mut invalid_path_span = bytes.clone();
    invalid_path_span[paths_offset as usize..paths_offset as usize + 4]
        .copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(ValidatedPackedShard::new(invalid_path_span, &catalog.root, 0).is_err());

    let mut invalid_object_reference = bytes.clone();
    invalid_object_reference[records_offset as usize + 8..records_offset as usize + 12]
        .copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(ValidatedPackedShard::new(invalid_object_reference, &catalog.root, 0).is_err());

    assert!(ValidatedPackedShard::new(bytes, &catalog.root, 1).is_err());
}

#[test]
fn packed_validator_accepts_wrapped_probe_clusters_across_table_sizes() {
    for record_count in [0, 1, 2, 3, 4, 7, 8, 15, 16, 31, 32, 63, 64, 127, 256] {
        let files = (0..record_count)
            .map(|index| {
                let ahash64 = format!("{index:016x}");
                (
                    format!("tex:probe-{index}.sty"),
                    ShardFile {
                        virtual_path: format!("/texlive/tex/probe-{index}.sty"),
                        object: format!("ahash64-v1-{ahash64}"),
                        ahash64,
                        bytes: index as u64,
                        dependencies: Vec::new(),
                    },
                )
            })
            .collect();
        let shard = ManifestShard {
            schema: INDEX_SHARD_SCHEMA,
            distribution: "probe-clusters".to_owned(),
            index: 0,
            files,
            fonts: Default::default(),
            legacy_mappings: Default::default(),
        };
        let root = ShardedManifestRoot {
            schema: SHARDED_ROOT_SCHEMA,
            distribution: shard.distribution.clone(),
            objects_base_url: "https://example.invalid/objects/".to_owned(),
            shard_bits: 0,
            shard_count: 1,
            shards: vec!["0".repeat(16)],
            formats: Default::default(),
        };
        let validated =
            ValidatedPackedShard::new(pack_shard(&shard).expect("packed probe shard"), &root, 0)
                .expect("valid wrapped probe clusters");
        for key in shard.files.keys() {
            let record = validated.lookup(key).expect("packed key lookup");
            assert_eq!(record.key(), key);
        }
    }
}
