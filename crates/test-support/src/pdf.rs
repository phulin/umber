//! Canonical structure projection for hermetic PDF parity fixtures.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail};
use hayro_syntax::object::Object;
use sha2::Digest;

use crate::pdf_probe::{
    PdfProbe, ProbeDictionary, ProbeLimits, ProbeObjectId, ProbeOperand, ProbeOperation,
    ProbeStream, ProbeValue,
};

/// Parse a PDF and project its stable semantic structure directly from Hayro's
/// borrowed objects. Object numbers and byte layout are intentionally omitted.
pub fn normalize_structure(bytes: &[u8]) -> Result<String> {
    normalize_structure_with_limits(bytes, ProbeLimits::default())
}

pub(crate) fn normalize_structure_with_limits(bytes: &[u8], limits: ProbeLimits) -> Result<String> {
    let probe = PdfProbe::new(bytes, limits).context("failed to parse PDF")?;
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
        normalized.push_str(&canonical_effective_resources(
            &page.dictionary,
            &pages_by_id,
        )?);
        normalized.push('\n');
        if let Some(beads) = page.dictionary.get(b"B") {
            normalized.push_str("beads ");
            normalized.push_str(&canonical_value(&beads, &pages_by_id, 0, &mut Vec::new())?);
            normalized.push('\n');
        }
        if let Some(content) = &page.content {
            let omit_noop_wrapper = content.operations.len() == 2
                && content.operations[0].operator == b"q"
                && content.operations[0].operands.is_empty()
                && content.operations[1].operator == b"Q"
                && content.operations[1].operands.is_empty();
            if !omit_noop_wrapper {
                append_operations(&mut normalized, &content.operations, "content")?;
            }
        }
    }
    append_document_extensions(&probe, &catalog, &pages_by_id, &mut normalized)?;
    Ok(normalized)
}

fn canonical_effective_resources(
    page: &ProbeDictionary<'_>,
    pages: &BTreeMap<ProbeObjectId, usize>,
) -> Result<String> {
    let mut layers = Vec::new();
    collect_page_resource_layers(page.clone(), 0, &mut layers)?;
    let mut effective: BTreeMap<Vec<u8>, BorrowedDictionaryValue<'_>> = BTreeMap::new();
    for layer in layers {
        for (key, value) in layer.entries() {
            if let Some(child) = value.as_dictionary() {
                let mut entries = match effective.get(&key) {
                    Some(BorrowedDictionaryValue::Value(parent)) => parent
                        .as_dictionary()
                        .map(|dictionary| dictionary.entries().collect())
                        .unwrap_or_default(),
                    Some(BorrowedDictionaryValue::Merged(parent)) => parent.clone(),
                    None => BTreeMap::new(),
                };
                entries.extend(child.entries());
                effective.insert(key, BorrowedDictionaryValue::Merged(entries));
            } else {
                effective.insert(key, BorrowedDictionaryValue::Value(value));
            }
        }
    }
    if effective.is_empty() {
        bail!("page has no Resources");
    }
    canonical_dictionary_entries(&effective, pages, &[], 0, &mut Vec::new())
}

#[derive(Clone)]
enum BorrowedDictionaryValue<'a> {
    Value(ProbeValue<'a>),
    Merged(BTreeMap<Vec<u8>, ProbeValue<'a>>),
}

fn collect_page_resource_layers<'a>(
    dictionary: ProbeDictionary<'a>,
    depth: usize,
    layers: &mut Vec<ProbeDictionary<'a>>,
) -> Result<()> {
    if depth > 32 {
        bail!("PDF page resource inheritance exceeds 32 levels");
    }
    if let Some(parent) = dictionary
        .get(b"Parent")
        .and_then(|value| value.as_dictionary())
    {
        collect_page_resource_layers(parent, depth + 1, layers)?;
    }
    if let Some(resources) = dictionary
        .get(b"Resources")
        .and_then(|value| value.as_dictionary())
    {
        layers.push(resources);
    }
    Ok(())
}

