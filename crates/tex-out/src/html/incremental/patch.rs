use std::collections::{BTreeMap, BTreeSet};

use tex_arith::Scaled;

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
    pub resource_releases: Vec<[u8; 8]>,
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
