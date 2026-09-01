//! Private compact resident node and typed word-annex substrate.

use core::marker::PhantomData;
use core::num::NonZeroU32;
use core::sync::atomic::{AtomicU32, Ordering};

use crate::fork_arena::PageMaterialLane;
use crate::glue::{GlueSpec, Order};
use crate::ids::FontId;
use crate::math::{
    FractionThickness, LimitType, MathChar, MathChoice, MathField, MathFraction, MathListNode,
    MathNoad, MathStyle, NoadClass, NoadKind,
};
use crate::node::{
    AdjustNode, BoxLr, BoxNode, BoxNodeFields, DiscKind, GlueKind, KernKind, LeaderPayload,
    MarginKernSide, Node, NodeKind, NodePdfActionIdentifier, NodeTokenKey, PdfAccessibilityControl,
    PdfDestinationKind, PdfDestinationNode, PdfLiteralMode, PdfThreadNode, Sign, UnsetKind,
    UnsetNode, UnsetNodeFields, Whatsit,
};
use crate::page_node_arena::PageListId;
use crate::scaled::{GlueSetRatio, Scaled};
use crate::token::OriginId;
use crate::world::{PrintSink, StreamSlot};

const ANNEX_WORDS_PER_BLOCK: usize = 16_384;
const HEADER_PRESENT: u32 = 1 << 31;
const KIND_MASK: u32 = 0x1f;
const SUBTYPE_SHIFT: u32 = 5;
const SUBTYPE_MASK: u32 = 0x1f << SUBTYPE_SHIFT;
const FLAGS_SHIFT: u32 = 10;
const FLAGS_MASK: u32 = !(KIND_MASK | SUBTYPE_MASK | HEADER_PRESENT);

static NEXT_ANNEX_OWNER: AtomicU32 = AtomicU32::new(1);

fn bool_word(value: bool) -> u32 {
    u32::from(value)
}

fn decode_bool(value: u32) -> Option<bool> {
    match value {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    }
}

fn scaled_word(value: Scaled) -> u32 {
    value.raw() as u32
}

fn decode_scaled(value: u32) -> Scaled {
    Scaled::from_raw(value as i32)
}

fn encode_option_scaled(value: Option<Scaled>) -> [u32; 2] {
    match value {
        Some(value) => [1, scaled_word(value)],
        None => [0, 0],
    }
}

fn decode_option_scaled(words: [u32; 2]) -> Option<Option<Scaled>> {
    match words[0] {
        0 if words[1] == 0 => Some(None),
        1 => Some(Some(decode_scaled(words[1]))),
        _ => None,
    }
}

fn encode_glue(value: GlueSpec) -> [u32; 4] {
    [
        scaled_word(value.width),
        scaled_word(value.stretch),
        scaled_word(value.shrink),
        (value.stretch_order as u32) | ((value.shrink_order as u32) << 8),
    ]
}

fn decode_order(value: u32) -> Option<Order> {
    match value {
        0 => Some(Order::Normal),
        1 => Some(Order::Fil),
        2 => Some(Order::Fill),
        3 => Some(Order::Filll),
        _ => None,
    }
}

fn decode_glue(words: [u32; 4]) -> Option<GlueSpec> {
    if words[3] & !0xffff != 0 {
        return None;
    }
    Some(GlueSpec {
        width: decode_scaled(words[0]),
        stretch: decode_scaled(words[1]),
        shrink: decode_scaled(words[2]),
        stretch_order: decode_order(words[3] & 0xff)?,
        shrink_order: decode_order((words[3] >> 8) & 0xff)?,
    })
}

fn append_words<const N: usize>(destination: &mut Vec<u32>, words: [u32; N]) {
    destination.extend(words);
}

fn take_words<const N: usize>(source: &[u32], cursor: &mut usize) -> Option<[u32; N]> {
    let end = cursor.checked_add(N)?;
    let words = source.get(*cursor..end)?.try_into().ok()?;
    *cursor = end;
    Some(words)
}

fn decode_node_kind(value: u32) -> Option<NodeKind> {
    NodeKind::ALL.get(value as usize).copied()
}

fn encode_kern_kind(value: KernKind) -> u8 {
    match value {
        KernKind::Explicit => 0,
        KernKind::Font => 1,
        KernKind::Auto => 2,
        KernKind::Accent => 3,
        KernKind::Mu => 4,
        KernKind::LeftMargin => 5,
        KernKind::RightMargin => 6,
    }
}

fn decode_kern_kind(value: u8) -> Option<KernKind> {
    Some(match value {
        0 => KernKind::Explicit,
        1 => KernKind::Font,
        2 => KernKind::Auto,
        3 => KernKind::Accent,
        4 => KernKind::Mu,
        5 => KernKind::LeftMargin,
        6 => KernKind::RightMargin,
        _ => return None,
    })
}

fn encode_margin_side(value: MarginKernSide) -> u8 {
    match value {
        MarginKernSide::Left => 0,
        MarginKernSide::Right => 1,
    }
}

fn decode_margin_side(value: u8) -> Option<MarginKernSide> {
    match value {
        0 => Some(MarginKernSide::Left),
        1 => Some(MarginKernSide::Right),
        _ => None,
    }
}

fn encode_glue_kind(value: GlueKind) -> u8 {
    match value {
        GlueKind::Normal => 0,
        GlueKind::SpaceSkip => 1,
        GlueKind::XSpaceSkip => 2,
        GlueKind::TabSkip => 3,
        GlueKind::BaselineSkip => 4,
        GlueKind::LineSkip => 5,
        GlueKind::TopSkip => 6,
        GlueKind::SplitTopSkip => 7,
        GlueKind::LeftSkip => 8,
        GlueKind::RightSkip => 9,
        GlueKind::ParSkip => 10,
        GlueKind::ParFillSkip => 11,
        GlueKind::AboveDisplaySkip => 12,
        GlueKind::BelowDisplaySkip => 13,
        GlueKind::AboveDisplayShortSkip => 14,
        GlueKind::BelowDisplayShortSkip => 15,
        GlueKind::Leaders => 16,
        GlueKind::Cleaders => 17,
        GlueKind::Xleaders => 18,
        GlueKind::MuSkip => 19,
        GlueKind::ThinMuSkip => 20,
        GlueKind::MedMuSkip => 21,
        GlueKind::ThickMuSkip => 22,
        GlueKind::NonScript => 23,
    }
}

fn decode_glue_kind(value: u8) -> Option<GlueKind> {
    Some(match value {
        0 => GlueKind::Normal,
        1 => GlueKind::SpaceSkip,
        2 => GlueKind::XSpaceSkip,
        3 => GlueKind::TabSkip,
        4 => GlueKind::BaselineSkip,
        5 => GlueKind::LineSkip,
        6 => GlueKind::TopSkip,
        7 => GlueKind::SplitTopSkip,
        8 => GlueKind::LeftSkip,
        9 => GlueKind::RightSkip,
        10 => GlueKind::ParSkip,
        11 => GlueKind::ParFillSkip,
        12 => GlueKind::AboveDisplaySkip,
        13 => GlueKind::BelowDisplaySkip,
        14 => GlueKind::AboveDisplayShortSkip,
        15 => GlueKind::BelowDisplayShortSkip,
        16 => GlueKind::Leaders,
        17 => GlueKind::Cleaders,
        18 => GlueKind::Xleaders,
        19 => GlueKind::MuSkip,
        20 => GlueKind::ThinMuSkip,
        21 => GlueKind::MedMuSkip,
        22 => GlueKind::ThickMuSkip,
        23 => GlueKind::NonScript,
        _ => return None,
    })
}

fn encode_disc_kind(value: DiscKind) -> u8 {
    match value {
        DiscKind::Discretionary => 0,
        DiscKind::ExplicitHyphen => 1,
        DiscKind::AutomaticHyphen => 2,
    }
}

fn decode_disc_kind(value: u8) -> Option<DiscKind> {
    match value {
        0 => Some(DiscKind::Discretionary),
        1 => Some(DiscKind::ExplicitHyphen),
        2 => Some(DiscKind::AutomaticHyphen),
        _ => None,
    }
}

fn encode_math_style(value: MathStyle) -> u8 {
    match value {
        MathStyle::Display => 0,
        MathStyle::Text => 1,
        MathStyle::Script => 2,
        MathStyle::ScriptScript => 3,
    }
}

fn decode_math_style(value: u8) -> Option<MathStyle> {
    match value {
        0 => Some(MathStyle::Display),
        1 => Some(MathStyle::Text),
        2 => Some(MathStyle::Script),
        3 => Some(MathStyle::ScriptScript),
        _ => None,
    }
}

fn encode_print_sink(value: PrintSink) -> u32 {
    match value {
        PrintSink::Terminal => 0,
        PrintSink::Log => 1,
        PrintSink::TerminalAndLog => 2,
        PrintSink::Stream(slot) => 3 | (u32::from(slot.raw()) << 8),
    }
}

