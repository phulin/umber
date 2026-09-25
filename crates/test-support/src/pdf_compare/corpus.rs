//! Bounded corpus projection. Indirect objects are named by deterministic
//! traversal order and emitted once, so shared fonts and cycles do not expand
//! once per page or reference edge.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;

use anyhow::{Context, Result, bail};
use flate2::read::ZlibDecoder;
use hayro_syntax::object::Object;
use sha2::{Digest, Sha256};

use crate::pdf_query::{
    PdfQuery, QueryDictionary, QueryLimits, QueryObjectId, QueryOperand, QueryPage, QueryStream,
    QueryValue,
};

use super::{MAX_PDF_BYTES, MAX_PROJECTION_BYTES, PdfProjection, check_framing, hex};

const MAX_GRAPH_NODES: usize = 100_000;
const MAX_RESOURCE_PARENTS: usize = 256;
const MAX_DIRECT_DEPTH: usize = 128;
const OPERATIONS_PER_CHUNK: usize = 256;

pub(super) fn project_pdf(bytes: &[u8]) -> Result<PdfProjection> {
    if bytes.len() > MAX_PDF_BYTES {
        bail!("PDF exceeds {MAX_PDF_BYTES} byte input limit");
    }
    check_framing(bytes)?;
    let query = PdfQuery::new(
        bytes,
        QueryLimits {
            max_depth: MAX_DIRECT_DEPTH,
            max_objects: MAX_GRAPH_NODES,
            max_values: 8_000_000,
            max_stream_bytes: 256 * 1024 * 1024,
        },
    )
    .context("Hayro could not parse PDF")?;
    let pages = query.pages().context("could not project PDF pages")?;
    if pages.is_empty() {
        bail!("PDF has no pages");
    }
    let page_ids = pages.iter().map(|p| (p.id, p.number)).collect();
    let mut graph = Graph::new(&query, page_ids);
    graph.line("pdf-corpus-structure-v2")?;
    graph.line(format!("pages {}", pages.len()))?;
    graph.catalog()?;
    let mut content_digest = Sha256::new();
    for page in &pages {
        graph.page(page, &mut content_digest)?;
    }
    graph.trailer()?;
    graph.drain_nodes()?;
    let text = graph.text;
    Ok(PdfProjection {
        pages: pages.len(),
        sha256: hex(&Sha256::digest(text.as_bytes())),
        decoded_content_sha256: hex(&content_digest.finalize()),
        text,
    })
}

enum Resource<'a> {
    Value(QueryValue<'a>),
    Merged(BTreeMap<Vec<u8>, QueryValue<'a>>),
}

struct Graph<'a> {
    query: &'a PdfQuery,
    pages: BTreeMap<QueryObjectId, usize>,
    ordinals: BTreeMap<QueryObjectId, usize>,
    nodes: Vec<(QueryObjectId, String)>,
    text: String,
}

impl<'a> Graph<'a> {
    fn new(query: &'a PdfQuery, pages: BTreeMap<QueryObjectId, usize>) -> Self {
        Self {
            query,
            pages,
            ordinals: BTreeMap::new(),
            nodes: Vec::new(),
            text: String::new(),
        }
    }

    fn line(&mut self, line: impl AsRef<str>) -> Result<()> {
        self.text.push_str(line.as_ref());
        self.text.push('\n');
        if self.text.len() > MAX_PROJECTION_BYTES {
            bail!("PDF projection exceeds {MAX_PROJECTION_BYTES} byte limit");
        }
        Ok(())
    }

    fn catalog(&mut self) -> Result<()> {
        let root = self.query.root().context("PDF has no catalog")?;
        let type_name = root.get(b"Type").and_then(|v| v.name());
        if type_name.as_deref() != Some(b"Catalog".as_slice()) {
            bail!("PDF root is not a /Catalog");
        }
        for (key, value) in root.entries() {
            if key == b"Pages" {
                continue;
            }
            let label = format!("catalog /{}", name(&key));
            let encoded = self.value(value, &label, 0)?;
            self.line(format!("{label} {encoded}"))?;
        }
        Ok(())
    }

