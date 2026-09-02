use super::*;
use core::hash::{Hash, Hasher};

use crate::node_sequence::semantic_value_identity;

#[derive(Hash)]
enum SemanticNodeTag {
    Char,
    Lig,
    Kern,
    MarginKern,
    Glue,
    Penalty,
    Rule,
    HList,
    VList,
    Unset,
    Disc,
    Mark,
    Ins,
    Whatsit,
    MathOn,
    MathOff,
    Direction,
    MathNoad,
    FractionNoad,
    MathStyle,
    MathChoice,
    MathList,
    Nonscript,
    Adjust,
}

struct SemanticRecord<'a> {
    record: NodeRecord<PageMaterialLane>,
    annex: NodeAnnexView<'a>,
}

impl Hash for SemanticRecord<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        macro_rules! fields {
            ($tag:ident $(, $field:expr)* $(,)?) => {{
                SemanticNodeTag::$tag.hash(state);
                $($field.hash(state);)*
            }};
        }

        let record = self.record;
        let annex = self.annex;
        let kind = record.kind().expect("published compact node kind");
        let subtype = record.subtype();
        let flags = record.flags();
        let words = record.words();
        match kind {
            NodeKind::Char => {
                let font = decode_font_words(&words[..4]);
                let ch = char::from_u32(words[4]).expect("published character");
                fields!(Char, font, ch);
            }
            NodeKind::Lig => {
                let payload = annex
                    .resolve_fixed_array::<LigaturePayload, 12>(key_from_record(record))
                    .expect("published ligature payload");
                let mut cursor = 0;
                let font = decode_font(&payload, &mut cursor).expect("published font");
                let ch = char::from_u32(payload[cursor]).expect("published ligature character");
                cursor += 1;
                let source = AnnexKey::<LigatureSource>::from_words(
                    take_words(&payload, &mut cursor).expect("ligature source key"),
                );
                let source = annex
                    .detach_span(source)
                    .expect("published ligature source");
                let count = source[0] as usize;
                let orig: Vec<char> = source[2..]
                    .chunks_exact(2)
                    .map(|pair| char::from_u32(pair[0]).expect("published ligature source char"))
                    .collect();
                assert_eq!(orig.len(), count);
                let left_hit = flags & 1 != 0;
                let right_hit = flags & 2 != 0;
                fields!(Lig, font, ch, orig, left_hit, right_hit);
            }
            NodeKind::Kern => {
                let amount = decode_scaled(words[0]);
                let kind = decode_kern_kind(subtype).expect("published kern kind");
                fields!(Kern, amount, kind);
            }
            NodeKind::MarginKern => {
                let amount = decode_scaled(words[0]);
                let side = decode_margin_side(subtype).expect("published margin side");
                let font = decode_font_words(&words[1..5]);
                let ch = words[5] as u8;
                fields!(MarginKern, amount, side, font, ch);
            }
            NodeKind::Glue => {
                let kind = decode_glue_kind(subtype).expect("published glue kind");
                let (spec, leader) = decode_glue_semantic(record, annex);
                fields!(Glue, spec, kind, leader);
            }
            NodeKind::Penalty => {
                let value = words[0] as i32;
                fields!(Penalty, value);
            }
            NodeKind::Rule => {
                let width = (flags & 1 != 0).then(|| decode_scaled(words[0]));
                let height = (flags & 2 != 0).then(|| decode_scaled(words[1]));
                let depth = (flags & 4 != 0).then(|| decode_scaled(words[2]));
                fields!(Rule, width, height, depth);
            }
            NodeKind::HList | NodeKind::VList => {
                let payload = annex
                    .resolve_fixed_array::<BoxPayload, 28>(key_from_record(record))
                    .expect("published box payload");
                let boxed = decode_box_payload(&payload).expect("published box");
                if kind == NodeKind::HList {
                    fields!(HList, boxed);
                } else {
                    fields!(VList, boxed);
                }
            }
            NodeKind::Unset => {
                let unset = decode_unset_semantic(record, annex);
                fields!(Unset, unset);
            }
            NodeKind::Disc => {
                let payload = annex
                    .resolve_fixed_array::<DiscPayload, 30>(key_from_record(record))
                    .expect("published discretionary payload");
                let mut cursor = 0;
                let kind = decode_disc_kind(subtype).expect("discretionary kind");
                let pre = decode_page_list(&payload, &mut cursor).expect("pre list");
                let post = decode_page_list(&payload, &mut cursor).expect("post list");
                let replace = decode_page_list(&payload, &mut cursor).expect("replace list");
                fields!(Disc, kind, pre, post, replace);
            }
            NodeKind::Mark => {
                let class = flags as u16;
                let tokens = NodeTokenKey::from_coordinates(
                    words[..6].try_into().expect("mark token coordinates"),
                );
                fields!(Mark, class, tokens);
            }
            NodeKind::Ins => {
                let payload = annex
                    .resolve_fixed_array::<InsertionPayload, 17>(key_from_record(record))
                    .expect("published insertion payload");
                let mut cursor = 0;
                let content = decode_page_list(&payload, &mut cursor).expect("insertion list");
                let skip = decode_glue(take_words(&payload, &mut cursor).expect("split skip"))
                    .expect("published split skip");
                let scalar: [u32; 3] = take_words(&payload, &mut cursor).expect("insertion scalar");
                let class = flags as u16;
                let size = decode_scaled(scalar[0]);
                let depth = decode_scaled(scalar[1]);
                let penalty = scalar[2] as i32;
                fields!(Ins, class, size, skip, depth, penalty, content);
            }
            NodeKind::Whatsit => {
                let whatsit = decode_whatsit_value(record, annex).expect("published whatsit");
                fields!(Whatsit, whatsit);
            }
            NodeKind::MathOn | NodeKind::MathOff => {
                let value = decode_scaled(words[0]);
                if kind == NodeKind::MathOn {
                    fields!(MathOn, value);
                } else {
                    fields!(MathOff, value);
                }
            }
            NodeKind::Direction => {
                let direction = match subtype {
                    0 => crate::node::Direction::BeginL,
                    1 => crate::node::Direction::EndL,
                    2 => crate::node::Direction::BeginR,
                    3 => crate::node::Direction::EndR,
                    4 => crate::node::Direction::BeginM,
                    5 => crate::node::Direction::EndM,
                    _ => unreachable!("published direction"),
                };
                fields!(Direction, direction);
            }
            NodeKind::MathNoad => {
                let noad = decode_noad_semantic(record, annex);
                fields!(MathNoad, noad);
            }
            NodeKind::FractionNoad => {
                let fraction = decode_fraction_semantic(record, annex);
                fields!(FractionNoad, fraction);
            }
            NodeKind::MathStyle => {
                let style = decode_math_style(subtype).expect("published math style");
                fields!(MathStyle, style);
            }
            NodeKind::MathChoice => {
                let choice = decode_choice_semantic(record, annex);
                fields!(MathChoice, choice);
            }
            NodeKind::MathList => {
                let list = record.math_list(annex).expect("published math list");
                fields!(MathList, list);
            }
            NodeKind::Nonscript => fields!(Nonscript),
            NodeKind::Adjust => {
                let payload = annex
                    .resolve_fixed_array::<ListPayload, 10>(key_from_record(record))
                    .expect("published adjustment");
                let mut cursor = 0;
                let adjust = AdjustNode {
                    pre: flags == 1,
                    content: decode_page_list(&payload, &mut cursor).expect("adjustment list"),
                };
                fields!(Adjust, adjust);
            }
        }
    }
}

