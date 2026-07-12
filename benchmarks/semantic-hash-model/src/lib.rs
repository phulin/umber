use ahash::RandomState;
use std::collections::HashMap;

const MIX: u64 = 0x9e37_79b9_7f4a_7c15;
const STATE_A: u64 = 0x6a09_e667_f3bc_c909;
const STATE_B: u64 = 0xbb67_ae85_84ca_a73b;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CsKind {
    Named,
    Active,
}

#[derive(Clone, Copy, Debug)]
pub enum Token {
    Char { scalar: u32, catcode: u8 },
    Cs(u32),
    Param(u8),
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SemanticId(pub u64, pub u64);

#[derive(Clone)]
pub struct Workload {
    names: Vec<(CsKind, String)>,
    lists: Vec<Vec<BlueprintToken>>,
    boundaries: Vec<Vec<usize>>,
}

#[derive(Clone, Copy)]
enum BlueprintToken {
    Char { scalar: u32, catcode: u8 },
    Cs(usize),
    Param(u8),
}

impl Workload {
    pub fn realistic() -> Self {
        const SYMBOLS: usize = 512;
        const LISTS: usize = 4_096;
        const BOUNDARIES: usize = 2_000;
        const ROOTS_PER_BOUNDARY: usize = 12;

        let names = (0..SYMBOLS)
            .map(|index| {
                let kind = if index % 31 == 0 {
                    CsKind::Active
                } else {
                    CsKind::Named
                };
                let name = match index {
                    0 => "def".to_owned(),
                    1 => "count".to_owned(),
                    2 => "advance".to_owned(),
                    3 => "ifnum".to_owned(),
                    4 => "expandafter".to_owned(),
                    5 => "the".to_owned(),
                    _ => format!("document-control-sequence-{index}"),
                };
                (kind, name)
            })
            .collect();

        let lengths = [8, 16, 32, 64, 128, 512];
        let lists = (0..LISTS)
            .map(|list| {
                let len = lengths[(mix_index(list) as usize) % lengths.len()];
                (0..len)
                    .map(|offset| {
                        let selector = mix_index(list * 1_009 + offset) % 100;
                        match selector {
                            0..=56 => BlueprintToken::Char {
                                scalar: u32::from(b'a' + ((list + offset) % 26) as u8),
                                catcode: 11,
                            },
                            57..=93 => {
                                let hot_or_cold = mix_index(list + offset * 17);
                                let symbol = if !hot_or_cold.is_multiple_of(5) {
                                    (hot_or_cold as usize) % 48
                                } else {
                                    (hot_or_cold as usize) % SYMBOLS
                                };
                                BlueprintToken::Cs(symbol)
                            }
                            _ => BlueprintToken::Param(((offset % 9) + 1) as u8),
                        }
                    })
                    .collect()
            })
            .collect();

        let boundaries = (0..BOUNDARIES)
            .map(|boundary| {
                (0..ROOTS_PER_BOUNDARY)
                    .map(|slot| {
                        if slot < 9 {
                            (boundary * 13 + slot * 29) % 256
                        } else {
                            (boundary * 97 + slot * 193) % LISTS
                        }
                    })
                    .collect()
            })
            .collect();

        Self {
            names,
            lists,
            boundaries,
        }
    }

    pub fn boundary_count(&self) -> usize {
        self.boundaries.len()
    }
}

pub struct CurrentSystem {
    symbols: Vec<SymbolRecord>,
    symbol_slots: HashMap<u32, usize, RandomState>,
    lists: Vec<Vec<Token>>,
}

struct SymbolRecord {
    global: u32,
    kind: CsKind,
    name: String,
}

impl CurrentSystem {
    pub fn build(workload: &Workload, reverse_symbols: bool) -> Self {
        let (symbols, symbol_slots, semantic_to_global) =
            build_symbols(&workload.names, reverse_symbols);
        let lists = build_lists(&workload.lists, &semantic_to_global);
        Self {
            symbols,
            symbol_slots,
            lists,
        }
    }