    fn page(&mut self, page: &QueryPage<'a>, content_digest: &mut Sha256) -> Result<()> {
        self.line(format!(
            "page {} media {} crop {} rotate {}",
            page.number,
            box_text(page.media_box)?,
            box_text(page.crop_box)?,
            page.rotation_degrees
        ))?;
        for (key, value) in page.dictionary.entries() {
            if [
                b"Type".as_slice(),
                b"Parent",
                b"Contents",
                b"Resources",
                b"MediaBox",
                b"CropBox",
                b"Rotate",
            ]
            .contains(&key.as_slice())
            {
                continue;
            }
            let label = format!("page {} /{}", page.number, name(&key));
            let encoded = self.value(value, &label, 0)?;
            self.line(format!("{label} {encoded}"))?;
        }
        for (key, resource) in effective_resources(page.dictionary.clone())? {
            let label = format!("page {} resource /{}", page.number, name(&key));
            let encoded = match resource {
                Resource::Value(value) => self.value(value, &label, 0)?,
                Resource::Merged(entries) => self.merged(entries, &label, 0)?,
            };
            self.line(format!("{label} {encoded}"))?;
        }
        match &page.content {
            Some(content) => {
                content_digest.update([1]);
                content_digest.update((content.decoded.len() as u64).to_be_bytes());
                content_digest.update(&content.decoded);
                self.line(format!(
                    "page {} content operations {}",
                    page.number,
                    content.operations.len()
                ))?;
                for (chunk_index, chunk) in
                    content.operations.chunks(OPERATIONS_PER_CHUNK).enumerate()
                {
                    let mut digest = Sha256::new();
                    for operation in chunk {
                        let mut encoded = String::new();
                        for operand in &operation.operands {
                            encoded.push_str(&self.operand(operand, 0)?);
                            encoded.push(' ');
                        }
                        encoded.push_str(&hex(&operation.operator));
                        digest.update((encoded.len() as u64).to_be_bytes());
                        digest.update(encoded.as_bytes());
                    }
                    self.line(format!(
                        "page {} content ops {}..{} sha256 {}",
                        page.number,
                        chunk_index * OPERATIONS_PER_CHUNK,
                        chunk_index * OPERATIONS_PER_CHUNK + chunk.len() - 1,
                        hex(&digest.finalize())
                    ))?;
                }
            }
            None => {
                content_digest.update([0]);
                self.line(format!("page {} no-content", page.number))?;
            }
        }
        Ok(())
    }

    fn trailer(&mut self) -> Result<()> {
        let trailer = self.query.trailer()?.context("PDF has no trailer")?;
        if let Some(info) = trailer.get(b"Info").and_then(|v| v.as_dictionary()) {
            for key in [b"Title".as_slice(), b"Subject"] {
                if let Some(value) = info.get(key) {
                    let label = format!("info /{}", name(key));
                    let encoded = self.value(value, &label, 0)?;
                    self.line(format!("{label} {encoded}"))?;
                }
            }
        }
        Ok(())
    }

    fn drain_nodes(&mut self) -> Result<()> {
        let mut next = 0;
        while next < self.nodes.len() {
            let (id, label) = self.nodes[next].clone();
            let value = self
                .query
                .object(id)
                .with_context(|| format!("resolve {label}"))?;
            if matches!(value.object(), Some(Object::Dict(_)))
                && let Some(dictionary) = value.as_dictionary()
            {
                self.line(format!("node {next} {label} dictionary"))?;
                for (key, entry) in dictionary.entries() {
                    let field = format!("{label}/{}", name(&key));
                    let encoded = self.value(entry, &field, 0)?;
                    self.line(format!(
                        "node {next} {field} sha256 {} {}",
                        hex(&Sha256::digest(encoded.as_bytes())),
                        preview(&encoded)
                    ))?;
                }
                next += 1;
                continue;
            }
            let encoded = self.value(value, &label, 0)?;
            self.line(format!(
                "node {next} {label} sha256 {} {}",
                hex(&Sha256::digest(encoded.as_bytes())),
                preview(&encoded)
            ))?;
            next += 1;
        }
        Ok(())
    }