fn decode_print_sink(value: u32) -> Option<PrintSink> {
    match value & 0xff {
        0 if value == 0 => Some(PrintSink::Terminal),
        1 if value == 1 => Some(PrintSink::Log),
        2 if value == 2 => Some(PrintSink::TerminalAndLog),
        3 if value >> 8 < 16 => Some(PrintSink::Stream(StreamSlot::new((value >> 8) as u8))),
        _ => None,
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct NodeRecord<Lane = PageMaterialLane> {
    header: NonZeroU32,
    words: [u32; 7],
    lane: PhantomData<fn(&Lane) -> &Lane>,
}

impl<Lane> NodeRecord<Lane> {
    pub(crate) fn new(kind: NodeKind, subtype: u8, flags: u32, words: [u32; 7]) -> Self {
        assert!(subtype < 32, "compact node subtype fits five bits");
        assert_eq!(
            flags & !0x1f_ffff,
            0,
            "compact node flags fit twenty-one bits"
        );
        let header = HEADER_PRESENT
            | kind as u32
            | (u32::from(subtype) << SUBTYPE_SHIFT)
            | (flags << FLAGS_SHIFT);
        Self {
            header: NonZeroU32::new(header).expect("resident node header is nonzero"),
            words,
            lane: PhantomData,
        }
    }

    pub(crate) fn kind(self) -> Option<NodeKind> {
        NodeKind::ALL
            .get((self.header.get() & KIND_MASK) as usize)
            .copied()
    }

    pub(crate) fn subtype(self) -> u8 {
        ((self.header.get() & SUBTYPE_MASK) >> SUBTYPE_SHIFT) as u8
    }

    pub(crate) fn flags(self) -> u32 {
        (self.header.get() & FLAGS_MASK) >> FLAGS_SHIFT
    }

    pub(crate) const fn words(self) -> [u32; 7] {
        self.words
    }
}

impl NodeRecord<PageMaterialLane> {
    fn with_key<Kind>(kind: NodeKind, subtype: u8, flags: u32, key: AnnexKey<Kind>) -> Self {
        let key = key_words(key);
        Self::new(
            kind,
            subtype,
            flags,
            [key[0], key[1], key[2], key[3], key[4], key[5], 0],
        )
    }

    pub(crate) fn encode_owned(node: Node, annex: &mut NodeAnnexArena) -> Self {
        match node {
            Node::Char { font, ch, origin } => {
                let font = font.words();
                Self::new(
                    NodeKind::Char,
                    0,
                    0,
                    [
                        font[0],
                        font[1],
                        font[2],
                        font[3],
                        ch as u32,
                        origin.raw(),
                        0,
                    ],
                )
            }
            Node::Lig {
                font,
                ch,
                orig,
                left_hit,
                right_hit,
                origins,
            } => {
                assert!(
                    origins.is_empty() || origins.len() == orig.len(),
                    "ligature origin rows are empty or parallel to source characters"
                );
                let mut source = Vec::with_capacity(2 + orig.len() * 2);
                source.push(u32::try_from(orig.len()).expect("ligature source length fits u32"));
                source.push(bool_word(origins.is_empty()));
                for (index, ch) in orig.into_iter().enumerate() {
                    source.push(ch as u32);
                    source.push(origins.get(index).copied().unwrap_or_default().raw());
                }
                let source = annex.append_span::<LigatureSource>(&source);
                let mut payload = Vec::with_capacity(11);
                encode_font(&mut payload, font);
                payload.push(ch as u32);
                append_words(&mut payload, source.words());
                let key = annex.append_fixed::<LigaturePayload>(&payload);
                Self::with_key(
                    NodeKind::Lig,
                    0,
                    bool_word(left_hit) | (bool_word(right_hit) << 1),
                    key,
                )
            }
            Node::Kern { amount, kind } => Self::new(
                NodeKind::Kern,
                encode_kern_kind(kind),
                0,
                [scaled_word(amount), 0, 0, 0, 0, 0, 0],
            ),
            Node::MarginKern {
                amount,
                side,
                font,
                ch,
            } => {
                let font = font.words();
                Self::new(
                    NodeKind::MarginKern,
                    encode_margin_side(side),
                    0,
                    [
                        scaled_word(amount),
                        font[0],
                        font[1],
                        font[2],
                        font[3],
                        u32::from(ch),
                        0,
                    ],
                )
            }
            Node::Glue { spec, kind, leader } => {
                let glue = encode_glue(spec);
                match leader {
                    None => Self::new(
                        NodeKind::Glue,
                        encode_glue_kind(kind),
                        0,
                        [glue[0], glue[1], glue[2], glue[3], 0, 0, 0],
                    ),
                    Some(LeaderPayload::Rule {
                        width,
                        height,
                        depth,
                    }) => {
                        let flags = 1
                            | (bool_word(width.is_some()) << 2)
                            | (bool_word(height.is_some()) << 3)
                            | (bool_word(depth.is_some()) << 4);
                        Self::new(
                            NodeKind::Glue,
                            encode_glue_kind(kind),
                            flags,
                            [
                                glue[0],
                                glue[1],
                                glue[2],
                                glue[3],
                                width.map_or(0, scaled_word),
                                height.map_or(0, scaled_word),
                                depth.map_or(0, scaled_word),
                            ],
                        )
                    }
                    Some(LeaderPayload::HList(boxed) | LeaderPayload::VList(boxed)) => {
                        let is_vertical = matches!(leader, Some(LeaderPayload::VList(_)));
                        let mut payload = Vec::with_capacity(32);
                        append_words(&mut payload, glue);
                        payload.extend(encode_box_payload(boxed));
                        let key = annex.append_fixed::<LeaderBoxPayload>(&payload);
                        Self::with_key(
                            NodeKind::Glue,
                            encode_glue_kind(kind),
                            if is_vertical { 3 } else { 2 },
                            key,
                        )
                    }
                }
            }
            Node::Penalty(value) => {
                Self::new(NodeKind::Penalty, 0, 0, [value as u32, 0, 0, 0, 0, 0, 0])
            }
            Node::Rule {
                width,
                height,
                depth,
            } => Self::new(
                NodeKind::Rule,
                0,
                bool_word(width.is_some())
                    | (bool_word(height.is_some()) << 1)
                    | (bool_word(depth.is_some()) << 2),
                [
                    width.map_or(0, scaled_word),
                    height.map_or(0, scaled_word),
                    depth.map_or(0, scaled_word),
                    0,
                    0,
                    0,
                    0,
                ],
            ),
            Node::HList(value) | Node::VList(value) => {
                let vertical = matches!(node, Node::VList(_));
                let key = annex.append_fixed::<BoxPayload>(&encode_box_payload(value));
                Self::with_key(
                    if vertical {
                        NodeKind::VList
                    } else {
                        NodeKind::HList
                    },
                    0,
                    0,
                    key,
                )
            }
            Node::Unset(value) => {
                let mut payload = Vec::with_capacity(15);
                encode_page_list(&mut payload, value.children);
                payload.extend([
                    scaled_word(value.width),
                    scaled_word(value.height),
                    scaled_word(value.depth),
                    scaled_word(value.stretch),
                    scaled_word(value.shrink),
                ]);
                let flags = u32::from(value.span_count)
                    | ((value.stretch_order as u32) << 16)
                    | ((value.shrink_order as u32) << 18)
                    | (u32::from(matches!(value.kind, UnsetKind::VBox)) << 20);
                Self::with_key(
                    NodeKind::Unset,
                    0,
                    flags,
                    annex.append_fixed::<UnsetPayload>(&payload),
                )
            }
            Node::Disc {
                kind,
                pre,
                post,
                replace,
                physical_replace_count,
            } => {
                let mut payload = Vec::with_capacity(30);
                encode_page_list(&mut payload, pre);
                encode_page_list(&mut payload, post);
                encode_page_list(&mut payload, replace);
                Self::with_key(
                    NodeKind::Disc,
                    encode_disc_kind(kind),
                    u32::from(physical_replace_count),
                    annex.append_fixed::<DiscPayload>(&payload),
                )
            }
            Node::Mark { class, tokens } => {
                let token = tokens.coordinates();
                Self::new(
                    NodeKind::Mark,
                    0,
                    u32::from(class),
                    [
                        token[0], token[1], token[2], token[3], token[4], token[5], 0,
                    ],
                )
            }
            Node::Ins {
                class,
                size,
                split_top_skip,
                split_max_depth,
                floating_penalty,
                content,
            } => {
                let mut payload = Vec::with_capacity(17);
                encode_page_list(&mut payload, content);
                append_words(&mut payload, encode_glue(split_top_skip));
                payload.extend([
                    scaled_word(size),
                    scaled_word(split_max_depth),
                    floating_penalty as u32,
                ]);
                Self::with_key(
                    NodeKind::Ins,
                    0,
                    u32::from(class),
                    annex.append_fixed::<InsertionPayload>(&payload),
                )
            }
            Node::Whatsit(value) => encode_whatsit(value, annex),
            Node::MathOn(value) => Self::new(
                NodeKind::MathOn,
                0,
                0,
                [scaled_word(value), 0, 0, 0, 0, 0, 0],
            ),
            Node::MathOff(value) => Self::new(
                NodeKind::MathOff,
                0,
                0,
                [scaled_word(value), 0, 0, 0, 0, 0, 0],
            ),
            Node::Direction(value) => Self::new(NodeKind::Direction, value as u8, 0, [0; 7]),
            Node::MathNoad(value) => {
                let mut payload = Vec::with_capacity(36);
                encode_noad_kind(&mut payload, value.kind);
                encode_math_field(&mut payload, value.nucleus);
                encode_math_field(&mut payload, value.subscript);
                encode_math_field(&mut payload, value.superscript);
                debug_assert_eq!(payload.len(), 36);
                Self::with_key(
                    NodeKind::MathNoad,
                    0,
                    0,
                    annex.append_fixed::<MathNoadPayload>(&payload),
                )
            }
            Node::FractionNoad(value) => {
                let mut payload = Vec::with_capacity(23);
                encode_page_list(&mut payload, value.numerator);
                encode_page_list(&mut payload, value.denominator);
                payload.push(match value.thickness {
                    FractionThickness::Default => u32::MAX,
                    FractionThickness::Explicit(value) => scaled_word(value),
                });
                payload.push(value.left_delimiter.unwrap_or_default());
                payload.push(value.right_delimiter.unwrap_or_default());
                let flags = bool_word(value.left_delimiter.is_some())
                    | (bool_word(value.right_delimiter.is_some()) << 1);
                Self::with_key(
                    NodeKind::FractionNoad,
                    0,
                    flags,
                    annex.append_fixed::<FractionPayload>(&payload),
                )
            }
            Node::MathStyle(style) => {
                Self::new(NodeKind::MathStyle, encode_math_style(style), 0, [0; 7])
            }
            Node::MathChoice(value) => {
                let mut payload = Vec::with_capacity(40);
                encode_page_list(&mut payload, value.display);
                encode_page_list(&mut payload, value.text);
                encode_page_list(&mut payload, value.script);
                encode_page_list(&mut payload, value.script_script);
                Self::with_key(
                    NodeKind::MathChoice,
                    0,
                    0,
                    annex.append_fixed::<MathChoicePayload>(&payload),
                )
            }
            Node::MathList(value) => {
                let mut payload = Vec::with_capacity(10);
                encode_page_list(&mut payload, value.content);
                Self::with_key(
                    NodeKind::MathList,
                    0,
                    bool_word(value.display),
                    annex.append_fixed::<ListPayload>(&payload),
                )
            }
            Node::Nonscript => Self::new(NodeKind::Nonscript, 0, 0, [0; 7]),
            Node::Adjust(value) => {
                let mut payload = Vec::with_capacity(10);
                encode_page_list(&mut payload, value.content);
                Self::with_key(
                    NodeKind::Adjust,
                    0,
                    bool_word(value.pre),
                    annex.append_fixed::<ListPayload>(&payload),
                )
            }
        }
    }

    pub(crate) fn decode_owned(self, annex: &NodeAnnexArena) -> Option<Node> {
        let kind = self.kind()?;
        let subtype = self.subtype();
        let flags = self.flags();
        let words = self.words();
        match kind {
            NodeKind::Char if subtype == 0 && flags == 0 && words[6] == 0 => Some(Node::Char {
                font: FontId::from_words(words[..4].try_into().ok()?)?,
                ch: char::from_u32(words[4])?,
                origin: OriginId::from_raw(words[5]),
            }),
            NodeKind::Lig if subtype == 0 && flags & !3 == 0 && words[6] == 0 => {
                let payload =
                    annex.resolve_fixed_shared(key_from_record::<LigaturePayload>(self))?;
                if payload.len() != 11 {
                    return None;
                }
                let mut cursor = 0;
                let font = decode_font(payload, &mut cursor)?;
                let ch = char::from_u32(*payload.get(cursor)?)?;
                cursor += 1;
                let source =
                    AnnexKey::<LigatureSource>::from_words(take_words(payload, &mut cursor)?);
                let source = annex.detach_span(source)?;
                let count = *source.first()? as usize;
                let origins_empty = decode_bool(*source.get(1)?)?;
                if source.len() != 2 + count * 2 {
                    return None;
                }
                let mut orig = Vec::with_capacity(count);
                let mut origins = (!origins_empty).then(|| Vec::with_capacity(count));
                for pair in source[2..].chunks_exact(2) {
                    orig.push(char::from_u32(pair[0])?);
                    if let Some(origins) = &mut origins {
                        origins.push(OriginId::from_raw(pair[1]));
                    } else if pair[1] != 0 {
                        return None;
                    }
                }
                Some(Node::Lig {
                    font,
                    ch,
                    orig,
                    left_hit: flags & 1 != 0,
                    right_hit: flags & 2 != 0,
                    origins: origins.unwrap_or_default(),
                })
            }
            NodeKind::Kern if flags == 0 && words[1..].iter().all(|word| *word == 0) => {
                Some(Node::Kern {
                    amount: decode_scaled(words[0]),
                    kind: decode_kern_kind(subtype)?,
                })
            }
            NodeKind::MarginKern if flags == 0 && words[6] == 0 && words[5] <= u8::MAX as u32 => {
                Some(Node::MarginKern {
                    amount: decode_scaled(words[0]),
                    side: decode_margin_side(subtype)?,
                    font: FontId::from_words(words[1..5].try_into().ok()?)?,
                    ch: words[5] as u8,
                })
            }
            NodeKind::Glue => {
                let kind = decode_glue_kind(subtype)?;
                match flags & 3 {
                    0 if flags == 0 && words[4..].iter().all(|word| *word == 0) => {
                        Some(Node::Glue {
                            spec: decode_glue(words[..4].try_into().ok()?)?,
                            kind,
                            leader: None,
                        })
                    }
                    1 if flags & !0x1d == 0 => Some(Node::Glue {
                        spec: decode_glue(words[..4].try_into().ok()?)?,
                        kind,
                        leader: Some(LeaderPayload::Rule {
                            width: (flags & 4 != 0).then(|| decode_scaled(words[4])),
                            height: (flags & 8 != 0).then(|| decode_scaled(words[5])),
                            depth: (flags & 16 != 0).then(|| decode_scaled(words[6])),
                        }),
                    }),
                    leader @ (2 | 3) if flags == leader && words[6] == 0 => {
                        let payload = annex
                            .resolve_fixed_shared(key_from_record::<LeaderBoxPayload>(self))?;
                        if payload.len() != 32 {
                            return None;
                        }
                        let spec = decode_glue(payload[..4].try_into().ok()?)?;
                        let boxed = decode_box_payload(&payload[4..])?;
                        Some(Node::Glue {
                            spec,
                            kind,
                            leader: Some(if leader == 2 {
                                LeaderPayload::HList(boxed)
                            } else {
                                LeaderPayload::VList(boxed)
                            }),
                        })
                    }
                    _ => None,
                }
            }
            NodeKind::Penalty
                if subtype == 0 && flags == 0 && words[1..].iter().all(|word| *word == 0) =>
            {
                Some(Node::Penalty(words[0] as i32))
            }
            NodeKind::Rule
                if subtype == 0 && flags & !7 == 0 && words[3..].iter().all(|word| *word == 0) =>
            {
                Some(Node::Rule {
                    width: (flags & 1 != 0).then(|| decode_scaled(words[0])),
                    height: (flags & 2 != 0).then(|| decode_scaled(words[1])),
                    depth: (flags & 4 != 0).then(|| decode_scaled(words[2])),
                })
            }
            NodeKind::HList | NodeKind::VList if subtype == 0 && flags == 0 && words[6] == 0 => {
                let payload = annex.resolve_fixed_shared(key_from_record::<BoxPayload>(self))?;
                let boxed = decode_box_payload(payload)?;
                Some(if kind == NodeKind::HList {
                    Node::HList(boxed)
                } else {
                    Node::VList(boxed)
                })
            }
            NodeKind::Unset if subtype == 0 && words[6] == 0 => {
                let payload = annex.resolve_fixed_shared(key_from_record::<UnsetPayload>(self))?;
                if payload.len() != 15 || flags & !0x1f_ffff != 0 {
                    return None;
                }
                let mut cursor = 0;
                let children = decode_page_list(payload, &mut cursor)?;
                let values: [u32; 5] = take_words(payload, &mut cursor)?;
                Some(Node::Unset(UnsetNode::new(UnsetNodeFields {
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
                    stretch_order: decode_order((flags >> 16) & 3)?,
                    shrink: decode_scaled(values[4]),
                    shrink_order: decode_order((flags >> 18) & 3)?,
                    children,
                })))
            }
            NodeKind::Disc if flags <= u8::MAX as u32 && words[6] == 0 => {
                let payload = annex.resolve_fixed_shared(key_from_record::<DiscPayload>(self))?;
                if payload.len() != 30 {
                    return None;
                }
                let mut cursor = 0;
                Some(Node::Disc {
                    kind: decode_disc_kind(subtype)?,
                    pre: decode_page_list(payload, &mut cursor)?,
                    post: decode_page_list(payload, &mut cursor)?,
                    replace: decode_page_list(payload, &mut cursor)?,
                    physical_replace_count: flags as u8,
                })
            }
            NodeKind::Mark if subtype == 0 && flags <= u16::MAX as u32 && words[6] == 0 => {
                Some(Node::Mark {
                    class: flags as u16,
                    tokens: NodeTokenKey::from_coordinates(words[..6].try_into().ok()?),
                })
            }
            NodeKind::Ins if subtype == 0 && flags <= u16::MAX as u32 && words[6] == 0 => {
                let payload =
                    annex.resolve_fixed_shared(key_from_record::<InsertionPayload>(self))?;
                if payload.len() != 17 {
                    return None;
                }
                let mut cursor = 0;
                let content = decode_page_list(payload, &mut cursor)?;
                let split_top_skip = decode_glue(take_words(payload, &mut cursor)?)?;
                let scalar: [u32; 3] = take_words(payload, &mut cursor)?;
                Some(Node::Ins {
                    class: flags as u16,
                    size: decode_scaled(scalar[0]),
                    split_top_skip,
                    split_max_depth: decode_scaled(scalar[1]),
                    floating_penalty: scalar[2] as i32,
                    content,
                })
            }
            NodeKind::Whatsit => decode_whatsit(self, annex),
            NodeKind::MathOn | NodeKind::MathOff
                if subtype == 0 && flags == 0 && words[1..].iter().all(|word| *word == 0) =>
            {
                Some(if kind == NodeKind::MathOn {
                    Node::MathOn(decode_scaled(words[0]))
                } else {
                    Node::MathOff(decode_scaled(words[0]))
                })
            }
            NodeKind::Direction if flags == 0 && words.iter().all(|word| *word == 0) => {
                Some(Node::Direction(match subtype {
                    0 => crate::node::Direction::BeginL,
                    1 => crate::node::Direction::EndL,
                    2 => crate::node::Direction::BeginR,
                    3 => crate::node::Direction::EndR,
                    4 => crate::node::Direction::BeginM,
                    5 => crate::node::Direction::EndM,
                    _ => return None,
                }))
            }
            NodeKind::MathNoad if subtype == 0 && flags == 0 && words[6] == 0 => {
                let payload =
                    annex.resolve_fixed_shared(key_from_record::<MathNoadPayload>(self))?;
                if payload.len() != 36 {
                    return None;
                }
                let mut cursor = 0;
                Some(Node::MathNoad(MathNoad {
                    kind: decode_noad_kind(payload, &mut cursor)?,
                    nucleus: decode_math_field(payload, &mut cursor)?,
                    subscript: decode_math_field(payload, &mut cursor)?,
                    superscript: decode_math_field(payload, &mut cursor)?,
                }))
            }
            NodeKind::FractionNoad if subtype == 0 && flags & !3 == 0 && words[6] == 0 => {
                let payload =
                    annex.resolve_fixed_shared(key_from_record::<FractionPayload>(self))?;
                if payload.len() != 23 {
                    return None;
                }
                let mut cursor = 0;
                let numerator = decode_page_list(payload, &mut cursor)?;
                let denominator = decode_page_list(payload, &mut cursor)?;
                let thickness = *payload.get(cursor)?;
                cursor += 1;
                let delimiters: [u32; 2] = take_words(payload, &mut cursor)?;
                Some(Node::FractionNoad(MathFraction {
                    numerator,
                    denominator,
                    thickness: if thickness == u32::MAX {
                        FractionThickness::Default
                    } else {
                        FractionThickness::Explicit(decode_scaled(thickness))
                    },
                    left_delimiter: (flags & 1 != 0).then_some(delimiters[0]),
                    right_delimiter: (flags & 2 != 0).then_some(delimiters[1]),
                }))
            }
            NodeKind::MathStyle if flags == 0 && words.iter().all(|word| *word == 0) => {
                Some(Node::MathStyle(decode_math_style(subtype)?))
            }
            NodeKind::MathChoice if subtype == 0 && flags == 0 && words[6] == 0 => {
                let payload =
                    annex.resolve_fixed_shared(key_from_record::<MathChoicePayload>(self))?;
                if payload.len() != 40 {
                    return None;
                }
                let mut cursor = 0;
                Some(Node::MathChoice(MathChoice {
                    display: decode_page_list(payload, &mut cursor)?,
                    text: decode_page_list(payload, &mut cursor)?,
                    script: decode_page_list(payload, &mut cursor)?,
                    script_script: decode_page_list(payload, &mut cursor)?,
                }))
            }
            NodeKind::MathList if subtype == 0 && flags <= 1 && words[6] == 0 => {
                let payload = annex.resolve_fixed_shared(key_from_record::<ListPayload>(self))?;
                if payload.len() != 10 {
                    return None;
                }
                let mut cursor = 0;
                Some(Node::MathList(MathListNode {
                    display: flags == 1,
                    content: decode_page_list(payload, &mut cursor)?,
                }))
            }
            NodeKind::Nonscript
                if subtype == 0 && flags == 0 && words.iter().all(|word| *word == 0) =>
            {
                Some(Node::Nonscript)
            }
            NodeKind::Adjust if subtype == 0 && flags <= 1 && words[6] == 0 => {
                let payload = annex.resolve_fixed_shared(key_from_record::<ListPayload>(self))?;
                if payload.len() != 10 {
                    return None;
                }
                let mut cursor = 0;
                Some(Node::Adjust(AdjustNode {
                    content: decode_page_list(payload, &mut cursor)?,
                    pre: flags == 1,
                }))
            }
            _ => None,
        }
    }
}

impl<Lane> core::fmt::Debug for NodeRecord<Lane> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("NodeRecord")
            .field("kind", &self.kind())
            .field("subtype", &self.subtype())
            .finish_non_exhaustive()
    }
}

