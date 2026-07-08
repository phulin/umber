use crate::ContentHash;
use tex_arith::Scaled;

/// A committed page artifact.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PageArtifact {
    pub job: JobInfo,
    pub fonts: Vec<FontResource>,
    pub counts: [i32; 10],
    pub root: PageNode,
    pub effects: Vec<PageEffect>,
}

impl PageArtifact {
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        crate::binary::to_bytes(self)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, crate::ParseError> {
        crate::binary::from_bytes(bytes)
    }

    #[must_use]
    pub fn content_hash(&self) -> ContentHash {
        ContentHash::from_bytes(&self.to_bytes())
    }
}

/// Job-level data captured at shipout for downstream output drivers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobInfo {
    pub mag: i32,
    pub banner: String,
}

impl Default for JobInfo {
    fn default() -> Self {
        Self {
            mag: 1000,
            banner: DEFAULT_BANNER.to_owned(),
        }
    }
}

pub const DEFAULT_BANNER: &str = "This is Umber, Version 0.1.0";

/// A font resource referenced by the page tree.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FontResource {
    pub font_id: u32,
    pub name: String,
    pub tfm_content_hash: ContentHash,
    pub tfm_checksum: u32,
    pub design_size: Scaled,
    pub at_size: Scaled,
}

/// Fixed-point glue-set ratio. The exact scale is chosen by the shipout
/// lowering code; the artifact format only requires deterministic integer
/// storage.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct GlueSetRatio {
    pub raw: i32,
}

/// A driver-facing shipped node.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PageNode {
    Char {
        font_id: u32,
        ch: u32,
    },
    Lig {
        font_id: u32,
        ch: u32,
        left: u32,
        right: u32,
    },
    Kern {
        amount: Scaled,
        kind: KernKind,
    },
    Glue {
        spec: GlueSpec,
        kind: GlueKind,
    },
    Penalty(i32),
    Rule {
        width: Option<Scaled>,
        height: Option<Scaled>,
        depth: Option<Scaled>,
    },
    HList(BoxNode),
    VList(BoxNode),
    Unset,
    WhatsitAnchor {
        effect_index: u32,
    },
    MathOn,
    MathOff,
}

/// A shipped hlist/vlist payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoxNode {
    pub width: Scaled,
    pub height: Scaled,
    pub depth: Scaled,
    pub shift: Scaled,
    pub glue_set: GlueSetRatio,
    pub glue_sign: GlueSign,
    pub glue_order: GlueOrder,
    pub children: Vec<PageNode>,
}

/// A concrete glue specification, lowered out of the live glue store.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GlueSpec {
    pub width: Scaled,
    pub stretch: Scaled,
    pub stretch_order: GlueOrder,
    pub shrink: Scaled,
    pub shrink_order: GlueOrder,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum GlueOrder {
    Normal,
    Fil,
    Fill,
    Filll,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum GlueSign {
    Normal,
    Stretching,
    Shrinking,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum KernKind {
    Explicit,
    Font,
    Accent,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum GlueKind {
    Normal,
    BaselineSkip,
    LineSkip,
    LeftSkip,
    RightSkip,
    ParFillSkip,
    Leaders,
    Cleaders,
    Xleaders,
}

/// One committed side-effect payload associated with the page.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PageEffect {
    OpenOut { stream: u8, path: String },
    CloseOut { stream: u8 },
    Write { sink: EffectSink, text: String },
    Special { class: String, payload: Vec<u8> },
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum EffectSink {
    Terminal,
    Log,
    TerminalAndLog,
    Stream(u8),
}
