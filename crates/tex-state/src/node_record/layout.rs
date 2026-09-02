use super::*;

const HEADER_PRESENT: u32 = 1 << 31;
const KIND_MASK: u32 = 0x1f;
const SUBTYPE_SHIFT: u32 = 5;
const SUBTYPE_MASK: u32 = 0x1f << SUBTYPE_SHIFT;
const FLAGS_SHIFT: u32 = 10;
const FLAGS_MASK: u32 = !(KIND_MASK | SUBTYPE_MASK | HEADER_PRESENT);

pub(super) fn bool_word(value: bool) -> u32 {
    u32::from(value)
}

pub(super) fn decode_bool(value: u32) -> Option<bool> {
    match value {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    }
}

pub(super) fn scaled_word(value: Scaled) -> u32 {
    value.raw() as u32
}

pub(super) fn decode_scaled(value: u32) -> Scaled {
    Scaled::from_raw(value as i32)
}

#[allow(dead_code)]
pub(super) fn encode_option_scaled(value: Option<Scaled>) -> [u32; 2] {
    match value {
        Some(value) => [1, scaled_word(value)],
        None => [0, 0],
    }
}

#[allow(dead_code)]
pub(super) fn decode_option_scaled(words: [u32; 2]) -> Option<Option<Scaled>> {
    match words[0] {
        0 if words[1] == 0 => Some(None),
        1 => Some(Some(decode_scaled(words[1]))),
        _ => None,
    }
}

pub(super) fn encode_glue(value: GlueSpec) -> [u32; 4] {
    [
        scaled_word(value.width),
        scaled_word(value.stretch),
        scaled_word(value.shrink),
        (value.stretch_order as u32) | ((value.shrink_order as u32) << 8),
    ]
}

pub(super) fn decode_order(value: u32) -> Option<Order> {
    match value {
        0 => Some(Order::Normal),
        1 => Some(Order::Fil),
        2 => Some(Order::Fill),
        3 => Some(Order::Filll),
        _ => None,
    }
}

pub(super) fn decode_glue(words: [u32; 4]) -> Option<GlueSpec> {
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

pub(super) fn append_words<const N: usize>(destination: &mut Vec<u32>, words: [u32; N]) {
    destination.extend(words);
}

pub(super) fn take_words<const N: usize>(source: &[u32], cursor: &mut usize) -> Option<[u32; N]> {
    let end = cursor.checked_add(N)?;
    let words = source.get(*cursor..end)?.try_into().ok()?;
    *cursor = end;
    Some(words)
}

#[allow(dead_code)]
pub(super) fn decode_node_kind(value: u32) -> Option<NodeKind> {
    NodeKind::ALL.get(value as usize).copied()
}

pub(super) fn encode_kern_kind(value: KernKind) -> u8 {
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

pub(super) fn decode_kern_kind(value: u8) -> Option<KernKind> {
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

pub(super) fn encode_margin_side(value: MarginKernSide) -> u8 {
    match value {
        MarginKernSide::Left => 0,
        MarginKernSide::Right => 1,
    }
}

pub(super) fn decode_margin_side(value: u8) -> Option<MarginKernSide> {
    match value {
        0 => Some(MarginKernSide::Left),
        1 => Some(MarginKernSide::Right),
        _ => None,
    }
}

pub(super) fn encode_glue_kind(value: GlueKind) -> u8 {
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

pub(super) fn decode_glue_kind(value: u8) -> Option<GlueKind> {
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

pub(super) fn encode_disc_kind(value: DiscKind) -> u8 {
    match value {
        DiscKind::Discretionary => 0,
        DiscKind::ExplicitHyphen => 1,
        DiscKind::AutomaticHyphen => 2,
    }
}

pub(super) fn decode_disc_kind(value: u8) -> Option<DiscKind> {
    match value {
        0 => Some(DiscKind::Discretionary),
        1 => Some(DiscKind::ExplicitHyphen),
        2 => Some(DiscKind::AutomaticHyphen),
        _ => None,
    }
}

pub(super) fn encode_math_style(value: MathStyle) -> u8 {
    match value {
        MathStyle::Display => 0,
        MathStyle::Text => 1,
        MathStyle::Script => 2,
        MathStyle::ScriptScript => 3,
    }
}

pub(super) fn decode_math_style(value: u8) -> Option<MathStyle> {
    match value {
        0 => Some(MathStyle::Display),
        1 => Some(MathStyle::Text),
        2 => Some(MathStyle::Script),
        3 => Some(MathStyle::ScriptScript),
        _ => None,
    }
}

pub(super) fn encode_print_sink(value: PrintSink) -> u32 {
    match value {
        PrintSink::Terminal => 0,
        PrintSink::Log => 1,
        PrintSink::TerminalAndLog => 2,
        PrintSink::Stream(slot) => 3 | (u32::from(slot.raw()) << 8),
    }
}

pub(super) fn decode_print_sink(value: u32) -> Option<PrintSink> {
    match value & 0xff {
        0 if value == 0 => Some(PrintSink::Terminal),
        1 if value == 1 => Some(PrintSink::Log),
        2 if value == 2 => Some(PrintSink::TerminalAndLog),
        3 if value >> 8 < 16 => Some(PrintSink::Stream(StreamSlot::new((value >> 8) as u8))),
        _ => None,
    }
}

#[repr(C)]
pub struct NodeRecord<Lane = PageMaterialLane> {
    header: NonZeroU32,
    words: [u32; 7],
    lane: PhantomData<fn(&Lane) -> &Lane>,
}

impl<Lane> Copy for NodeRecord<Lane> {}

impl<Lane> Clone for NodeRecord<Lane> {
    fn clone(&self) -> Self {
        *self
    }
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