impl<Lane> PartialEq for NodeRecord<Lane> {
    fn eq(&self, other: &Self) -> bool {
        self.header == other.header && self.words == other.words
    }
}

impl<Lane> Eq for NodeRecord<Lane> {}

const _: () = assert!(core::mem::size_of::<NodeRecord>() == 32);
const _: () = assert!(core::mem::align_of::<NodeRecord>() == 4);
const _: () = assert!(!core::mem::needs_drop::<NodeRecord>());
const _: () = assert!(core::mem::size_of::<Option<NodeRecord>>() == 32);

#[repr(C)]
pub(crate) struct AnnexKey<Kind> {
    owner: u32,
    block_ordinal: u32,
    logical_block_incarnation: u32,
    word_offset: u32,
    word_len: u32,
    publication_serial: u32,
    kind: PhantomData<fn(&Kind) -> &Kind>,
}

impl<Kind> Clone for AnnexKey<Kind> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<Kind> Copy for AnnexKey<Kind> {}

impl<Kind> core::fmt::Debug for AnnexKey<Kind> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("AnnexKey")
            .field("word_len", &self.word_len)
            .finish_non_exhaustive()
    }
}

impl<Kind> AnnexKey<Kind> {
    pub(crate) const fn words(self) -> [u32; 6] {
        [
            self.owner,
            self.block_ordinal,
            self.logical_block_incarnation,
            self.word_offset,
            self.word_len,
            self.publication_serial,
        ]
    }

