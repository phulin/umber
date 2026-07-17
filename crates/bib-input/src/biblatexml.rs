use std::collections::BTreeMap;
use std::fmt;

use bib_unicode::ExtendedDate;
use umber_vfs::{VfsSnapshot, VirtualPath};

use crate::xml::{XmlError, XmlLimits, XmlNode, parse_xml, parse_xml_from_snapshot};

pub const BIBLATEX_XML_NAMESPACE: &str = "http://biblatex-biber.sourceforge.net/biblatexml";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct XmlName {
    pub parts: BTreeMap<String, Vec<NamePart>>,
    pub attributes: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NamePart {
    pub value: String,
    pub initial: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct XmlAnnotation {
    pub field: String,
    pub name: String,
    pub item: Option<usize>,
    pub part: Option<String>,
    pub literal: bool,
    pub value: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum XmlFieldValue {
    Literal(String),
    Names {
        values: Vec<XmlName>,
        attributes: BTreeMap<String, String>,
    },
    List(Vec<XmlListItem>),
    Date(ExtendedDate),
    Range(Vec<(String, Option<String>)>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct XmlListItem {
    pub value: String,
    pub attributes: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BibLatexXmlEntry {
    pub id: String,
    pub entry_type: String,
    pub options: BTreeMap<String, String>,
    pub aliases: Vec<String>,
    pub fields: BTreeMap<String, XmlFieldValue>,
    pub annotations: Vec<XmlAnnotation>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BibLatexXmlData {
    pub entries: Vec<BibLatexXmlEntry>,
    aliases: BTreeMap<String, String>,
}

impl BibLatexXmlData {
    #[must_use]
    pub fn entry(&self, id: &str) -> Option<&BibLatexXmlEntry> {
        self.entries.iter().find(|entry| entry.id == id)
    }

    #[must_use]
    pub fn canonical_id(&self, alias: &str) -> Option<&str> {
        self.aliases.get(alias).map(String::as_str)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BibLatexXmlError {
    Xml(XmlError),
    Namespace { found: Option<String> },
    Schema(String),
    Date { field: String, value: String },
}

impl fmt::Display for BibLatexXmlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Xml(error) => error.fmt(formatter),
            Self::Namespace { found } => write!(
                formatter,
                "BibLaTeXML namespace is {:?}; expected {BIBLATEX_XML_NAMESPACE}",
                found.as_deref().unwrap_or("<missing>")
            ),
            Self::Schema(message) => {
                write!(formatter, "BibLaTeXML schema validation failed: {message}")
            }
            Self::Date { field, value } => write!(
                formatter,
                "invalid BibLaTeXML date `{value}` in field `{field}`"
            ),
        }
    }
}

impl std::error::Error for BibLatexXmlError {}

impl From<XmlError> for BibLatexXmlError {
    fn from(value: XmlError) -> Self {
        Self::Xml(value)
    }
}

pub fn validate_biblatexml_bytes(bytes: &[u8], limits: XmlLimits) -> Result<(), BibLatexXmlError> {
    let root = parse_xml(bytes, limits)?;
    validate_tree(&root)
}

pub fn parse_biblatexml_bytes(
    bytes: &[u8],
    limits: XmlLimits,
) -> Result<BibLatexXmlData, BibLatexXmlError> {
    from_tree(&parse_xml(bytes, limits)?)
}

pub fn parse_biblatexml(
    snapshot: &VfsSnapshot,
    path: &VirtualPath,
    limits: XmlLimits,
) -> Result<BibLatexXmlData, BibLatexXmlError> {
    from_tree(&parse_xml_from_snapshot(snapshot, path, limits)?)
}

fn validate_tree(root: &XmlNode) -> Result<(), BibLatexXmlError> {
    if root.name != "bltx:entries" {
        return Err(BibLatexXmlError::Schema(
            "root element must be bltx:entries".into(),
        ));
    }
    let namespace = root.attribute("xmlns:bltx").map(str::to_owned);
    if namespace.as_deref() != Some(BIBLATEX_XML_NAMESPACE) {
        return Err(BibLatexXmlError::Namespace { found: namespace });
    }
    for entry in root.children_named("entry") {
        if entry.name != "bltx:entry"
            || entry.attribute("id").is_none()
            || entry.attribute("entrytype").is_none()
        {
            return Err(BibLatexXmlError::Schema(
                "entry requires bltx namespace, id, and entrytype".into(),
            ));
        }
        for child in &entry.children {
            if !child.name.starts_with("bltx:") {
                return Err(BibLatexXmlError::Schema(
                    "all entry fields must use the bltx namespace".into(),
                ));
            }
        }
    }
    Ok(())
}

fn from_tree(root: &XmlNode) -> Result<BibLatexXmlData, BibLatexXmlError> {
    validate_tree(root)?;
    let mut result = BibLatexXmlData::default();
    for node in root.children_named("entry") {
        let entry = parse_entry(node)?;
        if result
            .entries
            .iter()
            .any(|existing| existing.id == entry.id)
        {
            return Err(BibLatexXmlError::Schema(format!(
                "duplicate entry id {}",
                entry.id
            )));
        }
        for alias in &entry.aliases {
            if result
                .aliases
                .insert(alias.clone(), entry.id.clone())
                .is_some()
            {
                return Err(BibLatexXmlError::Schema(format!("duplicate alias {alias}")));
            }
        }
        result.entries.push(entry);
    }
    Ok(result)
}

fn parse_entry(node: &XmlNode) -> Result<BibLatexXmlEntry, BibLatexXmlError> {
    let id = node.attribute("id").unwrap_or_default().to_owned();
    let entry_type = node.attribute("entrytype").unwrap_or_default().to_owned();
    let options = node
        .child("options")
        .map_or_else(BTreeMap::new, |node| parse_options(node.trimmed_text()));
    let aliases = node.child("ids").map_or_else(Vec::new, |ids| {
        ids.children_named("key")
            .map(|key| key.trimmed_text().to_owned())
            .collect()
    });
    let annotations = node
        .children_named("annotation")
        .map(parse_annotation)
        .collect::<Result<Vec<_>, _>>()?;
    let mut fields = BTreeMap::new();
    for child in &node.children {
        let name = child.local_name();
        if matches!(name, "options" | "ids" | "annotation") {
            continue;
        }
        let (field_name, value) = if name == "names" {
            let field_name = child
                .attribute("type")
                .ok_or_else(|| BibLatexXmlError::Schema("names requires type".into()))?;
            let values = child
                .children_named("name")
                .map(parse_name)
                .collect::<Result<Vec<_>, _>>()?;
            (
                field_name.to_owned(),
                XmlFieldValue::Names {
                    values,
                    attributes: attrs(child),
                },
            )
        } else if name == "date" {
            let prefix = child.attribute("type").unwrap_or("date");
            let field_name = if prefix == "date" {
                "date".to_owned()
            } else {
                format!("{prefix}date")
            };
            let raw = if child.children.is_empty() {
                child.trimmed_text().to_owned()
            } else {
                let start = child
                    .child("start")
                    .map(XmlNode::trimmed_text)
                    .unwrap_or_default();
                let end = child
                    .child("end")
                    .map(XmlNode::trimmed_text)
                    .unwrap_or_default();
                format!("{start}/{end}")
            };
            let date = ExtendedDate::parse(&raw).map_err(|_| BibLatexXmlError::Date {
                field: field_name.clone(),
                value: raw,
            })?;
            (field_name, XmlFieldValue::Date(date))
        } else if name == "pages" {
            let ranges = child
                .child("list")
                .into_iter()
                .flat_map(|list| list.children_named("item"))
                .map(|item| {
                    let start = item
                        .child("start")
                        .map(XmlNode::trimmed_text)
                        .unwrap_or_default()
                        .to_owned();
                    let end = item
                        .child("end")
                        .map(XmlNode::trimmed_text)
                        .filter(|value| !value.is_empty())
                        .map(str::to_owned);
                    (start, end)
                })
                .collect();
            (name.to_owned(), XmlFieldValue::Range(ranges))
        } else if let Some(list) = child.child("list") {
            let values = list
                .children_named("item")
                .map(|item| XmlListItem {
                    value: item.trimmed_text().to_owned(),
                    attributes: attrs(item),
                })
                .collect();
            (name.to_owned(), XmlFieldValue::List(values))
        } else {
            (
                name.to_owned(),
                XmlFieldValue::Literal(child.trimmed_text().to_owned()),
            )
        };
        if fields.insert(field_name.clone(), value).is_some() {
            return Err(BibLatexXmlError::Schema(format!(
                "duplicate field {field_name} in {id}"
            )));
        }
    }
    Ok(BibLatexXmlEntry {
        id,
        entry_type,
        options,
        aliases,
        fields,
        annotations,
    })
}

fn parse_name(node: &XmlNode) -> Result<XmlName, BibLatexXmlError> {
    let mut parts = BTreeMap::new();
    for part in node.children_named("namepart") {
        let kind = part
            .attribute("type")
            .ok_or_else(|| BibLatexXmlError::Schema("namepart requires type".into()))?
            .to_owned();
        let values = if part.children_named("namepart").next().is_some() {
            part.children_named("namepart")
                .map(|value| NamePart {
                    value: value.trimmed_text().to_owned(),
                    initial: value.attribute("initial").map(str::to_owned),
                })
                .collect()
        } else {
            vec![NamePart {
                value: part.trimmed_text().to_owned(),
                initial: part.attribute("initial").map(str::to_owned),
            }]
        };
        parts.insert(kind, values);
    }
    if parts.is_empty() {
        return Err(BibLatexXmlError::Schema(
            "name requires at least one namepart".into(),
        ));
    }
    Ok(XmlName {
        parts,
        attributes: attrs(node),
    })
}

fn parse_annotation(node: &XmlNode) -> Result<XmlAnnotation, BibLatexXmlError> {
    Ok(XmlAnnotation {
        field: node
            .attribute("field")
            .ok_or_else(|| BibLatexXmlError::Schema("annotation requires field".into()))?
            .to_owned(),
        name: node.attribute("name").unwrap_or("default").to_owned(),
        item: node
            .attribute("item")
            .map(str::parse)
            .transpose()
            .map_err(|_| BibLatexXmlError::Schema("annotation item must be an integer".into()))?,
        part: node.attribute("part").map(str::to_owned),
        literal: node.attribute("literal").is_some_and(|value| value == "1"),
        value: node.trimmed_text().to_owned(),
    })
}

fn parse_options(options: &str) -> BTreeMap<String, String> {
    options
        .split(',')
        .filter_map(|part| part.trim().split_once('='))
        .map(|(key, value)| (key.trim().to_owned(), value.trim().to_owned()))
        .collect()
}

fn attrs(node: &XmlNode) -> BTreeMap<String, String> {
    node.attributes
        .iter()
        .map(|attribute| (attribute.name.clone(), attribute.value.clone()))
        .collect()
}
