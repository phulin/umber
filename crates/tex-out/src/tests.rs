use crate::{
    BoxNode, ContentHash, DiscKind, EffectSink, FontResource, GlueKind, GlueOrder, GlueSetRatio,
    GlueSign, GlueSpec, JobInfo, KernKind, LeaderPayload, PageArtifact, PageEffect, PageNode,
    PageToken, ParseError, TokenCatcode,
};
use tex_arith::Scaled;

#[test]
fn page_artifact_round_trips() {
    let artifact = sample_artifact();

    let bytes = artifact.to_bytes();
    let parsed = PageArtifact::from_bytes(&bytes).expect("artifact parses");

    assert_eq!(parsed, artifact);
    assert_eq!(parsed.to_bytes(), bytes);
}

#[test]
fn artifact_bytes_and_hash_are_deterministic() {
    let first = sample_artifact();
    let second = sample_artifact();

    let first_bytes = first.to_bytes();
    let second_bytes = second.to_bytes();

    assert_eq!(first_bytes, second_bytes);
    assert_eq!(ContentHash::from_bytes(&first_bytes), first.content_hash());
    assert_eq!(first.content_hash(), second.content_hash());
}

#[test]
fn rejects_unknown_version() {
    let mut bytes = sample_artifact().to_bytes();
    bytes[4] = 99;

    assert_eq!(
        PageArtifact::from_bytes(&bytes),
        Err(ParseError::UnsupportedVersion(99))
    );
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
    PageArtifact {
        job: JobInfo {
            mag: 1200,
            banner: "This is Umber test".to_owned(),
        },
        fonts: vec![FontResource {
            font_id: 1,
            name: "cmr10".to_owned(),
            tfm_content_hash: ContentHash::from_bytes(b"cmr10.tfm"),
            tfm_checksum: 0x1234_5678,
            design_size: Scaled::from_raw(655_360),
            at_size: Scaled::from_raw(655_360),
        }],
        counts: [0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
        root: PageNode::VList(BoxNode {
            width: Scaled::from_raw(100),
            height: Scaled::from_raw(200),
            depth: Scaled::from_raw(30),
            shift: Scaled::from_raw(0),
            glue_set: GlueSetRatio::from_ratio_parts(1, 3),
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
}
