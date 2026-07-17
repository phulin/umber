use std::collections::BTreeMap;
use std::fmt;

use umber_vfs::{VfsSnapshot, VirtualPath};

use crate::xml::{XmlError, XmlLimits, XmlNode, parse_xml, parse_xml_from_snapshot};

pub const CONTROL_NAMESPACE: &str = "https://sourceforge.net/projects/biblatex";
pub const CONTROL_VERSION: &str = "3.11";

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum OptionComponent {
    Processor,
    Biblatex,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StructuredValue {
    pub content: String,
    pub attributes: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ControlOptionValue {
    Single(StructuredValue),
    Multiple(Vec<StructuredValue>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlOptionSet {
    pub component: OptionComponent,
    pub scope: String,
    pub values: BTreeMap<String, ControlOptionValue>,
}

impl ControlOptionSet {
    #[must_use]
    pub fn value(&self, key: &str) -> Option<&ControlOptionValue> {
        self.values.get(key)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Template {
    pub kind: String,
    pub name: String,
    pub elements: Vec<TemplateElement>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TemplateElement {
    pub name: String,
    pub content: String,
    pub attributes: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DataModel {
    pub entry_types: Vec<String>,
    pub fields: Vec<DataModelField>,
    pub constraints: Vec<TemplateElement>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DataModelField {
    pub name: String,
    pub datatype: Option<String>,
    pub attributes: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlSection {
    pub number: u32,
    pub citekeys: Vec<String>,
    pub datasources: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlFile {
    pub version: String,
    pub biblatex_version: String,
    pub options: Vec<ControlOptionSet>,
    pub templates: Vec<Template>,
    pub data_model: DataModel,
    pub sections: Vec<ControlSection>,
}

impl ControlFile {
    #[must_use]
    pub fn option_set(&self, component: OptionComponent, scope: &str) -> Option<&ControlOptionSet> {
        self.options
            .iter()
            .find(|set| set.component == component && set.scope == scope)
    }

    #[must_use]
    pub fn resolve_option(
        &self,
        component: OptionComponent,
        key: &str,
        entry_type: Option<&str>,
    ) -> Option<&ControlOptionValue> {
        entry_type
            .and_then(|scope| self.option_set(component, scope))
            .and_then(|set| set.value(key))
            .or_else(|| {
                self.option_set(component, "global")
                    .and_then(|set| set.value(key))
            })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ControlError {
    Xml(XmlError),
    Namespace {
        found: Option<String>,
    },
    Version {
        found: Option<String>,
        expected: &'static str,
    },
    Schema(String),
}

impl fmt::Display for ControlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Xml(error) => error.fmt(formatter),
            Self::Namespace { found } => write!(
                formatter,
                "control-file namespace is {:?}; expected {CONTROL_NAMESPACE}",
                found.as_deref().unwrap_or("<missing>")
            ),
            Self::Version { found, expected } => write!(
                formatter,
                "control-file version is {:?}; expected {expected}",
                found.as_deref().unwrap_or("<missing>")
            ),
            Self::Schema(message) => write!(
                formatter,
                "control-file schema validation failed: {message}"
            ),
        }
    }
}

impl std::error::Error for ControlError {}

impl From<XmlError> for ControlError {
    fn from(value: XmlError) -> Self {
        Self::Xml(value)
    }
}

pub fn validate_control_bytes(bytes: &[u8], limits: XmlLimits) -> Result<(), ControlError> {
    let root = parse_xml(bytes, limits)?;
    validate_control_tree(&root)
}

pub fn parse_control_bytes(bytes: &[u8], limits: XmlLimits) -> Result<ControlFile, ControlError> {
    let root = parse_xml(bytes, limits)?;
    control_from_tree(&root)
}

pub fn parse_control(
    snapshot: &VfsSnapshot,
    path: &VirtualPath,
    limits: XmlLimits,
) -> Result<ControlFile, ControlError> {
    let root = parse_xml_from_snapshot(snapshot, path, limits)?;
    control_from_tree(&root)
}

fn validate_control_tree(root: &XmlNode) -> Result<(), ControlError> {
    if root.name != "bcf:controlfile" || root.local_name() != "controlfile" {
        return Err(ControlError::Schema(
            "root element must be bcf:controlfile".into(),
        ));
    }
    let namespace = root.attribute("xmlns:bcf").map(str::to_owned);
    if namespace.as_deref() != Some(CONTROL_NAMESPACE) {
        return Err(ControlError::Namespace { found: namespace });
    }
    let version = root.attribute("version").map(str::to_owned);
    if version.as_deref() != Some(CONTROL_VERSION) {
        return Err(ControlError::Version {
            found: version,
            expected: CONTROL_VERSION,
        });
    }
    if root.attribute("bltxversion").is_none() {
        return Err(ControlError::Schema("missing bltxversion attribute".into()));
    }
    if root
        .children
        .iter()
        .any(|node| !node.name.starts_with("bcf:"))
    {
        return Err(ControlError::Schema(
            "all control-file elements must use the bcf namespace".into(),
        ));
    }
    for options in root.children_named("options") {
        if options.attribute("component").is_none() || options.attribute("type").is_none() {
            return Err(ControlError::Schema(
                "options requires component and type".into(),
            ));
        }
        for option in options.children_named("option") {
            let kind = option.attribute("type");
            if !matches!(kind, Some("singlevalued" | "multivalued"))
                || option.child("key").is_none()
                || option.children_named("value").next().is_none()
            {
                return Err(ControlError::Schema(
                    "option requires a supported type, key, and value".into(),
                ));
            }
        }
    }
    for section in root.children_named("section") {
        let number = section
            .attribute("number")
            .ok_or_else(|| ControlError::Schema("section requires number".into()))?;
        number.parse::<u32>().map_err(|_| {
            ControlError::Schema("section number must be an unsigned integer".into())
        })?;
    }
    Ok(())
}

fn control_from_tree(root: &XmlNode) -> Result<ControlFile, ControlError> {
    validate_control_tree(root)?;
    let options = root
        .children_named("options")
        .map(parse_option_set)
        .collect::<Result<Vec<_>, _>>()?;
    let templates = root
        .children
        .iter()
        .filter(|node| node.local_name().ends_with("template"))
        .map(parse_template)
        .collect();
    let data_model = root
        .child("datamodel")
        .map(parse_data_model)
        .unwrap_or_else(|| DataModel {
            entry_types: Vec::new(),
            fields: Vec::new(),
            constraints: Vec::new(),
        });
    let sections = root
        .children_named("section")
        .map(parse_section)
        .collect::<Result<_, _>>()?;
    Ok(ControlFile {
        version: root.attribute("version").unwrap_or_default().to_owned(),
        biblatex_version: root.attribute("bltxversion").unwrap_or_default().to_owned(),
        options,
        templates,
        data_model,
        sections,
    })
}

fn parse_option_set(node: &XmlNode) -> Result<ControlOptionSet, ControlError> {
    let component = match node.attribute("component") {
        Some("biber") => OptionComponent::Processor,
        Some("biblatex") => OptionComponent::Biblatex,
        Some(other) => {
            return Err(ControlError::Schema(format!(
                "unknown option component {other}"
            )));
        }
        None => return Err(ControlError::Schema("options missing component".into())),
    };
    let scope = node.attribute("type").unwrap_or_default().to_owned();
    let mut values = BTreeMap::new();
    for option in node.children_named("option") {
        let key = option
            .child("key")
            .map(XmlNode::trimmed_text)
            .unwrap_or_default()
            .to_owned();
        if key.is_empty() || values.contains_key(&key) {
            return Err(ControlError::Schema(
                "option keys must be nonempty and unique within a scope".into(),
            ));
        }
        let mut entries = option
            .children_named("value")
            .map(|value| StructuredValue {
                content: value.trimmed_text().to_owned(),
                attributes: attributes(value),
            })
            .collect::<Vec<_>>();
        entries.sort_by_key(|value| {
            value
                .attributes
                .get("order")
                .and_then(|order| order.parse::<u32>().ok())
                .unwrap_or(u32::MAX)
        });
        let value = match option.attribute("type") {
            Some("singlevalued") if entries.len() == 1 => {
                ControlOptionValue::Single(entries.remove(0))
            }
            Some("multivalued") => ControlOptionValue::Multiple(entries),
            _ => {
                return Err(ControlError::Schema(format!(
                    "invalid value count for option {key}"
                )));
            }
        };
        values.insert(key, value);
    }
    Ok(ControlOptionSet {
        component,
        scope,
        values,
    })
}

fn parse_template(node: &XmlNode) -> Template {
    Template {
        kind: node.local_name().to_owned(),
        name: node
            .attribute("name")
            .or_else(|| node.attribute("type"))
            .unwrap_or("global")
            .to_owned(),
        elements: descendants(node)
            .into_iter()
            .map(|element| TemplateElement {
                name: element.local_name().to_owned(),
                content: element.trimmed_text().to_owned(),
                attributes: attributes(element),
            })
            .collect(),
    }
}

fn parse_data_model(node: &XmlNode) -> DataModel {
    let descendants = descendants(node);
    DataModel {
        entry_types: descendants
            .iter()
            .filter(|child| child.local_name() == "entrytype")
            .map(|child| child.trimmed_text().to_owned())
            .filter(|value| !value.is_empty())
            .collect(),
        fields: descendants
            .iter()
            .filter(|child| child.local_name() == "field")
            .map(|child| DataModelField {
                name: child.trimmed_text().to_owned(),
                datatype: child.attribute("datatype").map(str::to_owned),
                attributes: attributes(child),
            })
            .filter(|field| !field.name.is_empty())
            .collect(),
        constraints: descendants
            .iter()
            .filter(|child| matches!(child.local_name(), "constraint" | "fieldor" | "fieldxor"))
            .map(|child| TemplateElement {
                name: child.local_name().to_owned(),
                content: child.trimmed_text().to_owned(),
                attributes: attributes(child),
            })
            .collect(),
    }
}

fn parse_section(node: &XmlNode) -> Result<ControlSection, ControlError> {
    let number = node
        .attribute("number")
        .unwrap_or_default()
        .parse()
        .map_err(|_| ControlError::Schema("invalid section number".into()))?;
    let descendants = descendants(node);
    Ok(ControlSection {
        number,
        citekeys: descendants
            .iter()
            .filter(|node| node.local_name() == "citekey")
            .map(|node| node.trimmed_text().to_owned())
            .collect(),
        datasources: descendants
            .iter()
            .filter(|node| node.local_name() == "datasource")
            .map(|node| node.trimmed_text().to_owned())
            .collect(),
    })
}

fn descendants(node: &XmlNode) -> Vec<&XmlNode> {
    let mut result = Vec::new();
    let mut stack = node.children.iter().rev().collect::<Vec<_>>();
    while let Some(child) = stack.pop() {
        result.push(child);
        stack.extend(child.children.iter().rev());
    }
    result
}

fn attributes(node: &XmlNode) -> BTreeMap<String, String> {
    node.attributes
        .iter()
        .map(|attribute| (attribute.name.clone(), attribute.value.clone()))
        .collect()
}
