//! Bounded comparison of two independently produced PDF files.
//!
//! This is a structural/content-operation projection, not a rendering oracle.
//! It walks the Hayro document graph once, preserves resource and inline-image
//! evidence, and records ordered page operations in bounded chunks. Complete
//! decoded page streams are separately attested in the receipt: lexical
//! whitespace and comments can vary even when operations match. Hayro's
//! content iterator is lenient, so projection equality does not claim full
//! syntax or visual parity.

use anyhow::Result;

mod corpus;

/// Maximum input bytes per side. The page/graph budgets are in `QueryLimits`.
pub const MAX_PDF_BYTES: usize = 64 * 1024 * 1024;
/// Maximum retained projection bytes per side.
pub const MAX_PROJECTION_BYTES: usize = 128 * 1024 * 1024;

#[derive(Debug)]
pub struct PdfProjection {
    pub pages: usize,
    pub text: String,
    pub sha256: String,
    pub decoded_content_sha256: String,
}

pub fn project_pdf(bytes: &[u8]) -> Result<PdfProjection> {
    corpus::project_pdf(bytes)
}

/// Check mandatory file framing before passing bytes to Hayro's recovery parser.
/// This is a sanity check, not a substitute for a strict PDF validator.
pub(super) fn check_framing(bytes: &[u8]) -> Result<()> {
    use anyhow::bail;
    if bytes.len() < 16 || !bytes.starts_with(b"%PDF-") {
        bail!("PDF header is missing");
    }
    let tail = &bytes[bytes.len().saturating_sub(1024)..];
    if !tail.windows(b"%%EOF".len()).any(|part| part == b"%%EOF") {
        bail!("PDF EOF marker is missing from final 1024 bytes");
    }
    if !tail
        .windows(b"startxref".len())
        .any(|part| part == b"startxref")
    {
        bail!("PDF startxref is missing from final 1024 bytes");
    }
    Ok(())
}

pub fn first_difference(reference: &str, umber: &str) -> Option<(usize, String, String)> {
    let mut reference_lines = reference.lines();
    let mut umber_lines = umber.lines();
    for line in 1.. {
        let expected = reference_lines.next();
        let actual = umber_lines.next();
        if expected == actual {
            expected?;
            continue;
        }
        return Some((
            line,
            excerpt(expected.unwrap_or("<end of projection>")),
            excerpt(actual.unwrap_or("<end of projection>")),
        ));
    }
    unreachable!()
}

fn excerpt(line: &str) -> String {
    const MAX_CHARS: usize = 240;
    let mut result: String = line.chars().take(MAX_CHARS).collect();
    if line.chars().count() > MAX_CHARS {
        result.push('…');
    }
    result
}

pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(test)]
mod tests;
