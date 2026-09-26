//! Typed, non-owning box-construction descriptors.

use super::annex::BOX_PAYLOAD_WORDS;

const COPIED_BOX_BODY_TAG: u32 = 0x4342_4f58;
const POSITIVE_BOX_BODY_TAG: u32 = 0x5042_4f58;

/// Non-owning provenance captured during the required recursive copy of a
/// box's children. The actual consumed wrapper remains the move authority.
#[derive(Clone, Debug)]
pub(crate) struct CopiedBoxBodyStamp {
    region: crate::node_region::NodeRegionId,
    node_body: std::ops::Range<usize>,
    annex_body: std::ops::Range<usize>,
    wrapper_node_position: usize,
    wrapper_annex_position: usize,
}

impl CopiedBoxBodyStamp {
    pub(crate) fn new(
        region: crate::node_region::NodeRegionId,
        node_body: std::ops::Range<usize>,
        annex_body: std::ops::Range<usize>,
        wrapper_node_position: usize,
        wrapper_annex_position: usize,
    ) -> Option<Self> {
        if node_body.start > node_body.end
            || annex_body.start > annex_body.end
            || (node_body.is_empty() && annex_body.is_empty())
            || node_body.end > wrapper_node_position
            || annex_body.end > wrapper_annex_position
            || [
                node_body.start,
                node_body.end,
                annex_body.start,
                annex_body.end,
                wrapper_node_position,
                wrapper_annex_position,
            ]
            .into_iter()
            .any(|position| u32::try_from(position).is_err())
        {
            return None;
        }
        Some(Self {
            region,
            node_body,
            annex_body,
            wrapper_node_position,
            wrapper_annex_position,
        })
    }

    fn words(&self) -> [u32; 15] {
        let region = self.region.words();
        [
            0,
            0,
            0,
            0,
            self.node_body.start as u32,
            self.node_body.end as u32,
            self.annex_body.start as u32,
            self.annex_body.end as u32,
            region[0],
            region[1],
            region[2],
            region[3],
            self.wrapper_node_position as u32,
            self.wrapper_annex_position as u32,
            COPIED_BOX_BODY_TAG,
        ]
    }

    /// Writes the descriptor while the required explicit-copy body is still
    /// private scratch. Publication may seal an annex group immediately.
    pub(crate) fn write_flat_body(&self, body: &mut [u32]) -> Option<()> {
        if body.len() != BOX_PAYLOAD_WORDS || body[28..].iter().any(|word| *word != 0) {
            return None;
        }
        body[28..].copy_from_slice(&self.words());
        Some(())
    }

    fn from_words(words: [u32; 15]) -> Option<Self> {
        if words[..4] != [0; 4] || words[14] != COPIED_BOX_BODY_TAG {
            return None;
        }
        Self::new(
            crate::node_region::NodeRegionId::from_words(words[8..12].try_into().ok()?)?,
            words[4] as usize..words[5] as usize,
            words[6] as usize..words[7] as usize,
            words[12] as usize,
            words[13] as usize,
        )
    }

    pub(crate) fn metadata_at_wrapper(
        &self,
        region: crate::node_region::NodeRegionId,
        wrapper_node: usize,
        wrapper_annex: usize,
    ) -> Option<crate::page_node_arena::PageBoxMigrationMetadata> {
        // A whole-region move may change both owner-relative offsets, but
        // keeps the spacing between this wrapper and its copied child block.
        if self.region.words()[..2] != region.words()[..2] {
            return None;
        }
        let node_shift =
            i64::try_from(wrapper_node).ok()? - i64::try_from(self.wrapper_node_position).ok()?;
        let annex_shift =
            i64::try_from(wrapper_annex).ok()? - i64::try_from(self.wrapper_annex_position).ok()?;
        let rebase = |position: usize, shift: i64| -> Option<u32> {
            u32::try_from(i64::try_from(position).ok()?.checked_add(shift)?).ok()
        };
        let node_start = rebase(self.node_body.start, node_shift)?;
        let node_end = rebase(self.node_body.end, node_shift)?;
        let annex_start = rebase(self.annex_body.start, annex_shift)?;
        let annex_end = rebase(self.annex_body.end, annex_shift)?;
        let wrapper_node = u32::try_from(wrapper_node).ok()?;
        let wrapper_annex = u32::try_from(wrapper_annex).ok()?;
        let segment = crate::node_region::PageBoxSegment::from_exclusion_bounds(
            region,
            [
                node_start,
                wrapper_node.checked_add(1)?,
                annex_start,
                wrapper_annex.checked_add(1)?,
            ],
        )?;
        let exclusion = crate::node_region::PageBoxSegment::from_exclusion_bounds(
            region,
            [node_end, wrapper_node, annex_end, wrapper_annex],
        );
        if node_end > wrapper_node || annex_end > wrapper_annex {
            return None;
        }
        Some(crate::page_node_arena::PageBoxMigrationMetadata {
            segment,
            exclusions: exclusion.into_iter().collect(),
            positive: None,
            sidecar_annex_range: None,
            wrapper_rebuild: true,
        })
    }
}