    pub(crate) const fn from_words(words: [u32; 6]) -> Self {
        Self {
            owner: words[0],
            block_ordinal: words[1],
            logical_block_incarnation: words[2],
            word_offset: words[3],
            word_len: words[4],
            publication_serial: words[5],
            kind: PhantomData,
        }
    }
}

const _: () = assert!(core::mem::size_of::<AnnexKey<()>>() == 24);
const _: () = assert!(core::mem::align_of::<AnnexKey<()>>() == 4);
const _: () = assert!(!core::mem::needs_drop::<AnnexKey<()>>());

struct AnnexBlock {
    words: Box<[u32]>,
    initialized: usize,
    logical_incarnation: u32,
}

impl AnnexBlock {
    fn new(logical_incarnation: u32) -> Self {
        Self {
            words: vec![0; ANNEX_WORDS_PER_BLOCK].into_boxed_slice(),
            initialized: 0,
            logical_incarnation,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct NodeAnnexMetrics {
    pub(crate) superblocks_allocated: u64,
    pub(crate) superblocks_reclaimed: u64,
    pub(crate) words_published: u64,
    pub(crate) words_rolled_back: u64,
    pub(crate) boundary_padding_words: u64,
    pub(crate) direct_lookups: u64,
    pub(crate) stale_rejections: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NodeAnnexMark {
    owner: u32,
    words: usize,
}

pub(crate) struct NodeAnnexArena {
    owner: u32,
    blocks: Vec<AnnexBlock>,
    words: usize,
    next_publication_serial: u32,
    next_logical_incarnation: u32,
    metrics: NodeAnnexMetrics,
}

enum LigaturePayload {}
enum LigatureSource {}
enum BoxPayload {}
enum LeaderBoxPayload {}
enum UnsetPayload {}
enum DiscPayload {}
enum InsertionPayload {}
enum MathNoadPayload {}
enum FractionPayload {}
enum MathChoicePayload {}
enum ListPayload {}
enum Utf8Span {}
enum ByteSpan {}
enum SpecialPayload {}
enum DeferredSpecialPayload {}
enum PdfDestinationPayload {}
enum PdfThreadPayload {}

fn key_words<Kind>(key: AnnexKey<Kind>) -> [u32; 6] {
    key.words()
}

fn key_from_record<Kind>(record: NodeRecord) -> AnnexKey<Kind> {
    let words = record.words();
    AnnexKey::from_words([words[0], words[1], words[2], words[3], words[4], words[5]])
}

fn encode_page_list(destination: &mut Vec<u32>, list: PageListId) {
    append_words(destination, list.words());
}

fn decode_page_list(source: &[u32], cursor: &mut usize) -> Option<PageListId> {
    PageListId::from_words(take_words(source, cursor)?)
}

fn encode_font(destination: &mut Vec<u32>, font: FontId) {
    append_words(destination, font.words());
}

fn decode_font(source: &[u32], cursor: &mut usize) -> Option<FontId> {
    FontId::from_words(take_words(source, cursor)?)
}

fn encode_box_payload(value: BoxNode<PageListId>) -> Vec<u32> {
    let mut words = Vec::with_capacity(28);
    append_words(
        &mut words,
        [
            scaled_word(value.width),
            scaled_word(value.height),
            scaled_word(value.depth),
            scaled_word(value.shift),
            value.glue_set.numerator() as u32,
            value.glue_set.denominator() as u32,
            value.box_lr as u32
                | ((value.glue_sign as u32) << 8)
                | ((value.glue_order as u32) << 16)
                | (bool_word(value.diagnostic_children.is_some()) << 24),
        ],
    );
    encode_page_list(&mut words, value.children);
    encode_page_list(
        &mut words,
        value.diagnostic_children.unwrap_or_else(PageListId::empty),
    );
    words.push(value.allocator_high_cell_overlap);
    debug_assert_eq!(words.len(), 28);
    words
}

fn decode_box_payload(words: &[u32]) -> Option<BoxNode<PageListId>> {
    if words.len() != 28 {
        return None;
    }
    let mut cursor = 0;
    let scalar: [u32; 7] = take_words(words, &mut cursor)?;
    if scalar[5] == 0 || scalar[6] & 0xfe00_00f8 != 0 {
        return None;
    }
    let box_lr = match scalar[6] & 0xff {
        0 => BoxLr::Normal,
        1 => BoxLr::Reversed,
        2 => BoxLr::DList,
        _ => return None,
    };
    let glue_sign = match (scalar[6] >> 8) & 0xff {
        0 => Sign::Normal,
        1 => Sign::Stretching,
        2 => Sign::Shrinking,
        _ => return None,
    };
    let glue_order = decode_order((scalar[6] >> 16) & 0xff)?;
    let diagnostic_present = decode_bool((scalar[6] >> 24) & 1)?;
    let children = decode_page_list(words, &mut cursor)?;
    let diagnostic = decode_page_list(words, &mut cursor)?;
    let overlap = *words.get(cursor)?;
    if diagnostic_present == diagnostic.is_empty() {
        return None;
    }
    let mut value = BoxNode::new(BoxNodeFields {
        width: decode_scaled(scalar[0]),
        height: decode_scaled(scalar[1]),
        depth: decode_scaled(scalar[2]),
        shift: decode_scaled(scalar[3]),
        box_lr,
        glue_set: GlueSetRatio::try_from_ratio_parts(scalar[4] as i32, scalar[5] as i32).ok()?,
        glue_sign,
        glue_order,
        children,
    });
    value.diagnostic_children = diagnostic_present.then_some(diagnostic);
    value.allocator_high_cell_overlap = overlap;
    Some(value)
}

fn encode_math_field(destination: &mut Vec<u32>, field: MathField<PageListId>) {
    match field {
        MathField::Empty => destination.extend([0; 11]),
        MathField::MathChar(value) | MathField::MathTextChar(value) => {
            let tag = if matches!(field, MathField::MathChar(_)) {
                1
            } else {
                2
            };
            destination.extend([
                tag,
                u32::from(value.family),
                value.character as u32,
                value.origin.raw(),
                0,
                0,
                0,
                0,
                0,
                0,
                0,
            ]);
        }
        MathField::SubBox(list) | MathField::SubMlist(list) => {
            let tag = if matches!(field, MathField::SubBox(_)) {
                3
            } else {
                4
            };
            destination.push(tag);
            encode_page_list(destination, list);
        }
    }
}

fn decode_math_field(words: &[u32], cursor: &mut usize) -> Option<MathField<PageListId>> {
    let field: [u32; 11] = take_words(words, cursor)?;
    match field[0] {
        0 if field[1..].iter().all(|word| *word == 0) => Some(MathField::Empty),
        tag @ (1 | 2)
            if field[4..].iter().all(|word| *word == 0)
                && field[1] < crate::math::MATH_FAMILY_COUNT as u32 =>
        {
            let value = MathChar {
                family: field[1] as u8,
                character: char::from_u32(field[2])?,
                origin: OriginId::from_raw(field[3]),
            };
            Some(if tag == 1 {
                MathField::MathChar(value)
            } else {
                MathField::MathTextChar(value)
            })
        }
        tag @ (3 | 4) => {
            let list = PageListId::from_words(field[1..].try_into().ok()?)?;
            Some(if tag == 3 {
                MathField::SubBox(list)
            } else {
                MathField::SubMlist(list)
            })
        }
        _ => None,
    }
}

fn encode_noad_kind(destination: &mut Vec<u32>, kind: NoadKind) {
    let words = match kind {
        NoadKind::Normal(class) => [class as u32, 0, 0],
        NoadKind::Operator(limit) => [1 << 8 | limit as u32, 0, 0],
        NoadKind::Radical { delimiter } => [2 << 8, delimiter, 0],
        NoadKind::Accent { accent } => [
            3 << 8 | u32::from(accent.family),
            accent.character as u32,
            accent.origin.raw(),
        ],
        NoadKind::LeftDelimiter { delimiter } => [4 << 8, delimiter, 0],
        NoadKind::RightDelimiter { delimiter } => [5 << 8, delimiter, 0],
        NoadKind::MiddleDelimiter { delimiter } => [6 << 8, delimiter, 0],
        NoadKind::Underline => [7 << 8, 0, 0],
        NoadKind::Overline => [8 << 8, 0, 0],
        NoadKind::VCenter => [9 << 8, 0, 0],
    };
    destination.extend(words);
}

fn decode_noad_kind(words: &[u32], cursor: &mut usize) -> Option<NoadKind> {
    let [tagged, value, origin]: [u32; 3] = take_words(words, cursor)?;
    let tag = tagged >> 8;
    let low = tagged & 0xff;
    match (tag, low, value, origin) {
        (0, class, 0, 0) => Some(NoadKind::Normal(match class {
            0 => NoadClass::Ord,
            1 => NoadClass::Op,
            2 => NoadClass::Bin,
            3 => NoadClass::Rel,
            4 => NoadClass::Open,
            5 => NoadClass::Close,
            6 => NoadClass::Punct,
            7 => NoadClass::Inner,
            _ => return None,
        })),
        (1, limit, 0, 0) => Some(NoadKind::Operator(match limit {
            0 => LimitType::DisplayLimits,
            1 => LimitType::Limits,
            2 => LimitType::NoLimits,
            _ => return None,
        })),
        (2, 0, delimiter, 0) => Some(NoadKind::Radical { delimiter }),
        (3, family, character, origin) if family < crate::math::MATH_FAMILY_COUNT as u32 => {
            Some(NoadKind::Accent {
                accent: MathChar {
                    family: family as u8,
                    character: char::from_u32(character)?,
                    origin: OriginId::from_raw(origin),
                },
            })
        }
        (4, 0, delimiter, 0) => Some(NoadKind::LeftDelimiter { delimiter }),
        (5, 0, delimiter, 0) => Some(NoadKind::RightDelimiter { delimiter }),
        (6, 0, delimiter, 0) => Some(NoadKind::MiddleDelimiter { delimiter }),
        (7, 0, 0, 0) => Some(NoadKind::Underline),
        (8, 0, 0, 0) => Some(NoadKind::Overline),
        (9, 0, 0, 0) => Some(NoadKind::VCenter),
        _ => None,
    }
}

fn append_bytes<Kind>(annex: &mut NodeAnnexArena, bytes: &[u8]) -> AnnexKey<Kind> {
    let mut body = Vec::with_capacity(1 + bytes.len().div_ceil(4));
    body.push(u32::try_from(bytes.len()).expect("node annex byte length fits u32"));
    for chunk in bytes.chunks(4) {
        let mut packed = [0; 4];
        packed[..chunk.len()].copy_from_slice(chunk);
        body.push(u32::from_le_bytes(packed));
    }
    annex.append_span(&body)
}

fn detach_bytes<Kind>(annex: &NodeAnnexArena, key: AnnexKey<Kind>) -> Option<Vec<u8>> {
    let body = annex.detach_span(key)?;
    let len = *body.first()? as usize;
    if body.len() != 1 + len.div_ceil(4) {
        return None;
    }
    let mut bytes = Vec::with_capacity(len);
    for word in &body[1..] {
        bytes.extend(word.to_le_bytes());
    }
    bytes.truncate(len);
    Some(bytes)
}

fn encode_identifier(identifier: NodePdfActionIdentifier<NodeTokenKey>) -> (u32, [u32; 6]) {
    match identifier {
        NodePdfActionIdentifier::Name(tokens) => (0, tokens.coordinates()),
        NodePdfActionIdentifier::Number(number) => (1, [number, 0, 0, 0, 0, 0]),
        NodePdfActionIdentifier::Raw(tokens) => (2, tokens.coordinates()),
    }
}

fn decode_identifier(tag: u32, words: [u32; 6]) -> Option<NodePdfActionIdentifier<NodeTokenKey>> {
    match tag {
        0 => Some(NodePdfActionIdentifier::Name(
            NodeTokenKey::from_coordinates(words),
        )),
        1 if words[1..].iter().all(|word| *word == 0) => {
            Some(NodePdfActionIdentifier::Number(words[0]))
        }
        2 => Some(NodePdfActionIdentifier::Raw(
            NodeTokenKey::from_coordinates(words),
        )),
        _ => None,
    }
}

fn encode_pdf_dimensions(value: crate::PdfAnnotationDimensions) -> ([u32; 3], u32) {
    (
        [
            value.width.map_or(0, scaled_word),
            value.height.map_or(0, scaled_word),
            value.depth.map_or(0, scaled_word),
        ],
        bool_word(value.width.is_some())
            | (bool_word(value.height.is_some()) << 1)
            | (bool_word(value.depth.is_some()) << 2),
    )
}

fn decode_pdf_dimensions(words: [u32; 3], presence: u32) -> Option<crate::PdfAnnotationDimensions> {
    if presence & !7 != 0 {
        return None;
    }
    Some(crate::PdfAnnotationDimensions {
        width: (presence & 1 != 0).then(|| decode_scaled(words[0])),
        height: (presence & 2 != 0).then(|| decode_scaled(words[1])),
        depth: (presence & 4 != 0).then(|| decode_scaled(words[2])),
    })
}

fn encode_destination_kind(value: PdfDestinationKind) -> (u32, [u32; 3], u32) {
    match value {
        PdfDestinationKind::Xyz { zoom } => (
            0,
            [zoom.unwrap_or_default() as u32, 0, 0],
            bool_word(zoom.is_some()),
        ),
        PdfDestinationKind::FitBoundingBoxHorizontal => (1, [0; 3], 0),
        PdfDestinationKind::FitBoundingBoxVertical => (2, [0; 3], 0),
        PdfDestinationKind::FitBoundingBox => (3, [0; 3], 0),
        PdfDestinationKind::FitHorizontal => (4, [0; 3], 0),
        PdfDestinationKind::FitVertical => (5, [0; 3], 0),
        PdfDestinationKind::FitRectangle(dimensions) => {
            let (words, presence) = encode_pdf_dimensions(dimensions);
            (6, words, presence)
        }
        PdfDestinationKind::Fit => (7, [0; 3], 0),
    }
}

fn decode_destination_kind(tag: u32, words: [u32; 3], presence: u32) -> Option<PdfDestinationKind> {
    match tag {
        0 if presence <= 1 && words[1] == 0 && words[2] == 0 => Some(PdfDestinationKind::Xyz {
            zoom: (presence == 1).then_some(words[0] as i32),
        }),
        1 if words == [0; 3] && presence == 0 => Some(PdfDestinationKind::FitBoundingBoxHorizontal),
        2 if words == [0; 3] && presence == 0 => Some(PdfDestinationKind::FitBoundingBoxVertical),
        3 if words == [0; 3] && presence == 0 => Some(PdfDestinationKind::FitBoundingBox),
        4 if words == [0; 3] && presence == 0 => Some(PdfDestinationKind::FitHorizontal),
        5 if words == [0; 3] && presence == 0 => Some(PdfDestinationKind::FitVertical),
        6 => Some(PdfDestinationKind::FitRectangle(decode_pdf_dimensions(
            words, presence,
        )?)),
        7 if words == [0; 3] && presence == 0 => Some(PdfDestinationKind::Fit),
        _ => None,
    }
}

fn encode_whatsit(value: Whatsit, annex: &mut NodeAnnexArena) -> NodeRecord {
    match value {
        Whatsit::OpenOut { slot, path } => {
            let key = append_bytes::<Utf8Span>(annex, path.as_bytes()).words();
            NodeRecord::new(
                NodeKind::Whatsit,
                0,
                0,
                [
                    key[0],
                    key[1],
                    key[2],
                    key[3],
                    key[4],
                    key[5],
                    u32::from(slot.raw()),
                ],
            )
        }
        Whatsit::CloseOut { slot } => NodeRecord::new(
            NodeKind::Whatsit,
            1,
            bool_word(slot.is_some()),
            [
                slot.map_or(0, |slot| u32::from(slot.raw())),
                0,
                0,
                0,
                0,
                0,
                0,
            ],
        ),
        Whatsit::DeferredWrite { sink, tokens } => {
            let key = tokens.coordinates();
            NodeRecord::new(
                NodeKind::Whatsit,
                2,
                0,
                [
                    key[0],
                    key[1],
                    key[2],
                    key[3],
                    key[4],
                    key[5],
                    encode_print_sink(sink),
                ],
            )
        }
        Whatsit::Special { class, payload } => {
            let class = append_bytes::<Utf8Span>(annex, class.as_bytes());
            let payload = append_bytes::<ByteSpan>(annex, &payload);
            let mut body = Vec::with_capacity(12);
            append_words(&mut body, class.words());
            append_words(&mut body, payload.words());
            NodeRecord::with_key(
                NodeKind::Whatsit,
                3,
                0,
                annex.append_fixed::<SpecialPayload>(&body),
            )
        }
        Whatsit::DeferredSpecial { class, tokens } => {
            let class = append_bytes::<Utf8Span>(annex, class.as_bytes());
            let mut body = Vec::with_capacity(12);
            append_words(&mut body, class.words());
            append_words(&mut body, tokens.coordinates());
            NodeRecord::with_key(
                NodeKind::Whatsit,
                4,
                0,
                annex.append_fixed::<DeferredSpecialPayload>(&body),
            )
        }
        Whatsit::PdfReferenceObject { object } => {
            NodeRecord::new(NodeKind::Whatsit, 5, 0, [object, 0, 0, 0, 0, 0, 0])
        }
        Whatsit::PdfAccessibility(value) => NodeRecord::new(
            NodeKind::Whatsit,
            6,
            match value {
                PdfAccessibilityControl::InterwordSpaceOn => 0,
                PdfAccessibilityControl::InterwordSpaceOff => 1,
                PdfAccessibilityControl::FakeSpace => 2,
            },
            [0; 7],
        ),
        Whatsit::PdfAnnotation { object } => {
            NodeRecord::new(NodeKind::Whatsit, 7, 0, [object, 0, 0, 0, 0, 0, 0])
        }
        Whatsit::PdfLinkStart { object } => {
            NodeRecord::new(NodeKind::Whatsit, 8, 0, [object, 0, 0, 0, 0, 0, 0])
        }
        Whatsit::PdfLinkEnd { object } => {
            NodeRecord::new(NodeKind::Whatsit, 9, 0, [object, 0, 0, 0, 0, 0, 0])
        }
        Whatsit::PdfRunningLink(running) => {
            NodeRecord::new(NodeKind::Whatsit, 10, bool_word(running), [0; 7])
        }
        Whatsit::PdfLiteral { mode, payload } => {
            let key = append_bytes::<ByteSpan>(annex, &payload);
            NodeRecord::with_key(NodeKind::Whatsit, 11, encode_literal_mode(mode), key)
        }
        Whatsit::DeferredPdfLiteral { mode, tokens } => {
            let key = tokens.coordinates();
            NodeRecord::new(
                NodeKind::Whatsit,
                12,
                encode_literal_mode(mode),
                [key[0], key[1], key[2], key[3], key[4], key[5], 0],
            )
        }
        Whatsit::PdfSetMatrix { payload } => NodeRecord::with_key(
            NodeKind::Whatsit,
            13,
            0,
            append_bytes::<ByteSpan>(annex, &payload),
        ),
        Whatsit::PdfSave => NodeRecord::new(NodeKind::Whatsit, 14, 0, [0; 7]),
        Whatsit::PdfRestore => NodeRecord::new(NodeKind::Whatsit, 15, 0, [0; 7]),
        Whatsit::PdfColorStack { id, action } => {
            let (action, key) = match action {
                crate::PdfColorStackAction::Set(bytes) => {
                    (0, Some(append_bytes::<ByteSpan>(annex, &bytes)))
                }
                crate::PdfColorStackAction::Push(bytes) => {
                    (1, Some(append_bytes::<ByteSpan>(annex, &bytes)))
                }
                crate::PdfColorStackAction::Pop => (2, None),
                crate::PdfColorStackAction::Current => (3, None),
            };
            let key = key.map_or([0; 6], AnnexKey::words);
            NodeRecord::new(
                NodeKind::Whatsit,
                16,
                action,
                [key[0], key[1], key[2], key[3], key[4], key[5], id],
            )
        }
        Whatsit::PdfSavePos => NodeRecord::new(NodeKind::Whatsit, 17, 0, [0; 7]),
        Whatsit::PdfSnapRefPoint => NodeRecord::new(NodeKind::Whatsit, 18, 0, [0; 7]),
        Whatsit::PdfSnapY { glue } => {
            let glue = encode_glue(glue);
            NodeRecord::new(
                NodeKind::Whatsit,
                19,
                0,
                [glue[0], glue[1], glue[2], glue[3], 0, 0, 0],
            )
        }
        Whatsit::PdfSnapYComp { ratio } => NodeRecord::new(
            NodeKind::Whatsit,
            20,
            0,
            [u32::from(ratio), 0, 0, 0, 0, 0, 0],
        ),
        Whatsit::PdfRefXForm {
            object,
            width,
            height,
            depth,
        }
        | Whatsit::PdfRefXImage {
            object,
            width,
            height,
            depth,
        } => NodeRecord::new(
            NodeKind::Whatsit,
            if matches!(value, Whatsit::PdfRefXImage { .. }) {
                22
            } else {
                21
            },
            0,
            [
                object,
                scaled_word(width),
                scaled_word(height),
                scaled_word(depth),
                0,
                0,
                0,
            ],
        ),
        Whatsit::PdfDestination(destination) => {
            let (identifier_tag, identifier) = encode_identifier(destination.identifier);
            let (kind_tag, kind_words, kind_presence) = encode_destination_kind(destination.kind);
            let mut body = Vec::with_capacity(12);
            body.push(identifier_tag);
            append_words(&mut body, identifier);
            body.push(destination.structure.unwrap_or_default());
            body.push(kind_tag);
            append_words(&mut body, kind_words);
            NodeRecord::with_key(
                NodeKind::Whatsit,
                23,
                bool_word(destination.structure.is_some()) | (kind_presence << 1),
                annex.append_fixed::<PdfDestinationPayload>(&body),
            )
        }
        Whatsit::PdfThread(thread) => {
            let (identifier_tag, identifier) = encode_identifier(thread.identifier);
            let (dimensions, presence) = encode_pdf_dimensions(thread.dimensions);
            let mut body = Vec::with_capacity(17);
            body.push(identifier_tag);
            append_words(&mut body, identifier);
            append_words(&mut body, dimensions);
            append_words(&mut body, thread.attributes.coordinates());
            NodeRecord::with_key(
                NodeKind::Whatsit,
                24,
                presence | (bool_word(thread.running) << 3),
                annex.append_fixed::<PdfThreadPayload>(&body),
            )
        }
        Whatsit::PdfEndThread => NodeRecord::new(NodeKind::Whatsit, 25, 0, [0; 7]),
        Whatsit::Language {
            language,
            left_hyphen_min,
            right_hyphen_min,
        } => NodeRecord::new(
            NodeKind::Whatsit,
            26,
            0,
            [
                u32::from(language)
                    | (u32::from(left_hyphen_min) << 8)
                    | (u32::from(right_hyphen_min) << 16),
                0,
                0,
                0,
                0,
                0,
                0,
            ],
        ),
    }
}

fn encode_literal_mode(value: PdfLiteralMode) -> u32 {
    match value {
        PdfLiteralMode::Origin => 0,
        PdfLiteralMode::Page => 1,
        PdfLiteralMode::Direct => 2,
    }
}

fn decode_literal_mode(value: u32) -> Option<PdfLiteralMode> {
    match value {
        0 => Some(PdfLiteralMode::Origin),
        1 => Some(PdfLiteralMode::Page),
        2 => Some(PdfLiteralMode::Direct),
        _ => None,
    }
}

fn decode_whatsit(record: NodeRecord, annex: &NodeAnnexArena) -> Option<Node> {
    let subtype = record.subtype();
    let flags = record.flags();
    let words = record.words();
    let zero_tail = |start: usize| words[start..].iter().all(|word| *word == 0);
    let value = match subtype {
        0 if flags == 0 && words[6] < 16 => {
            let path = detach_bytes(
                annex,
                AnnexKey::<Utf8Span>::from_words(words[..6].try_into().ok()?),
            )?;
            Whatsit::OpenOut {
                slot: StreamSlot::new(words[6] as u8),
                path: String::from_utf8(path).ok()?,
            }
        }
        1 if flags <= 1 && zero_tail(1) && words[0] < 16 => Whatsit::CloseOut {
            slot: (flags == 1).then(|| StreamSlot::new(words[0] as u8)),
        },
        2 if flags == 0 => Whatsit::DeferredWrite {
            sink: decode_print_sink(words[6])?,
            tokens: NodeTokenKey::from_coordinates(words[..6].try_into().ok()?),
        },
        3 if flags == 0 && words[6] == 0 => {
            let payload = annex.resolve_fixed_shared(key_from_record::<SpecialPayload>(record))?;
            if payload.len() != 12 {
                return None;
            }
            Whatsit::Special {
                class: String::from_utf8(detach_bytes(
                    annex,
                    AnnexKey::<Utf8Span>::from_words(payload[..6].try_into().ok()?),
                )?)
                .ok()?,
                payload: detach_bytes(
                    annex,
                    AnnexKey::<ByteSpan>::from_words(payload[6..].try_into().ok()?),
                )?,
            }
        }
        4 if flags == 0 && words[6] == 0 => {
            let payload =
                annex.resolve_fixed_shared(key_from_record::<DeferredSpecialPayload>(record))?;
            if payload.len() != 12 {
                return None;
            }
            Whatsit::DeferredSpecial {
                class: String::from_utf8(detach_bytes(
                    annex,
                    AnnexKey::<Utf8Span>::from_words(payload[..6].try_into().ok()?),
                )?)
                .ok()?,
                tokens: NodeTokenKey::from_coordinates(payload[6..].try_into().ok()?),
            }
        }
        5 if flags == 0 && zero_tail(1) => Whatsit::PdfReferenceObject { object: words[0] },
        6 if flags <= 2 && words.iter().all(|word| *word == 0) => {
            Whatsit::PdfAccessibility(match flags {
                0 => PdfAccessibilityControl::InterwordSpaceOn,
                1 => PdfAccessibilityControl::InterwordSpaceOff,
                2 => PdfAccessibilityControl::FakeSpace,
                _ => return None,
            })
        }
        7 if flags == 0 && zero_tail(1) => Whatsit::PdfAnnotation { object: words[0] },
        8 if flags == 0 && zero_tail(1) => Whatsit::PdfLinkStart { object: words[0] },
        9 if flags == 0 && zero_tail(1) => Whatsit::PdfLinkEnd { object: words[0] },
        10 if flags <= 1 && words.iter().all(|word| *word == 0) => {
            Whatsit::PdfRunningLink(flags == 1)
        }
        11 if flags <= 2 && words[6] == 0 => Whatsit::PdfLiteral {
            mode: decode_literal_mode(flags)?,
            payload: detach_bytes(annex, key_from_record::<ByteSpan>(record))?,
        },
        12 if flags <= 2 && words[6] == 0 => Whatsit::DeferredPdfLiteral {
            mode: decode_literal_mode(flags)?,
            tokens: NodeTokenKey::from_coordinates(words[..6].try_into().ok()?),
        },
        13 if flags == 0 && words[6] == 0 => Whatsit::PdfSetMatrix {
            payload: detach_bytes(annex, key_from_record::<ByteSpan>(record))?,
        },
        14 if flags == 0 && words.iter().all(|word| *word == 0) => Whatsit::PdfSave,
        15 if flags == 0 && words.iter().all(|word| *word == 0) => Whatsit::PdfRestore,
        16 if flags <= 3 => {
            let bytes = || {
                detach_bytes(
                    annex,
                    AnnexKey::<ByteSpan>::from_words(words[..6].try_into().ok()?),
                )
            };
            let action = match flags {
                0 => crate::PdfColorStackAction::Set(bytes()?),
                1 => crate::PdfColorStackAction::Push(bytes()?),
                2 if words[..6].iter().all(|word| *word == 0) => crate::PdfColorStackAction::Pop,
                3 if words[..6].iter().all(|word| *word == 0) => {
                    crate::PdfColorStackAction::Current
                }
                _ => return None,
            };
            Whatsit::PdfColorStack {
                id: words[6],
                action,
            }
        }
        17 if flags == 0 && words.iter().all(|word| *word == 0) => Whatsit::PdfSavePos,
        18 if flags == 0 && words.iter().all(|word| *word == 0) => Whatsit::PdfSnapRefPoint,
        19 if flags == 0 && zero_tail(4) => Whatsit::PdfSnapY {
            glue: decode_glue(words[..4].try_into().ok()?)?,
        },
        20 if flags == 0 && words[0] <= u16::MAX as u32 && zero_tail(1) => Whatsit::PdfSnapYComp {
            ratio: words[0] as u16,
        },
        kind @ (21 | 22) if flags == 0 && zero_tail(4) => {
            let fields = (
                words[0],
                decode_scaled(words[1]),
                decode_scaled(words[2]),
                decode_scaled(words[3]),
            );
            if kind == 21 {
                Whatsit::PdfRefXForm {
                    object: fields.0,
                    width: fields.1,
                    height: fields.2,
                    depth: fields.3,
                }
            } else {
                Whatsit::PdfRefXImage {
                    object: fields.0,
                    width: fields.1,
                    height: fields.2,
                    depth: fields.3,
                }
            }
        }
        23 if words[6] == 0 => {
            let payload =
                annex.resolve_fixed_shared(key_from_record::<PdfDestinationPayload>(record))?;
            if payload.len() != 12 || flags >> 8 != 0 {
                return None;
            }
            let identifier = decode_identifier(payload[0], payload[1..7].try_into().ok()?)?;
            let structure = (flags & 1 != 0).then_some(payload[7]);
            let kind =
                decode_destination_kind(payload[8], payload[9..12].try_into().ok()?, flags >> 1)?;
            Whatsit::PdfDestination(Box::new(PdfDestinationNode {
                identifier,
                structure,
                kind,
            }))
        }
        24 if words[6] == 0 && flags & !0xf == 0 => {
            let payload =
                annex.resolve_fixed_shared(key_from_record::<PdfThreadPayload>(record))?;
            if payload.len() != 17 {
                return None;
            }
            Whatsit::PdfThread(Box::new(PdfThreadNode {
                identifier: decode_identifier(payload[0], payload[1..7].try_into().ok()?)?,
                dimensions: decode_pdf_dimensions(payload[7..10].try_into().ok()?, flags & 7)?,
                attributes: NodeTokenKey::from_coordinates(payload[10..16].try_into().ok()?),
                running: flags & 8 != 0,
            }))
        }
        25 if flags == 0 && words.iter().all(|word| *word == 0) => Whatsit::PdfEndThread,
        26 if flags == 0 && zero_tail(1) && words[0] >> 24 == 0 => Whatsit::Language {
            language: words[0] as u8,
            left_hyphen_min: (words[0] >> 8) as u8,
            right_hyphen_min: (words[0] >> 16) as u8,
        },
        _ => return None,
    };
    Some(Node::Whatsit(value))
}

impl Default for NodeAnnexArena {
    fn default() -> Self {
        Self::new()
    }
}

impl NodeAnnexArena {
    pub(crate) fn new() -> Self {
        Self {
            owner: NEXT_ANNEX_OWNER
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                    value.checked_add(1)
                })
                .expect("node annex owner domain exhausted"),
            blocks: Vec::new(),
            words: 0,
            next_publication_serial: 1,
            next_logical_incarnation: 1,
            metrics: NodeAnnexMetrics::default(),
        }
    }

    pub(crate) const fn mark(&self) -> NodeAnnexMark {
        NodeAnnexMark {
            owner: self.owner,
            words: self.words,
        }
    }

    pub(crate) const fn metrics(&self) -> NodeAnnexMetrics {
        self.metrics
    }

    pub(crate) const fn len(&self) -> usize {
        self.words
    }

    fn ensure_block(&mut self, ordinal: usize) {
        while self.blocks.len() <= ordinal {
            let incarnation = self.next_logical_incarnation;
            self.next_logical_incarnation = incarnation
                .checked_add(1)
                .expect("node annex logical incarnation exhausted");
            self.blocks.push(AnnexBlock::new(incarnation));
            self.metrics.superblocks_allocated += 1;
        }
    }

    fn next_serial(&mut self) -> u32 {
        let serial = self.next_publication_serial;
        self.next_publication_serial = serial
            .checked_add(1)
            .expect("node annex publication serial exhausted");
        serial
    }

    pub(crate) fn append_fixed<Kind>(&mut self, body: &[u32]) -> AnnexKey<Kind> {
        let word_len = body
            .len()
            .checked_add(1)
            .expect("node annex fixed record length overflow");
        assert!(
            word_len <= 41,
            "fixed node annex record exceeds design maximum"
        );
        let tail = self.words % ANNEX_WORDS_PER_BLOCK;
        if tail != 0 && tail + word_len > ANNEX_WORDS_PER_BLOCK {
            let padding = ANNEX_WORDS_PER_BLOCK - tail;
            self.words += padding;
            self.metrics.boundary_padding_words += padding as u64;
        }
        self.append_contiguous(body)
    }

    pub(crate) fn append_span<Kind>(&mut self, body: &[u32]) -> AnnexKey<Kind> {
        self.append_contiguous(body)
    }

    fn append_contiguous<Kind>(&mut self, body: &[u32]) -> AnnexKey<Kind> {
        let serial = self.next_serial();
        let start = self.words;
        let total = body
            .len()
            .checked_add(1)
            .expect("node annex span length overflow");
        for (index, word) in core::iter::once(serial)
            .chain(body.iter().copied())
            .enumerate()
        {
            let position = start + index;
            let ordinal = position / ANNEX_WORDS_PER_BLOCK;
            let offset = position % ANNEX_WORDS_PER_BLOCK;
            self.ensure_block(ordinal);
            let block = &mut self.blocks[ordinal];
            block.words[offset] = word;
            block.initialized = block.initialized.max(offset + 1);
        }
        self.words = start
            .checked_add(total)
            .expect("node annex logical length overflow");
        self.metrics.words_published += total as u64;
        let ordinal = start / ANNEX_WORDS_PER_BLOCK;
        AnnexKey {
            owner: self.owner,
            block_ordinal: u32::try_from(ordinal).expect("node annex block ordinal overflow"),
            logical_block_incarnation: self.blocks[ordinal].logical_incarnation,
            word_offset: u32::try_from(start % ANNEX_WORDS_PER_BLOCK)
                .expect("node annex offset overflow"),
            word_len: u32::try_from(total).expect("node annex record length overflow"),
            publication_serial: serial,
            kind: PhantomData,
        }
    }

    pub(crate) fn rollback(&mut self, mark: NodeAnnexMark) -> bool {
        if mark.owner != self.owner || mark.words > self.words {
            return false;
        }
        let removed = self.words - mark.words;
        self.words = mark.words;
        let retained_blocks = self.words.div_ceil(ANNEX_WORDS_PER_BLOCK);
        while self.blocks.len() > retained_blocks {
            self.blocks.pop();
            self.metrics.superblocks_reclaimed += 1;
        }
        if let Some(block) = self.blocks.last_mut() {
            block.initialized = self.words % ANNEX_WORDS_PER_BLOCK;
            if block.initialized == 0 && self.words != 0 {
                block.initialized = ANNEX_WORDS_PER_BLOCK;
            }
        }
        self.metrics.words_rolled_back += removed as u64;
        true
    }

    pub(crate) fn resolve_fixed<Kind>(&mut self, key: AnnexKey<Kind>) -> Option<&[u32]> {
        let reject = |arena: &mut Self| {
            arena.metrics.stale_rejections += 1;
            None
        };
        if key.owner != self.owner || key.word_len == 0 {
            return reject(self);
        }
        let ordinal = key.block_ordinal as usize;
        let offset = key.word_offset as usize;
        let len = key.word_len as usize;
        let Some(block) = self.blocks.get(ordinal) else {
            return reject(self);
        };
        if block.logical_incarnation != key.logical_block_incarnation
            || offset
                .checked_add(len)
                .is_none_or(|end| end > block.initialized)
            || block.words.get(offset).copied() != Some(key.publication_serial)
        {
            return reject(self);
        }
        self.metrics.direct_lookups += 1;
        block.words.get(offset + 1..offset + len)
    }

    pub(crate) fn resolve_fixed_shared<Kind>(&self, key: AnnexKey<Kind>) -> Option<&[u32]> {
        if key.owner != self.owner || key.word_len == 0 {
            return None;
        }
        let block = self.blocks.get(key.block_ordinal as usize)?;
        let offset = key.word_offset as usize;
        let len = key.word_len as usize;
        if block.logical_incarnation != key.logical_block_incarnation
            || offset.checked_add(len)? > block.initialized
            || block.words.get(offset).copied()? != key.publication_serial
        {
            return None;
        }
        block.words.get(offset + 1..offset + len)
    }

    pub(crate) fn resolve_word<Kind>(&self, key: AnnexKey<Kind>, index: usize) -> Option<u32> {
        let len = key.word_len as usize;
        if key.owner != self.owner || index + 1 >= len {
            return None;
        }
        let absolute = (key.block_ordinal as usize)
            .checked_mul(ANNEX_WORDS_PER_BLOCK)?
            .checked_add(key.word_offset as usize)?;
        let serial_ordinal = absolute / ANNEX_WORDS_PER_BLOCK;
        let serial_offset = absolute % ANNEX_WORDS_PER_BLOCK;
        let serial_block = self.blocks.get(serial_ordinal)?;
        if serial_block.logical_incarnation != key.logical_block_incarnation
            || serial_block.words.get(serial_offset).copied() != Some(key.publication_serial)
        {
            return None;
        }
        let position = absolute.checked_add(index + 1)?;
        if position >= self.words {
            return None;
        }
        self.blocks
            .get(position / ANNEX_WORDS_PER_BLOCK)?
            .words
            .get(position % ANNEX_WORDS_PER_BLOCK)
            .copied()
    }

    fn detach_span<Kind>(&self, key: AnnexKey<Kind>) -> Option<Vec<u32>> {
        let body_len = (key.word_len as usize).checked_sub(1)?;
        (0..body_len)
            .map(|index| self.resolve_word(key, index))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    enum Fixed {}

    #[test]
    fn exact_record_and_key_layouts_are_copy_only() {
        assert_eq!(core::mem::size_of::<NodeRecord>(), 32);
        assert_eq!(core::mem::align_of::<NodeRecord>(), 4);
        assert!(!core::mem::needs_drop::<NodeRecord>());
        assert_eq!(core::mem::size_of::<Option<NodeRecord>>(), 32);
        assert_eq!(core::mem::size_of::<AnnexKey<Fixed>>(), 24);
        assert_eq!(core::mem::align_of::<AnnexKey<Fixed>>(), 4);
        assert!(!core::mem::needs_drop::<AnnexKey<Fixed>>());
    }

    #[test]
    fn rollback_reuse_rejects_old_publication_serial() {
        let mut arena = NodeAnnexArena::new();
        let mark = arena.mark();
        let stale = arena.append_fixed::<Fixed>(&[7, 8]);
        assert_eq!(arena.resolve_fixed(stale), Some([7, 8].as_slice()));
        assert!(arena.rollback(mark));
        let current = arena.append_fixed::<Fixed>(&[9, 10]);
        assert!(arena.resolve_fixed(stale).is_none());
        assert_eq!(arena.resolve_fixed(current), Some([9, 10].as_slice()));
    }

    #[test]
    fn fixed_records_pad_instead_of_crossing_a_superblock() {
        let mut arena = NodeAnnexArena::new();
        let body = vec![0; ANNEX_WORDS_PER_BLOCK - 2];
        let _ = arena.append_span::<()>(&body);
        let fixed = arena.append_fixed::<Fixed>(&[1, 2, 3]);
        assert_eq!(fixed.word_offset, 0);
        assert_eq!(arena.metrics().boundary_padding_words, 1);
        assert_eq!(arena.resolve_fixed(fixed), Some([1, 2, 3].as_slice()));
    }

    fn box_node() -> BoxNode<PageListId> {
        BoxNode::new(BoxNodeFields {
            width: Scaled::from_raw(10),
            height: Scaled::from_raw(20),
            depth: Scaled::from_raw(3),
            shift: Scaled::from_raw(-4),
            box_lr: BoxLr::Reversed,
            glue_set: GlueSetRatio::from_ratio_parts(-3, 7),
            glue_sign: Sign::Shrinking,
            glue_order: Order::Fill,
            children: PageListId::empty(),
        })
    }

    fn token_key(seed: u32) -> NodeTokenKey {
        NodeTokenKey::from_coordinates([seed, 2, 3, 4, 5, 6])
    }

    fn whatsits() -> Vec<Whatsit> {
        vec![
            Whatsit::OpenOut {
                slot: StreamSlot::new(3),
                path: "out-µ.txt".into(),
            },
            Whatsit::CloseOut {
                slot: Some(StreamSlot::new(4)),
            },
            Whatsit::CloseOut { slot: None },
            Whatsit::DeferredWrite {
                sink: PrintSink::Stream(StreamSlot::new(5)),
                tokens: token_key(10),
            },
            Whatsit::Special {
                class: "pdf:code".into(),
                payload: vec![0, 1, 2, 255, 7],
            },
            Whatsit::DeferredSpecial {
                class: "pdf:code".into(),
                tokens: token_key(11),
            },
            Whatsit::PdfReferenceObject { object: 17 },
            Whatsit::PdfAccessibility(PdfAccessibilityControl::InterwordSpaceOff),
            Whatsit::PdfAnnotation { object: 18 },
            Whatsit::PdfLinkStart { object: 19 },
            Whatsit::PdfLinkEnd { object: 19 },
            Whatsit::PdfRunningLink(true),
            Whatsit::PdfLiteral {
                mode: PdfLiteralMode::Direct,
                payload: b"q 1 0 0 1".to_vec(),
            },
            Whatsit::DeferredPdfLiteral {
                mode: PdfLiteralMode::Page,
                tokens: token_key(12),
            },
            Whatsit::PdfSetMatrix {
                payload: b"1 0 0 1".to_vec(),
            },
            Whatsit::PdfSave,
            Whatsit::PdfRestore,
            Whatsit::PdfColorStack {
                id: 2,
                action: crate::PdfColorStackAction::Set(vec![1, 2, 3]),
            },
            Whatsit::PdfColorStack {
                id: 2,
                action: crate::PdfColorStackAction::Push(vec![4, 5]),
            },
            Whatsit::PdfColorStack {
                id: 2,
                action: crate::PdfColorStackAction::Pop,
            },
            Whatsit::PdfColorStack {
                id: 2,
                action: crate::PdfColorStackAction::Current,
            },
            Whatsit::PdfSavePos,
            Whatsit::PdfSnapRefPoint,
            Whatsit::PdfSnapY {
                glue: GlueSpec {
                    width: Scaled::from_raw(1),
                    stretch: Scaled::from_raw(2),
                    stretch_order: Order::Fil,
                    shrink: Scaled::from_raw(3),
                    shrink_order: Order::Fill,
                },
            },
            Whatsit::PdfSnapYComp { ratio: 511 },
            Whatsit::PdfRefXForm {
                object: 20,
                width: Scaled::from_raw(1),
                height: Scaled::from_raw(2),
                depth: Scaled::from_raw(3),
            },
            Whatsit::PdfRefXImage {
                object: 21,
                width: Scaled::from_raw(4),
                height: Scaled::from_raw(5),
                depth: Scaled::from_raw(6),
            },
            Whatsit::PdfDestination(Box::new(PdfDestinationNode {
                identifier: NodePdfActionIdentifier::Name(token_key(13)),
                structure: Some(22),
                kind: PdfDestinationKind::FitRectangle(crate::PdfAnnotationDimensions {
                    width: Some(Scaled::from_raw(7)),
                    height: None,
                    depth: Some(Scaled::from_raw(9)),
                }),
            })),
            Whatsit::PdfThread(Box::new(PdfThreadNode {
                identifier: NodePdfActionIdentifier::Number(23),
                dimensions: crate::PdfAnnotationDimensions {
                    width: None,
                    height: Some(Scaled::from_raw(10)),
                    depth: None,
                },
                attributes: token_key(14),
                running: true,
            })),
            Whatsit::PdfEndThread,
            Whatsit::Language {
                language: 7,
                left_hyphen_min: 2,
                right_hyphen_min: 3,
            },
        ]
    }

    fn all_node_kinds() -> Vec<Node> {
        let empty = PageListId::empty();
        let glue = GlueSpec {
            width: Scaled::from_raw(10),
            stretch: Scaled::from_raw(2),
            stretch_order: Order::Fill,
            shrink: Scaled::from_raw(1),
            shrink_order: Order::Fil,
        };
        vec![
            Node::Char {
                font: crate::font::NULL_FONT,
                ch: 'λ',
                origin: OriginId::from_raw(88),
            },
            Node::Lig {
                font: crate::font::NULL_FONT,
                ch: 'ﬃ',
                orig: vec!['f', 'f', 'i'],
                left_hit: true,
                right_hit: false,
                origins: vec![
                    OriginId::from_raw(1),
                    OriginId::from_raw(2),
                    OriginId::from_raw(3),
                ],
            },
            Node::Kern {
                amount: Scaled::from_raw(-11),
                kind: KernKind::Auto,
            },
            Node::MarginKern {
                amount: Scaled::from_raw(12),
                side: MarginKernSide::Right,
                font: crate::font::NULL_FONT,
                ch: b'A',
            },
            Node::Glue {
                spec: glue,
                kind: GlueKind::Cleaders,
                leader: Some(LeaderPayload::HList(box_node())),
            },
            Node::Penalty(-50),
            Node::Rule {
                width: Some(Scaled::from_raw(1)),
                height: None,
                depth: Some(Scaled::from_raw(3)),
            },
            Node::HList(box_node()),
            Node::VList(box_node()),
            Node::Unset(UnsetNode::new(UnsetNodeFields {
                kind: UnsetKind::VBox,
                width: Scaled::from_raw(1),
                height: Scaled::from_raw(2),
                depth: Scaled::from_raw(3),
                span_count: 65_535,
                stretch: Scaled::from_raw(4),
                stretch_order: Order::Filll,
                shrink: Scaled::from_raw(5),
                shrink_order: Order::Fil,
                children: empty,
            })),
            Node::Disc {
                kind: DiscKind::AutomaticHyphen,
                pre: empty,
                post: empty,
                replace: empty,
                physical_replace_count: 255,
            },
            Node::Mark {
                class: 65_535,
                tokens: token_key(20),
            },
            Node::Ins {
                class: 65_535,
                size: Scaled::from_raw(6),
                split_top_skip: glue,
                split_max_depth: Scaled::from_raw(7),
                floating_penalty: -100,
                content: empty,
            },
            Node::Whatsit(whatsits().remove(0)),
            Node::MathOn(Scaled::from_raw(8)),
            Node::MathOff(Scaled::from_raw(9)),
            Node::Direction(crate::node::Direction::BeginR),
            Node::MathNoad(MathNoad {
                kind: NoadKind::Accent {
                    accent: MathChar {
                        family: 15,
                        character: '^',
                        origin: OriginId::from_raw(91),
                    },
                },
                nucleus: MathField::MathChar(MathChar {
                    family: 3,
                    character: 'x',
                    origin: OriginId::from_raw(92),
                }),
                subscript: MathField::SubBox(empty),
                superscript: MathField::SubMlist(empty),
            }),
            Node::FractionNoad(MathFraction {
                numerator: empty,
                denominator: empty,
                thickness: FractionThickness::Explicit(Scaled::from_raw(-1)),
                left_delimiter: Some(0),
                right_delimiter: Some(u32::MAX),
            }),
            Node::MathStyle(MathStyle::ScriptScript),
            Node::MathChoice(MathChoice {
                display: empty,
                text: empty,
                script: empty,
                script_script: empty,
            }),
            Node::MathList(MathListNode {
                display: true,
                content: empty,
            }),
            Node::Nonscript,
            Node::Adjust(AdjustNode {
                content: empty,
                pre: true,
            }),
        ]
    }

    #[test]
    fn every_node_kind_round_trips_through_record_and_annex() {
        let mut arena = NodeAnnexArena::new();
        let nodes = all_node_kinds();
        assert_eq!(nodes.len(), NodeKind::ALL.len());
        for (node, expected_kind) in nodes.into_iter().zip(NodeKind::ALL) {
            assert_eq!(node.kind(), expected_kind);
            let record = NodeRecord::encode_owned(node.clone(), &mut arena);
            assert_eq!(record.kind(), Some(expected_kind));
            assert_eq!(record.decode_owned(&arena), Some(node), "{expected_kind:?}");
        }
    }

    #[test]
    fn every_whatsit_subtype_round_trips() {
        let mut arena = NodeAnnexArena::new();
        for whatsit in whatsits() {
            let node = Node::Whatsit(whatsit);
            let record = NodeRecord::encode_owned(node.clone(), &mut arena);
            assert_eq!(record.kind(), Some(NodeKind::Whatsit));
            assert_eq!(record.decode_owned(&arena), Some(node));
        }
    }
}