    pub fn run_session(&self, workload: &Workload) -> SemanticId {
        let mut chain = SemanticHasher::new(0x6375_7272_656e_745f);
        for roots in &workload.boundaries {
            chain.id(self.hash_boundary(roots));
        }
        chain.finish()
    }

    pub fn hash_boundary_at(&self, workload: &Workload, index: usize) -> SemanticId {
        self.hash_boundary(&workload.boundaries[index])
    }

    fn hash_boundary(&self, roots: &[usize]) -> SemanticId {
        let mut hasher = SemanticHasher::new(0x626f_756e_6461_7279);
        hasher.word(roots.len() as u64);
        for &root in roots {
            hash_list_current(
                &self.lists[root],
                &self.symbols,
                &self.symbol_slots,
                &mut hasher,
            );
        }
        hasher.finish()
    }
}

pub struct PromotedSystem {
    symbol_atoms: Vec<SemanticId>,
    lists: Vec<Vec<Token>>,
    promoted: Vec<Option<SemanticId>>,
}

impl PromotedSystem {
    pub fn build(workload: &Workload, reverse_symbols: bool) -> Self {
        let (symbols, _, semantic_to_global) = build_symbols(&workload.names, reverse_symbols);
        let mut symbol_atoms = vec![SemanticId(0, 0); symbols.len()];
        for record in symbols {
            symbol_atoms[record.global as usize] = hash_symbol(record.kind, &record.name);
        }
        let lists = build_lists(&workload.lists, &semantic_to_global);
        let promoted = vec![None; lists.len()];
        Self {
            symbol_atoms,
            lists,
            promoted,
        }
    }

    pub fn run_session(&mut self, workload: &Workload) -> SemanticId {
        let mut chain = SemanticHasher::new(0x7072_6f6d_6f74_6564);
        for roots in &workload.boundaries {
            chain.id(self.hash_boundary(roots));
        }
        chain.finish()
    }

    pub fn promote_all_session_roots(&mut self, workload: &Workload) {
        for roots in &workload.boundaries {
            for &root in roots {
                self.promote(root);
            }
        }
    }

    pub fn hash_boundary_at(&mut self, workload: &Workload, index: usize) -> SemanticId {
        self.hash_boundary(&workload.boundaries[index])
    }

    fn hash_boundary(&mut self, roots: &[usize]) -> SemanticId {
        let mut hasher = SemanticHasher::new(0x626f_756e_6461_7279);
        hasher.word(roots.len() as u64);
        for &root in roots {
            hasher.id(self.promote(root));
        }
        hasher.finish()
    }