    fn value(&mut self, value: QueryValue<'a>, path: &str, depth: usize) -> Result<String> {
        if depth > MAX_DIRECT_DEPTH {
            bail!("PDF direct object nesting exceeds {MAX_DIRECT_DEPTH} at {path}");
        }
        if let Some(id) = value.referenced_id() {
            if let Some(page) = self.pages.get(&id) {
                return Ok(format!("page {page}"));
            }
            if value.is_unresolved() {
                bail!("unresolved PDF reference at {path}");
            }
            let ordinal = match self.ordinals.get(&id) {
                Some(ordinal) => *ordinal,
                None => {
                    if self.nodes.len() >= MAX_GRAPH_NODES {
                        bail!("PDF graph exceeds {MAX_GRAPH_NODES} indirect objects");
                    }
                    let ordinal = self.nodes.len();
                    self.ordinals.insert(id, ordinal);
                    self.nodes.push((id, path.to_owned()));
                    ordinal
                }
            };
            return Ok(format!("@{ordinal}"));
        }
        match value.object().context("missing PDF value")? {
            Object::Null(_) => Ok("null".to_owned()),
            Object::Boolean(v) => Ok(v.to_string()),
            Object::Number(v) => number(v.as_f64()),
            Object::String(v) => Ok(format!(
                "string {} sha256 {}",
                v.as_bytes().len(),
                hex(&Sha256::digest(v.as_bytes()))
            )),
            Object::Name(v) => Ok(format!("/{}", name(v.as_ref()))),
            Object::Array(_) => {
                let array = value.array().context("PDF array disappeared")?;
                let mut entries = Vec::new();
                for (index, item) in array.iter().enumerate() {
                    entries.push(self.value(item, &format!("{path}[{index}]"), depth + 1)?);
                }
                Ok(format!("[{}]", entries.join(" ")))
            }
            Object::Dict(_) => self.dictionary(
                value
                    .as_dictionary()
                    .context("PDF dictionary disappeared")?,
                path,
                depth + 1,
                &[],
            ),
            Object::Stream(_) => self.stream(
                value.as_stream().context("PDF stream disappeared")?,
                path,
                depth + 1,
            ),
        }
    }

    fn dictionary(
        &mut self,
        dict: QueryDictionary<'a>,
        path: &str,
        depth: usize,
        omit: &[&[u8]],
    ) -> Result<String> {
        let mut entries = Vec::new();
        for (key, value) in dict.entries() {
            if omit.contains(&key.as_slice()) {
                continue;
            }
            let label = format!("{path}/{}", name(&key));
            entries.push(format!(
                "/{} {}",
                name(&key),
                self.value(value, &label, depth + 1)?
            ));
        }
        Ok(format!("<<{}>>", entries.join(" ")))
    }

    fn merged(
        &mut self,
        entries: BTreeMap<Vec<u8>, QueryValue<'a>>,
        path: &str,
        depth: usize,
    ) -> Result<String> {
        let mut rendered = Vec::new();
        for (key, value) in entries {
            let label = format!("{path}/{}", name(&key));
            rendered.push(format!(
                "/{} {}",
                name(&key),
                self.value(value, &label, depth + 1)?
            ));
        }
        Ok(format!("<<{}>>", rendered.join(" ")))
    }

    fn stream(&mut self, stream: QueryStream<'a>, path: &str, depth: usize) -> Result<String> {
        validate_simple_flate(&stream.dictionary, &stream.raw)
            .with_context(|| format!("validate lossless PDF stream at {path}"))?;
        let lossless = lossless_query_filters(&stream.dictionary)?;
        let omit: &[&[u8]] = if lossless {
            &[b"Length", b"Filter", b"DecodeParms", b"F", b"DP"]
        } else {
            &[]
        };
        let dictionary = self.dictionary(stream.dictionary, path, depth + 1, omit)?;
        if lossless {
            if !stream.decoded_ok {
                bail!("could not decode lossless PDF stream at {path}");
            }
            Ok(format!(
                "stream {dictionary} decoded bytes {} sha256 {}",
                stream.decoded.len(),
                hex(&stream.decoded_sha256)
            ))
        } else {
            Ok(format!(
                "encoded stream {dictionary} bytes {} sha256 {}",
                stream.raw.len(),
                hex(&Sha256::digest(&stream.raw))
            ))
        }
    }

