//! Canonical structure projection for hermetic PDF parity fixtures.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail};
use sha2::Digest;

use crate::pdf_probe::{
    PdfProbe, ProbeDictionary, ProbeLimits, ProbeObjectId, ProbeOperation, ProbeStream, ProbeValue,
};

/// Parses a PDF and projects catalog, ordered pages, resources, media boxes,
/// and decoded content operations without preserving object numbers or byte
/// layout.
pub fn normalize_structure(bytes: &[u8]) -> Result<String> {
    let probe = PdfProbe::new(bytes, ProbeLimits::default()).context("failed to parse PDF")?;
    let catalog = probe.root().context("PDF has no catalog")?;
    require_name(
        catalog.get(b"Type").context("catalog has no Type")?,
        b"Catalog",
    )?;
    let pages = probe.pages()?;
    let pages_by_id = pages
        .iter()
        .map(|page| (page.id, page.number))
        .collect::<BTreeMap<_, _>>();
    let (major, minor) = probe.version();
    let mut normalized = format!(
        "pdf-structure-v1\nversion {major}.{minor}\ncatalog /Catalog\npages {}\n",
        pages.len()
    );

    for page in &pages {
        require_name(
            page.dictionary.get(b"Type").context("page has no Type")?,
            b"Page",
        )?;
        normalized.push_str(&format!(
            "page {}\nmedia-box {}\n",
            page.number,
            page.media_box
                .iter()
                .map(|value| canonical_number(*value))
                .collect::<Result<Vec<_>>>()?
                .join(" ")
        ));
        normalized.push_str("resources ");
        let resources = page
            .dictionary
            .get(b"Resources")
            .context("page has no Resources")?;
        normalized.push_str(&canonical_value(
            resources,
            &pages_by_id,
            0,
            &mut Vec::new(),
        )?);
        normalized.push('\n');
        if let Some(beads) = page.dictionary.get(b"B") {
            normalized.push_str("beads ");
            normalized.push_str(&canonical_value(beads, &pages_by_id, 0, &mut Vec::new())?);
            normalized.push('\n');
        }
        if let Some(content) = &page.content {
            let omit_noop_wrapper = content.operations.len() == 2
                && content.operations[0].operator == b"q"
                && content.operations[0].operands.is_empty()
                && content.operations[1].operator == b"Q"
                && content.operations[1].operands.is_empty();
            if !omit_noop_wrapper {
                append_operations(
                    &mut normalized,
                    &content.operations,
                    &pages_by_id,
                    "content",
                )?;
            }
        }
    }
    append_document_extensions(&probe, &catalog, &pages_by_id, &mut normalized)?;
    Ok(normalized)
}

fn append_document_extensions(
    probe: &PdfProbe,
    catalog: &ProbeDictionary,
    pages: &BTreeMap<ProbeObjectId, usize>,
    normalized: &mut String,
) -> Result<()> {
    let mut extensions = Vec::new();
    let catalog_entries =
        selected_dictionary(catalog, &[b"PageMode", b"ViewerPreferences"], pages)?;
    if !catalog_entries.is_empty() {
        extensions.push(format!("catalog-extensions {catalog_entries}"));
    }
    if let Some(action) = catalog.get(b"OpenAction") {
        extensions.push(format!("open-action {}", canonical_action(action, pages)?));
    }
    for (key, label) in [
        (b"Names".as_slice(), "names"),
        (b"Outlines", "outlines"),
        (b"Threads", "threads"),
    ] {
        if let Some(value) = catalog.get(key) {
            extensions.push(format!(
                "{label} {}",
                canonical_value(value, pages, 0, &mut Vec::new())?
            ));
        }
    }

    let trailer = probe.trailer()?.context("PDF has no trailer")?;
    if let Some(info) = trailer.get(b"Info").and_then(ProbeValue::as_dictionary) {
        let selected = selected_dictionary(info, &[b"Title", b"Subject"], pages)?;
        if !selected.is_empty() {
            extensions.push(format!("info {selected}"));
        }
    }
    let selected = selected_dictionary(&trailer, &[b"Custom"], pages)?;
    if !selected.is_empty() {
        extensions.push(format!("trailer {selected}"));
    }

    let size = trailer.get(b"Size").map(number).transpose()?.unwrap_or(0.0) as i32;
    let mut user_objects = BTreeSet::new();
    for number in 1..size {
        let Ok(value) = probe.object(ProbeObjectId::new(number, 0)) else {
            continue;
        };
        match value.resolved() {
            ProbeValue::Dictionary(dictionary) if dictionary.get(b"Kind").is_some() => {
                user_objects.insert(format!(
                    "object {}",
                    canonical_dictionary(dictionary, pages, &[])?
                ));
            }
            ProbeValue::Stream(stream)
                if stream.dictionary.get(b"Subtype").is_some()
                    && !is_form_xobject(&stream.dictionary) =>
            {
                user_objects.insert(format!(
                    "stream {} data <{}>",
                    canonical_dictionary(&stream.dictionary, pages, &[b"Length"])?,
                    hex(&stream.decoded)
                ));
            }
            _ => {}
        }
    }
    extensions.extend(user_objects);
    if !extensions.is_empty() {
        normalized.push_str("document-extensions\n");
        for extension in extensions {
            normalized.push_str(&extension);
            normalized.push('\n');
        }
    }
    Ok(())
}