pub(super) enum BoxConstructionDescriptor {
    None,
    Original {
        segment: crate::node_region::PageBoxSegment,
        migrations: Option<crate::page_node_arena::PageBoxMigrationKey>,
    },
    Copied(CopiedBoxBodyStamp),
    Positive {
        region: crate::node_region::NodeRegionId,
        key: crate::page_node_arena::PageBoxPositiveKey,
        wrapper_node: usize,
        wrapper_annex: usize,
    },
}

pub(super) fn decode_box_construction_descriptor(
    words: &[u32],
) -> Option<BoxConstructionDescriptor> {
    if words.len() != BOX_PAYLOAD_WORDS {
        return None;
    }
    let words: [u32; 15] = words.get(28..43)?.try_into().ok()?;
    if words.iter().all(|word| *word == 0) {
        return Some(BoxConstructionDescriptor::None);
    }
    if words[..4] == [0; 4] {
        return Some(BoxConstructionDescriptor::Copied(
            CopiedBoxBodyStamp::from_words(words)?,
        ));
    }
    // Word 13 is the original sidecar key's nonzero list length. A copied
    // descriptor has a zero region prefix, and an absent original key has an
    // all-zero tail, so this reserved zero-length/tag pair is disjoint even
    // when an original publication serial equals the positive tag.
    if words[13] == 0 && words[14] == POSITIVE_BOX_BODY_TAG {
        return Some(BoxConstructionDescriptor::Positive {
            region: crate::node_region::NodeRegionId::from_words(words[..4].try_into().ok()?)?,
            key: crate::page_node_arena::PageBoxPositiveKey::from_words(
                words[4..11].try_into().ok()?,
            ),
            wrapper_node: words[11] as usize,
            wrapper_annex: words[12] as usize,
        });
    }
    let segment = crate::node_region::PageBoxSegment::from_words(words[..8].try_into().ok()?)?;
    let key: [u32; 7] = words[8..].try_into().ok()?;
    if key.iter().any(|word| *word != 0) && key[5] == 0 {
        return None;
    }
    let migrations = (!key.iter().all(|word| *word == 0))
        .then(|| crate::page_node_arena::PageBoxMigrationKey::from_words(key));
    Some(BoxConstructionDescriptor::Original {
        segment,
        migrations,
    })
}

pub(crate) fn write_positive_box_body(
    body: &mut [u32],
    region: crate::node_region::NodeRegionId,
    key: crate::page_node_arena::PageBoxPositiveKey,
    wrapper_node: usize,
    wrapper_annex: usize,
) -> Option<()> {
    if body.len() != BOX_PAYLOAD_WORDS || body[28..].iter().any(|word| *word != 0) {
        return None;
    }
    let mut words = [0; 15];
    words[..4].copy_from_slice(&region.words());
    words[4..11].copy_from_slice(&key.words());
    words[11] = u32::try_from(wrapper_node).ok()?;
    words[12] = u32::try_from(wrapper_annex).ok()?;
    words[14] = POSITIVE_BOX_BODY_TAG;
    body[28..].copy_from_slice(&words);
    Some(())
}

/// Encodes original construction geometry in private fixed-body scratch.
/// Both the paired sidecar and final wrapper positions are authenticated by
/// the caller's publication reservation before this body becomes visible.
pub(crate) fn write_original_box_body(
    body: &mut [u32],
    segment: crate::node_region::PageBoxSegment,
    migrations: Option<crate::page_node_arena::PageBoxMigrationKey>,
) -> Option<()> {
    if body.len() != BOX_PAYLOAD_WORDS || body[28..].iter().any(|word| *word != 0) {
        return None;
    }
    body[28..36].copy_from_slice(&segment.words());
    if let Some(migrations) = migrations {
        body[36..43].copy_from_slice(&migrations.words());
    }
    Some(())
}

pub(crate) fn valid_box_exclusions(
    region: [u32; 4],
    segments: &[crate::node_region::PageBoxSegment],
) -> bool {
    let mut previous = None;
    for &segment in segments {
        let words = segment.words();
        if words[..4] != region
            || words[4] > words[5]
            || words[6] > words[7]
            || (words[4] == words[5] && words[6] == words[7])
            || previous.is_some_and(|end: [u32; 2]| end[0] > words[4] || end[1] > words[6])
        {
            return false;
        }
        previous = Some([words[5], words[7]]);
    }
    true
}
