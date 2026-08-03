use std::collections::BTreeSet;

use sha2::{Digest as _, Sha256};

use super::{
    PatchApplyError, PatchLimits, PatchOp, PatchPlan, RenderNodeValue, RenderRevision, apply_patch,
};

pub const PATCH_SCHEMA_VERSION: u16 = 1;
pub const PATCH_CAP_TYPED_DOM: u32 = 1;
const PATCH_MAGIC: [u8; 4] = *b"UMHP";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProtocolLimits {
    pub patch: PatchLimits,
    pub max_wire_bytes: usize,
    pub max_strings: usize,
    pub max_string_bytes: usize,
    pub max_total_string_bytes: usize,
}

impl Default for ProtocolLimits {
    fn default() -> Self {
        Self {
            patch: PatchLimits::default(),
            max_wire_bytes: 256 * 1024 * 1024,
            max_strings: 1_000_000,
            max_string_bytes: 16 * 1024 * 1024,
            max_total_string_bytes: 64 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PatchCounts {
    pub operations: u64,
    pub pages: u64,
    pub nodes: u64,
    pub resources: u64,
    pub resource_bytes: u64,
    pub strings: u64,
    pub string_bytes: u64,
    pub projected_wire_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PatchEnvelope {
    magic: [u8; 4],
    pub schema_version: u16,
    pub required_capabilities: u32,
    pub counts: PatchCounts,
    pub patch: PatchPlan,
}

impl PatchEnvelope {
    #[must_use]
    pub fn new(patch: PatchPlan) -> Self {
        let counts = count_patch(&patch);
        Self {
            magic: PATCH_MAGIC,
            schema_version: PATCH_SCHEMA_VERSION,
            required_capabilities: PATCH_CAP_TYPED_DOM,
            counts,
            patch,
        }
    }

    #[must_use]
    pub const fn magic(&self) -> [u8; 4] {
        self.magic
    }

    #[must_use]
    pub fn canonical_fingerprint(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(b"umber-html-patch-envelope-v1\0");
        hash.update(self.magic);
        hash.update(self.schema_version.to_le_bytes());
        hash.update(self.required_capabilities.to_le_bytes());
        for count in [
            self.counts.operations,
            self.counts.pages,
            self.counts.nodes,
            self.counts.resources,
            self.counts.resource_bytes,
            self.counts.strings,
            self.counts.string_bytes,
            self.counts.projected_wire_bytes,
        ] {
            hash.update(count.to_le_bytes());
        }
        hash.update(self.patch.session_id.as_bytes());
        hash.update(self.patch.base_revision.to_le_bytes());
        hash.update(self.patch.target_revision.to_le_bytes());
        hash.update(self.patch.before_digest.as_bytes());
        hash.update(self.patch.after_digest.as_bytes());
        for operation in &self.patch.operations {
            fingerprint_operation(&mut hash, operation);
        }
        hash.finalize().into()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PatchDelivery {
    Applied(RenderRevision),
    Duplicate,
    ResyncRequired(PatchProtocolError),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PatchProtocolError {
    BadMagic,
    UnsupportedSchema(u16),
    UnsupportedCapabilities(u32),
    DeclaredCountsMismatch,
    WireTooLarge { bytes: u64, limit: usize },
    TooManyStrings { count: u64, limit: usize },
    StringTooLarge { bytes: usize, limit: usize },
    StringsTooLarge { bytes: u64, limit: usize },
    ResourceDigestMismatch,
    DuplicateResourceRelease,
    InvalidString,
    Apply(PatchApplyError),
}

impl std::fmt::Display for PatchProtocolError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "incremental HTML protocol error: {self:?}")
    }
}

impl std::error::Error for PatchProtocolError {}

pub fn validate_delivery(
    mounted: &RenderRevision,
    envelope: &PatchEnvelope,
    limits: ProtocolLimits,
) -> PatchDelivery {
    if envelope.patch.session_id == mounted.session_id
        && envelope.patch.target_revision == mounted.revision
        && envelope.patch.after_digest == mounted.digest
    {
        return PatchDelivery::Duplicate;
    }
    if let Err(error) = preflight(envelope, limits) {
        return PatchDelivery::ResyncRequired(error);
    }
    match apply_patch(mounted, &envelope.patch, limits.patch) {
        Ok(target) => PatchDelivery::Applied(target),
        Err(error) => PatchDelivery::ResyncRequired(PatchProtocolError::Apply(error)),
    }
}

fn preflight(envelope: &PatchEnvelope, limits: ProtocolLimits) -> Result<(), PatchProtocolError> {
    if envelope.magic != PATCH_MAGIC {
        return Err(PatchProtocolError::BadMagic);
    }
    if envelope.schema_version != PATCH_SCHEMA_VERSION
        || envelope.patch.schema_version != PATCH_SCHEMA_VERSION
    {
        return Err(PatchProtocolError::UnsupportedSchema(
            envelope.schema_version,
        ));
    }
    let unsupported = envelope.required_capabilities & !PATCH_CAP_TYPED_DOM;
    if unsupported != 0 {
        return Err(PatchProtocolError::UnsupportedCapabilities(unsupported));
    }
    let actual = count_patch(&envelope.patch);
    if actual != envelope.counts {
        return Err(PatchProtocolError::DeclaredCountsMismatch);
    }
    if actual.projected_wire_bytes > limits.max_wire_bytes as u64 {
        return Err(PatchProtocolError::WireTooLarge {
            bytes: actual.projected_wire_bytes,
            limit: limits.max_wire_bytes,
        });
    }
    if actual.strings > limits.max_strings as u64 {
        return Err(PatchProtocolError::TooManyStrings {
            count: actual.strings,
            limit: limits.max_strings,
        });
    }
    if actual.string_bytes > limits.max_total_string_bytes as u64 {
        return Err(PatchProtocolError::StringsTooLarge {
            bytes: actual.string_bytes,
            limit: limits.max_total_string_bytes,
        });
    }
    validate_strings(&envelope.patch, limits.max_string_bytes)?;
    for resource in &envelope.patch.resource_additions {
        let digest: [u8; 32] = Sha256::digest(&resource.bytes).into();
        if digest != resource.identity {
            return Err(PatchProtocolError::ResourceDigestMismatch);
        }
    }
    let releases = envelope
        .patch
        .resource_releases
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    if releases.len() != envelope.patch.resource_releases.len() {
        return Err(PatchProtocolError::DuplicateResourceRelease);
    }
    Ok(())
}

fn count_patch(patch: &PatchPlan) -> PatchCounts {
    let mut counts = PatchCounts {
        operations: patch.operations.len() as u64,
        resources: patch.resource_additions.len() as u64 + patch.resource_releases.len() as u64,
        resource_bytes: patch
            .resource_additions
            .iter()
            .map(|resource| resource.bytes.len() as u64)
            .sum(),
        ..PatchCounts::default()
    };
    add_string(&mut counts, patch.title.as_deref());
    add_string(&mut counts, patch.language.as_deref());
    for resource in &patch.resource_additions {
        add_string(&mut counts, Some(&resource.provenance));
    }
    for operation in &patch.operations {
        match operation {
            PatchOp::InsertPage { page, .. } => {
                counts.pages += 1;
                counts.nodes += page.nodes.len() as u64;
                for node in &page.nodes {
                    count_node_strings(&mut counts, node);
                }
            }
            PatchOp::InsertNode { node, .. } | PatchOp::UpdateNode { node, .. } => {
                counts.nodes += 1;
                count_node_strings(&mut counts, node);
            }
            PatchOp::UpdatePage(_) => counts.pages += 1,
            PatchOp::RemoveNode { .. }
            | PatchOp::RemovePage { .. }
            | PatchOp::MovePage { .. }
            | PatchOp::MoveNode { .. } => {}
        }
    }
    counts.projected_wire_bytes = 256u64
        .saturating_add(counts.operations.saturating_mul(160))
        .saturating_add(counts.nodes.saturating_mul(128))
        .saturating_add(counts.string_bytes)
        .saturating_add(counts.resource_bytes);
    counts
}

fn count_node_strings(counts: &mut PatchCounts, node: &super::RenderNode) {
    match &node.value {
        RenderNodeValue::Text(text) => {
            add_string(counts, Some(&text.text));
            add_string(counts, Some(&text.font.name));
            add_string(counts, text.language.as_deref());
            add_string(counts, text.color.as_deref());
            add_string(counts, text.link.as_deref());
        }
        RenderNodeValue::Rule(rule) => add_string(counts, rule.color.as_deref()),
        RenderNodeValue::Special(special) => add_string(counts, Some(&special.class)),
        RenderNodeValue::Box(_)
        | RenderNodeValue::MathStart(_)
        | RenderNodeValue::MathGlyph(_)
        | RenderNodeValue::MathRule(_)
        | RenderNodeValue::MathEnd => {}
    }
}

fn add_string(counts: &mut PatchCounts, value: Option<&str>) {
    if let Some(value) = value {
        counts.strings = counts.strings.saturating_add(1);
        counts.string_bytes = counts.string_bytes.saturating_add(value.len() as u64);
    }
}

fn validate_strings(patch: &PatchPlan, max: usize) -> Result<(), PatchProtocolError> {
    let mut strings = Vec::new();
    strings.extend(patch.title.iter().map(String::as_str));
    strings.extend(patch.language.iter().map(String::as_str));
    strings.extend(
        patch
            .resource_additions
            .iter()
            .map(|resource| resource.provenance.as_str()),
    );
    for operation in &patch.operations {
        let nodes: &[super::RenderNode] = match operation {
            PatchOp::InsertPage { page, .. } => &page.nodes,
            PatchOp::InsertNode { node, .. } | PatchOp::UpdateNode { node, .. } => {
                std::slice::from_ref(node)
            }
            _ => &[],
        };
        for node in nodes {
            match &node.value {
                RenderNodeValue::Text(text) => {
                    strings.push(&text.text);
                    strings.push(&text.font.name);
                    strings.extend(text.language.iter().map(String::as_str));
                    strings.extend(text.color.iter().map(String::as_str));
                    strings.extend(text.link.iter().map(String::as_str));
                }
                RenderNodeValue::Rule(rule) => {
                    strings.extend(rule.color.iter().map(String::as_str));
                }
                RenderNodeValue::Special(special) => strings.push(&special.class),
                _ => {}
            }
        }
    }
    for value in strings {
        if value.len() > max {
            return Err(PatchProtocolError::StringTooLarge {
                bytes: value.len(),
                limit: max,
            });
        }
        if value.chars().any(|ch| ch == '\0') {
            return Err(PatchProtocolError::InvalidString);
        }
    }
    Ok(())
}

fn fingerprint_operation(hash: &mut Sha256, operation: &PatchOp) {
    match operation {
        PatchOp::RemoveNode { page, key } => {
            hash.update([0]);
            hash.update(page.as_bytes());
            hash.update(key.as_bytes());
        }
        PatchOp::RemovePage { key } => {
            hash.update([1]);
            hash.update(key.as_bytes());
        }
        PatchOp::InsertPage { index, page } => {
            hash.update([2]);
            hash.update((*index as u64).to_le_bytes());
            hash.update(page.key.as_bytes());
            hash.update(page.digest.as_bytes());
        }
        PatchOp::MovePage { key, index } => {
            hash.update([3]);
            hash.update(key.as_bytes());
            hash.update((*index as u64).to_le_bytes());
        }
        PatchOp::InsertNode { page, index, node } => {
            hash.update([4]);
            hash.update(page.as_bytes());
            hash.update((*index as u64).to_le_bytes());
            hash.update(node.key.as_bytes());
            hash.update(node.digest.as_bytes());
        }
        PatchOp::MoveNode { page, key, index } => {
            hash.update([5]);
            hash.update(page.as_bytes());
            hash.update(key.as_bytes());
            hash.update((*index as u64).to_le_bytes());
        }
        PatchOp::UpdatePage(page) => {
            hash.update([6]);
            hash.update(page.key.as_bytes());
            hash.update(page.digest.as_bytes());
        }
        PatchOp::UpdateNode { page, node } => {
            hash.update([7]);
            hash.update(page.as_bytes());
            hash.update(node.key.as_bytes());
            hash.update(node.digest.as_bytes());
        }
    }
}
