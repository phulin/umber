use std::collections::{BTreeMap, BTreeSet};

use tex_arith::Scaled;

use super::digest::{page_digest, revision_digest};
use super::{
    RenderDigest, RenderKey, RenderNode, RenderPage, RenderResource, RenderRevision,
    RenderSessionId,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PatchLimits {
    pub max_operations: usize,
    pub max_nodes: usize,
    pub max_resource_bytes: usize,
}

impl Default for PatchLimits {
    fn default() -> Self {
        Self {
            max_operations: 250_000,
            max_nodes: 1_000_000,
            max_resource_bytes: 256 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PatchPlan {
    pub schema_version: u16,
    pub session_id: RenderSessionId,
    pub base_revision: u64,
    pub target_revision: u64,
    pub before_digest: RenderDigest,
    pub after_digest: RenderDigest,
    pub title: Option<String>,
    pub language: Option<String>,
    pub resource_additions: Vec<RenderResource>,
    pub resource_releases: Vec<[u8; 32]>,
    pub operations: Vec<PatchOp>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PatchOp {
    RemoveNode {
        page: RenderKey,
        key: RenderKey,
    },
    RemovePage {
        key: RenderKey,
    },
    InsertPage {
        index: usize,
        page: RenderPage,
    },
    MovePage {
        key: RenderKey,
        index: usize,
    },
    InsertNode {
        page: RenderKey,
        index: usize,
        node: RenderNode,
    },
    MoveNode {
        page: RenderKey,
        key: RenderKey,
        index: usize,
    },
    UpdatePage(RenderPageHeader),
    UpdateNode {
        page: RenderKey,
        node: RenderNode,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RenderPageHeader {
    pub key: RenderKey,
    pub digest: RenderDigest,
    pub match_digest: RenderDigest,
    pub ordinal: u32,
    pub width: Scaled,
    pub height: Scaled,
    pub origin_x: Scaled,
    pub origin_y: Scaled,
    pub mag: i32,
    pub counts: [i32; 10],
}

impl From<&RenderPage> for RenderPageHeader {
    fn from(page: &RenderPage) -> Self {
        Self {
            key: page.key,
            digest: page.digest,
            match_digest: page.match_digest,
            ordinal: page.ordinal,
            width: page.width,
            height: page.height,
            origin_x: page.origin_x,
            origin_y: page.origin_y,
            mag: page.mag,
            counts: page.counts,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PatchPlanError {
    SessionMismatch,
    RevisionMismatch { base: u64, target: u64 },
    TooManyNodes { count: usize, limit: usize },
    TooManyOperations { count: usize, limit: usize },
    ResourcesTooLarge { bytes: usize, limit: usize },
}

impl std::fmt::Display for PatchPlanError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SessionMismatch => formatter.write_str("cannot diff render sessions"),
            Self::RevisionMismatch { base, target } => {
                write!(formatter, "render revision {target} does not follow {base}")
            }
            Self::TooManyNodes { count, limit } => {
                write!(formatter, "patch compares {count} nodes, exceeding {limit}")
            }
            Self::TooManyOperations { count, limit } => {
                write!(
                    formatter,
                    "patch requires {count} operations, exceeding {limit}"
                )
            }
            Self::ResourcesTooLarge { bytes, limit } => write!(
                formatter,
                "patch adds {bytes} resource bytes, exceeding {limit}"
            ),
        }
    }
}

impl std::error::Error for PatchPlanError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PatchApplyError {
    WrongSession,
    WrongBaseRevision { mounted: u64, patch: u64 },
    WrongBaseDigest,
    InvalidOperation,
    DuplicateKey,
    MissingResource,
    TargetDigestMismatch,
    Limits(PatchPlanError),
}

impl std::fmt::Display for PatchApplyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "incremental HTML patch rejected: {self:?}")
    }
}

impl std::error::Error for PatchApplyError {}

pub fn plan_patch(
    base: &RenderRevision,
    target: &RenderRevision,
    limits: PatchLimits,
) -> Result<PatchPlan, PatchPlanError> {
    if base.session_id != target.session_id {
        return Err(PatchPlanError::SessionMismatch);
    }
    if target.revision != base.revision.saturating_add(1) {
        return Err(PatchPlanError::RevisionMismatch {
            base: base.revision,
            target: target.revision,
        });
    }
    let node_count = base
        .pages
        .iter()
        .chain(&target.pages)
        .map(|page| page.nodes.len())
        .sum::<usize>();
    if node_count > limits.max_nodes {
        return Err(PatchPlanError::TooManyNodes {
            count: node_count,
            limit: limits.max_nodes,
        });
    }
    let base_resources = base
        .resources
        .iter()
        .map(|resource| (resource.identity, resource))
        .collect::<BTreeMap<_, _>>();
    let target_resources = target
        .resources
        .iter()
        .map(|resource| (resource.identity, resource))
        .collect::<BTreeMap<_, _>>();
    let resource_additions = target_resources
        .iter()
        .filter_map(|(identity, resource)| {
            (!base_resources.contains_key(identity)).then_some((*resource).clone())
        })
        .collect::<Vec<_>>();
    let resource_bytes = resource_additions
        .iter()
        .map(|resource| resource.bytes.len())
        .sum::<usize>();
    if resource_bytes > limits.max_resource_bytes {
        return Err(PatchPlanError::ResourcesTooLarge {
            bytes: resource_bytes,
            limit: limits.max_resource_bytes,
        });
    }
    let resource_releases = base_resources
        .keys()
        .filter(|identity| !target_resources.contains_key(*identity))
        .copied()
        .collect();
    let base_pages = base
        .pages
        .iter()
        .map(|page| (page.key, page))
        .collect::<BTreeMap<_, _>>();
    let target_pages = target
        .pages
        .iter()
        .map(|page| (page.key, page))
        .collect::<BTreeMap<_, _>>();
    let mut operations = Vec::new();

    // Children are removed before their parent page.
    for page in &base.pages {
        let Some(target_page) = target_pages.get(&page.key) else {
            continue;
        };
        let target_keys = target_page
            .nodes
            .iter()
            .map(|node| node.key)
            .collect::<BTreeSet<_>>();
        for node in page.nodes.iter().rev() {
            if !target_keys.contains(&node.key) {
                operations.push(PatchOp::RemoveNode {
                    page: page.key,
                    key: node.key,
                });
            }
        }
    }
    for page in base.pages.iter().rev() {
        if !target_pages.contains_key(&page.key) {
            operations.push(PatchOp::RemovePage { key: page.key });
        }
    }
    for (index, page) in target.pages.iter().enumerate() {
        if !base_pages.contains_key(&page.key) {
            operations.push(PatchOp::InsertPage {
                index,
                page: page.clone(),
            });
        }
    }
    for (index, page) in target.pages.iter().enumerate() {
        let Some(base_page) = base_pages.get(&page.key) else {
            continue;
        };
        if base.pages.get(index).map(|candidate| candidate.key) != Some(page.key) {
            operations.push(PatchOp::MovePage {
                key: page.key,
                index,
            });
        }
        let base_nodes = base_page
            .nodes
            .iter()
            .map(|node| (node.key, node))
            .collect::<BTreeMap<_, _>>();
        for (node_index, node) in page.nodes.iter().enumerate() {
            match base_nodes.get(&node.key) {
                None => operations.push(PatchOp::InsertNode {
                    page: page.key,
                    index: node_index,
                    node: node.clone(),
                }),
                Some(old) => {
                    if base_page.nodes.get(node_index).map(|value| value.key) != Some(node.key) {
                        operations.push(PatchOp::MoveNode {
                            page: page.key,
                            key: node.key,
                            index: node_index,
                        });
                    }
                    if old.digest != node.digest {
                        operations.push(PatchOp::UpdateNode {
                            page: page.key,
                            node: node.clone(),
                        });
                    }
                }
            }
        }
        if RenderPageHeader::from(*base_page) != RenderPageHeader::from(page) {
            operations.push(PatchOp::UpdatePage(RenderPageHeader::from(page)));
        }
    }
    if operations.len() > limits.max_operations {
        return Err(PatchPlanError::TooManyOperations {
            count: operations.len(),
            limit: limits.max_operations,
        });
    }
    Ok(PatchPlan {
        schema_version: super::RENDER_SCHEMA_VERSION,
        session_id: base.session_id,
        base_revision: base.revision,
        target_revision: target.revision,
        before_digest: base.digest,
        after_digest: target.digest,
        title: (base.title != target.title).then(|| target.title.clone()),
        language: (base.language != target.language).then(|| target.language.clone()),
        resource_additions,
        resource_releases,
        operations,
    })
}

pub fn apply_patch(
    base: &RenderRevision,
    patch: &PatchPlan,
    limits: PatchLimits,
) -> Result<RenderRevision, PatchApplyError> {
    if base.session_id != patch.session_id {
        return Err(PatchApplyError::WrongSession);
    }
    if base.revision != patch.base_revision {
        return Err(PatchApplyError::WrongBaseRevision {
            mounted: base.revision,
            patch: patch.base_revision,
        });
    }
    if base.digest != patch.before_digest {
        return Err(PatchApplyError::WrongBaseDigest);
    }
    if patch.target_revision != base.revision.saturating_add(1) {
        return Err(PatchApplyError::WrongBaseRevision {
            mounted: base.revision,
            patch: patch.base_revision,
        });
    }
    if patch.operations.len() > limits.max_operations {
        return Err(PatchApplyError::Limits(PatchPlanError::TooManyOperations {
            count: patch.operations.len(),
            limit: limits.max_operations,
        }));
    }
    let mut candidate = base.clone();
    candidate.revision = patch.target_revision;
    if let Some(title) = &patch.title {
        candidate.title.clone_from(title);
    }
    if let Some(language) = &patch.language {
        candidate.language.clone_from(language);
    }
    apply_resources(&mut candidate.resources, patch)?;
    for operation in &patch.operations {
        apply_operation(&mut candidate.pages, operation)?;
    }
    validate_keys_and_resources(&candidate, limits)?;
    for page in &mut candidate.pages {
        page.digest = page_digest(page);
    }
    candidate.digest = revision_digest(
        &candidate.title,
        &candidate.language,
        &candidate.pages,
        &candidate.resources,
    );
    if candidate.digest != patch.after_digest {
        return Err(PatchApplyError::TargetDigestMismatch);
    }
    Ok(candidate)
}

fn apply_resources(
    resources: &mut Vec<RenderResource>,
    patch: &PatchPlan,
) -> Result<(), PatchApplyError> {
    let additions = patch
        .resource_additions
        .iter()
        .map(|resource| (resource.identity, resource))
        .collect::<BTreeMap<_, _>>();
    if additions.len() != patch.resource_additions.len() {
        return Err(PatchApplyError::DuplicateKey);
    }
    resources.retain(|resource| !patch.resource_releases.contains(&resource.identity));
    for resource in additions.into_values() {
        if resources
            .iter()
            .any(|current| current.identity == resource.identity)
        {
            return Err(PatchApplyError::DuplicateKey);
        }
        resources.push(resource.clone());
    }
    resources.sort_by_key(|resource| resource.identity);
    Ok(())
}

fn apply_operation(
    pages: &mut Vec<RenderPage>,
    operation: &PatchOp,
) -> Result<(), PatchApplyError> {
    match operation {
        PatchOp::RemoveNode { page, key } => {
            let nodes = &mut page_mut(pages, *page)?.nodes;
            let index = nodes
                .iter()
                .position(|node| node.key == *key)
                .ok_or(PatchApplyError::InvalidOperation)?;
            nodes.remove(index);
        }
        PatchOp::RemovePage { key } => {
            let index = pages
                .iter()
                .position(|page| page.key == *key)
                .ok_or(PatchApplyError::InvalidOperation)?;
            pages.remove(index);
        }
        PatchOp::InsertPage { index, page } => {
            if *index > pages.len() || pages.iter().any(|current| current.key == page.key) {
                return Err(PatchApplyError::InvalidOperation);
            }
            pages.insert(*index, page.clone());
        }
        PatchOp::MovePage { key, index } => {
            let old = pages
                .iter()
                .position(|page| page.key == *key)
                .ok_or(PatchApplyError::InvalidOperation)?;
            let page = pages.remove(old);
            if *index > pages.len() {
                return Err(PatchApplyError::InvalidOperation);
            }
            pages.insert(*index, page);
        }
        PatchOp::InsertNode { page, index, node } => {
            let nodes = &mut page_mut(pages, *page)?.nodes;
            if *index > nodes.len() || nodes.iter().any(|current| current.key == node.key) {
                return Err(PatchApplyError::InvalidOperation);
            }
            nodes.insert(*index, node.clone());
        }
        PatchOp::MoveNode { page, key, index } => {
            let nodes = &mut page_mut(pages, *page)?.nodes;
            let old = nodes
                .iter()
                .position(|node| node.key == *key)
                .ok_or(PatchApplyError::InvalidOperation)?;
            let node = nodes.remove(old);
            if *index > nodes.len() {
                return Err(PatchApplyError::InvalidOperation);
            }
            nodes.insert(*index, node);
        }
        PatchOp::UpdatePage(header) => {
            let page = page_mut(pages, header.key)?;
            page.digest = header.digest;
            page.match_digest = header.match_digest;
            page.ordinal = header.ordinal;
            page.width = header.width;
            page.height = header.height;
            page.origin_x = header.origin_x;
            page.origin_y = header.origin_y;
            page.mag = header.mag;
            page.counts = header.counts;
        }
        PatchOp::UpdateNode { page, node } => {
            let nodes = &mut page_mut(pages, *page)?.nodes;
            let old = nodes
                .iter_mut()
                .find(|current| current.key == node.key)
                .ok_or(PatchApplyError::InvalidOperation)?;
            *old = node.clone();
        }
    }
    Ok(())
}

fn page_mut(pages: &mut [RenderPage], key: RenderKey) -> Result<&mut RenderPage, PatchApplyError> {
    pages
        .iter_mut()
        .find(|page| page.key == key)
        .ok_or(PatchApplyError::InvalidOperation)
}

fn validate_keys_and_resources(
    revision: &RenderRevision,
    limits: PatchLimits,
) -> Result<(), PatchApplyError> {
    let resources = revision
        .resources
        .iter()
        .map(|resource| resource.identity)
        .collect::<BTreeSet<_>>();
    if resources.len() != revision.resources.len() {
        return Err(PatchApplyError::DuplicateKey);
    }
    let resource_bytes = revision
        .resources
        .iter()
        .map(|resource| resource.bytes.len())
        .sum::<usize>();
    if resource_bytes > limits.max_resource_bytes {
        return Err(PatchApplyError::Limits(PatchPlanError::ResourcesTooLarge {
            bytes: resource_bytes,
            limit: limits.max_resource_bytes,
        }));
    }
    let mut keys = BTreeSet::new();
    let mut nodes = 0usize;
    for page in &revision.pages {
        if !keys.insert(page.key) {
            return Err(PatchApplyError::DuplicateKey);
        }
        for node in &page.nodes {
            nodes = nodes.saturating_add(1);
            if !keys.insert(node.key) {
                return Err(PatchApplyError::DuplicateKey);
            }
            if let super::RenderNodeValue::Text(text) = &node.value
                && !resources.contains(&text.resource)
            {
                return Err(PatchApplyError::MissingResource);
            }
        }
    }
    if nodes > limits.max_nodes {
        return Err(PatchApplyError::Limits(PatchPlanError::TooManyNodes {
            count: nodes,
            limit: limits.max_nodes,
        }));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::digest::{node_value_digest, page_digest, revision_digest};
    use super::super::{RENDER_SCHEMA_VERSION, RenderNodeValue};
    use super::*;
    use crate::MathRule;

    #[test]
    fn five_thousand_generated_model_edits_equal_fresh_targets() {
        let session_id = RenderSessionId::from_bytes([31; 16]);
        let mut model = vec![(1_u32, 10_i32), (2, 20), (3, 30)];
        let mut mounted = revision(session_id, 1, &model);
        let mut random = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next_id = 4_u32;

        for revision_number in 2..=5_001 {
            random = random
                .wrapping_mul(2_862_933_555_777_941_757)
                .wrapping_add(3_037_000_493);
            let choice = (random >> 32) as usize;
            match choice % 4 {
                0 if model.len() < 16 => {
                    let index = choice % (model.len() + 1);
                    model.insert(index, (next_id, random as i32));
                    next_id += 1;
                }
                1 if model.len() > 1 => {
                    model.remove(choice % model.len());
                }
                2 if model.len() > 1 => {
                    let value = model.remove(choice % model.len());
                    let index = (random as usize) % (model.len() + 1);
                    model.insert(index, value);
                }
                _ => {
                    let index = choice % model.len();
                    model[index].1 = model[index].1.wrapping_add((random >> 16) as i32 | 1);
                }
            }

            let fresh = revision(session_id, revision_number, &model);
            let patch =
                plan_patch(&mounted, &fresh, PatchLimits::default()).expect("generated patch");
            mounted = apply_patch(&mounted, &patch, PatchLimits::default())
                .unwrap_or_else(|error| panic!("revision {revision_number}: {error:?}"));
            assert_eq!(mounted, fresh, "revision {revision_number}");
        }
    }

    fn revision(
        session_id: RenderSessionId,
        revision: u64,
        model: &[(u32, i32)],
    ) -> RenderRevision {
        let mut pages = model
            .iter()
            .enumerate()
            .map(|(ordinal, &(id, content))| page(id, content, ordinal as u32))
            .collect::<Vec<_>>();
        for page in &mut pages {
            page.digest = page_digest(page);
        }
        let resources = Vec::new();
        let digest = revision_digest("", "en", &pages, &resources);
        RenderRevision {
            schema_version: RENDER_SCHEMA_VERSION,
            session_id,
            revision,
            title: String::new(),
            language: "en".to_owned(),
            pages,
            resources,
            digest,
        }
    }

    fn page(id: u32, content: i32, ordinal: u32) -> RenderPage {
        let node_count = content.unsigned_abs() as usize % 3 + 1;
        let mut nodes = (0..node_count)
            .map(|slot| {
                let value = RenderNodeValue::MathRule(MathRule {
                    x: Scaled::from_raw(slot as i32),
                    y: Scaled::from_raw(0),
                    width: Scaled::from_raw(content.wrapping_add(slot as i32)),
                    height: Scaled::from_raw(1),
                });
                RenderNode {
                    key: key(id, slot as u32 + 1),
                    digest: node_value_digest(&value, false),
                    match_digest: node_value_digest(&value, true),
                    event_ordinal: slot as u32,
                    value,
                }
            })
            .collect::<Vec<_>>();
        if content & 1 != 0 {
            nodes.reverse();
        }
        RenderPage {
            key: key(id, 0),
            digest: RenderDigest([0; 32]),
            match_digest: RenderDigest([content as u8; 32]),
            ordinal,
            width: Scaled::from_raw(100),
            height: Scaled::from_raw(200),
            origin_x: Scaled::from_raw(0),
            origin_y: Scaled::from_raw(0),
            mag: 1_000,
            counts: [content, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            nodes,
        }
    }

    fn key(id: u32, slot: u32) -> RenderKey {
        let mut bytes = [0; 16];
        bytes[..4].copy_from_slice(&id.to_le_bytes());
        bytes[4..8].copy_from_slice(&slot.to_le_bytes());
        RenderKey(bytes)
    }
}
