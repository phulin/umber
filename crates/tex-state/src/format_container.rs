//! Portable, versioned container for frozen format-image sections.
//!
//! This module owns only the outer wire contract. Section payload schemas are
//! deliberately independent so they can migrate from the schema-v9 semantic
//! reconstruction path to runtime-ready frozen stores incrementally.

use std::borrow::Cow;
use std::fmt;
use std::io::{Read, Write};

pub(crate) const MAGIC: [u8; 8] = *b"UMBRFMT\0";
pub(crate) const SCHEMA_VERSION: u32 = 10;
pub(crate) const HEADER_LEN: usize = 80;
const DIRECTORY_ENTRY_LEN: usize = 40;
const CHECKSUM_OFFSET: usize = 56;
const CHECKSUM_LEN: usize = 8;
const MAX_SECTIONS: usize = 64;
const MAX_ALIGNMENT: u32 = 4096;
const MAX_LOGICAL_SECTION_LEN: usize = 512 * 1024 * 1024;
const DEFLATE_FLAG: u32 = 1;

/// Transitional Universe-level metadata payload; store data lives elsewhere.
pub(crate) const TRANSITIONAL_SEMANTIC_SECTION: u32 = 1;

pub(crate) const ABI_FINGERPRINT: u64 = fingerprint(
    b"umber.format.container.v2;le;header=80;directory=40;refs=relative-or-index;sections=deflate;checksum=fnv1a64-zero-field",
);
pub(crate) const LOOKUP_CONFIGURATION_FINGERPRINT: u64 = fingerprint(
    b"umber.format.lookup.v2;fnv1a64;seed=cbf29ce484222325;capacity=pow2-lte-3/4;probe=linear;empty=ffffffff;tokens=direct-target-u32",
);
const LEGACY_ABI_FINGERPRINT: u64 = fingerprint(
    b"umber.format.container.v1;le;header=80;directory=40;refs=relative-or-index;checksum=fnv1a64-zero-field",
);
const LEGACY_LOOKUP_CONFIGURATION_FINGERPRINT: u64 = fingerprint(
    b"umber.format.lookup.v1;fnv1a64;seed=cbf29ce484222325;capacity=pow2-lte-3/4;probe=linear;empty=ffffffff",
);

