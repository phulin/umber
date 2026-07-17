use std::sync::Arc;

use crate::{DataListId, EntryId, EntryType, OptionId, SectionId};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OptionValue {
    Boolean(bool),
    Integer(i64),
    String(String),
    Strings(Vec<String>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OptionScope {
    CompiledDefault,
    ToolConfiguration,
    UserConfiguration,
    Command,
    ControlGlobal,
    Section(SectionId),
    EntryType(EntryType),
    Entry(EntryId),
    Name,
    List(DataListId),
}

impl OptionScope {
    fn precedence(&self) -> u8 {
        match self {
            Self::CompiledDefault => 0,
            Self::ToolConfiguration => 1,
            Self::UserConfiguration => 2,
            Self::Command => 3,
            Self::ControlGlobal => 4,
            Self::Section(_) => 5,
            Self::EntryType(_) => 6,
            Self::Entry(_) => 7,
            Self::Name => 8,
            Self::List(_) => 9,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OptionLayer {
    scope: OptionScope,
    values: Arc<[(OptionId, OptionValue)]>,
}

impl OptionLayer {
    #[must_use]
    pub const fn scope(&self) -> &OptionScope {
        &self.scope
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&OptionId, &OptionValue)> {
        self.values.iter().map(|(id, value)| (id, value))
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ScopedOptions(Arc<[OptionLayer]>);

impl ScopedOptions {
    #[must_use]
    pub fn resolve(&self, id: &OptionId) -> Option<&OptionValue> {
        self.0.iter().rev().find_map(|layer| {
            layer
                .values
                .iter()
                .find(|(candidate, _)| candidate == id)
                .map(|(_, value)| value)
        })
    }

    pub fn layers(&self) -> impl ExactSizeIterator<Item = &OptionLayer> {
        self.0.iter()
    }
}

#[derive(Clone, Debug, Default)]
pub struct ScopedOptionsBuilder {
    layers: Vec<OptionLayer>,
}

impl ScopedOptionsBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_layer(
        &mut self,
        scope: OptionScope,
        values: impl IntoIterator<Item = (OptionId, OptionValue)>,
    ) -> Result<&mut Self, &'static str> {
        if self
            .layers
            .last()
            .is_some_and(|layer| layer.scope.precedence() > scope.precedence())
        {
            return Err("option layers must be added in nondecreasing precedence order");
        }
        let values = values.into_iter().collect::<Vec<_>>();
        for (index, (id, _)) in values.iter().enumerate() {
            if values[..index].iter().any(|(existing, _)| existing == id) {
                return Err("an option layer cannot contain duplicate option identifiers");
            }
        }
        self.layers.push(OptionLayer {
            scope,
            values: values.into(),
        });
        Ok(self)
    }

    #[must_use]
    pub fn freeze(self) -> ScopedOptions {
        ScopedOptions(self.layers.into())
    }
}
