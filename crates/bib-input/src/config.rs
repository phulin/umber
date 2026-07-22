use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use umber_vfs::{VfsSnapshot, VirtualPath};

use crate::control::{StructuredValue, Template, TemplateElement};
use crate::xml::{
    XmlError, XmlLimits, XmlNode, parse_xml, parse_xml_from_snapshot,
    parse_xml_from_snapshot_with_paths,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConfigValue {
    Scalar(String),
    Attributes(BTreeMap<String, String>),
    List(Vec<StructuredValue>),
    Tree(Vec<TemplateElement>),
}

/// Input accepted by Biber-compatible boolean option conversion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BooleanInput<'a> {
    Text(&'a str),
    Number(i64),
}

/// Representation requested from Biber-compatible boolean option conversion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BooleanOutput {
    Number,
    Text,
}

/// Result of converting a boolean configuration value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MappedBoolean {
    Number(u8),
    Text(&'static str),
}

/// Convert the textual and numeric boolean forms accepted by Biber.
#[must_use]
pub fn map_boolean(input: BooleanInput<'_>, output: BooleanOutput) -> Option<MappedBoolean> {
    let value = match input {
        BooleanInput::Text(value) if value.eq_ignore_ascii_case("true") => true,
        BooleanInput::Text(value) if value.eq_ignore_ascii_case("false") => false,
        BooleanInput::Number(1) => true,
        BooleanInput::Number(0) => false,
        BooleanInput::Text(_) | BooleanInput::Number(_) => return None,
    };
    Some(match output {
        BooleanOutput::Number => MappedBoolean::Number(u8::from(value)),
        BooleanOutput::Text => MappedBoolean::Text(if value { "true" } else { "false" }),
    })
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ConfigurationFile {
    values: BTreeMap<String, ConfigValue>,
    pub templates: Vec<Template>,
}

impl ConfigurationFile {
    #[must_use]
    pub fn value(&self, key: &str) -> Option<&ConfigValue> {
        self.values.get(key)
    }

    pub fn values(&self) -> impl ExactSizeIterator<Item = (&str, &ConfigValue)> {
        self.values.iter().map(|(key, value)| (key.as_str(), value))
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ConfigurationLayer {
    CompiledDefaults,
    ToolConfiguration,
    UserConfiguration,
    Command,
    ControlFile,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ResolvedConfiguration {
    layers: Vec<(ConfigurationLayer, BTreeMap<String, ConfigValue>)>,
}

impl ResolvedConfiguration {
    pub fn push(
        &mut self,
        layer: ConfigurationLayer,
        values: impl IntoIterator<Item = (String, ConfigValue)>,
    ) -> Result<(), ConfigError> {
        if self
            .layers
            .last()
            .is_some_and(|(previous, _)| *previous > layer)
        {
            return Err(ConfigError::Precedence);
        }
        self.layers.push((layer, values.into_iter().collect()));
        Ok(())
    }

    #[must_use]
    pub fn resolve(&self, key: &str) -> Option<&ConfigValue> {
        if key == "sourcemap" {
            return self.layers.iter().find_map(|(_, values)| values.get(key));
        }
        self.layers
            .iter()
            .rev()
            .find_map(|(_, values)| values.get(key))
    }

    #[must_use]
    pub fn merged_list(&self, key: &str) -> Vec<&StructuredValue> {
        self.layers
            .iter()
            .filter_map(|(_, values)| match values.get(key) {
                Some(ConfigValue::List(values)) => Some(values.iter()),
                _ => None,
            })
            .flatten()
            .collect()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConfigError {
    Xml(XmlError),
    Schema(String),
    Precedence,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Xml(error) => error.fmt(formatter),
            Self::Schema(message) => write!(
                formatter,
                "configuration schema validation failed: {message}"
            ),
            Self::Precedence => {
                formatter.write_str("configuration layers are out of precedence order")
            }
        }
    }
}

impl std::error::Error for ConfigError {}

impl From<XmlError> for ConfigError {
    fn from(value: XmlError) -> Self {
        Self::Xml(value)
    }
}

pub fn validate_config_bytes(bytes: &[u8], limits: XmlLimits) -> Result<(), ConfigError> {
    validate_config_tree(&parse_xml(bytes, limits)?)
}

pub fn parse_config_bytes(
    bytes: &[u8],
    limits: XmlLimits,
) -> Result<ConfigurationFile, ConfigError> {
    config_from_tree(&parse_xml(bytes, limits)?)
}

pub fn parse_config(
    snapshot: &VfsSnapshot,
    path: &VirtualPath,
    limits: XmlLimits,
) -> Result<ConfigurationFile, ConfigError> {
    config_from_tree(&parse_xml_from_snapshot(snapshot, path, limits)?)
}

pub fn parse_config_with_paths(
    snapshot: &VfsSnapshot,
    path: &VirtualPath,
    limits: XmlLimits,
) -> Result<(ConfigurationFile, BTreeSet<VirtualPath>), ConfigError> {
    let (root, paths) = parse_xml_from_snapshot_with_paths(snapshot, path, limits)?;
    Ok((config_from_tree(&root)?, paths))
}

fn validate_config_tree(root: &XmlNode) -> Result<(), ConfigError> {
    if root.name != "config" {
        return Err(ConfigError::Schema(
            "root element must be config without a namespace".into(),
        ));
    }
    if root.attribute("xmlns").is_some() || root.name.contains(':') {
        return Err(ConfigError::Schema(
            "configuration namespace is not supported by the pinned schema".into(),
        ));
    }
    for child in &root.children {
        if child.local_name().is_empty() {
            return Err(ConfigError::Schema(
                "empty configuration element name".into(),
            ));
        }
        if !child.children.is_empty() && !child.trimmed_text().is_empty() {
            return Err(ConfigError::Schema(format!(
                "mixed content is not allowed in {}",
                child.name
            )));
        }
    }
    Ok(())
}

fn config_from_tree(root: &XmlNode) -> Result<ConfigurationFile, ConfigError> {
    validate_config_tree(root)?;
    let mut values = BTreeMap::new();
    let mut templates = Vec::new();
    for child in &root.children {
        let key = child.local_name().to_owned();
        if key.ends_with("template") {
            templates.push(Template {
                kind: key,
                name: child
                    .attribute("name")
                    .or_else(|| child.attribute("type"))
                    .unwrap_or("global")
                    .to_owned(),
                elements: flatten_children(child),
            });
            continue;
        }
        let value = if child.children.is_empty() {
            ConfigValue::Scalar(child.trimmed_text().to_owned())
        } else if child
            .children
            .iter()
            .all(|node| node.local_name() == "option")
        {
            let list = child
                .children
                .iter()
                .map(|node| StructuredValue {
                    content: node
                        .attribute("value")
                        .unwrap_or_else(|| node.trimmed_text())
                        .to_owned(),
                    attributes: attrs(node),
                })
                .collect::<Vec<_>>();
            if list.len() == 1 && list[0].attributes.contains_key("name") {
                ConfigValue::Attributes(list[0].attributes.clone())
            } else {
                ConfigValue::List(list)
            }
        } else if matches!(key.as_str(), "noinits" | "nolabels" | "nosort") {
            ConfigValue::List(
                child
                    .children
                    .iter()
                    .map(|node| StructuredValue {
                        content: node
                            .attribute("value")
                            .unwrap_or_else(|| node.trimmed_text())
                            .to_owned(),
                        attributes: attrs(node),
                    })
                    .collect(),
            )
        } else {
            ConfigValue::Tree(flatten_children(child))
        };
        values.insert(key, value);
    }
    Ok(ConfigurationFile { values, templates })
}

fn flatten_children(node: &XmlNode) -> Vec<TemplateElement> {
    let mut result = Vec::new();
    let mut stack = node.children.iter().rev().collect::<Vec<_>>();
    while let Some(child) = stack.pop() {
        result.push(TemplateElement {
            name: child.local_name().to_owned(),
            content: child.trimmed_text().to_owned(),
            attributes: attrs(child),
        });
        stack.extend(child.children.iter().rev());
    }
    result
}

fn attrs(node: &XmlNode) -> BTreeMap<String, String> {
    node.attributes
        .iter()
        .map(|attribute| (attribute.name.clone(), attribute.value.clone()))
        .collect()
}