const fn fingerprint(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let mut index = 0;
    while index < bytes.len() {
        hash ^= bytes[index] as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        index += 1;
    }
    hash
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct SectionInput<'a> {
    pub kind: u32,
    pub alignment: u32,
    pub bytes: &'a [u8],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DecodedSection<'a> {
    pub kind: u32,
    #[cfg_attr(not(test), allow(dead_code))]
    pub alignment: u32,
    #[cfg_attr(not(test), allow(dead_code))]
    pub offset: usize,
    pub bytes: Cow<'a, [u8]>,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct DecodedContainer<'a> {
    pub checksum: u64,
    pub sections: Vec<DecodedSection<'a>>,
}

impl<'a> DecodedContainer<'a> {
    pub fn section(&self, kind: u32) -> Option<&DecodedSection<'a>> {
        self.sections.iter().find(|section| section.kind == kind)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ContainerError {
    BadMagic,
    UnsupportedVersion(u32),
    Truncated,
    TrailingBytes,
    Checksum,
    IncompatibleAbi(u64),
    IncompatibleLookupConfiguration(u64),
    Invalid(&'static str),
}

impl fmt::Display for ContainerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadMagic => f.write_str("bad magic"),
            Self::UnsupportedVersion(version) => write!(f, "unsupported schema {version}"),
            Self::Truncated => f.write_str("truncated container"),
            Self::TrailingBytes => f.write_str("trailing bytes"),
            Self::Checksum => f.write_str("checksum mismatch"),
            Self::IncompatibleAbi(found) => write!(f, "incompatible ABI fingerprint {found:#018x}"),
            Self::IncompatibleLookupConfiguration(found) => {
                write!(f, "incompatible lookup configuration {found:#018x}")
            }
            Self::Invalid(message) => f.write_str(message),
        }
    }
}

pub(crate) fn encode(sections: &[SectionInput<'_>]) -> Result<Vec<u8>, ContainerError> {
    encode_with_compression(sections, true)
}

fn encode_with_compression(
    sections: &[SectionInput<'_>],
    compress_sections: bool,
) -> Result<Vec<u8>, ContainerError> {
    if sections.is_empty() || sections.len() > MAX_SECTIONS {
        return Err(ContainerError::Invalid("invalid section count"));
    }
    let mut sections = sections.to_vec();
    sections.sort_unstable_by_key(|section| section.kind);
    validate_inputs(&sections)?;

    let directory_len = sections
        .len()
        .checked_mul(DIRECTORY_ENTRY_LEN)
        .ok_or(ContainerError::Invalid("directory length overflow"))?;
    let stored_sections = sections
        .iter()
        .map(|section| {
            if compress_sections {
                compress(section.bytes)
            } else {
                Ok(section.bytes.to_vec())
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut cursor = HEADER_LEN
        .checked_add(directory_len)
        .ok_or(ContainerError::Invalid("directory length overflow"))?;
    let mut records = Vec::with_capacity(sections.len());
    for (section, stored) in sections.iter().zip(&stored_sections) {
        cursor = align_up(cursor, section.alignment)?;
        let end = cursor
            .checked_add(stored.len())
            .ok_or(ContainerError::Invalid("section length overflow"))?;
        records.push((cursor, end));
        cursor = end;
    }

    let file_len = u64::try_from(cursor)
        .map_err(|_| ContainerError::Invalid("container exceeds u64 offsets"))?;
    let mut bytes = vec![0; cursor];
    bytes[..8].copy_from_slice(&MAGIC);
    bytes[8..12].copy_from_slice(&SCHEMA_VERSION.to_le_bytes());
    bytes[12..16].copy_from_slice(&(HEADER_LEN as u32).to_le_bytes());
    bytes[16..20].copy_from_slice(&(DIRECTORY_ENTRY_LEN as u32).to_le_bytes());
    bytes[20..24].copy_from_slice(&(sections.len() as u32).to_le_bytes());
    bytes[24..32].copy_from_slice(&(HEADER_LEN as u64).to_le_bytes());
    bytes[32..40].copy_from_slice(&file_len.to_le_bytes());
    bytes[40..48].copy_from_slice(&ABI_FINGERPRINT.to_le_bytes());
    bytes[48..56].copy_from_slice(&LOOKUP_CONFIGURATION_FINGERPRINT.to_le_bytes());

    for (index, ((section, stored), &(offset, end))) in sections
        .iter()
        .zip(&stored_sections)
        .zip(&records)
        .enumerate()
    {
        let record = HEADER_LEN + index * DIRECTORY_ENTRY_LEN;
        bytes[record..record + 4].copy_from_slice(&section.kind.to_le_bytes());
        bytes[record + 4..record + 8]
            .copy_from_slice(&(if compress_sections { DEFLATE_FLAG } else { 0 }).to_le_bytes());
        bytes[record + 8..record + 16].copy_from_slice(&(offset as u64).to_le_bytes());
        bytes[record + 16..record + 24].copy_from_slice(&(stored.len() as u64).to_le_bytes());
        bytes[record + 24..record + 32]
            .copy_from_slice(&(section.bytes.len() as u64).to_le_bytes());
        bytes[record + 32..record + 36].copy_from_slice(&section.alignment.to_le_bytes());
        bytes[offset..end].copy_from_slice(stored);
    }
    let checksum = image_checksum(&bytes);
    bytes[CHECKSUM_OFFSET..CHECKSUM_OFFSET + CHECKSUM_LEN].copy_from_slice(&checksum.to_le_bytes());
    Ok(bytes)
}

pub(crate) fn decode(bytes: &[u8]) -> Result<DecodedContainer<'_>, ContainerError> {
    if bytes.len() < HEADER_LEN {
        return Err(ContainerError::Truncated);
    }
    if bytes[..8] != MAGIC {
        return Err(ContainerError::BadMagic);
    }
    let version = read_u32(bytes, 8);
    if version != SCHEMA_VERSION {
        return Err(ContainerError::UnsupportedVersion(version));
    }
    if read_u32(bytes, 12) != HEADER_LEN as u32
        || read_u32(bytes, 16) != DIRECTORY_ENTRY_LEN as u32
        || read_u64(bytes, 24) != HEADER_LEN as u64
    {
        return Err(ContainerError::Invalid("non-canonical header geometry"));
    }
    let declared_len =
        usize::try_from(read_u64(bytes, 32)).map_err(|_| ContainerError::Truncated)?;
    if bytes.len() < declared_len {
        return Err(ContainerError::Truncated);
    }
    if bytes.len() > declared_len {
        return Err(ContainerError::TrailingBytes);
    }
    let expected_checksum = read_u64(bytes, CHECKSUM_OFFSET);
    if image_checksum(bytes) != expected_checksum {
        return Err(ContainerError::Checksum);
    }
    let abi = read_u64(bytes, 40);
    if abi != ABI_FINGERPRINT && abi != LEGACY_ABI_FINGERPRINT {
        return Err(ContainerError::IncompatibleAbi(abi));
    }
    let configuration = read_u64(bytes, 48);
    let compatible_pair = (abi == ABI_FINGERPRINT
        && configuration == LOOKUP_CONFIGURATION_FINGERPRINT)
        || (abi == LEGACY_ABI_FINGERPRINT
            && configuration == LEGACY_LOOKUP_CONFIGURATION_FINGERPRINT);
    if !compatible_pair {
        return Err(ContainerError::IncompatibleLookupConfiguration(
            configuration,
        ));
    }
    if read_u32(bytes, 64) != 0 || bytes[68..HEADER_LEN].iter().any(|byte| *byte != 0) {
        return Err(ContainerError::Invalid("nonzero reserved header field"));
    }

    let section_count = read_u32(bytes, 20) as usize;
    if section_count == 0 || section_count > MAX_SECTIONS {
        return Err(ContainerError::Invalid("invalid section count"));
    }
    let directory_end = HEADER_LEN
        .checked_add(
            section_count
                .checked_mul(DIRECTORY_ENTRY_LEN)
                .ok_or(ContainerError::Invalid("directory length overflow"))?,
        )
        .ok_or(ContainerError::Invalid("directory length overflow"))?;
    if directory_end > bytes.len() {
        return Err(ContainerError::Truncated);
    }

    let mut sections = Vec::with_capacity(section_count);
    let mut cursor = directory_end;
    let mut previous_kind = 0;
    for index in 0..section_count {
        let record = HEADER_LEN + index * DIRECTORY_ENTRY_LEN;
        let kind = read_u32(bytes, record);
        if kind == 0 || kind <= previous_kind {
            return Err(ContainerError::Invalid(
                "section kinds are not unique and sorted",
            ));
        }
        previous_kind = kind;
        let flags = read_u32(bytes, record + 4);
        if flags & !DEFLATE_FLAG != 0 || read_u32(bytes, record + 36) != 0 {
            return Err(ContainerError::Invalid("nonzero reserved section field"));
        }
        let alignment = read_u32(bytes, record + 32);
        validate_alignment(alignment)?;
        let offset = usize::try_from(read_u64(bytes, record + 8))
            .map_err(|_| ContainerError::Invalid("section offset exceeds usize"))?;
        let stored_len = usize::try_from(read_u64(bytes, record + 16))
            .map_err(|_| ContainerError::Invalid("section length exceeds usize"))?;
        let logical_len = usize::try_from(read_u64(bytes, record + 24))
            .map_err(|_| ContainerError::Invalid("section length exceeds usize"))?;
        if flags == 0 && logical_len != stored_len {
            return Err(ContainerError::Invalid(
                "uncompressed section length mismatch",
            ));
        }
        let canonical_offset = align_up(cursor, alignment)?;
        if offset != canonical_offset {
            return Err(ContainerError::Invalid("non-canonical section offset"));
        }
        if bytes[cursor..offset].iter().any(|byte| *byte != 0) {
            return Err(ContainerError::Invalid("nonzero alignment padding"));
        }
        let end = offset
            .checked_add(stored_len)
            .ok_or(ContainerError::Invalid("section range overflow"))?;
        if end > bytes.len() {
            return Err(ContainerError::Truncated);
        }
        let payload = &bytes[offset..end];
        let section_bytes = if flags == DEFLATE_FLAG {
            Cow::Owned(decompress(payload, logical_len)?)
        } else {
            Cow::Borrowed(payload)
        };
        sections.push(DecodedSection {
            kind,
            alignment,
            offset,
            bytes: section_bytes,
        });
        cursor = end;
    }
    if cursor != bytes.len() {
        return Err(ContainerError::Invalid(
            "non-canonical bytes after final section",
        ));
    }
    Ok(DecodedContainer {
        checksum: expected_checksum,
        sections,
    })
}

fn compress(bytes: &[u8]) -> Result<Vec<u8>, ContainerError> {
    let mut encoder = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::new(6));
    encoder
        .write_all(bytes)
        .map_err(|_| ContainerError::Invalid("section compression failed"))?;
    encoder
        .finish()
        .map_err(|_| ContainerError::Invalid("section compression failed"))
}

fn decompress(bytes: &[u8], logical_len: usize) -> Result<Vec<u8>, ContainerError> {
    if logical_len > MAX_LOGICAL_SECTION_LEN {
        return Err(ContainerError::Invalid(
            "compressed section exceeds size limit",
        ));
    }
    let decoder = flate2::read::DeflateDecoder::new(bytes);
    let mut decoder = decoder.take(logical_len as u64 + 1);
    let mut decoded = Vec::new();
    decoder
        .read_to_end(&mut decoded)
        .map_err(|_| ContainerError::Invalid("invalid compressed section"))?;
    if decoded.len() != logical_len {
        return Err(ContainerError::Invalid(
            "compressed section length mismatch",
        ));
    }
    Ok(decoded)
}

fn validate_inputs(sections: &[SectionInput<'_>]) -> Result<(), ContainerError> {
    let mut previous_kind = 0;
    for section in sections {
        if section.kind == 0 || section.kind == previous_kind {
            return Err(ContainerError::Invalid("duplicate or zero section kind"));
        }
        previous_kind = section.kind;
        validate_alignment(section.alignment)?;
    }
    Ok(())
}

fn validate_alignment(alignment: u32) -> Result<(), ContainerError> {
    if !(8..=MAX_ALIGNMENT).contains(&alignment) || !alignment.is_power_of_two() {
        return Err(ContainerError::Invalid("invalid section alignment"));
    }
    Ok(())
}

fn align_up(value: usize, alignment: u32) -> Result<usize, ContainerError> {
    let mask = alignment as usize - 1;
    value
        .checked_add(mask)
        .map(|value| value & !mask)
        .ok_or(ContainerError::Invalid("alignment overflow"))
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("validated fixed field"),
    )
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(
        bytes[offset..offset + 8]
            .try_into()
            .expect("validated fixed field"),
    )
}

fn image_checksum(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for (index, byte) in bytes.iter().copied().enumerate() {
        let covered = if (CHECKSUM_OFFSET..CHECKSUM_OFFSET + CHECKSUM_LEN).contains(&index) {
            0
        } else {
            byte
        };
        hash ^= u64::from(covered);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
pub(crate) fn refresh_checksum(bytes: &mut [u8]) {
    bytes[CHECKSUM_OFFSET..CHECKSUM_OFFSET + CHECKSUM_LEN].fill(0);
    let checksum = image_checksum(bytes);
    bytes[CHECKSUM_OFFSET..CHECKSUM_OFFSET + CHECKSUM_LEN].copy_from_slice(&checksum.to_le_bytes());
}

#[cfg(test)]
mod tests;