fn decode_font_words(words: &[u32]) -> FontId {
    FontId::from_words(words.try_into().expect("four font words"))
        .expect("published font coordinate")
}

fn decode_glue_semantic(
    record: NodeRecord<PageMaterialLane>,
    annex: NodeAnnexView<'_>,
) -> (GlueSpec, Option<LeaderPayload<PageListId>>) {
    let flags = record.flags();
    let words = record.words();
    match flags & 3 {
        0 => (
            decode_glue(words[..4].try_into().expect("glue words")).expect("published glue"),
            None,
        ),
        1 => (
            decode_glue(words[..4].try_into().expect("glue words")).expect("published glue"),
            Some(LeaderPayload::Rule {
                width: (flags & 4 != 0).then(|| decode_scaled(words[4])),
                height: (flags & 8 != 0).then(|| decode_scaled(words[5])),
                depth: (flags & 16 != 0).then(|| decode_scaled(words[6])),
            }),
        ),
        leader @ (2 | 3) => {
            let payload = annex
                .resolve_fixed_array::<LeaderBoxPayload, 32>(key_from_record(record))
                .expect("published leader payload");
            let spec = decode_glue(payload[..4].try_into().expect("leader glue"))
                .expect("published leader glue");
            let boxed = decode_box_payload(&payload[4..]).expect("published leader box");
            (
                spec,
                Some(if leader == 2 {
                    LeaderPayload::HList(boxed)
                } else {
                    LeaderPayload::VList(boxed)
                }),
            )
        }
        _ => unreachable!("published glue leader tag"),
    }
}