    fn operand(&self, operand: &QueryOperand, depth: usize) -> Result<String> {
        if depth > MAX_DIRECT_DEPTH {
            bail!("PDF content operand nesting exceeds {MAX_DIRECT_DEPTH}");
        }
        match operand {
            QueryOperand::Null => Ok("null".to_owned()),
            QueryOperand::Boolean(v) => Ok(v.to_string()),
            QueryOperand::Number(v) => number(*v),
            QueryOperand::String(v) => Ok(format!(
                "string {} sha256 {}",
                v.len(),
                hex(&Sha256::digest(v))
            )),
            QueryOperand::Name(v) => Ok(format!("/{}", name(v))),
            QueryOperand::Array(values) => Ok(format!(
                "[{}]",
                values
                    .iter()
                    .map(|v| self.operand(v, depth + 1))
                    .collect::<Result<Vec<_>>>()?
                    .join(" ")
            )),
            QueryOperand::Dictionary(entries) => self.operand_dict(entries, depth + 1, &[]),
            QueryOperand::Stream {
                dictionary,
                raw_len,
                raw_sha256,
                decoded_len,
                decoded_sha256,
            } => {
                let lossless = lossless_operand_filters(dictionary)?;
                let omit: &[&[u8]] = if lossless {
                    &[b"Length", b"Filter", b"DecodeParms", b"F", b"DP"]
                } else {
                    &[]
                };
                let dictionary = self.operand_dict(dictionary, depth + 1, omit)?;
                if lossless {
                    let (Some(len), Some(digest)) = (decoded_len, decoded_sha256) else {
                        bail!("could not decode lossless inline PDF stream");
                    };
                    Ok(format!(
                        "inline stream {dictionary} decoded bytes {len} sha256 {}",
                        hex(digest)
                    ))
                } else {
                    Ok(format!(
                        "encoded inline stream {dictionary} bytes {raw_len} sha256 {}",
                        hex(raw_sha256)
                    ))
                }
            }
        }
    }

    fn operand_dict(
        &self,
        entries: &BTreeMap<Vec<u8>, QueryOperand>,
        depth: usize,
        omit: &[&[u8]],
    ) -> Result<String> {
        let mut rendered = Vec::new();
        for (key, value) in entries {
            if !omit.contains(&key.as_slice()) {
                rendered.push(format!(
                    "/{} {}",
                    name(key),
                    self.operand(value, depth + 1)?
                ));
            }
        }
        Ok(format!("<<{}>>", rendered.join(" ")))
    }
}

fn effective_resources<'a>(page: QueryDictionary<'a>) -> Result<BTreeMap<Vec<u8>, Resource<'a>>> {
    let mut layers = Vec::new();
    let mut current = Some(page);
    let mut seen = BTreeSet::new();
    while let Some(dict) = current {
        if layers.len() >= MAX_RESOURCE_PARENTS {
            bail!("PDF page resource ancestry exceeds {MAX_RESOURCE_PARENTS}");
        }
        if let Some(id) = dict.id()
            && !seen.insert(id)
        {
            bail!("PDF page resource ancestry contains a cycle");
        }
        current = dict.get(b"Parent").and_then(|v| v.as_dictionary());
        layers.push(dict);
    }
    let mut result: BTreeMap<Vec<u8>, Resource<'a>> = BTreeMap::new();
    for layer in layers.into_iter().rev() {
        if let Some(resources) = layer.get(b"Resources").and_then(|v| v.as_dictionary()) {
            for (key, value) in resources.entries() {
                if let Some(child) = value.as_dictionary() {
                    let mut merged = match result.remove(&key) {
                        Some(Resource::Merged(entries)) => entries,
                        _ => BTreeMap::new(),
                    };
                    merged.extend(child.entries());
                    result.insert(key, Resource::Merged(merged));
                } else {
                    result.insert(key, Resource::Value(value));
                }
            }
        }
    }
    Ok(result)
}

