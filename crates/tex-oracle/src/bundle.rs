//! Canonical detached oracle evidence transport.

use crate::{Event, NormalizedEvent, ObservationStream};

/// Portable command and geometry evidence captured from one engine run.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OracleBundle {
    pub semantic: Vec<NormalizedEvent>,
    pub geometry: Vec<NormalizedEvent>,
}

/// Versioned hard limits for the canonical bundle transport.
pub const ORACLE_BUNDLE_SCHEMA: u32 = 2;
pub const MAX_BUNDLE_EVENTS_PER_STREAM: usize = 1_000_000;
pub const MAX_BUNDLE_EVENT_BYTES: usize = 1024 * 1024;
pub const MAX_BUNDLE_STRING_BYTES: usize = 256 * 1024;
pub const MAX_BUNDLE_NESTING_DEPTH: usize = 64;
pub const MAX_BUNDLE_BYTES: usize = 64 * 1024 * 1024;
pub const ORACLE_BUNDLE_MAGIC: &[u8; 8] = b"UMBREVID";

/// Encodes already-normalized events beneath a pinned stream's exact header.
pub fn canonical_bundle_json_lines(
    events: &[NormalizedEvent],
    oracle: &[u8],
) -> Result<Vec<u8>, String> {
    let oracle = ObservationStream::from_canonical_json_lines(oracle)
        .map_err(|error| format!("pinned oracle stream is invalid: {error}"))?;
    let mut bytes = serde_json::to_vec(&oracle.header).map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    for (sequence, event) in events.iter().enumerate() {
        if event.sequence != sequence as u64 {
            return Err(format!(
                "detached evidence sequence {} is not expected sequence {sequence}",
                event.sequence
            ));
        }
        bytes.extend_from_slice(&serde_json::to_vec(event).map_err(|error| error.to_string())?);
        bytes.push(b'\n');
    }
    ObservationStream::from_canonical_json_lines(&bytes).map_err(|error| error.to_string())?;
    Ok(bytes)
}

/// Encodes both independently sequenced streams in the canonical bundle.
pub fn encode_oracle_bundle(bundle: &OracleBundle) -> Result<Vec<u8>, String> {
    validate_bundle(bundle)?;
    let mut out = Vec::new();
    out.extend_from_slice(ORACLE_BUNDLE_MAGIC);
    out.extend_from_slice(&ORACLE_BUNDLE_SCHEMA.to_le_bytes());
    out.extend_from_slice(&(bundle.semantic.len() as u32).to_le_bytes());
    out.extend_from_slice(&(bundle.geometry.len() as u32).to_le_bytes());
    for event in bundle.semantic.iter().chain(&bundle.geometry) {
        let bytes = serde_json::to_vec(event).map_err(|error| error.to_string())?;
        if bytes.len() > MAX_BUNDLE_EVENT_BYTES {
            return Err("detached evidence event exceeds byte limit".into());
        }
        validate_json_shape(&bytes)?;
        out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(&bytes);
        if out.len() > MAX_BUNDLE_BYTES {
            return Err("detached evidence exceeds total byte limit".into());
        }
    }
    Ok(out)
}

