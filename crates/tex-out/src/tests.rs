use crate::{
    ArtifactCodecLimits, ArtifactValidationError, ArtifactValidationLimits, BoxNode,
    CodecLimitKind, ContentHash, DiscKind, EffectSink, FontResource, GlueKind, GlueOrder,
    GlueSetRatio, GlueSign, GlueSpec, JobInfo, KernKind, LeaderPayload, OpenTypeFontResource,
    PageArtifact, PageEffect, PageNode, PageToken, ParseError, SerializeError, TokenCatcode,
};
use tex_arith::Scaled;

#[test]
fn page_artifact_round_trips() {
    let artifact = sample_artifact();

    let bytes = artifact.to_bytes().expect("artifact serializes");
    let parsed = PageArtifact::from_bytes(&bytes).expect("artifact parses");

    assert_eq!(parsed, artifact);
    assert_eq!(
        parsed.to_bytes().expect("parsed artifact serializes"),
        bytes
    );
}

#[test]
fn margin_kern_sides_round_trip() {
    let mut artifact = sample_artifact();
    let PageNode::VList(root) = &mut artifact.testing_mut().root else {
        unreachable!("sample root is a vlist");
    };
    root.children = vec![
        PageNode::Kern {
            amount: Scaled::from_raw(-123),
            kind: KernKind::LeftMargin,
        },
        PageNode::Kern {
            amount: Scaled::from_raw(456),
            kind: KernKind::RightMargin,
        },
    ];

    let bytes = artifact.to_bytes().expect("artifact serializes");
    assert_eq!(
        PageArtifact::from_bytes(&bytes).expect("artifact parses"),
        artifact
    );
}

#[test]
fn streamed_v10_builder_is_byte_identical_to_owned_encoding() {
    let page = sample_artifact();
    let (root, vertical) = match &page.root {
        PageNode::HList(root) => (root, false),
        PageNode::VList(root) => (root, true),
        _ => unreachable!("validated sample root is a box"),
    };
    let mut builder = crate::V10ArtifactBuilder::new(page.job.clone(), page.counts, root, vertical);
    for child in &root.children {
        builder.push_node(child).expect("stream child");
    }
    let streamed = builder
        .finish(&page.fonts, &page.effects)
        .expect("finish stream");

    assert_eq!(streamed, page.to_bytes().expect("owned encoding"));
}

#[test]
fn streamed_fixed_width_leaves_are_byte_identical_to_owned_encoding() {
    let mut page = sample_artifact();
    let leaves = vec![
        PageNode::Char {
            font_id: 1,
            ch: 'A' as u32,
            width: Scaled::from_raw(42),
        },
        PageNode::Lig {
            font_id: 1,
            ch: 'f' as u32,
            source: vec!['f' as u32, 'i' as u32],
            width: Scaled::from_raw(84),
        },
        PageNode::Kern {
            amount: Scaled::from_raw(-10),
            kind: KernKind::Font,
        },
        PageNode::Penalty(-50),
        PageNode::WhatsitAnchor { effect_index: 0 },
        PageNode::MathOn(Scaled::from_raw(7)),
        PageNode::MathOff(Scaled::from_raw(-7)),
    ];
    let PageNode::VList(root) = &mut page.testing_mut().root else {
        unreachable!("sample root is a vlist");
    };
    root.children = leaves;
    let root = match &page.root {
        PageNode::VList(root) => root,
        _ => unreachable!("sample root is a vlist"),
    };
    let mut builder = crate::V10ArtifactBuilder::new(page.job.clone(), page.counts, root, true);
    for child in &root.children {
        builder.push_node(child).expect("stream fixed leaf");
    }
    let streamed = builder
        .finish(&page.fonts, &page.effects)
        .expect("finish stream");

    assert_eq!(streamed, page.to_bytes().expect("owned encoding"));
}

#[test]
fn artifact_bytes_and_hash_are_deterministic() {
    let first = sample_artifact();
    let second = sample_artifact();

    let first_bytes = first.to_bytes().expect("first artifact serializes");
    let second_bytes = second.to_bytes().expect("second artifact serializes");

    assert_eq!(first_bytes, second_bytes);
    assert_eq!(
        ContentHash::for_domain(crate::ContentDomain::Artifact, &first_bytes),
        first.content_hash().expect("first artifact hashes")
    );
    assert_eq!(first.content_hash(), second.content_hash());
}

