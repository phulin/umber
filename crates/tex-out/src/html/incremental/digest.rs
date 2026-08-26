use sha2::{Digest as _, Sha256};
use tex_arith::Scaled;

use super::{
    HtmlFontKey, MathStart, PositionedPage, RENDER_SCHEMA_VERSION, RenderDigest, RenderDirection,
    RenderKey, RenderMathDrawing, RenderMathGlyph, RenderNode, RenderNodeValue, RenderPage,
    RenderResource, RenderSpecialAction, TextUnit,
};
use crate::MathGlyphSelection;
use crate::positioned::BoxKind;

pub(super) fn derive_key(
    parent: RenderKey,
    digest: RenderDigest,
    occurrence: u64,
    revision: u64,
) -> RenderKey {
    let mut hash = Sha256::new();
    hash.update(b"umber-html-render-key-v1\0");
    hash.update(parent.0);
    hash.update(digest.0);
    hash.update(occurrence.to_le_bytes());
    hash.update(revision.to_le_bytes());
    let bytes: [u8; 32] = hash.finalize().into();
    RenderKey(bytes[..16].try_into().expect("digest prefix"))
}

pub(super) fn page_match_digest(page: &PositionedPage, nodes: &[RenderNode]) -> RenderDigest {
    let mut hash = CanonicalHash::new(b"umber-html-page-match-v1\0");
    hash.scaled(page.width);
    hash.scaled(page.height);
    for node in nodes {
        hash.bytes(&node.digest.0);
    }
    hash.finish()
}

pub(super) fn page_digest(page: &RenderPage) -> RenderDigest {
    let mut hash = CanonicalHash::new(b"umber-html-page-v1\0");
    hash.bytes(&page.key.0);
    hash.u32(page.ordinal);
    hash.scaled(page.width);
    hash.scaled(page.height);
    hash.scaled(page.origin_x);
    hash.scaled(page.origin_y);
    hash.i32(page.mag);
    for count in page.counts {
        hash.i32(count);
    }
    for node in &page.nodes {
        hash.bytes(&node.key.0);
        hash.bytes(&node.digest.0);
    }
    hash.finish()
}

pub(super) fn revision_digest(
    title: &str,
    language: &str,
    pages: &[RenderPage],
    resources: &[RenderResource],
) -> RenderDigest {
    let mut hash = CanonicalHash::new(b"umber-html-revision-v1\0");
    hash.u16(RENDER_SCHEMA_VERSION);
    hash.string(title);
    hash.string(language);
    for page in pages {
        hash.bytes(&page.digest.0);
    }
    for resource in resources {
        hash.bytes(&resource.identity);
        hash.u64(resource.bytes.len() as u64);
        hash.string(&resource.family);
    }
    hash.finish()
}

pub(super) fn node_value_digest(value: &RenderNodeValue, matching: bool) -> RenderDigest {
    let mut hash = CanonicalHash::new(if matching {
        b"umber-html-node-match-v1\0"
    } else {
        b"umber-html-node-v1\0"
    });
    match value {
        RenderNodeValue::Box(value) => {
            hash.u8(1);
            hash.u8(match value.kind {
                BoxKind::Horizontal => 0,
                BoxKind::Vertical => 1,
            });
            if !matching {
                hash.u32(value.id);
                encode_geometry(&mut hash, value.x, value.y, value.width, value.height);
                hash.scaled(value.baseline);
            }
        }
        RenderNodeValue::Rule(value) => {
            hash.u8(2);
            if !matching {
                encode_geometry(&mut hash, value.x, value.y, value.width, value.height);
                hash.option_string(value.color.as_deref());
            }
        }
        RenderNodeValue::Text(value) => {
            hash.u8(3);
            encode_font_key(&mut hash, &value.font);
            hash.option_u32(value.accessibility_line);
            if !matching {
                hash.scaled(value.x);
                hash.scaled(value.baseline);
                for position in &value.positions {
                    hash.scaled(*position);
                }
                for unit in &value.units {
                    match unit {
                        TextUnit::Code(code) => {
                            hash.u8(0);
                            hash.u32(*code);
                        }
                        TextUnit::Space => hash.u8(1),
                    }
                }
                hash.string(&value.text);
                hash.string(&value.family);
                hash.bytes(&value.resource);
                hash.u8(match value.direction {
                    RenderDirection::LeftToRight => 0,
                    RenderDirection::RightToLeft => 1,
                });
                hash.option_bytes4(value.script);
                hash.option_string(value.language.as_deref());
                for (tag, setting) in &value.features {
                    hash.bytes(tag);
                    hash.u32(*setting);
                }
                for (tag, coordinate) in &value.variations {
                    hash.bytes(tag);
                    hash.i32(*coordinate);
                }
                hash.option_string(value.color.as_deref());
                hash.option_string(value.link.as_deref());
            }
        }
        RenderNodeValue::Special(value) => {
            hash.u8(4);
            hash.string(&value.class);
            if !matching {
                hash.scaled(value.x);
                hash.scaled(value.y);
                hash.bytes(&value.payload);
                encode_special_action(&mut hash, &value.action);
            }
        }
        RenderNodeValue::MathStart(value) => {
            hash.u8(5);
            if !matching {
                encode_math_start(&mut hash, value);
            }
        }
        RenderNodeValue::MathGlyph(value) => {
            hash.u8(6);
            hash.bytes(&value.glyph.font_instance.bytes());
            hash.u16(value.glyph.glyph_id);
            if !matching {
                encode_math_glyph(&mut hash, value);
            }
        }
        RenderNodeValue::MathRule(value) => {
            hash.u8(7);
            if !matching {
                encode_geometry(&mut hash, value.x, value.y, value.width, value.height);
            }
        }
        RenderNodeValue::MathEnd => hash.u8(8),
    }
    hash.finish()
}