fn lossless_query_filters(dict: &QueryDictionary<'_>) -> Result<bool> {
    let value = dict.get(b"Filter").or_else(|| dict.get(b"F"));
    let Some(value) = value else { return Ok(true) };
    let names = if let Some(name) = value.name() {
        vec![name.as_ref().to_vec()]
    } else if let Some(array) = value.array() {
        array
            .iter()
            .map(|v| {
                v.name()
                    .map(|n| n.as_ref().to_vec())
                    .context("non-name PDF stream filter")
            })
            .collect::<Result<Vec<_>>>()?
    } else {
        bail!("PDF stream filter is neither name nor array");
    };
    classify_filters(&names)
}

/// Hayro intentionally recovers from broken Flate data; require a valid zlib
/// envelope before treating a singly filtered stream's decoded bytes as
/// equivalent to another encoding. Chained filters retain Hayro's projection
/// and are separately identified by their dictionary.
fn validate_simple_flate(dict: &QueryDictionary<'_>, raw: &[u8]) -> Result<()> {
    let value = dict.get(b"Filter").or_else(|| dict.get(b"F"));
    let Some(value) = value else { return Ok(()) };
    let Some(filter) = value.name() else {
        return Ok(());
    };
    if !matches!(filter.as_ref(), b"FlateDecode" | b"Fl") {
        return Ok(());
    }
    let decoder = ZlibDecoder::new(raw);
    let decoded_len = std::io::copy(
        &mut decoder.take(256 * 1024 * 1024 + 1),
        &mut std::io::sink(),
    )
    .context("invalid FlateDecode zlib data")?;
    if decoded_len > 256 * 1024 * 1024 {
        bail!("FlateDecode stream exceeds 256 MiB decoded limit");
    }
    Ok(())
}

fn lossless_operand_filters(dict: &BTreeMap<Vec<u8>, QueryOperand>) -> Result<bool> {
    let value = dict
        .get(b"Filter".as_slice())
        .or_else(|| dict.get(b"F".as_slice()));
    let Some(value) = value else { return Ok(true) };
    let names = match value {
        QueryOperand::Name(name) => vec![name.clone()],
        QueryOperand::Array(values) => values
            .iter()
            .map(|v| match v {
                QueryOperand::Name(name) => Ok(name.clone()),
                _ => bail!("non-name inline PDF stream filter"),
            })
            .collect::<Result<Vec<_>>>()?,
        _ => bail!("inline PDF stream filter is neither name nor array"),
    };
    classify_filters(&names)
}

fn classify_filters(filters: &[Vec<u8>]) -> Result<bool> {
    let mut lossless = true;
    for filter in filters {
        match filter.as_slice() {
            b"ASCIIHexDecode" | b"AHx" | b"ASCII85Decode" | b"A85" | b"LZWDecode" | b"LZW"
            | b"FlateDecode" | b"Fl" | b"RunLengthDecode" | b"RL" => {}
            b"DCTDecode" | b"DCT" | b"JPXDecode" | b"CCITTFaxDecode" | b"CCF" | b"JBIG2Decode" => {
                lossless = false
            }
            _ => bail!("unsupported PDF stream filter /{}", name(filter)),
        }
    }
    Ok(lossless)
}

fn box_text(values: [f64; 4]) -> Result<String> {
    values
        .into_iter()
        .map(number)
        .collect::<Result<Vec<_>>>()
        .map(|v| v.join(" "))
}

fn number(value: f64) -> Result<String> {
    if !value.is_finite() {
        bail!("nonfinite PDF number")
    }
    Ok(value.to_string())
}

fn name(bytes: &[u8]) -> String {
    if bytes
        .iter()
        .all(|byte| byte.is_ascii_graphic() && *byte != b'#')
    {
        String::from_utf8_lossy(bytes).into_owned()
    } else {
        format!("#{}", hex(bytes))
    }
}

fn preview(encoded: &str) -> String {
    const MAX_CHARS: usize = 96;
    let mut result: String = encoded.chars().take(MAX_CHARS).collect();
    if encoded.chars().count() > MAX_CHARS {
        result.push('…');
    }
    result
}