#[test]
fn artifact_decode_canonicalizes_glue_set_ratios_once() {
    let artifact = sample_artifact();
    let canonical = artifact.to_bytes().expect("artifact serializes");
    let mut noncanonical = canonical.clone();
    replace_unique_ratio(&mut noncanonical, (37, 101), (74, 202));

    let parsed = PageArtifact::from_bytes(&noncanonical).expect("reducible ratio decodes");
    let PageNode::VList(root) = &parsed.root else {
        panic!("sample root is a vlist");
    };
    assert_eq!(root.glue_set, GlueSetRatio::from_ratio_parts(37, 101));
    assert_eq!(
        parsed.to_bytes().expect("parsed artifact serializes"),
        canonical
    );
    assert_eq!(parsed.content_hash(), artifact.content_hash());
}

#[test]
fn artifact_decode_rejects_invalid_glue_set_ratios() {
    for malformed in [(37, 0), (37, -101), (i32::MIN, 101)] {
        let mut bytes = sample_artifact().to_bytes().expect("artifact serializes");
        replace_unique_ratio(&mut bytes, (37, 101), malformed);
        assert_eq!(
            PageArtifact::from_bytes(&bytes),
            Err(ParseError::InvalidGlueSetRatio {
                numerator: malformed.0,
                denominator: malformed.1,
            })
        );
    }
}

#[test]
fn rejects_unknown_version() {
    let mut bytes = sample_artifact().to_bytes().expect("artifact serializes");
    bytes[4] = 99;

    assert_eq!(
        PageArtifact::from_bytes(&bytes),
        Err(ParseError::UnsupportedVersion(99))
    );
}

#[test]
fn rejects_pre_content_identity_v2_artifact_version() {
    let mut bytes = sample_artifact().to_bytes().expect("artifact serializes");
    assert_eq!(bytes[4], 13);
    bytes[4] = 11;

    assert_eq!(
        PageArtifact::from_bytes(&bytes),
        Err(ParseError::UnsupportedVersion(11))
    );
}

#[test]
fn codec_rejects_limits_with_structured_errors() {
    let artifact = sample_artifact();
    let bytes = artifact.to_bytes().expect("artifact serializes");

    let limits = ArtifactCodecLimits {
        max_bytes: bytes.len() - 1,
        ..ArtifactCodecLimits::default()
    };
    assert_eq!(
        PageArtifact::from_bytes_with_limits(&bytes, limits),
        Err(ParseError::LimitExceeded {
            kind: CodecLimitKind::Bytes,
            actual: bytes.len(),
            limit: bytes.len() - 1,
        })
    );

    let limits = ArtifactCodecLimits {
        max_depth: 1,
        ..ArtifactCodecLimits::default()
    };
    assert_eq!(
        PageArtifact::from_bytes_with_limits(&bytes, limits),
        Err(ParseError::LimitExceeded {
            kind: CodecLimitKind::Depth,
            actual: 2,
            limit: 1,
        })
    );

    let limits = ArtifactCodecLimits {
        max_nodes: 1,
        ..ArtifactCodecLimits::default()
    };
    assert_eq!(
        artifact.to_bytes_with_limits(limits),
        Err(SerializeError::LimitExceeded {
            kind: CodecLimitKind::Nodes,
            actual: 2,
            limit: 1,
        })
    );
}

#[test]
fn tiny_input_cannot_request_a_large_collection_allocation() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"UMPG");
    bytes.push(12);
    bytes.extend_from_slice(&1000_i32.to_le_bytes());
    bytes.extend_from_slice(&0_u32.to_le_bytes());
    bytes.extend_from_slice(&0_i32.to_le_bytes());
    bytes.extend_from_slice(&0_i32.to_le_bytes());
    bytes.extend_from_slice(&u32::MAX.to_le_bytes());

    assert_eq!(
        PageArtifact::from_bytes(&bytes),
        Err(ParseError::LimitExceeded {
            kind: CodecLimitKind::CollectionLength,
            actual: u32::MAX as usize,
            limit: ArtifactCodecLimits::default().max_collection_len,
        })
    );

    let len_offset = bytes.len() - std::mem::size_of::<u32>();
    bytes[len_offset..].copy_from_slice(&1000_u32.to_le_bytes());
    assert_eq!(
        PageArtifact::from_bytes(&bytes),
        Err(ParseError::UnexpectedEof)
    );
}