    fn promote(&mut self, id: usize) -> SemanticId {
        if let Some(identity) = self.promoted[id] {
            return identity;
        }
        let mut hasher = SemanticHasher::new(0x746f_6b65_6e5f_6c73);
        hasher.word(self.lists[id].len() as u64);
        for &token in &self.lists[id] {
            match token {
                Token::Char { scalar, catcode } => {
                    hasher.word(0);
                    hasher.word(u64::from(scalar));
                    hasher.word(u64::from(catcode));
                }
                Token::Cs(symbol) => {
                    hasher.word(1);
                    hasher.id(self.symbol_atoms[symbol as usize]);
                }
                Token::Param(slot) => {
                    hasher.word(2);
                    hasher.word(u64::from(slot));
                }
            }
        }
        let identity = hasher.finish();
        self.promoted[id] = Some(identity);
        identity
    }
}

fn build_symbols(
    names: &[(CsKind, String)],
    reverse: bool,
) -> (
    Vec<SymbolRecord>,
    HashMap<u32, usize, RandomState>,
    Vec<u32>,
) {
    let mut order = (0..names.len()).collect::<Vec<_>>();
    if reverse {
        order.reverse();
    }
    let mut records = Vec::with_capacity(names.len());
    let mut slots = HashMap::with_capacity_and_hasher(names.len(), RandomState::new());
    let mut semantic_to_global = vec![0; names.len()];
    for (global, semantic) in order.into_iter().enumerate() {
        let (kind, name) = &names[semantic];
        semantic_to_global[semantic] = global as u32;
        slots.insert(global as u32, records.len());
        records.push(SymbolRecord {
            global: global as u32,
            kind: *kind,
            name: name.clone(),
        });
    }
    (records, slots, semantic_to_global)
}

fn build_lists(lists: &[Vec<BlueprintToken>], symbols: &[u32]) -> Vec<Vec<Token>> {
    lists
        .iter()
        .map(|list| {
            list.iter()
                .map(|token| match *token {
                    BlueprintToken::Char { scalar, catcode } => Token::Char { scalar, catcode },
                    BlueprintToken::Cs(symbol) => Token::Cs(symbols[symbol]),
                    BlueprintToken::Param(slot) => Token::Param(slot),
                })
                .collect()
        })
        .collect()
}

fn hash_list_current(
    tokens: &[Token],
    symbols: &[SymbolRecord],
    slots: &HashMap<u32, usize, RandomState>,
    hasher: &mut SemanticHasher,
) {
    hasher.word(tokens.len() as u64);
    for &token in tokens {
        match token {
            Token::Char { scalar, catcode } => {
                hasher.word(0);
                hasher.word(u64::from(scalar));
                hasher.word(u64::from(catcode));
            }
            Token::Cs(symbol) => {
                let record = &symbols[slots[&symbol]];
                hasher.word(1);
                hasher.word(match record.kind {
                    CsKind::Named => 0,
                    CsKind::Active => 1,
                });
                hasher.bytes(record.name.as_bytes());
            }
            Token::Param(slot) => {
                hasher.word(2);
                hasher.word(u64::from(slot));
            }
        }
    }
}

fn hash_symbol(kind: CsKind, name: &str) -> SemanticId {
    let mut hasher = SemanticHasher::new(0x6373_5f61_746f_6d5f);
    hasher.word(match kind {
        CsKind::Named => 0,
        CsKind::Active => 1,
    });
    hasher.bytes(name.as_bytes());
    hasher.finish()
}

struct SemanticHasher {
    a: u64,
    b: u64,
}

impl SemanticHasher {
    fn new(domain: u64) -> Self {
        Self {
            a: STATE_A ^ domain,
            b: STATE_B ^ domain.rotate_left(29),
        }
    }

    fn word(&mut self, value: u64) {
        self.a = splitmix64(self.a ^ value.wrapping_add(MIX));
        self.b = splitmix64(self.b ^ value.rotate_left(23).wrapping_add(MIX));
    }

    fn bytes(&mut self, bytes: &[u8]) {
        self.word(bytes.len() as u64);
        for chunk in bytes.chunks(8) {
            let mut word = 0;
            for (offset, byte) in chunk.iter().copied().enumerate() {
                word |= u64::from(byte) << (offset * 8);
            }
            self.word(word);
        }
    }

    fn id(&mut self, identity: SemanticId) {
        self.word(identity.0);
        self.word(identity.1);
    }

    fn finish(self) -> SemanticId {
        SemanticId(splitmix64(self.a), splitmix64(self.b))
    }
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(MIX);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn mix_index(value: usize) -> u64 {
    let state = RandomState::with_seeds(1, 2, 3, 4);
    state.hash_one(value)
}

#[cfg(test)]
mod tests {
    use super::{CurrentSystem, PromotedSystem, Workload};

    #[test]
    fn both_models_are_symbol_allocation_order_independent() {
        let workload = Workload::realistic();
        let current_forward = CurrentSystem::build(&workload, false).run_session(&workload);
        let current_reverse = CurrentSystem::build(&workload, true).run_session(&workload);
        assert_eq!(current_forward, current_reverse);

        let promoted_forward = PromotedSystem::build(&workload, false).run_session(&workload);
        let promoted_reverse = PromotedSystem::build(&workload, true).run_session(&workload);
        assert_eq!(promoted_forward, promoted_reverse);
    }
}