fn append_document_extensions(
    probe: &PdfProbe,
    catalog: &ProbeDictionary<'_>,
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
        extensions.push(format!("open-action {}", canonical_action(&action, pages)?));
    }
    for (key, label) in [
        (b"Names".as_slice(), "names"),
        (b"Outlines", "outlines"),
        (b"Threads", "threads"),
    ] {
        if let Some(value) = catalog.get(key) {
            extensions.push(format!(
                "{label} {}",
                canonical_value(&value, pages, 0, &mut Vec::new())?
            ));
        }
    }

    let trailer = probe.trailer()?.context("PDF has no trailer")?;
    if let Some(info) = trailer.get(b"Info").and_then(|value| value.as_dictionary()) {
        let selected = selected_dictionary(&info, &[b"Title", b"Subject"], pages)?;
        if !selected.is_empty() {
            extensions.push(format!("info {selected}"));
        }
    }
    let selected = selected_dictionary(&trailer, &[b"Custom"], pages)?;
    if !selected.is_empty() {
        extensions.push(format!("trailer {selected}"));
    }

    let size = trailer
        .get(b"Size")
        .map(|value| number(&value))
        .transpose()?
        .unwrap_or(0.0) as i32;
    let mut user_objects = BTreeSet::new();
    for number in 1..size {
        let Ok(value) = probe.object(ProbeObjectId::new(number, 0)) else {
            continue;
        };
        if let Some(dictionary) = value.as_dictionary()
            && dictionary.get(b"Kind").is_some()
        {
            let id = ProbeObjectId::new(number, 0);
            user_objects.insert(format!(
                "object {}",
                canonical_dictionary_inner(&dictionary, pages, &[], 0, &mut vec![id])?
            ));
        } else if let Some(stream) = value.as_stream()
            && stream.dictionary.get(b"Subtype").is_some()
            && !is_form_xobject(&stream.dictionary)
        {
            user_objects.insert(format!(
                "stream {} data <{}>",
                canonical_dictionary(&stream.dictionary, pages, &[b"Length"])?,
                hex(&stream.decoded)
            ));
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
    dictionary: &ProbeDictionary<'_>,
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
                canonical_value(&value, pages, 0, &mut Vec::new())?
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(if entries.is_empty() {
        String::new()
    } else {
        format!("<<{}>>", entries.join(" "))
    })
}

fn canonical_action(
    value: &ProbeValue<'_>,
    pages: &BTreeMap<ProbeObjectId, usize>,
) -> Result<String> {
    let dictionary = value
        .as_dictionary()
        .context("OpenAction is not a dictionary")?;
    let entries = dictionary
        .entries()
        .map(|(key, value)| {
            let value = if key == b"D" {
                canonical_action_destination(&value, pages)?
            } else {
                canonical_value(&value, pages, 0, &mut Vec::new())?
            };
            Ok(format!("/{} {value}", String::from_utf8_lossy(&key)))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(format!("<<{}>>", entries.join(" ")))
}

fn canonical_action_destination(
    value: &ProbeValue<'_>,
    pages: &BTreeMap<ProbeObjectId, usize>,
) -> Result<String> {
    let Some(values) = value.array() else {
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
                    canonical_value(&value, pages, 0, &mut Vec::new())
                }
            })
            .collect::<Result<Vec<_>>>()?
            .join(" ")
    ))
}

fn canonical_value(
    value: &ProbeValue<'_>,
    pages: &BTreeMap<ProbeObjectId, usize>,
    depth: usize,
    references: &mut Vec<ProbeObjectId>,
) -> Result<String> {
    if depth > 32 {
        bail!("PDF fixture object nesting exceeds 32 levels");
    }
    if let Some(id) = value.referenced_id() {
        if let Some(page) = pages.get(&id) {
            return Ok(format!("page {page}"));
        }
        if value.is_unresolved() {
            return Ok(format!("{} {} R", id.number, id.generation));
        }
        if let Some(index) = references.iter().position(|existing| *existing == id) {
            return Ok(format!("@{index}"));
        }
        references.push(id);
        let result = canonical_resolved_value(value, pages, depth + 1, references);
        references.pop();
        return result;
    }
    canonical_resolved_value(value, pages, depth, references)
}

fn canonical_resolved_value(
    value: &ProbeValue<'_>,
    pages: &BTreeMap<ProbeObjectId, usize>,
    depth: usize,
    references: &mut Vec<ProbeObjectId>,
) -> Result<String> {
    Ok(match value.object().context("unresolved PDF object")? {
        Object::Null(_) => "null".into(),
        Object::Boolean(value) => value.to_string(),
        Object::Number(value) => canonical_number(value.as_f64())?,
        Object::String(value) => format!("<{}>", hex(value.as_bytes())),
        Object::Name(name) => format!("/{}", String::from_utf8_lossy(name.as_ref())),
        Object::Array(_) => {
            let values = value.array().expect("matched PDF array");
            format!(
                "[{}]",
                values
                    .iter()
                    .map(|value| canonical_value(&value, pages, depth + 1, references))
                    .collect::<Result<Vec<_>>>()?
                    .join(" ")
            )
        }
        Object::Dict(_) => canonical_dictionary_inner(
            &value.as_dictionary().expect("matched PDF dictionary"),
            pages,
            &[],
            depth + 1,
            references,
        )?,
        Object::Stream(_) => {
            let stream = value.as_stream().expect("matched PDF stream");
            if is_form_xobject(&stream.dictionary) {
                canonical_form_stream(&stream, pages, depth + 1, references)?
            } else {
                format!(
                    "stream {} bytes {} sha256 {}",
                    canonical_dictionary(&stream.dictionary, pages, &[])?,
                    stream.raw.len(),
                    hex(&sha2::Sha256::digest(&stream.raw))
                )
            }
        }
    })
}

