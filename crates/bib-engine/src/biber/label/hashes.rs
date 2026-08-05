use bib_model::{Name, NameList};
use bib_unicode::{compatibility_hash, normalise_nfc, normalise_string_hash};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NameHashes {
    pub name_hash: String,
    pub full_hash: String,
    pub full_hash_raw: String,
    pub per_name: Vec<String>,
}

#[must_use]
pub fn hash_name(name: &Name, initials_only: bool) -> String {
    if let Some(id) = name.hash_id() {
        return compatibility_hash(&normalise_nfc(id));
    }
    compatibility_hash(&name_identity(name, initials_only, false))
}

#[must_use]
pub fn hash_name_list(names: &NameList, visible: usize) -> NameHashes {
    let visible = visible.min(names.len());
    let mut short = String::new();
    let mut full = String::new();
    let mut raw = String::new();
    let mut per_name = Vec::with_capacity(names.len());
    for (index, name) in names.iter().enumerate() {
        let initial = name_identity(name, true, false);
        let complete = name_identity(name, false, false);
        let unnormalised = name_identity(name, false, true);
        if index < visible {
            short.push_str(&initial);
        }
        full.push_str(&complete);
        raw.push_str(&unnormalised);
        per_name.push(compatibility_hash(&complete));
    }
    if names.has_others() {
        short.push_str("others");
        full.push_str("others");
        raw.push_str("others");
    }
    NameHashes {
        name_hash: compatibility_hash(&short),
        full_hash: compatibility_hash(&full),
        full_hash_raw: compatibility_hash(&raw),
        per_name,
    }
}

fn name_identity(name: &Name, initials_only: bool, raw: bool) -> String {
    let mut value = String::new();
    for part in [name.prefix(), name.family(), name.suffix(), name.given()] {
        let Some(part) = part else { continue };
        let initial_storage;
        let part = if initials_only {
            let initials = part.initials().collect::<String>();
            initial_storage = initials;
            if initial_storage.is_empty() {
                part.value().as_str()
            } else {
                &initial_storage
            }
        } else {
            part.value().as_str()
        };
        if raw {
            value.push_str(part);
        } else {
            value.push_str(&normalise_string_hash(part));
        }
    }
    normalise_nfc(&value)
}
