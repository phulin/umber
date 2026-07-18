use super::{FormatEnvEntry, FormatEnvValue, FormatListKey, StoreFormatError};

pub(crate) const FROZEN_ENV_SECTION: u32 = 528;

const VERSION: u32 = 1;
const HEADER_LEN: usize = 16;
const RECORD_LEN: usize = 24;
const RAW_TAG: u8 = 0;
const BOX_TAG: u8 = 1;

pub(crate) fn encode(entries: &[FormatEnvEntry]) -> Result<Vec<u8>, StoreFormatError> {
    let count = u32::try_from(entries.len())
        .map_err(|_| StoreFormatError::Invalid("frozen environment count exceeds u32"))?;
    let len = entries
        .len()
        .checked_mul(RECORD_LEN)
        .and_then(|len| HEADER_LEN.checked_add(len))
        .ok_or(StoreFormatError::Invalid(
            "frozen environment section overflow",
        ))?;
    let mut bytes = vec![0_u8; len];
    bytes[0..4].copy_from_slice(&VERSION.to_le_bytes());
    bytes[4..8].copy_from_slice(&count.to_le_bytes());
    bytes[8..12].copy_from_slice(&(HEADER_LEN as u32).to_le_bytes());
    let mut previous = None;
    for (index, entry) in entries.iter().enumerate() {
        if previous.is_some_and(|previous| previous >= entry.cell) {
            return Err(StoreFormatError::Invalid(
                "noncanonical frozen environment order",
            ));
        }
        previous = Some(entry.cell);
        let offset = HEADER_LEN + index * RECORD_LEN;
        bytes[offset..offset + 8].copy_from_slice(&entry.cell.to_le_bytes());
        let (tag, payload) = match entry.value {
            FormatEnvValue::Raw(word) => (RAW_TAG, word),
            FormatEnvValue::Box(key) => {
                if key.survivor_root.is_some() {
                    return Err(StoreFormatError::Invalid("noncanonical frozen box key"));
                }
                (BOX_TAG, u64::from(key.start) | (u64::from(key.len) << 32))
            }
        };
        bytes[offset + 8] = tag;
        bytes[offset + 16..offset + 24].copy_from_slice(&payload.to_le_bytes());
    }
    Ok(bytes)
}

pub(crate) fn decode(bytes: &[u8]) -> Result<Vec<FormatEnvEntry>, StoreFormatError> {
    if bytes.len() < HEADER_LEN {
        return Err(StoreFormatError::Invalid(
            "truncated frozen environment header",
        ));
    }
    if read_u32(bytes, 0) != VERSION || read_u32(bytes, 8) != HEADER_LEN as u32 {
        return Err(StoreFormatError::Invalid("frozen environment header"));
    }
    if read_u32(bytes, 12) != 0 {
        return Err(StoreFormatError::Invalid(
            "frozen environment reserved header",
        ));
    }
    let count = usize::try_from(read_u32(bytes, 4))
        .map_err(|_| StoreFormatError::Invalid("frozen environment count"))?;
    let expected = count
        .checked_mul(RECORD_LEN)
        .and_then(|len| HEADER_LEN.checked_add(len))
        .ok_or(StoreFormatError::Invalid(
            "frozen environment section overflow",
        ))?;
    if bytes.len() != expected {
        return Err(StoreFormatError::Invalid(
            "frozen environment section geometry",
        ));
    }
    let mut entries = Vec::with_capacity(count);
    let mut previous = None;
    for index in 0..count {
        let offset = HEADER_LEN + index * RECORD_LEN;
        let cell = read_u64(bytes, offset);
        if previous.is_some_and(|previous| previous >= cell) {
            return Err(StoreFormatError::Invalid(
                "noncanonical frozen environment order",
            ));
        }
        previous = Some(cell);
        if bytes[offset + 9..offset + 16].iter().any(|&byte| byte != 0) {
            return Err(StoreFormatError::Invalid(
                "frozen environment reserved record",
            ));
        }
        let payload = read_u64(bytes, offset + 16);
        let value = match bytes[offset + 8] {
            RAW_TAG => FormatEnvValue::Raw(payload),
            BOX_TAG => FormatEnvValue::Box(FormatListKey {
                survivor_root: None,
                start: payload as u32,
                len: (payload >> 32) as u32,
            }),
            _ => return Err(StoreFormatError::Invalid("frozen environment value tag")),
        };
        entries.push(FormatEnvEntry { cell, value });
    }
    Ok(entries)
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("checked range"))
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().expect("checked range"))
}
