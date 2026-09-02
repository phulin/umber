use super::{
    PdfAnnotationAction, PdfDestinationNameTreeChildren, PdfDestinationPage,
    PdfDestinationStructure, PdfDestinationTarget, PdfDictionary, PdfDocument, PdfIndirectObject,
    PdfName, PdfObject, PdfObjectId, PdfValue, UnvalidatedPdfDocument,
};

/// Read-only, canonically ordered access to a detached PDF graph.
pub(super) struct PdfGraphView<'a> {
    document: &'a UnvalidatedPdfDocument,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PdfObjectRole {
    Catalog,
    Info,
    Page,
    Ordinary,
}

pub(super) struct PdfGraphObject<'a> {
    pub(super) id: PdfObjectId,
    pub(super) object: &'a PdfObject,
    pub(super) role: PdfObjectRole,
}

pub(super) enum PdfValueEvent<'a> {
    Value { value: &'a PdfValue, depth: usize },
    DictionaryKey(&'a PdfName),
    DictionaryRaw(&'a [u8]),
}

pub(super) struct PdfValueCursor<'a> {
    stack: Vec<ValueCursorEntry<'a>>,
}

enum ValueCursorEntry<'a> {
    Value(&'a PdfValue, usize),
    DictionaryKey(&'a PdfName),
    DictionaryRaw(&'a [u8]),
}

impl<'a> PdfGraphView<'a> {
    pub(super) fn unvalidated(document: &'a UnvalidatedPdfDocument) -> Self {
        Self { document }
    }

    pub(super) fn validated(document: &'a PdfDocument) -> Self {
        Self {
            document: &document.0,
        }
    }

    pub(super) fn objects(
        &self,
    ) -> impl DoubleEndedIterator<Item = PdfGraphObject<'a>> + ExactSizeIterator + '_ {
        self.document.objects.iter().map(|indirect| PdfGraphObject {
            id: indirect.id,
            object: &indirect.object,
            role: self.role(indirect),
        })
    }

    pub(super) fn contains(&self, id: PdfObjectId) -> bool {
        self.document
            .objects
            .binary_search_by_key(&id, |object| object.id)
            .is_ok()
    }

    pub(super) fn object(&self, id: PdfObjectId) -> Option<&'a PdfObject> {
        self.document
            .objects
            .binary_search_by_key(&id, |object| object.id)
            .ok()
            .map(|index| &self.document.objects[index].object)
    }

    pub(super) fn dictionary(&self, id: PdfObjectId) -> Option<&'a PdfDictionary> {
        match self.object(id) {
            Some(PdfObject::Value(PdfValue::Dictionary(dictionary))) => Some(dictionary),
            _ => None,
        }
    }

    pub(super) fn values(&self, object: &'a PdfObject) -> PdfValueCursor<'a> {
        PdfValueCursor::object(object)
    }

    pub(super) fn last_id(&self) -> Option<PdfObjectId> {
        self.document.objects.last().map(|object| object.id)
    }

    fn role(&self, indirect: &PdfIndirectObject) -> PdfObjectRole {
        if indirect.id == self.document.catalog {
            PdfObjectRole::Catalog
        } else if self.document.trailer.info == Some(indirect.id) {
            PdfObjectRole::Info
        } else if matches!(
            &indirect.object,
            PdfObject::Value(PdfValue::Dictionary(dictionary))
                if is_type(dictionary, b"Page")
        ) {
            PdfObjectRole::Page
        } else {
            PdfObjectRole::Ordinary
        }
    }
}

impl<'a> PdfGraphObject<'a> {
    /// References represented by typed objects rather than nested `PdfValue`s.
    pub(super) fn typed_references(&self) -> Vec<PdfObjectId> {
        let mut references = Vec::new();
        match self.object {
            PdfObject::Value(_)
            | PdfObject::Raw(_)
            | PdfObject::Stream { .. }
            | PdfObject::EncodedStream { .. }
            | PdfObject::FormXObject { .. }
            | PdfObject::PdfStringSyntax(_) => {}
            PdfObject::ImageXObject { image, .. } => references.extend(image.soft_mask),
            PdfObject::Annotation(annotation) => {
                if let Some(action) = &annotation.action {
                    action_references(action, &mut references);
                }
            }
            PdfObject::Destination(destination) | PdfObject::NamedDestination(destination) => {
                references.push(destination.page);
            }
            PdfObject::DestinationNameTree(tree) => match &tree.children {
                PdfDestinationNameTreeChildren::Names(entries) => {
                    references.extend(entries.iter().map(|(_, id)| *id));
                }
                PdfDestinationNameTreeChildren::Kids(kids) => {
                    references.extend(kids.iter().copied());
                }
            },
            PdfObject::Names(names) => references.extend(names.destinations),
            PdfObject::Action(action) => action_references(action, &mut references),
            PdfObject::Outline(outline) => references.extend([outline.first, outline.last]),
            PdfObject::OutlineItem(item) => references.extend(
                [
                    Some(item.title),
                    Some(item.action),
                    Some(item.parent),
                    item.previous,
                    item.next,
                    item.first,
                    item.last,
                ]
                .into_iter()
                .flatten(),
            ),
            PdfObject::ThreadList(threads) => references.extend(threads.iter().copied()),
            PdfObject::Thread(thread) => references.push(thread.first_bead),
            PdfObject::Bead(bead) => references.extend(
                [
                    bead.thread,
                    Some(bead.previous),
                    Some(bead.next),
                    Some(bead.page),
                    Some(bead.rectangle),
                ]
                .into_iter()
                .flatten(),
            ),
        }
        references
    }
}

impl<'a> PdfValueCursor<'a> {
    pub(super) fn value(value: &'a PdfValue) -> Self {
        Self {
            stack: vec![ValueCursorEntry::Value(value, 1)],
        }
    }

    pub(super) fn dictionary(dictionary: &'a PdfDictionary) -> Self {
        let mut stack = Vec::new();
        push_dictionary(&mut stack, dictionary, 1);
        Self { stack }
    }

    fn object(object: &'a PdfObject) -> Self {
        match object {
            PdfObject::Value(value) => Self::value(value),
            PdfObject::Stream { dictionary, .. }
            | PdfObject::EncodedStream { dictionary, .. }
            | PdfObject::FormXObject { dictionary, .. }
            | PdfObject::ImageXObject { dictionary, .. } => Self::dictionary(dictionary),
            PdfObject::Annotation(_)
            | PdfObject::Destination(_)
            | PdfObject::NamedDestination(_)
            | PdfObject::DestinationNameTree(_)
            | PdfObject::Names(_)
            | PdfObject::Action(_)
            | PdfObject::PdfStringSyntax(_)
            | PdfObject::Outline(_)
            | PdfObject::OutlineItem(_)
            | PdfObject::ThreadList(_)
            | PdfObject::Thread(_)
            | PdfObject::Bead(_)
            | PdfObject::Raw(_) => Self { stack: Vec::new() },
        }
    }
}

impl<'a> Iterator for PdfValueCursor<'a> {
    type Item = PdfValueEvent<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let entry = self.stack.pop()?;
        Some(match entry {
            ValueCursorEntry::Value(value, depth) => {
                match value {
                    PdfValue::Array(values) => {
                        self.stack.extend(
                            values
                                .iter()
                                .rev()
                                .map(|value| ValueCursorEntry::Value(value, depth + 1)),
                        );
                    }
                    PdfValue::Dictionary(dictionary) => {
                        push_dictionary(&mut self.stack, dictionary, depth + 1);
                    }
                    PdfValue::Null
                    | PdfValue::Bool(_)
                    | PdfValue::Integer(_)
                    | PdfValue::Number(_)
                    | PdfValue::Name(_)
                    | PdfValue::String(_)
                    | PdfValue::Reference(_) => {}
                }
                PdfValueEvent::Value { value, depth }
            }
            ValueCursorEntry::DictionaryKey(key) => PdfValueEvent::DictionaryKey(key),
            ValueCursorEntry::DictionaryRaw(raw) => PdfValueEvent::DictionaryRaw(raw),
        })
    }
}

fn push_dictionary<'a>(
    stack: &mut Vec<ValueCursorEntry<'a>>,
    dictionary: &'a PdfDictionary,
    depth: usize,
) {
    stack.push(ValueCursorEntry::DictionaryRaw(dictionary.raw_entries()));
    for (key, value) in dictionary.iter().rev() {
        stack.push(ValueCursorEntry::Value(value, depth));
        stack.push(ValueCursorEntry::DictionaryKey(key));
    }
}

fn action_references(action: &PdfAnnotationAction, references: &mut Vec<PdfObjectId>) {
    let PdfAnnotationAction::Destination(action) = action else {
        return;
    };
    match action.target {
        PdfDestinationTarget::Reference(id)
        | PdfDestinationTarget::Page {
            page: PdfDestinationPage::Internal(id),
            ..
        } => references.push(id),
        PdfDestinationTarget::Page {
            page: PdfDestinationPage::External(_),
            ..
        }
        | PdfDestinationTarget::Name(_)
        | PdfDestinationTarget::Number(_) => {}
    }
    if let Some(PdfDestinationStructure::Internal(id)) = action.structure {
        references.push(id);
    }
}

fn is_type(dictionary: &PdfDictionary, expected: &[u8]) -> bool {
    matches!(dictionary.get(b"Type"), Some(PdfValue::Name(name)) if name.as_bytes() == expected)
}