#[test]
fn adversarial_nesting_hits_depth_limit_without_recursive_decode() {
    let depth = ArtifactCodecLimits::default().max_depth + 100;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"UMPG");
    bytes.push(12);
    bytes.extend_from_slice(&1000_i32.to_le_bytes());
    bytes.extend_from_slice(&0_u32.to_le_bytes());
    bytes.extend_from_slice(&0_i32.to_le_bytes());
    bytes.extend_from_slice(&0_i32.to_le_bytes());
    bytes.extend_from_slice(&0_u32.to_le_bytes());
    for _ in 0..10 {
        bytes.extend_from_slice(&0_i32.to_le_bytes());
    }
    for level in 0..depth {
        bytes.push(6);
        for _ in 0..5 {
            bytes.extend_from_slice(&0_i32.to_le_bytes());
        }
        bytes.extend_from_slice(&1_i32.to_le_bytes());
        bytes.push(0);
        bytes.push(0);
        bytes.extend_from_slice(&u32::from(level + 1 < depth).to_le_bytes());
    }
    bytes.extend_from_slice(&0_u32.to_le_bytes());

    let limits = ArtifactCodecLimits::default();
    assert_eq!(
        PageArtifact::from_bytes_with_limits(&bytes, limits),
        Err(ParseError::LimitExceeded {
            kind: CodecLimitKind::Depth,
            actual: limits.max_depth + 1,
            limit: limits.max_depth,
        })
    );
}

#[test]
fn validation_rejects_malformed_graph_references() {
    let mut missing_font = (*sample_artifact()).clone();
    let PageNode::VList(root) = &mut missing_font.root else {
        unreachable!("sample root is a vlist");
    };
    let PageNode::HList(line) = &mut root.children[0] else {
        unreachable!("sample first child is an hlist");
    };
    let PageNode::Char { font_id, .. } = &mut line.children[0] else {
        unreachable!("sample first line child is a character");
    };
    *font_id = 99;
    assert_eq!(
        missing_font.validate(),
        Err(ArtifactValidationError::MissingFont { font_id: 99 })
    );

    let mut missing_effect = (*sample_artifact()).clone();
    let PageNode::VList(root) = &mut missing_effect.root else {
        unreachable!("sample root is a vlist");
    };
    let PageNode::HList(line) = &mut root.children[0] else {
        unreachable!("sample first child is an hlist");
    };
    let PageNode::WhatsitAnchor { effect_index } = &mut line.children[3] else {
        unreachable!("sample fourth line child is an effect anchor");
    };
    *effect_index = 100;
    assert_eq!(
        missing_effect.validate(),
        Err(ArtifactValidationError::MissingEffect { effect_index: 100 })
    );
}

#[test]
fn validation_rejects_invalid_roots_and_duplicate_resources() {
    let mut invalid_root = (*sample_artifact()).clone();
    invalid_root.root = PageNode::Penalty(0);
    assert_eq!(
        invalid_root.validate(),
        Err(ArtifactValidationError::RootNotBox)
    );

    let mut duplicate_font = (*sample_artifact()).clone();
    duplicate_font.fonts.push(duplicate_font.fonts[0].clone());
    assert_eq!(
        duplicate_font.validate(),
        Err(ArtifactValidationError::DuplicateFont { font_id: 1 })
    );
}

#[test]
fn validation_enforces_traversal_budgets_iteratively() {
    let artifact = (*sample_artifact()).clone();
    let limits = ArtifactValidationLimits {
        max_nodes: 2,
        ..ArtifactValidationLimits::default()
    };
    assert_eq!(
        artifact.validate_with_limits(limits),
        Err(ArtifactValidationError::TooManyNodes { count: 3, limit: 2 })
    );

    let mut nested = (*sample_artifact()).clone();
    nested.root = PageNode::VList(empty_box(vec![PageNode::HList(empty_box(vec![]))]));
    let limits = ArtifactValidationLimits {
        max_depth: 1,
        ..ArtifactValidationLimits::default()
    };
    assert_eq!(
        nested.validate_with_limits(limits),
        Err(ArtifactValidationError::NestingTooDeep { depth: 2, limit: 1 })
    );
}

fn empty_box(children: Vec<PageNode>) -> BoxNode {
    BoxNode {
        width: Scaled::from_raw(0),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
        shift: Scaled::from_raw(0),
        glue_set: GlueSetRatio::ZERO,
        glue_sign: GlueSign::Normal,
        glue_order: GlueOrder::Normal,
        children,
    }
}

