//! Canonical structure projection for hermetic minimal-PDF parity fixtures.

use anyhow::{Context, Result, bail};
use lopdf::{Document, Object};

/// Parses a PDF and projects catalog, ordered pages, resources, media boxes,
/// and decoded content operations without preserving object numbers or byte
/// layout.
pub fn normalize_structure(bytes: &[u8]) -> Result<String> {
    let document = Document::load_mem(bytes).context("failed to parse PDF")?;
    let catalog = document.catalog().context("PDF has no catalog")?;
    require_name(
        catalog.get(b"Type").context("catalog has no Type")?,
        b"Catalog",
    )?;
    let pages = document.get_pages();
    let mut normalized = format!(
        "pdf-structure-v1\nversion {}\ncatalog /Catalog\npages {}\n",
        document.version,
        pages.len()
    );

    for (number, page_id) in pages {
        let page = document
            .get_dictionary(page_id)
            .with_context(|| format!("page {number} is not a dictionary"))?;
        require_name(page.get(b"Type").context("page has no Type")?, b"Page")?;
        normalized.push_str(&format!("page {number}\n"));
        normalized.push_str("media-box ");
        let media_box = page
            .get_deref(b"MediaBox", &document)
            .context("page has no MediaBox")?
            .as_array()
            .context("MediaBox is not an array")?;
        if media_box.len() != 4 {
            bail!("MediaBox must contain four numbers");
        }
        normalized.push_str(
            &media_box
                .iter()
                .map(canonical_number)
                .collect::<Result<Vec<_>>>()?
                .join(" "),
        );
        normalized.push('\n');

        normalized.push_str("resources ");
        let resources = page
            .get_deref(b"Resources", &document)
            .context("page has no Resources")?;
        normalized.push_str(&canonical_object(&document, resources, 0)?);
        normalized.push('\n');

        let content = document
            .get_and_decode_page_content(page_id)
            .with_context(|| format!("failed to decode page {number} content"))?;
        for operation in content.operations {
            normalized.push_str("content");
            for operand in operation.operands {
                normalized.push(' ');
                normalized.push_str(&canonical_object(&document, &operand, 0)?);
            }
            normalized.push(' ');
            normalized.push_str(&operation.operator);
            normalized.push('\n');
        }
    }
    Ok(normalized)
}

fn require_name(object: &Object, expected: &[u8]) -> Result<()> {
    let actual = object.as_name().context("object is not a name")?;
    if actual != expected {
        bail!(
            "expected name /{}, found /{}",
            String::from_utf8_lossy(expected),
            String::from_utf8_lossy(actual)
        );
    }
    Ok(())
}

fn canonical_object(document: &Document, object: &Object, depth: usize) -> Result<String> {
    if depth > 32 {
        bail!("PDF fixture object nesting exceeds 32 levels");
    }
    let (_, object) = document
        .dereference(object)
        .context("failed to resolve PDF object reference")?;
    Ok(match object {
        Object::Null => "null".to_owned(),
        Object::Boolean(value) => value.to_string(),
        Object::Integer(_) | Object::Real(_) => canonical_number(object)?,
        Object::Name(name) => format!("/{}", String::from_utf8_lossy(name)),
        Object::String(bytes, _) => format!("<{}>", hex(bytes)),
        Object::Array(values) => {
            let values = values
                .iter()
                .map(|value| canonical_object(document, value, depth + 1))
                .collect::<Result<Vec<_>>>()?;
            format!("[{}]", values.join(" "))
        }
        Object::Dictionary(dictionary) => {
            let mut entries = dictionary.iter().collect::<Vec<_>>();
            entries.sort_unstable_by(|(left, _), (right, _)| left.cmp(right));
            let entries = entries
                .into_iter()
                .map(|(key, value)| {
                    Ok(format!(
                        "/{} {}",
                        String::from_utf8_lossy(key),
                        canonical_object(document, value, depth + 1)?
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            format!("<<{}>>", entries.join(" "))
        }
        Object::Stream(_) => bail!("resource structure contains an unexpected stream"),
        Object::Reference(_) => unreachable!("references were dereferenced"),
    })
}

fn canonical_number(object: &Object) -> Result<String> {
    let value = match object {
        Object::Integer(value) => *value as f64,
        Object::Real(value) => f64::from(*value),
        _ => bail!("expected PDF number"),
    };
    if !value.is_finite() {
        bail!("PDF number is not finite");
    }
    let milli = (value * 1_000.0).round() as i64;
    Ok(format_milli(milli))
}

fn format_milli(milli: i64) -> String {
    let negative = milli < 0;
    let absolute = milli.unsigned_abs();
    let whole = absolute / 1_000;
    let fraction = absolute % 1_000;
    let mut value = if fraction == 0 {
        whole.to_string()
    } else {
        let mut fraction = format!("{fraction:03}");
        while fraction.ends_with('0') {
            fraction.pop();
        }
        format!("{whole}.{fraction}")
    };
    if negative {
        value.insert(0, '-');
    }
    value
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    output
}