fn decode_unset_semantic(
    record: NodeRecord<PageMaterialLane>,
    annex: NodeAnnexView<'_>,
) -> UnsetNode<PageListId> {
    let flags = record.flags();
    let payload = annex
        .resolve_fixed_array::<UnsetPayload, 15>(key_from_record(record))
        .expect("published unset payload");
    let mut cursor = 0;
    let children = decode_page_list(&payload, &mut cursor).expect("unset child");
    let values: [u32; 5] = take_words(&payload, &mut cursor).expect("unset scalars");
    UnsetNode::new(UnsetNodeFields {
        kind: if flags & (1 << 20) == 0 {
            UnsetKind::HBox
        } else {
            UnsetKind::VBox
        },
        width: decode_scaled(values[0]),
        height: decode_scaled(values[1]),
        depth: decode_scaled(values[2]),
        span_count: flags as u16,
        stretch: decode_scaled(values[3]),
        stretch_order: decode_order((flags >> 16) & 3).expect("stretch order"),
        shrink: decode_scaled(values[4]),
        shrink_order: decode_order((flags >> 18) & 3).expect("shrink order"),
        children,
    })
}

fn decode_noad_semantic(
    record: NodeRecord<PageMaterialLane>,
    annex: NodeAnnexView<'_>,
) -> MathNoad<PageListId> {
    let payload = annex
        .resolve_fixed_array::<MathNoadPayload, 36>(key_from_record(record))
        .expect("published math noad");
    let mut cursor = 0;
    MathNoad {
        kind: decode_noad_kind(&payload, &mut cursor).expect("noad kind"),
        nucleus: decode_math_field(&payload, &mut cursor).expect("nucleus"),
        subscript: decode_math_field(&payload, &mut cursor).expect("subscript"),
        superscript: decode_math_field(&payload, &mut cursor).expect("superscript"),
    }
}

fn decode_fraction_semantic(
    record: NodeRecord<PageMaterialLane>,
    annex: NodeAnnexView<'_>,
) -> MathFraction<PageListId> {
    let flags = record.flags();
    let payload = annex
        .resolve_fixed_array::<FractionPayload, 23>(key_from_record(record))
        .expect("published fraction");
    let mut cursor = 0;
    let numerator = decode_page_list(&payload, &mut cursor).expect("numerator");
    let denominator = decode_page_list(&payload, &mut cursor).expect("denominator");
    let thickness_word = payload[cursor];
    cursor += 1;
    let delimiters: [u32; 2] = take_words(&payload, &mut cursor).expect("delimiters");
    MathFraction {
        numerator,
        denominator,
        thickness: if flags & 4 != 0 {
            FractionThickness::Default
        } else {
            FractionThickness::Explicit(decode_scaled(thickness_word))
        },
        left_delimiter: (flags & 1 != 0).then_some(delimiters[0]),
        right_delimiter: (flags & 2 != 0).then_some(delimiters[1]),
    }
}

fn decode_choice_semantic(
    record: NodeRecord<PageMaterialLane>,
    annex: NodeAnnexView<'_>,
) -> MathChoice<PageListId> {
    let payload = annex
        .resolve_fixed_array::<MathChoicePayload, 40>(key_from_record(record))
        .expect("published math choice");
    let mut cursor = 0;
    MathChoice {
        display: decode_page_list(&payload, &mut cursor).expect("display choice"),
        text: decode_page_list(&payload, &mut cursor).expect("text choice"),
        script: decode_page_list(&payload, &mut cursor).expect("script choice"),
        script_script: decode_page_list(&payload, &mut cursor).expect("scriptscript choice"),
    }
}

impl NodeRecord<PageMaterialLane> {
    pub(crate) fn semantic_identity(self, annex: NodeAnnexView<'_>) -> u64 {
        semantic_value_identity(&SemanticRecord {
            record: self,
            annex,
        })
    }
}
