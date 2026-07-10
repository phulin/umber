//! Output artifact substrate.
//!
//! `tex-out` is downstream of the commit barrier. It owns the committed page
//! artifact model and its compact binary encoding, but it has no dependency on
//! live engine state. Shipout code lowers frozen engine nodes into these
//! driver-facing types, stores the serialized bytes through `World`, and output
//! drivers consume the artifact bytes later.
//!
//! # Page artifact binary format
//!
//! All integers are little-endian. Lengths are `u32`. `Scaled` values are their
//! raw `i32` scaled-point representation. Strings and byte arrays are encoded
//! as a `u32` byte length followed by exact bytes. The stream begins with
//! `b"UMPG"` followed by one version byte; version `8` is the only accepted
//! version.
//!
//! ```text
//! magic[4] version:u8
//! job_mag:i32 job_banner
//! fonts_len:u32 font*
//! count0_to_count9:i32[10]
//! root_node
//! effects_len:u32 effect*
//! ```
//!
//! Font resources are serialized in caller-provided order and nodes refer to
//! them by their driver-visible TeX font number. Node and effect variants are
//! tagged with stable `u8` discriminants. Unknown tags or unsupported versions
//! are parse errors. Glue-set ratios are stored as a fixed-point signed `i32`;
//! the artifact path does not contain floats.

mod binary;
pub mod dvi;
mod hash;
mod model;

#[cfg(test)]
mod tests;

pub use binary::ParseError;
pub use hash::ContentHash;
pub use model::{
    BoxNode, DEFAULT_BANNER, DiscKind, EffectSink, FontResource, GlueKind, GlueOrder, GlueSetRatio,
    GlueSign, GlueSpec, JobInfo, KernKind, LeaderPayload, PageArtifact, PageEffect, PageNode,
    PageToken, TokenCatcode,
};