fn selected_dictionary(
    dictionary: &ProbeDictionary,
    keys: &[&[u8]],
    pages: &BTreeMap<ProbeObjectId, usize>,
) -> Result<String> {
    let entries = keys
        .iter()
        .filter_map(|key| dictionary.get(*key).map(|value| (*key, value)))
        .map(|(key, value)| {
            Ok(format!(
                "/{} {}",
                String::from_utf8_lossy(key),
                canonical_value(value, pages, 0, &mut Vec::new())?
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(if entries.is_empty() {
        String::new()
    } else {
        format!("<<{}>>", entries.join(" "))
    })
}

fn canonical_action(value: &ProbeValue, pages: &BTreeMap<ProbeObjectId, usize>) -> Result<String> {
    let dictionary = value
        .as_dictionary()
        .context("OpenAction is not a dictionary")?;
    let entries = dictionary
        .entries
        .iter()
        .map(|(key, value)| {
            let value = if key == b"D" {
                canonical_action_destination(value, pages)?
            } else {
                canonical_value(value, pages, 0, &mut Vec::new())?
            };
            Ok(format!("/{} {value}", String::from_utf8_lossy(key)))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(format!("<<{}>>", entries.join(" ")))
}

fn canonical_action_destination(
    value: &ProbeValue,
    pages: &BTreeMap<ProbeObjectId, usize>,
) -> Result<String> {
    let Some(values) = value.as_array() else {
        return canonical_value(value, pages, 0, &mut Vec::new());
    };
    Ok(format!(
        "[{}]",
        values
            .iter()
            .map(|value| {
                if let Some(page) = value.referenced_id().and_then(|id| pages.get(&id)) {
                    Ok(format!("page {page}"))
                } else {
                    canonical_value(value, pages, 0, &mut Vec::new())
                }
            })
            .collect::<Result<Vec<_>>>()?
            .join(" ")
    ))
}

fn canonical_value(
    value: &ProbeValue,
    pages: &BTreeMap<ProbeObjectId, usize>,
    depth: usize,
    references: &mut Vec<ProbeObjectId>,
) -> Result<String> {
    if depth > 32 {
        bail!("PDF fixture object nesting exceeds 32 levels");
    }
    if let Some(id) = value.referenced_id()
        && let Some(page) = pages.get(&id)
    {
        return Ok(format!("page {page}"));
    }
    Ok(match value {
        ProbeValue::Reference { id, target } => {
            if let Some(index) = references.iter().position(|existing| existing == id) {
                return Ok(format!("@{index}"));
            }
            references.push(*id);
            let result = canonical_value(target, pages, depth + 1, references);
            references.pop();
            result?
        }
        ProbeValue::BackReference(id) => format!(
            "@{}",
            references
                .iter()
                .position(|existing| existing == id)
                .unwrap_or(0)
        ),
        ProbeValue::UnresolvedReference(id) => format!("{} {} R", id.number, id.generation),
        ProbeValue::Null => "null".into(),
        ProbeValue::Boolean(value) => value.to_string(),
        ProbeValue::Number(value) => canonical_number(*value)?,
        ProbeValue::String(bytes) => format!("<{}>", hex(bytes)),
        ProbeValue::Name(name) => format!("/{}", String::from_utf8_lossy(name)),
        ProbeValue::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(|value| canonical_value(value, pages, depth + 1, references))
                .collect::<Result<Vec<_>>>()?
                .join(" ")
        ),
        ProbeValue::Dictionary(dictionary) => {
            canonical_dictionary_inner(dictionary, pages, &[], depth + 1, references)?
        }
        ProbeValue::Stream(stream) if is_form_xobject(&stream.dictionary) => {
            canonical_form_stream(stream, pages, depth + 1, references)?
        }
        ProbeValue::Stream(stream) => format!(
            "stream {} bytes {} sha256 {}",
            canonical_dictionary(&stream.dictionary, pages, &[])?,
            stream.raw.len(),
            hex(&sha2::Sha256::digest(&stream.raw))
        ),
    })
}

fn canonical_dictionary(
    dictionary: &ProbeDictionary,
    pages: &BTreeMap<ProbeObjectId, usize>,
    omitted: &[&[u8]],
) -> Result<String> {
    canonical_dictionary_inner(dictionary, pages, omitted, 0, &mut Vec::new())
}

fn canonical_dictionary_inner(
    dictionary: &ProbeDictionary,
    pages: &BTreeMap<ProbeObjectId, usize>,
    omitted: &[&[u8]],
    depth: usize,
    references: &mut Vec<ProbeObjectId>,
) -> Result<String> {
    let entries = dictionary
        .entries
        .iter()
        .filter(|(key, _)| !omitted.contains(&key.as_slice()))
        .map(|(key, value)| {
            Ok(format!(
                "/{} {}",
                String::from_utf8_lossy(key),
                canonical_value(value, pages, depth + 1, references)?
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(format!("<<{}>>", entries.join(" ")))
}

fn is_form_xobject(dictionary: &ProbeDictionary) -> bool {
    dictionary
        .get(b"Subtype")
        .is_some_and(|value| matches!(value.resolved(), ProbeValue::Name(name) if name == b"Form"))
}

fn canonical_form_stream(
    stream: &ProbeStream,
    pages: &BTreeMap<ProbeObjectId, usize>,
    depth: usize,
    references: &mut Vec<ProbeObjectId>,
) -> Result<String> {
    let dictionary = canonical_dictionary_inner(
        &stream.dictionary,
        pages,
        &[
            b"Length",
            b"PTEX.FileName",
            b"PTEX.InfoDict",
            b"PTEX.PageNumber",
        ],
        depth + 1,
        references,
    )?;
    let mut normalized = format!("form-stream {dictionary}");
    for operation in &stream.operations {
        normalized.push_str(" content");
        for operand in &operation.operands {
            normalized.push(' ');
            normalized.push_str(&canonical_value(operand, pages, depth + 1, references)?);
        }
        normalized.push(' ');
        normalized.push_str(&String::from_utf8_lossy(&operation.operator));
    }
    if depth > 32 {
        bail!("PDF fixture object nesting exceeds 32 levels");
    }
    Ok(normalized)
}

fn append_operations(
    output: &mut String,
    operations: &[ProbeOperation],
    pages: &BTreeMap<ProbeObjectId, usize>,
    prefix: &str,
) -> Result<()> {
    for operation in operations {
        output.push_str(prefix);
        for operand in &operation.operands {
            output.push(' ');
            output.push_str(&canonical_value(operand, pages, 0, &mut Vec::new())?);
        }
        output.push(' ');
        output.push_str(&String::from_utf8_lossy(&operation.operator));
        output.push('\n');
    }
    Ok(())
}

fn require_name(value: &ProbeValue, expected: &[u8]) -> Result<()> {
    match value.resolved() {
        ProbeValue::Name(actual) if actual == expected => Ok(()),
        ProbeValue::Name(actual) => bail!(
            "expected name /{}, found /{}",
            String::from_utf8_lossy(expected),
            String::from_utf8_lossy(actual)
        ),
        _ => bail!("object is not a name"),
    }
}

fn number(value: &ProbeValue) -> Result<f64> {
    match value.resolved() {
        ProbeValue::Number(value) => Ok(*value),
        _ => bail!("expected PDF number"),
    }
}

fn canonical_number(value: f64) -> Result<String> {
    if !value.is_finite() {
        bail!("PDF number is not finite");
    }
    let milli = (value * 1_000.0).round() as i64;
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
    Ok(value)
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