fn encode_font_key(hash: &mut CanonicalHash, value: &HtmlFontKey) {
    hash.string(&value.name);
    hash.bytes(&value.tfm_content_hash);
    hash.u32(value.tfm_checksum);
    hash.i32(value.design_size_raw);
    hash.i32(value.at_size_raw);
    hash.option_digest(value.opentype_program_identity.map(|value| value.bytes()));
    hash.option_digest(value.opentype_instance_identity.map(|value| value.bytes()));
}

fn encode_special_action(hash: &mut CanonicalHash, value: &RenderSpecialAction) {
    match value {
        RenderSpecialAction::ColorPush(value) => {
            hash.u8(0);
            hash.string(value);
        }
        RenderSpecialAction::ColorPop => hash.u8(1),
        RenderSpecialAction::LinkStart(value) => {
            hash.u8(2);
            hash.string(value);
        }
        RenderSpecialAction::LinkEnd => hash.u8(3),
        RenderSpecialAction::Destination(value) => {
            hash.u8(4);
            hash.string(value);
        }
        RenderSpecialAction::Inert => hash.u8(5),
    }
}

fn encode_math_start(hash: &mut CanonicalHash, value: &MathStart) {
    hash.u32(value.id);
    hash.scaled(value.x);
    hash.scaled(value.baseline);
    hash.scaled(value.width);
    hash.scaled(value.height);
    hash.scaled(value.depth);
}

fn encode_math_glyph(hash: &mut CanonicalHash, value: &RenderMathGlyph) {
    let glyph = value.glyph;
    hash.bytes(&glyph.font_instance.bytes());
    hash.u16(glyph.glyph_id);
    match glyph.selection {
        MathGlyphSelection::Cmap { scalar } => {
            hash.u8(0);
            hash.u32(scalar);
        }
        MathGlyphSelection::OutlineFallback => hash.u8(1),
    }
    hash.u8(glyph.ssty);
    hash.scaled(glyph.x);
    hash.scaled(glyph.baseline);
    hash.scaled(glyph.width);
    hash.scaled(glyph.height);
    hash.scaled(glyph.depth);
    match &value.drawing {
        RenderMathDrawing::Text {
            scalar,
            family,
            font_size_raw,
            variations,
        } => {
            hash.u8(0);
            hash.u32(*scalar as u32);
            hash.string(family);
            hash.i32(*font_size_raw);
            hash.u64(variations.len() as u64);
            for (tag, setting) in variations {
                hash.bytes(tag);
                hash.i32(*setting);
            }
        }
        RenderMathDrawing::Outline {
            path,
            units_per_em,
            font_size_raw,
        } => {
            hash.u8(1);
            hash.string(path);
            hash.u16(*units_per_em);
            hash.i32(*font_size_raw);
        }
    }
}

fn encode_geometry(hash: &mut CanonicalHash, x: Scaled, y: Scaled, width: Scaled, height: Scaled) {
    hash.scaled(x);
    hash.scaled(y);
    hash.scaled(width);
    hash.scaled(height);
}

struct CanonicalHash(Sha256);

impl CanonicalHash {
    fn new(domain: &[u8]) -> Self {
        let mut hash = Sha256::new();
        hash.update(domain);
        Self(hash)
    }

    fn bytes(&mut self, value: &[u8]) {
        self.u64(value.len() as u64);
        self.0.update(value);
    }

    fn string(&mut self, value: &str) {
        self.bytes(value.as_bytes());
    }

    fn u8(&mut self, value: u8) {
        self.0.update([value]);
    }

    fn u16(&mut self, value: u16) {
        self.0.update(value.to_le_bytes());
    }

    fn u32(&mut self, value: u32) {
        self.0.update(value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.0.update(value.to_le_bytes());
    }

    fn i32(&mut self, value: i32) {
        self.0.update(value.to_le_bytes());
    }

    fn scaled(&mut self, value: Scaled) {
        self.i32(value.raw());
    }

    fn option_u32(&mut self, value: Option<u32>) {
        match value {
            Some(value) => {
                self.u8(1);
                self.u32(value);
            }
            None => self.u8(0),
        }
    }

    fn option_string(&mut self, value: Option<&str>) {
        match value {
            Some(value) => {
                self.u8(1);
                self.string(value);
            }
            None => self.u8(0),
        }
    }

    fn option_bytes4(&mut self, value: Option<[u8; 4]>) {
        match value {
            Some(value) => {
                self.u8(1);
                self.bytes(&value);
            }
            None => self.u8(0),
        }
    }

    fn option_digest(&mut self, value: Option<[u8; 8]>) {
        match value {
            Some(value) => {
                self.u8(1);
                self.bytes(&value);
            }
            None => self.u8(0),
        }
    }

    fn finish(self) -> RenderDigest {
        RenderDigest(self.0.finalize().into())
    }
}
