use std::collections::BTreeSet;
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
struct ProjectedElement {
    name: String,
    attributes: Vec<XmlAttribute>,
    children: Vec<usize>,
    text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct XmlProjection {
    elements: Vec<ProjectedElement>,
    root: usize,
}

impl XmlProjection {
    pub(crate) fn root(&self) -> XmlNode<'_> {
        XmlNode {
            projection: self,
            index: self.root,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct XmlNode<'a> {
    projection: &'a XmlProjection,
    index: usize,
}

impl<'a> XmlNode<'a> {
    fn element(self) -> &'a ProjectedElement {
        &self.projection.elements[self.index]
    }

    pub(crate) fn name(self) -> &'a str {
        &self.element().name
    }

    #[must_use]
    pub(crate) fn local_name(self) -> &'a str {
        self.name()
            .rsplit_once(':')
            .map_or(self.name(), |(_, local)| local)
    }

    #[must_use]
    pub(crate) fn attribute(self, name: &str) -> Option<&'a str> {
        self.element()
            .attributes
            .iter()
            .find(|attribute| attribute.name == name)
            .map(|attribute| attribute.value.as_str())
    }

    pub(crate) fn attributes(self) -> impl Iterator<Item = (&'a str, &'a str)> + 'a {
        self.element()
            .attributes
            .iter()
            .map(|attribute| (attribute.name.as_str(), attribute.value.as_str()))
    }

    pub(crate) fn children(self) -> impl ExactSizeIterator<Item = Self> + DoubleEndedIterator + 'a {
        self.element().children.iter().map(move |&index| Self {
            projection: self.projection,
            index,
        })
    }

    pub(crate) fn child(self, local_name: &str) -> Option<Self> {
        self.children()
            .find(|child| child.local_name() == local_name)
    }

    pub(crate) fn children_named(self, local_name: &'a str) -> impl Iterator<Item = Self> + 'a {
        self.children()
            .filter(move |child| child.local_name() == local_name)
    }

    #[must_use]
    pub(crate) fn trimmed_text(self) -> &'a str {
        self.element().text.trim()
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

pub(crate) fn parse_xml(bytes: &[u8], limits: XmlLimits) -> Result<XmlProjection, XmlError> {
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
    let mut elements = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
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
                let element = decode_start(&reader, &start, &mut attributes, limits)?;
                let index = append_element(&mut elements, &stack, &mut root, element)?;
                stack.push(index);
            }
            Event::Empty(start) => {
                nodes = checked_increment(nodes, limits.max_nodes, "node")?;
                let element = decode_start(&reader, &start, &mut attributes, limits)?;
                append_element(&mut elements, &stack, &mut root, element)?;
            }
            Event::End(_) => {
                stack
                    .pop()
                    .ok_or_else(|| XmlError::Malformed("unexpected end element".into()))?;
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
                if let Some(&index) = stack.last() {
                    elements[index].text.push_str(&value);
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
                if let Some(&index) = stack.last() {
                    elements[index].text.push_str(&value);
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
                if let Some(&index) = stack.last() {
                    elements[index].text.push_str(value);
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
    Ok(XmlProjection {
        elements,
        root: root.ok_or(XmlError::MissingRoot)?,
    })
}

pub fn parse_xml_from_snapshot(
    snapshot: &VfsSnapshot,
    path: &VirtualPath,
    limits: XmlLimits,
) -> Result<XmlProjection, XmlError> {
    parse_xml_from_snapshot_with_paths(snapshot, path, limits).map(|(node, _)| node)
}

pub(crate) fn parse_xml_from_snapshot_with_paths(
    snapshot: &VfsSnapshot,
    path: &VirtualPath,
    limits: XmlLimits,
) -> Result<(XmlProjection, BTreeSet<VirtualPath>), XmlError> {
    IncludeProjector {
        snapshot,
        limits,
        stack: Vec::new(),
        includes: 0,
        paths: BTreeSet::new(),
    }
    .project(path)
}

struct IncludeProjector<'a> {
    snapshot: &'a VfsSnapshot,
    limits: XmlLimits,
    stack: Vec<VirtualPath>,
    includes: usize,
    paths: BTreeSet<VirtualPath>,
}

impl IncludeProjector<'_> {
    fn project(
        mut self,
        path: &VirtualPath,
    ) -> Result<(XmlProjection, BTreeSet<VirtualPath>), XmlError> {
        let projection = self.parse(path)?;
        Ok((projection, self.paths))
    }

    fn parse(&mut self, path: &VirtualPath) -> Result<XmlProjection, XmlError> {
        if self.stack.contains(path) {
            return Err(XmlError::IncludeCycle(path.clone()));
        }
        let file = self
            .snapshot
            .get(path)
            .map_err(|error| XmlError::Vfs(error.to_string()))?
            .ok_or_else(|| XmlError::MissingResource(path.clone()))?;
        self.paths.insert(path.clone());
        self.stack.push(path.clone());
        let mut projection = parse_xml(file.bytes(), self.limits)?;
        let root = projection.root;
        self.expand(path, &mut projection, root)?;
        self.stack.pop();
        Ok(projection)
    }

    fn expand(
        &mut self,
        current_path: &VirtualPath,
        projection: &mut XmlProjection,
        element: usize,
    ) -> Result<(), XmlError> {
        let children = std::mem::take(&mut projection.elements[element].children);
        let mut expanded = Vec::with_capacity(children.len());
        for child in children {
            if projection.elements[child].name == "xi:include" {
                self.includes =
                    checked_increment(self.includes, self.limits.max_includes, "include")?;
                let href = projection.elements[child]
                    .attributes
                    .iter()
                    .find(|attribute| attribute.name == "href")
                    .map(|attribute| attribute.value.as_str())
                    .ok_or_else(|| XmlError::InvalidInclude("missing href".into()))?;
                if projection.elements[child]
                    .attributes
                    .iter()
                    .find(|attribute| attribute.name == "parse")
                    .is_some_and(|attribute| attribute.value != "xml")
                {
                    return Err(XmlError::InvalidInclude(
                        "only parse=xml is supported".into(),
                    ));
                }
                let included_path = resolve_include(current_path, href)?;
                let included = self.parse(&included_path)?;
                expanded.push(import_projection(projection, &included, included.root));
            } else {
                self.expand(current_path, projection, child)?;
                expanded.push(child);
            }
        }
        projection.elements[element].children = expanded;
        Ok(())
    }
}

fn import_projection(
    destination: &mut XmlProjection,
    source: &XmlProjection,
    source_index: usize,
) -> usize {
    let source_element = &source.elements[source_index];
    let index = destination.elements.len();
    destination.elements.push(ProjectedElement {
        name: source_element.name.clone(),
        attributes: source_element.attributes.clone(),
        children: Vec::new(),
        text: source_element.text.clone(),
    });
    let children = source_element
        .children
        .iter()
        .map(|&child| import_projection(destination, source, child))
        .collect();
    destination.elements[index].children = children;
    index
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
) -> Result<ProjectedElement, XmlError> {
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
    Ok(ProjectedElement {
        name,
        attributes: decoded,
        children: Vec::new(),
        text: String::new(),
    })
}

fn append_element(
    elements: &mut Vec<ProjectedElement>,
    stack: &[usize],
    root: &mut Option<usize>,
    element: ProjectedElement,
) -> Result<usize, XmlError> {
    let index = elements.len();
    elements.push(element);
    if let Some(&parent) = stack.last() {
        elements[parent].children.push(index);
    } else if root.replace(index).is_some() {
        return Err(XmlError::MultipleRoots);
    }
    Ok(index)
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
