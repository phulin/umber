use crate::model::{Library, LibrarySymbols, SymbolEntry, SymbolSidecar};

pub(crate) struct Symbolizer {
    sidecar: SymbolSidecar,
}

impl Symbolizer {
    pub(crate) const fn new(sidecar: SymbolSidecar) -> Self {
        Self { sidecar }
    }

    pub(crate) fn resolve(&self, library: &Library, address: u64) -> Option<Vec<String>> {
        let symbols = self.sidecar.data.iter().find(|symbols| {
            symbols.debug_name == library.debug_name
                && code_ids_match(symbols.code_id.as_deref(), library.code_id.as_deref())
        })?;
        let entry = find_entry(symbols, address)?;
        let indexes = entry
            .frames
            .as_ref()
            .filter(|frames| !frames.is_empty())
            .map_or_else(
                || vec![entry.symbol],
                |frames| frames.iter().map(|frame| frame.function).collect(),
            );
        indexes
            .into_iter()
            .map(|index| self.sidecar.string_table.get(index).cloned())
            .collect()
    }
}

fn code_ids_match(left: Option<&str>, right: Option<&str>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left.eq_ignore_ascii_case(right),
        _ => true,
    }
}

fn find_entry(symbols: &LibrarySymbols, address: u64) -> Option<&SymbolEntry> {
    if let Ok(index) = symbols
        .known_addresses
        .binary_search_by_key(&address, |(known, _)| *known)
    {
        return symbols.symbol_table.get(symbols.known_addresses[index].1);
    }
    let index = symbols
        .symbol_table
        .partition_point(|entry| entry.rva <= address)
        .checked_sub(1)?;
    let entry = &symbols.symbol_table[index];
    (address < entry.rva.saturating_add(entry.size)).then_some(entry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{SymbolFrame, SymbolSidecar};

    #[test]
    fn resolves_known_address_and_expands_inline_frames() {
        let symbolizer = Symbolizer::new(SymbolSidecar {
            string_table: vec!["outer".into(), "inner".into()],
            data: vec![LibrarySymbols {
                debug_name: "app".into(),
                code_id: Some("ABC".into()),
                symbol_table: vec![SymbolEntry {
                    rva: 100,
                    size: 20,
                    symbol: 0,
                    frames: Some(vec![
                        SymbolFrame { function: 1 },
                        SymbolFrame { function: 0 },
                    ]),
                }],
                known_addresses: vec![(105, 0)],
            }],
        });
        let library = Library {
            name: "app".into(),
            debug_name: "app".into(),
            code_id: Some("abc".into()),
        };
        assert_eq!(
            symbolizer.resolve(&library, 105),
            Some(vec!["inner".into(), "outer".into()])
        );
    }
}
