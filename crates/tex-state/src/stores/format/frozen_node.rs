//! Portable schema-10 node-graph section.
//!
//! List metadata is a fixed-width table. Node DTO payloads use bincode's
//! explicitly selected little-endian fixed-integer vocabulary; they are
//! detached values, never native node words or runtime handles.

use super::{FormatNode, FormatNodeList, StoreFormat, StoreFormatError};
use bincode::Options;

pub(crate) const FROZEN_NODES_SECTION: u32 = 512;
const VERSION: u32 = 1;
const HEADER_LEN: usize = 32;
const RECORD_LEN: usize = 40;

pub(crate) struct FrozenNodeSection<'a> {
    pub(crate) bytes: &'a [u8],
}

pub(crate) struct DecodedFrozenNodes {
    pub(crate) lists: Vec<FormatNodeList>,
    pub(crate) semantic_ids: Vec<u64>,
}

fn options() -> impl Options {
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_little_endian()
        .reject_trailing_bytes()
}

pub(super) fn encode(
    format: &StoreFormat,
    _: &crate::stores::Stores,
) -> Result<Vec<u8>, StoreFormatError> {
    let count = u32::try_from(format.node_lists.len())
        .map_err(|_| StoreFormatError::Invalid("frozen node-list count exceeds u32"))?;
    let payload_offset = HEADER_LEN
        .checked_add(format.node_lists.len().checked_mul(RECORD_LEN).ok_or(
            StoreFormatError::Invalid("frozen node record table overflow"),
        )?)
        .ok_or(StoreFormatError::Invalid(
            "frozen node record table overflow",
        ))?;
    let mut payload = Vec::new();
    let mut records = Vec::with_capacity(format.node_lists.len());
    for list in &format.node_lists {
        let start = u32::try_from(payload.len())
            .map_err(|_| StoreFormatError::Invalid("frozen node payload exceeds u32"))?;
        let bytes = options()
            .serialize(&list.nodes)
            .map_err(|error| StoreFormatError::Codec(error.to_string()))?;
        let len = u32::try_from(bytes.len())
            .map_err(|_| StoreFormatError::Invalid("frozen node payload exceeds u32"))?;
        let node_count = u32::try_from(list.nodes.len())
            .map_err(|_| StoreFormatError::Invalid("frozen node list exceeds u32"))?;
        payload.extend_from_slice(&bytes);
        records.push((list, start, len, node_count));
    }
    let payload_len = u32::try_from(payload.len())
        .map_err(|_| StoreFormatError::Invalid("frozen node payload exceeds u32"))?;
    let total = payload_offset
        .checked_add(payload.len())
        .ok_or(StoreFormatError::Invalid("frozen node section overflow"))?;
    let mut out = vec![0_u8; total];
    put_u32(&mut out, 0, VERSION);
    put_u32(&mut out, 4, count);
    put_u32(&mut out, 8, HEADER_LEN as u32);
    put_u32(&mut out, 12, payload_offset as u32);
    put_u32(&mut out, 16, payload_len);
    for (index, (list, start, len, node_count)) in records.into_iter().enumerate() {
        let at = HEADER_LEN + index * RECORD_LEN;
        put_u32(&mut out, at, list.key.survivor_root.unwrap_or(u32::MAX));
        put_u32(&mut out, at + 4, list.key.start);
        put_u32(&mut out, at + 8, list.key.len);
        put_u32(&mut out, at + 12, start);
        put_u32(&mut out, at + 16, len);
        put_u32(&mut out, at + 20, node_count);
        put_u64(&mut out, at + 24, list.semantic_id);
        // at + 32..40 is reserved.
    }
    out[payload_offset..].copy_from_slice(&payload);
    Ok(out)
}

pub(super) fn decode(
    section: FrozenNodeSection<'_>,
) -> Result<DecodedFrozenNodes, StoreFormatError> {
    let bytes = section.bytes;
    if bytes.len() < HEADER_LEN {
        return Err(StoreFormatError::Invalid("truncated frozen node header"));
    }
    if get_u32(bytes, 0) != VERSION || get_u32(bytes, 8) != HEADER_LEN as u32 {
        return Err(StoreFormatError::Invalid("frozen node header"));
    }
    if bytes[20..HEADER_LEN].iter().any(|byte| *byte != 0) {
        return Err(StoreFormatError::Invalid("frozen node reserved header"));
    }
    let count = get_u32(bytes, 4) as usize;
    let records_end = HEADER_LEN
        .checked_add(
            count
                .checked_mul(RECORD_LEN)
                .ok_or(StoreFormatError::Invalid(
                    "frozen node record table overflow",
                ))?,
        )
        .ok_or(StoreFormatError::Invalid(
            "frozen node record table overflow",
        ))?;
    let payload_offset = get_u32(bytes, 12) as usize;
    let payload_len = get_u32(bytes, 16) as usize;
    if payload_offset != records_end || payload_offset.checked_add(payload_len) != Some(bytes.len())
    {
        return Err(StoreFormatError::Invalid("frozen node section geometry"));
    }
    let mut lists = Vec::with_capacity(count);
    let mut semantic_ids = Vec::with_capacity(count);
    let mut previous_end = 0_usize;
    for index in 0..count {
        let at = HEADER_LEN + index * RECORD_LEN;
        if bytes[at + 32..at + RECORD_LEN]
            .iter()
            .any(|byte| *byte != 0)
        {
            return Err(StoreFormatError::Invalid("frozen node reserved record"));
        }
        let start = get_u32(bytes, at + 12) as usize;
        let len = get_u32(bytes, at + 16) as usize;
        let end = start
            .checked_add(len)
            .ok_or(StoreFormatError::Invalid("frozen node payload range"))?;
        if start != previous_end || end > payload_len {
            return Err(StoreFormatError::Invalid(
                "noncanonical frozen node payload spans",
            ));
        }
        previous_end = end;
        let payload = &bytes[payload_offset + start..payload_offset + end];
        let nodes: Vec<FormatNode> = options().deserialize(payload).map_err(|error| {
            StoreFormatError::Codec(format!("frozen node payload codec: {error}"))
        })?;
        if nodes.len() != get_u32(bytes, at + 20) as usize {
            return Err(StoreFormatError::Invalid("frozen node count mismatch"));
        }
        let root = get_u32(bytes, at);
        let semantic = get_u64(bytes, at + 24);
        lists.push(FormatNodeList {
            key: super::FormatListKey {
                survivor_root: (root != u32::MAX).then_some(root),
                start: get_u32(bytes, at + 4),
                len: get_u32(bytes, at + 8),
            },
            semantic_id: semantic,
            nodes,
        });
        semantic_ids.push(semantic);
    }
    if previous_end != payload_len {
        return Err(StoreFormatError::Invalid("frozen node trailing payload"));
    }
    Ok(DecodedFrozenNodes {
        lists,
        semantic_ids,
    })
}

fn get_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().expect("validated fixed field"))
}

fn get_u64(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().expect("validated fixed field"))
}

fn put_u32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}
