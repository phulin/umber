use std::fmt;

use quick_xml::Reader;
use quick_xml::events::Event;
use umber_vfs::{VfsSnapshot, VirtualPath};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct XmlLimits {
    pub max_bytes: usize,
    pub max_depth: usize,
    pub max_nodes: usize,
    pub max_attributes: usize,
    pub max_text_bytes: usize,
    pub max_includes: usize,
}

impl Default for XmlLimits {
    fn default() -> Self {
        Self {
            max_bytes: 16 * 1024 * 1024,
            max_depth: 128,
            max_nodes: 250_000,
            max_attributes: 1_000_000,
            max_text_bytes: 32 * 1024 * 1024,
            max_includes: 32,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct XmlAttribute {
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct XmlNode {
    pub name: String,
    pub attributes: Vec<XmlAttribute>,
    pub children: Vec<XmlNode>,
    pub text: String,
}

impl XmlNode {
    #[must_use]
    pub fn local_name(&self) -> &str {
        self.name
            .rsplit_once(':')
            .map_or(&self.name, |(_, local)| local)
    }

    #[must_use]
    pub fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|attribute| attribute.name == name)
            .map(|attribute| attribute.value.as_str())
    }

    pub fn child(&self, local_name: &str) -> Option<&Self> {
        self.children
            .iter()
            .find(|child| child.local_name() == local_name)
    }

    pub fn children_named<'a>(
        &'a self,
        local_name: &'a str,
    ) -> impl Iterator<Item = &'a Self> + 'a {
        self.children
            .iter()
            .filter(move |child| child.local_name() == local_name)
    }

    #[must_use]
    pub fn trimmed_text(&self) -> &str {
        self.text.trim()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum XmlError {
    Limit { kind: &'static str, limit: usize },
    Malformed(String),
    ForbiddenDoctype,
    MissingRoot,
    MultipleRoots,
    MissingResource(VirtualPath),
    IncludeCycle(VirtualPath),
    InvalidInclude(String),
    Vfs(String),
}

impl fmt::Display for XmlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Limit { kind, limit } => write!(formatter, "XML {kind} limit {limit} exceeded"),
            Self::Malformed(message) => write!(formatter, "malformed XML: {message}"),
            Self::ForbiddenDoctype => {
                formatter.write_str("XML document types and entities are forbidden")
            }
            Self::MissingRoot => formatter.write_str("XML document has no root element"),
            Self::MultipleRoots => formatter.write_str("XML document has multiple root elements"),
            Self::MissingResource(path) => write!(formatter, "missing XML resource {path}"),
            Self::IncludeCycle(path) => write!(formatter, "XML include cycle at {path}"),
            Self::InvalidInclude(message) => write!(formatter, "invalid XML include: {message}"),
            Self::Vfs(message) => write!(formatter, "VFS snapshot error: {message}"),
        }
    }
}

impl std::error::Error for XmlError {}

pub fn parse_xml(bytes: &[u8], limits: XmlLimits) -> Result<XmlNode, XmlError> {
    if bytes.len() > limits.max_bytes {
        return Err(XmlError::Limit {
            kind: "byte",
            limit: limits.max_bytes,
        });
    }
    if bytes
        .windows(9)
        .any(|window| window.eq_ignore_ascii_case(b"<!DOCTYPE"))
    {
        return Err(XmlError::ForbiddenDoctype);
    }

    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().trim_text(false);
    reader.config_mut().check_end_names = true;
    let mut stack: Vec<XmlNode> = Vec::new();
    let mut root = None;
    let mut nodes = 0usize;
    let mut attributes = 0usize;
    let mut text_bytes = 0usize;

    loop {
        let event = reader
            .read_event()
            .map_err(|error| XmlError::Malformed(error.to_string()))?;
        match event {
            Event::Start(start) => {
                if stack.len() >= limits.max_depth {
                    return Err(XmlError::Limit {
                        kind: "nesting",
                        limit: limits.max_depth,
                    });
                }
                nodes = checked_increment(nodes, limits.max_nodes, "node")?;
                let node = decode_start(&reader, &start, &mut attributes, limits)?;
                stack.push(node);
            }
            Event::Empty(start) => {
                nodes = checked_increment(nodes, limits.max_nodes, "node")?;
                let node = decode_start(&reader, &start, &mut attributes, limits)?;
                append_node(&mut stack, &mut root, node)?;
            }
            Event::End(_) => {
                let node = stack
                    .pop()
                    .ok_or_else(|| XmlError::Malformed("unexpected end element".into()))?;
                append_node(&mut stack, &mut root, node)?;
            }
            Event::Text(text) => {
                let value = text
                    .xml_content()
                    .map_err(|error| XmlError::Malformed(error.to_string()))?;
                text_bytes = text_bytes.checked_add(value.len()).ok_or(XmlError::Limit {
                    kind: "text byte",
                    limit: limits.max_text_bytes,
                })?;
                if text_bytes > limits.max_text_bytes {
                    return Err(XmlError::Limit {
                        kind: "text byte",
                        limit: limits.max_text_bytes,
                    });
                }
                if let Some(node) = stack.last_mut() {
                    node.text.push_str(&value);
                } else if !value.trim().is_empty() {
                    return Err(XmlError::Malformed("text outside root element".into()));
                }
            }
            Event::CData(text) => {
                let value = reader
                    .decoder()
                    .decode(text.as_ref())
                    .map_err(|error| XmlError::Malformed(error.to_string()))?;
                text_bytes = text_bytes.checked_add(value.len()).ok_or(XmlError::Limit {
                    kind: "text byte",
                    limit: limits.max_text_bytes,
                })?;
                if text_bytes > limits.max_text_bytes {
                    return Err(XmlError::Limit {
                        kind: "text byte",
                        limit: limits.max_text_bytes,
                    });
                }
                if let Some(node) = stack.last_mut() {
                    node.text.push_str(&value);
                }
            }
            Event::GeneralRef(reference) => {
                let name = reader
                    .decoder()
                    .decode(reference.as_ref())
                    .map_err(|error| XmlError::Malformed(error.to_string()))?;
                let value = match name.as_ref() {
                    "amp" => "&",
                    "lt" => "<",
                    "gt" => ">",
                    "apos" => "'",
                    "quot" => "\"",
                    _ => return Err(XmlError::Malformed(format!("unsupported entity &{name};"))),
                };
                text_bytes = text_bytes.checked_add(value.len()).ok_or(XmlError::Limit {
                    kind: "text byte",
                    limit: limits.max_text_bytes,
                })?;
                if text_bytes > limits.max_text_bytes {
                    return Err(XmlError::Limit {
                        kind: "text byte",
                        limit: limits.max_text_bytes,
                    });
                }
                if let Some(node) = stack.last_mut() {
                    node.text.push_str(value);
                }
            }
            Event::DocType(_) => return Err(XmlError::ForbiddenDoctype),
            Event::Eof => break,
            Event::Decl(_) | Event::PI(_) | Event::Comment(_) => {}
        }
    }
    if !stack.is_empty() {
        return Err(XmlError::Malformed("unclosed element".into()));
    }
    root.ok_or(XmlError::MissingRoot)
}

pub fn parse_xml_from_snapshot(
    snapshot: &VfsSnapshot,
    path: &VirtualPath,
    limits: XmlLimits,
) -> Result<XmlNode, XmlError> {
    let mut stack = Vec::new();
    let mut includes = 0usize;
    parse_included(snapshot, path, limits, &mut stack, &mut includes)
}

fn parse_included(
    snapshot: &VfsSnapshot,
    path: &VirtualPath,
    limits: XmlLimits,
    stack: &mut Vec<VirtualPath>,
    includes: &mut usize,
) -> Result<XmlNode, XmlError> {
    if stack.contains(path) {
        return Err(XmlError::IncludeCycle(path.clone()));
    }
    let file = snapshot
        .get(path)
        .map_err(|error| XmlError::Vfs(error.to_string()))?
        .ok_or_else(|| XmlError::MissingResource(path.clone()))?;
    stack.push(path.clone());
    let mut root = parse_xml(file.bytes(), limits)?;
    expand_includes(snapshot, path, limits, stack, includes, &mut root)?;
    stack.pop();
    Ok(root)
}

fn expand_includes(
    snapshot: &VfsSnapshot,
    current_path: &VirtualPath,
    limits: XmlLimits,
    stack: &mut Vec<VirtualPath>,
    includes: &mut usize,
    node: &mut XmlNode,
) -> Result<(), XmlError> {
    let mut expanded = Vec::with_capacity(node.children.len());
    for mut child in node.children.drain(..) {
        if child.name == "xi:include" {
            *includes = checked_increment(*includes, limits.max_includes, "include")?;
            let href = child
                .attribute("href")
                .ok_or_else(|| XmlError::InvalidInclude("missing href".into()))?;
            if child.attribute("parse").is_some_and(|parse| parse != "xml") {
                return Err(XmlError::InvalidInclude(
                    "only parse=xml is supported".into(),
                ));
            }
            let included_path = resolve_include(current_path, href)?;
            expanded.push(parse_included(
                snapshot,
                &included_path,
                limits,
                stack,
                includes,
            )?);
        } else {
            expand_includes(snapshot, current_path, limits, stack, includes, &mut child)?;
            expanded.push(child);
        }
    }
    node.children = expanded;
    Ok(())
}

fn resolve_include(current: &VirtualPath, href: &str) -> Result<VirtualPath, XmlError> {
    if href.contains("://") || href.starts_with("/texlive/") {
        return Err(XmlError::InvalidInclude(
            "includes must remain in the current virtual root".into(),
        ));
    }
    if href.starts_with('/') {
        return VirtualPath::user(href)
            .map_err(|error| XmlError::InvalidInclude(error.to_string()));
    }
    let (directory, _) = current
        .as_str()
        .rsplit_once('/')
        .ok_or_else(|| XmlError::InvalidInclude("including path has no directory".into()))?;
    VirtualPath::user(&format!("{directory}/{href}"))
        .map_err(|error| XmlError::InvalidInclude(error.to_string()))
}

fn decode_start(
    reader: &Reader<&[u8]>,
    start: &quick_xml::events::BytesStart<'_>,
    attribute_count: &mut usize,
    limits: XmlLimits,
) -> Result<XmlNode, XmlError> {
    let name = reader
        .decoder()
        .decode(start.name().as_ref())
        .map_err(|error| XmlError::Malformed(error.to_string()))?
        .into_owned();
    let mut decoded = Vec::new();
    for attribute in start.attributes() {
        *attribute_count = checked_increment(*attribute_count, limits.max_attributes, "attribute")?;
        let attribute = attribute.map_err(|error| XmlError::Malformed(error.to_string()))?;
        let attribute_name = reader
            .decoder()
            .decode(attribute.key.as_ref())
            .map_err(|error| XmlError::Malformed(error.to_string()))?
            .into_owned();
        let value = attribute
            .decode_and_unescape_value(reader.decoder())
            .map_err(|error| XmlError::Malformed(error.to_string()))?
            .into_owned();
        decoded.push(XmlAttribute {
            name: attribute_name,
            value,
        });
    }
    Ok(XmlNode {
        name,
        attributes: decoded,
        children: Vec::new(),
        text: String::new(),
    })
}

fn append_node(
    stack: &mut [XmlNode],
    root: &mut Option<XmlNode>,
    node: XmlNode,
) -> Result<(), XmlError> {
    if let Some(parent) = stack.last_mut() {
        parent.children.push(node);
    } else if root.replace(node).is_some() {
        return Err(XmlError::MultipleRoots);
    }
    Ok(())
}

fn checked_increment(value: usize, limit: usize, kind: &'static str) -> Result<usize, XmlError> {
    let value = value
        .checked_add(1)
        .ok_or(XmlError::Limit { kind, limit })?;
    if value > limit {
        return Err(XmlError::Limit { kind, limit });
    }
    Ok(value)
}
