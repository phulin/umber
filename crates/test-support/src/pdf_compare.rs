//! Bounded comparison of two independently produced PDF files.
//!
//! This is a strict semantic projection, not a rendering oracle. It reuses the
//! established Hayro graph/content projection and additionally observes page
//! crop boxes and rotation. Complete decoded page streams are separately
//! attested in the receipt: lexical whitespace and comments can vary even when
//! the parsed content operations are the same. Hayro's content iterator is
//! lenient, so projection equality does not claim full syntax or visual parity.

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

use crate::pdf::normalize_structure;
use crate::pdf_query::{PdfQuery, QueryLimits};

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
    if bytes.len() > MAX_PDF_BYTES {
        bail!("PDF exceeds {MAX_PDF_BYTES} byte input limit");
    }
    check_framing(bytes)?;
    let limits = QueryLimits::default();
    let query = PdfQuery::new(bytes, limits).context("Hayro could not parse PDF")?;
    let pages = query.pages().context("could not project PDF pages")?;
    let mut text = normalize_structure(bytes).context("could not normalize PDF structure")?;
    if text.len() > MAX_PROJECTION_BYTES {
        bail!("PDF projection exceeds {MAX_PROJECTION_BYTES} byte limit");
    }
    let mut content_digest = Sha256::new();
    for page in &pages {
        text.push_str(&format!("page-extra {} crop-box", page.number));
        for value in page.crop_box {
            if !value.is_finite() {
                bail!("page {} has nonfinite crop box", page.number);
            }
            text.push_str(&format!(" {value:.6}"));
        }
        text.push_str(&format!(" rotation {}\n", page.rotation_degrees));
        match &page.content {
            Some(content) => {
                content_digest.update([1]);
                content_digest.update((content.decoded.len() as u64).to_be_bytes());
                content_digest.update(&content.decoded);
            }
            None => content_digest.update([0]),
        }
        if text.len() > MAX_PROJECTION_BYTES {
            bail!("PDF projection exceeds {MAX_PROJECTION_BYTES} byte limit");
        }
    }
    let sha256 = hex(&Sha256::digest(text.as_bytes()));
    let decoded_content_sha256 = hex(&content_digest.finalize());
    Ok(PdfProjection {
        pages: pages.len(),
        text,
        sha256,
        decoded_content_sha256,
    })
}

/// Check mandatory file framing before passing bytes to Hayro's recovery parser.
/// This is a sanity check, not a substitute for a strict PDF validator.
fn check_framing(bytes: &[u8]) -> Result<()> {
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