/// Decodes and validates a complete canonical oracle bundle.
pub fn decode_oracle_bundle(bytes: &[u8]) -> Result<OracleBundle, String> {
    if bytes.len() > MAX_BUNDLE_BYTES || bytes.len() < 20 || &bytes[..8] != ORACLE_BUNDLE_MAGIC {
        return Err("invalid or oversized detached evidence".into());
    }
    let read = |offset: usize| -> Result<u32, String> {
        Ok(u32::from_le_bytes(
            bytes
                .get(offset..offset + 4)
                .ok_or("truncated detached evidence")?
                .try_into()
                .map_err(|_| "truncated detached evidence")?,
        ))
    };
    if read(8)? != ORACLE_BUNDLE_SCHEMA {
        return Err("unsupported detached evidence schema".into());
    }
    let semantic_count = usize::try_from(read(12)?).map_err(|_| "invalid semantic count")?;
    let geometry_count = usize::try_from(read(16)?).map_err(|_| "invalid geometry count")?;
    if semantic_count > MAX_BUNDLE_EVENTS_PER_STREAM
        || geometry_count > MAX_BUNDLE_EVENTS_PER_STREAM
    {
        return Err("detached evidence event count exceeds limit".into());
    }
    let frame_count = semantic_count
        .checked_add(geometry_count)
        .ok_or("detached evidence frame count overflow")?;
    let minimum_bytes = frame_count
        .checked_mul(4)
        .and_then(|length| length.checked_add(20))
        .ok_or("detached evidence frame length overflow")?;
    if minimum_bytes > bytes.len() {
        return Err("truncated detached evidence frames".into());
    }
    let mut preflight_offset = 20usize;
    for _ in 0..frame_count {
        let length_end = preflight_offset
            .checked_add(4)
            .ok_or("detached evidence length overflow")?;
        let length = usize::try_from(u32::from_le_bytes(
            bytes
                .get(preflight_offset..length_end)
                .ok_or("truncated detached evidence")?
                .try_into()
                .map_err(|_| "truncated detached evidence")?,
        ))
        .map_err(|_| "invalid event length")?;
        if length > MAX_BUNDLE_EVENT_BYTES {
            return Err("detached evidence event exceeds byte limit".into());
        }
        let event_end = length_end
            .checked_add(length)
            .ok_or("detached evidence length overflow")?;
        validate_json_shape(
            bytes
                .get(length_end..event_end)
                .ok_or("truncated detached evidence")?,
        )?;
        preflight_offset = event_end;
    }
    if preflight_offset != bytes.len() {
        return Err("trailing detached evidence data".into());
    }
    let mut offset = 20usize;
    let mut decode_stream = |count: usize| -> Result<Vec<NormalizedEvent>, String> {
        let mut events = Vec::new();
        for sequence in 0..count {
            let length_end = offset
                .checked_add(4)
                .ok_or("detached evidence length overflow")?;
            let length = usize::try_from(u32::from_le_bytes(
                bytes
                    .get(offset..length_end)
                    .ok_or("truncated detached evidence")?
                    .try_into()
                    .map_err(|_| "truncated detached evidence")?,
            ))
            .map_err(|_| "invalid event length")?;
            offset = length_end;
            let event_end = offset
                .checked_add(length)
                .ok_or("detached evidence length overflow")?;
            let encoded = bytes
                .get(offset..event_end)
                .ok_or("truncated detached evidence")?;
            let event: NormalizedEvent =
                serde_json::from_slice(encoded).map_err(|error| error.to_string())?;
            if event.sequence != sequence as u64
                || serde_json::to_vec(&event).map_err(|error| error.to_string())? != encoded
            {
                return Err("noncanonical detached evidence sequence or encoding".into());
            }
            events.push(event);
            offset = event_end;
        }
        Ok(events)
    };
    let bundle = OracleBundle {
        semantic: decode_stream(semantic_count)?,
        geometry: decode_stream(geometry_count)?,
    };
    if offset != bytes.len() {
        return Err("trailing detached evidence data".into());
    }
    validate_bundle(&bundle)?;
    Ok(bundle)
}

fn validate_json_shape(bytes: &[u8]) -> Result<(), String> {
    let mut depth = 0usize;
    let mut string_start = None;
    let mut escaped = false;
    for (index, byte) in bytes.iter().copied().enumerate() {
        if let Some(start) = string_start {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                if index - start > MAX_BUNDLE_STRING_BYTES {
                    return Err("detached evidence string exceeds byte limit".into());
                }
                string_start = None;
            }
            continue;
        }
        match byte {
            b'"' => string_start = Some(index + 1),
            b'{' | b'[' => {
                depth = depth
                    .checked_add(1)
                    .ok_or("detached evidence nesting overflow")?;
                if depth > MAX_BUNDLE_NESTING_DEPTH {
                    return Err("detached evidence nesting exceeds depth limit".into());
                }
            }
            b'}' | b']' => {
                depth = depth
                    .checked_sub(1)
                    .ok_or("malformed detached evidence nesting")?;
            }
            _ => {}
        }
    }
    if string_start.is_some() || depth != 0 {
        return Err("malformed detached evidence JSON shape".into());
    }
    Ok(())
}

fn validate_bundle(bundle: &OracleBundle) -> Result<(), String> {
    if bundle.semantic.len() > MAX_BUNDLE_EVENTS_PER_STREAM
        || bundle.geometry.len() > MAX_BUNDLE_EVENTS_PER_STREAM
    {
        return Err("detached evidence event count exceeds limit".into());
    }
    for (index, event) in bundle.semantic.iter().enumerate() {
        if event.sequence != index as u64 || matches!(event.semantic, Event::Geometry(_)) {
            return Err("invalid semantic evidence stream".into());
        }
    }
    for (index, event) in bundle.geometry.iter().enumerate() {
        if event.sequence != index as u64 || !matches!(event.semantic, Event::Geometry(_)) {
            return Err("invalid geometry evidence stream".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
