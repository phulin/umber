use sha2::{Digest, Sha256};
use tex_arith::Scaled;

use crate::{
    BoxNode, ContentHash, FontResource, GlueOrder, GlueSetRatio, GlueSign, JobInfo, PageEffect,
    PageNode, UnvalidatedPageArtifact,
};

use super::{HtmlError, HtmlFontKey, HtmlFontResolver, HtmlOptions, WebFont, write_html};

fn sp(raw: i32) -> Scaled {
    Scaled::from_raw(raw)
}

struct Resolver {
    missing_b: bool,
}

impl HtmlFontResolver for Resolver {
    fn resolve(&mut self, font: &FontResource) -> Result<WebFont, String> {
        let bytes = b"deterministic-test-woff2".to_vec();
        let mut encoding = vec![None; 256];
        encoding[usize::from(b'A')] = Some("A".to_owned());
        if !self.missing_b {
            encoding[usize::from(b'B')] = Some("<&B".to_owned());
        }
        Ok(WebFont {
            key: HtmlFontKey::from(font),
            sha256: Sha256::digest(&bytes).into(),
            woff2: bytes,
            encoding,
            provenance: "test fixture".to_owned(),
            embeddable: true,
        })
    }
}

#[test]
fn serialization_is_deterministic_exact_and_escaped() {
    let page = page();
    let mut first_resolver = Resolver { missing_b: false };
    let first = write_html(
        std::slice::from_ref(&page),
        &mut first_resolver,
        &HtmlOptions::default(),
    )
    .expect("first HTML");
    let mut second_resolver = Resolver { missing_b: false };
    let second =
        write_html(&[page], &mut second_resolver, &HtmlOptions::default()).expect("second HTML");
    assert_eq!(first, second);
    let html = String::from_utf8(first.html).expect("UTF-8 HTML");
    assert!(html.contains("data-umber-x-sp=\"17\""));
    assert!(html.contains("data-umber-baseline-sp=\"53\""));
    assert!(html.contains("A&lt;&amp;B"));
    assert!(!html.contains("<script>alert(1)</script>"));
    assert!(
        html.contains(
            "data-umber-special-hex=\"3c7363726970743e616c6572742831293c2f7363726970743e\""
        )
    );
}

#[test]
fn unavailable_text_mapping_is_actionable() {
    let mut resolver = Resolver { missing_b: true };
    let error =
        write_html(&[page()], &mut resolver, &HtmlOptions::default()).expect_err("mapping failure");
    assert_eq!(
        error,
        HtmlError::MissingTextMapping {
            font: "cmr10".to_owned(),
            code: b'B'
        }
    );
}

#[test]
fn allowlisted_color_link_and_destination_are_typed_and_escaped() {
    let mut page = page();
    page.testing_mut().effects = vec![
        PageEffect::Special {
            class: "html".to_owned(),
            payload: b"color push red".to_vec(),
        },
        PageEffect::Special {
            class: "html".to_owned(),
            payload: b"link https://example.test/path?a=1&b=2".to_vec(),
        },
        PageEffect::Special {
            class: "html".to_owned(),
            payload: b"endlink".to_vec(),
        },
        PageEffect::Special {
            class: "html".to_owned(),
            payload: b"dest section.1".to_vec(),
        },
    ];
    let PageNode::HList(root) = &mut page.testing_mut().root else {
        unreachable!()
    };
    root.children = vec![
        PageNode::WhatsitAnchor { effect_index: 0 },
        PageNode::WhatsitAnchor { effect_index: 1 },
        PageNode::Char {
            font_id: 7,
            ch: b'A' as u32,
            width: sp(30),
        },
        PageNode::WhatsitAnchor { effect_index: 2 },
        PageNode::WhatsitAnchor { effect_index: 3 },
    ];
    let mut resolver = Resolver { missing_b: false };
    let output = write_html(&[page], &mut resolver, &HtmlOptions::default()).expect("special HTML");
    let html = String::from_utf8(output.html).expect("UTF-8");
    assert!(html.contains("<svg class=\"umber-run\""));
    assert!(
        html.contains(";color:red\"><rect class=\"umber-baseline\"")
            && html.contains("<a href=\"https://example.test/path?a=1&amp;b=2\""),
        "{html}"
    );
    assert!(html.contains("id=\"umber-dest-section.1\""));
}

#[test]
fn dangerous_link_special_fails_without_markup_injection() {
    let mut page = page();
    page.testing_mut().effects[0] = PageEffect::Special {
        class: "html".to_owned(),
        payload: b"link javascript:alert(1)".to_vec(),
    };
    let mut resolver = Resolver { missing_b: false };
    assert!(matches!(
        write_html(&[page], &mut resolver, &HtmlOptions::default()),
        Err(HtmlError::InvalidSpecial { .. })
    ));
}

fn page() -> crate::PageArtifact {
    let font = FontResource {
        font_id: 7,
        name: "cmr10".to_owned(),
        tfm_content_hash: ContentHash::from_bytes(b"cmr10"),
        tfm_checksum: 123,
        design_size: sp(655_360),
        at_size: sp(655_360),
    };
    UnvalidatedPageArtifact {
        job: JobInfo {
            mag: 1000,
            banner: "test".to_owned(),
            h_offset: sp(17),
            v_offset: sp(13),
        },
        fonts: vec![font],
        counts: [0; 10],
        root: PageNode::HList(BoxNode {
            width: sp(200),
            height: sp(40),
            depth: sp(5),
            shift: sp(0),
            glue_set: GlueSetRatio::ZERO,
            glue_sign: GlueSign::Normal,
            glue_order: GlueOrder::Normal,
            children: vec![
                PageNode::Char {
                    font_id: 7,
                    ch: b'A' as u32,
                    width: sp(30),
                },
                PageNode::Char {
                    font_id: 7,
                    ch: b'B' as u32,
                    width: sp(30),
                },
                PageNode::WhatsitAnchor { effect_index: 0 },
            ],
        }),
        effects: vec![PageEffect::Special {
            class: "dvi".to_owned(),
            payload: b"<script>alert(1)</script>".to_vec(),
        }],
    }
    .validate()
    .expect("valid page")
}
