use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum AnnotationKind {
    Field,
    Item,
    Part,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Annotation {
    pub kind: AnnotationKind,
    pub field: String,
    pub name: String,
    pub item: Option<usize>,
    pub part: Option<String>,
    pub replace: bool,
    pub value: String,
}

type AnnotationKey = (
    AnnotationKind,
    String,
    String,
    Option<usize>,
    Option<String>,
);

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AnnotationMap(BTreeMap<AnnotationKey, Annotation>);

impl AnnotationMap {
    pub fn insert(&mut self, annotation: Annotation) {
        let key = (
            annotation.kind,
            annotation.field.clone(),
            annotation.name.clone(),
            annotation.item,
            annotation.part.clone(),
        );
        if annotation.replace || !self.0.contains_key(&key) {
            self.0.insert(key, annotation);
        } else if let Some(existing) = self.0.get_mut(&key) {
            existing.value.push_str(", ");
            existing.value.push_str(&annotation.value);
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &Annotation> {
        self.0.values()
    }
}