#[test]
fn page_output_path_rejects_float_arithmetic_sources() {
    let mut violations = Vec::new();

    for (source_path, source) in PAGE_OUTPUT_FLOAT_GUARD_SOURCES {
        for (line_number, line) in source.lines().enumerate() {
            for reason in forbidden_float_usage_reasons(line) {
                if !is_allowed_float_guard_hit(source_path, line) {
                    violations.push(format!(
                        "{}:{}: {reason}: {}",
                        source_path,
                        line_number + 1,
                        line.trim()
                    ));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "page output path must stay fixed-point; forbidden float usage found:\n{}",
        violations.join("\n")
    );
}

const PAGE_OUTPUT_FLOAT_GUARD_SOURCES: &[(&str, &str)] = &[
    (
        "crates/tex-state/src/node.rs",
        include_str!("../../tex-state/src/node.rs"),
    ),
    (
        "crates/tex-typeset/src/packing.rs",
        include_str!("../../tex-typeset/src/packing.rs"),
    ),
    (
        "crates/tex-exec/src/assignments/shipout.rs",
        include_str!("../../tex-exec/src/assignments/shipout.rs"),
    ),
    (
        "crates/tex-exec/src/assignments/hmode.rs",
        include_str!("../../tex-exec/src/assignments/hmode.rs"),
    ),
    ("crates/tex-out/src/model.rs", include_str!("model.rs")),
    ("crates/tex-out/src/binary.rs", include_str!("binary.rs")),
    ("crates/tex-out/src/dvi.rs", include_str!("dvi.rs")),
    (
        "crates/tex-out/src/dvi/movement.rs",
        include_str!("dvi/movement.rs"),
    ),
    (
        "crates/tex-out/src/dvi/tests.rs",
        include_str!("dvi/tests.rs"),
    ),
    (
        "crates/umber/src/lib.rs",
        include_str!("../../umber/src/lib.rs"),
    ),
    (
        "crates/umber/src/main.rs",
        include_str!("../../umber/src/main.rs"),
    ),
];

const FLOAT_ROUNDING_API_PATTERNS: &[&str] = &[
    ".round(",
    ".floor(",
    ".ceil(",
    ".trunc(",
    "round_ties_even(",
];

fn forbidden_float_usage_reasons(line: &str) -> Vec<&'static str> {
    let mut reasons = Vec::new();

    if contains_ident_token(line, "f32") {
        reasons.push("f32 token");
    }
    if contains_ident_token(line, "f64") {
        reasons.push("f64 token");
    }
    for pattern in FLOAT_ROUNDING_API_PATTERNS {
        if line.contains(pattern) {
            reasons.push("float rounding API");
        }
    }

    reasons
}

fn contains_ident_token(line: &str, token: &str) -> bool {
    line.match_indices(token).any(|(start, _)| {
        let before = line[..start].chars().next_back();
        let after = line[start + token.len()..].chars().next();
        before.is_none_or(|ch| !is_rust_ident_char(ch))
            && after.is_none_or(|ch| !is_rust_ident_char(ch))
    })
}

fn is_rust_ident_char(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

fn is_allowed_float_guard_hit(source_path: &str, line: &str) -> bool {
    FLOAT_GUARD_ALLOWLIST.iter().any(|entry| {
        assert!(
            !entry.reason.is_empty(),
            "float guard allowlist entries must document a reason"
        );
        entry.source_path == source_path && line.contains(entry.needle)
    })
}

struct FloatGuardAllow {
    source_path: &'static str,
    needle: &'static str,
    reason: &'static str,
}

// Non-arithmetic false positives only. These fixture font names exercise DVI
// font-definition ordering and are not float types or computations.
const FLOAT_GUARD_ALLOWLIST: &[FloatGuardAllow] = &[FloatGuardAllow {
    source_path: "crates/tex-out/src/dvi/tests.rs",
    needle: "b\"f64\"",
    reason: "DVI test font name fixture",
}];

fn sample_artifact() -> PageArtifact {
    let glue = GlueSpec {
        width: Scaled::from_raw(65_536),
        stretch: Scaled::from_raw(32_768),
        stretch_order: GlueOrder::Fil,
        shrink: Scaled::from_raw(8_192),
        shrink_order: GlueOrder::Normal,
    };
    crate::UnvalidatedPageArtifact {
        job: JobInfo {
            mag: 1200,
            banner: "This is Umber test".to_owned(),
            h_offset: Scaled::from_raw(12_345),
            v_offset: Scaled::from_raw(-54_321),
        },
        fonts: vec![FontResource {
            font_id: 1,
            name: "cmr10".to_owned(),
            tfm_content_hash: ContentHash::from_bytes(b"cmr10.tfm"),
            tfm_checksum: 0x1234_5678,
            design_size: Scaled::from_raw(655_360),
            at_size: Scaled::from_raw(655_360),
            opentype: Some(OpenTypeFontResource {
                program_identity: tex_fonts::FontProgramIdentity::from_bytes([1; 32]),
                object_identity: tex_fonts::FontObjectIdentity::from_bytes([2; 32]),
                instance_identity: tex_fonts::FontInstanceIdentity::from_bytes([3; 32]),
                container: tex_fonts::FontContainer::Woff2,
            }),
        }],
        counts: [0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
        root: PageNode::VList(BoxNode {
            width: Scaled::from_raw(100),
            height: Scaled::from_raw(200),
            depth: Scaled::from_raw(30),
            shift: Scaled::from_raw(0),
            glue_set: GlueSetRatio::from_ratio_parts(37, 101),
            glue_sign: GlueSign::Stretching,
            glue_order: GlueOrder::Fil,
            children: vec![
                PageNode::HList(BoxNode {
                    width: Scaled::from_raw(300),
                    height: Scaled::from_raw(40),
                    depth: Scaled::from_raw(5),
                    shift: Scaled::from_raw(-2),
                    glue_set: GlueSetRatio::ZERO,
                    glue_sign: GlueSign::Normal,
                    glue_order: GlueOrder::Normal,
                    children: vec![
                        PageNode::Char {
                            font_id: 1,
                            ch: 'A' as u32,
                            width: Scaled::from_raw(42),
                        },
                        PageNode::Glue {
                            spec: glue,
                            kind: GlueKind::Leaders,
                            leader: Some(LeaderPayload::Rule {
                                width: None,
                                height: Some(Scaled::from_raw(3)),
                                depth: Some(Scaled::from_raw(1)),
                            }),
                        },
                        PageNode::Kern {
                            amount: Scaled::from_raw(-10),
                            kind: KernKind::Font,
                        },
                        PageNode::WhatsitAnchor { effect_index: 0 },
                    ],
                }),
                PageNode::Rule {
                    width: Some(Scaled::from_raw(50)),
                    height: None,
                    depth: Some(Scaled::from_raw(7)),
                },
                PageNode::Penalty(-50),
                PageNode::Disc {
                    kind: DiscKind::ExplicitHyphen,
                    pre: vec![PageNode::Char {
                        font_id: 1,
                        ch: '-' as u32,
                        width: Scaled::from_raw(20),
                    }],
                    post: Vec::new(),
                    replace: vec![PageNode::Penalty(10)],
                },
                PageNode::Mark {
                    class: 2,
                    tokens: vec![
                        PageToken::Char {
                            ch: 'x' as u32,
                            cat: TokenCatcode::Letter,
                        },
                        PageToken::ControlSequence("foo".to_owned()),
                        PageToken::ActiveControlSequence('~' as u32),
                        PageToken::Param(1),
                    ],
                },
                PageNode::Insert {
                    class: 4,
                    content: vec![PageNode::Kern {
                        amount: Scaled::from_raw(9),
                        kind: KernKind::Explicit,
                    }],
                },
                PageNode::Adjust(vec![PageNode::Glue {
                    spec: glue,
                    kind: GlueKind::Normal,
                    leader: None,
                }]),
            ],
        }),
        effects: vec![
            PageEffect::Write {
                sink: EffectSink::Stream(1),
                text: "expanded write".to_owned(),
            },
            PageEffect::Special {
                class: "dvi".to_owned(),
                payload: b"paper=letter".to_vec(),
            },
            PageEffect::OpenOut {
                stream: 2,
                path: "job.aux".to_owned(),
            },
            PageEffect::CloseOut { stream: 2 },
        ],
    }
    .validate()
    .expect("sample artifact validates")
}

fn replace_unique_ratio(bytes: &mut [u8], old: (i32, i32), new: (i32, i32)) {
    let old = [old.0.to_le_bytes(), old.1.to_le_bytes()].concat();
    let replacement = [new.0.to_le_bytes(), new.1.to_le_bytes()].concat();
    let offsets: Vec<_> = bytes
        .windows(old.len())
        .enumerate()
        .filter_map(|(offset, window)| (window == old).then_some(offset))
        .collect();
    assert_eq!(offsets.len(), 1, "ratio wire must occur exactly once");
    bytes[offsets[0]..offsets[0] + replacement.len()].copy_from_slice(&replacement);
}