fn canonical_dictionary(
    dictionary: &ProbeDictionary<'_>,
    pages: &BTreeMap<ProbeObjectId, usize>,
    omitted: &[&[u8]],
) -> Result<String> {
    canonical_dictionary_inner(dictionary, pages, omitted, 0, &mut Vec::new())
}

fn canonical_dictionary_inner(
    dictionary: &ProbeDictionary<'_>,
    pages: &BTreeMap<ProbeObjectId, usize>,
    omitted: &[&[u8]],
    depth: usize,
    references: &mut Vec<ProbeObjectId>,
) -> Result<String> {
    let entries = dictionary
        .entries()
        .filter(|(key, _)| !omitted.contains(&key.as_slice()))
        .map(|(key, value)| {
            Ok(format!(
                "/{} {}",
                String::from_utf8_lossy(&key),
                canonical_value(&value, pages, depth + 1, references)?
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(format!("<<{}>>", entries.join(" ")))
}

fn canonical_dictionary_entries(
    entries: &BTreeMap<Vec<u8>, BorrowedDictionaryValue<'_>>,
    pages: &BTreeMap<ProbeObjectId, usize>,
    omitted: &[&[u8]],
    depth: usize,
    references: &mut Vec<ProbeObjectId>,
) -> Result<String> {
    let entries = entries
        .iter()
        .filter(|(key, _)| !omitted.contains(&key.as_slice()))
        .map(|(key, value)| {
            let value = match value {
                BorrowedDictionaryValue::Value(value) => {
                    canonical_value(value, pages, depth + 1, references)?
                }
                BorrowedDictionaryValue::Merged(entries) => canonical_dictionary_entries(
                    &entries
                        .iter()
                        .map(|(key, value)| {
                            (key.clone(), BorrowedDictionaryValue::Value(value.clone()))
                        })
                        .collect(),
                    pages,
                    &[],
                    depth + 1,
                    references,
                )?,
            };
            Ok(format!("/{} {value}", String::from_utf8_lossy(key)))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(format!("<<{}>>", entries.join(" ")))
}

fn is_form_xobject(dictionary: &ProbeDictionary<'_>) -> bool {
    dictionary
        .get(b"Subtype")
        .and_then(|value| value.name())
        .is_some_and(|name| name.as_ref() == b"Form")
}

fn canonical_form_stream(
    stream: &ProbeStream<'_>,
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
    for operation in stream.operations(ProbeLimits::default())? {
        normalized.push_str(" content");
        for operand in &operation.operands {
            normalized.push(' ');
            normalized.push_str(&canonical_operand(operand)?);
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
    prefix: &str,
) -> Result<()> {
    for operation in operations {
        output.push_str(prefix);
        for operand in &operation.operands {
            output.push(' ');
            output.push_str(&canonical_operand(operand)?);
        }
        output.push(' ');
        output.push_str(&String::from_utf8_lossy(&operation.operator));
        output.push('\n');
    }
    Ok(())
}

fn canonical_operand(value: &ProbeOperand) -> Result<String> {
    Ok(match value {
        ProbeOperand::Null => "null".into(),
        ProbeOperand::Boolean(value) => value.to_string(),
        ProbeOperand::Number(value) => canonical_number(*value)?,
        ProbeOperand::String(bytes) => format!("<{}>", hex(bytes)),
        ProbeOperand::Name(name) => format!("/{}", String::from_utf8_lossy(name)),
        ProbeOperand::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(canonical_operand)
                .collect::<Result<Vec<_>>>()?
                .join(" ")
        ),
        ProbeOperand::Dictionary(entries) => format!(
            "<<{}>>",
            entries
                .iter()
                .map(|(key, value)| Ok(format!(
                    "/{} {}",
                    String::from_utf8_lossy(key),
                    canonical_operand(value)?
                )))
                .collect::<Result<Vec<_>>>()?
                .join(" ")
        ),
    })
}

fn require_name(value: ProbeValue<'_>, expected: &[u8]) -> Result<()> {
    match value.name() {
        Some(actual) if actual.as_ref() == expected => Ok(()),
        Some(actual) => bail!(
            "expected name /{}, found /{}",
            String::from_utf8_lossy(expected),
            String::from_utf8_lossy(actual.as_ref())
        ),
        None => bail!("object is not a name"),
    }
}

fn number(value: &ProbeValue<'_>) -> Result<f64> {
    value.number().context("expected PDF number")
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

#[cfg(test)]
mod tests;
