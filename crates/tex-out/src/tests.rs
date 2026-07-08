use crate::{
    BoxNode, ContentHash, EffectSink, FontResource, GlueKind, GlueOrder, GlueSetRatio, GlueSign,
    GlueSpec, JobInfo, KernKind, PageArtifact, PageEffect, PageNode, ParseError,
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
            glue_set: GlueSetRatio::from_raw(12_345),
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
                            kind: GlueKind::Normal,
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
