use super::*;
use serde::{Deserialize, Serialize};

mod node;
use node::FormatNode;

mod font_validation;
#[cfg(test)]
mod tests;
#[cfg(test)]
pub(crate) use font_validation::{TestingFontFormatCorruption, testing_corrupt_font_format};

#[derive(Debug)]
pub(crate) enum StoreFormatError {
    OpenGroups(u32),
    Codec(String),
    Invalid(&'static str),
    InvalidFontMetrics {
        font: usize,
        source: FontMetricsValidationError,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct StoreFormat {
    names: Vec<FormatName>,
    token_lists: Vec<Vec<FormatToken>>,
    macros: Vec<FormatMacro>,
    glue: Vec<FormatGlue>,
    fonts: Vec<FormatFont>,
    node_lists: Vec<FormatNodeList>,
    env: Vec<FormatEnvEntry>,
    code_tables: Vec<FormatCodeTables>,
    hyphenation: HyphenationTable,
    prepared_mag: Option<i32>,
    last_loaded_font: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct FormatEnvEntry {
    cell: u64,
    value: FormatEnvValue,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
enum FormatEnvValue {
    Raw(u64),
    Box(FormatListKey),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct FormatName {
    active: bool,
    text: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
enum FormatToken {
    Char { ch: char, cat: u8 },
    Cs(u32),
    Param(u8),
    Frozen(u8),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct FormatMacro {
    flags: u8,
    parameter_text: u32,
    replacement_text: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct FormatGlue {
    width: i32,
    stretch: i32,
    stretch_order: u8,
    shrink: i32,
    shrink_order: u8,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct FormatFont {
    name: String,
    path: std::path::PathBuf,
    content_hash: [u8; 32],
    checksum: u32,
    design_size: i32,
    size: i32,
    parameters: Vec<i32>,
    characters: Vec<Option<tex_fonts::CharMetrics>>,
    lig_kern_program: Vec<tex_fonts::LigKernInstruction>,
    right_boundary_char: Option<u8>,
    left_boundary_program: Option<u16>,
    extensible_recipes: Vec<tex_fonts::metrics::ExtensibleRecipe>,
    identifier: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct FormatCodeTables {
    code: u32,
    catcode: u8,
    lccode: u32,
    uccode: u32,
    sfcode: u16,
    mathcode: u32,
    delcode: i32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
struct FormatListKey {
    survivor_root: Option<u32>,
    start: u32,
    len: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct FormatNodeList {
    key: FormatListKey,
    nodes: Vec<FormatNode>,
}

fn capture_env_word(
    stores: &Stores,
    cell: crate::cell::CellId,
    word: u64,
) -> (crate::cell::CellId, u64) {
    let index = if cell.bank() == crate::cell::BankTag::Meaning {
        stores
            .resolve_stored_symbol(Symbol::new(cell.index()))
            .raw()
    } else {
        cell.index()
    };
    let cell = crate::cell::CellId::new(cell.bank(), index);
    let word = if cell.bank() == crate::cell::BankTag::CurrentFont {
        let symbol_plus_one = word >> 32;
        let symbol = if symbol_plus_one == 0 {
            0
        } else {
            u64::from(
                stores
                    .resolve_stored_symbol(Symbol::new((symbol_plus_one - 1) as u32))
                    .raw(),
            ) + 1
        };
        (symbol << 32) | u64::from(word as u32)
    } else {
        word
    };
    (cell, word)
}

fn restore_current_font_word(stores: &Stores, word: u64) -> Result<u64, StoreFormatError> {
    let symbol_plus_one = word >> 32;
    let symbol = if symbol_plus_one == 0 {
        0
    } else {
        let slot = u32::try_from(symbol_plus_one - 1)
            .map_err(|_| StoreFormatError::Invalid("current-font identifier is not live"))?;
        u64::from(
            stores
                .interner
                .symbol_at_slot(slot)
                .ok_or(StoreFormatError::Invalid(
                    "current-font identifier is not live",
                ))?
                .raw(),
        ) + 1
    };
    Ok((symbol << 32) | u64::from(word as u32))
}

impl Stores {
    pub(crate) fn encode_format(&self) -> Result<Vec<u8>, StoreFormatError> {
        if self.env.group_depth() != 0 {
            return Err(StoreFormatError::OpenGroups(self.env.group_depth()));
        }
        let format = StoreFormat::capture(self)?;
        bincode::serialize(&format).map_err(|error| StoreFormatError::Codec(error.to_string()))
    }

    pub(crate) fn decode_format(bytes: &[u8]) -> Result<Self, StoreFormatError> {
        let format: StoreFormat = bincode::deserialize(bytes)
            .map_err(|error| StoreFormatError::Codec(error.to_string()))?;
        format.restore()
    }
}

impl StoreFormat {
    fn capture(stores: &Stores) -> Result<Self, StoreFormatError> {
        let names = (0..stores.interner.len())
            .map(|raw| {
                let symbol = stores
                    .interner
                    .symbol_at_slot(raw as u32)
                    .expect("captured interner slot should be live");
                FormatName {
                    active: stores.interner.kind(symbol) == ControlSequenceKind::ActiveCharacter,
                    text: stores.interner.resolve(symbol).to_owned(),
                }
            })
            .collect();
        let token_mark = stores.tokens.watermark();
        let token_lists = (0..token_mark.spans)
            .map(|raw| {
                stores
                    .tokens
                    .get(stores.resolve_stored_token_list(TokenListId::new(raw)))
                    .iter()
                    .copied()
                    .map(|token| FormatToken::capture(stores, token))
                    .collect()
            })
            .collect();
        let macro_mark = stores.macros.watermark();
        let macros = (0..macro_mark.definitions)
            .map(|raw| {
                let meaning = stores.macros.get(
                    stores
                        .macros
                        .resolve_stored(MacroDefinitionId::new(raw))
                        .expect("captured macro slot should be live"),
                );
                FormatMacro {
                    flags: meaning.flags().bits(),
                    parameter_text: meaning.parameter_text().raw(),
                    replacement_text: meaning.replacement_text().raw(),
                }
            })
            .collect();
        let glue_mark = stores.glue.watermark();
        let glue = (0..glue_mark.specs)
            .map(|raw| {
                FormatGlue::capture(
                    stores
                        .glue
                        .get(stores.resolve_stored_glue(GlueId::new(raw))),
                )
            })
            .collect();
        let font_mark = stores.fonts.watermark();
        let fonts = (0..font_mark.len)
            .map(|raw| {
                FormatFont::capture(&stores.fonts, stores.resolve_stored_font(FontId::new(raw)))
            })
            .collect();
        let mut env_words = Vec::new();
        stores.env.for_each_semantic_non_default_word(|cell, word| {
            env_words.push(capture_env_word(stores, cell, word));
        });
        let roots: Vec<_> = env_words
            .iter()
            .filter_map(|&(cell, word)| {
                (cell.bank() == crate::cell::BankTag::Box)
                    .then(|| NodeListId::decode_box_word(word))
                    .flatten()
            })
            .collect();
        let mut seen = std::collections::BTreeSet::new();
        let mut visiting = std::collections::BTreeSet::new();
        let mut survivor_roots = std::collections::BTreeMap::new();
        let mut node_lists = Vec::new();
        for root in roots {
            capture_node_list(
                stores,
                root,
                &mut seen,
                &mut visiting,
                &mut survivor_roots,
                &mut node_lists,
            )?;
        }
        let env = env_words
            .into_iter()
            .map(|(cell, word)| {
                let value = if cell.bank() == crate::cell::BankTag::Box {
                    let id = NodeListId::decode_box_word(word)
                        .expect("non-default box format entry should contain a list");
                    FormatEnvValue::Box(FormatListKey::capture(stores, id, &mut survivor_roots))
                } else {
                    FormatEnvValue::Raw(word)
                };
                FormatEnvEntry {
                    cell: cell.raw(),
                    value,
                }
            })
            .collect();
        let code_tables = (0..=255)
            .map(|code| {
                let ch = char::from_u32(code).expect("byte code is scalar");
                FormatCodeTables {
                    code,
                    catcode: stores.code_tables.catcode(ch) as u8,
                    lccode: stores.code_tables.lccode(ch),
                    uccode: stores.code_tables.uccode(ch),
                    sfcode: stores.code_tables.sfcode(ch),
                    mathcode: stores.code_tables.mathcode(ch),
                    delcode: stores.code_tables.delcode(ch),
                }
            })
            .collect();
        Ok(Self {
            names,
            token_lists,
            macros,
            glue,
            fonts,
            node_lists,
            env,
            code_tables,
            hyphenation: (*stores.hyphenation).clone(),
            prepared_mag: stores.prepared_mag,
            last_loaded_font: stores.last_loaded_font.raw(),
        })
    }

    fn restore(self) -> Result<Stores, StoreFormatError> {
        self.validate_font_state()?;
        let mut stores = Stores::new();
        for (raw, name) in self.names.into_iter().enumerate() {
            let symbol = if name.active {
                let mut chars = name.text.chars();
                let ch = chars
                    .next()
                    .ok_or(StoreFormatError::Invalid("empty active name"))?;
                if chars.next().is_some() {
                    return Err(StoreFormatError::Invalid("multi-character active name"));
                }
                stores.interner.intern_active(ch)
            } else {
                stores.interner.intern(&name.text)
            }
            .map_err(|_| StoreFormatError::Invalid("symbol capacity"))?;
            if symbol.raw() as usize != raw {
                return Err(StoreFormatError::Invalid("non-canonical symbol order"));
            }
        }
        for (raw, tokens) in self.token_lists.into_iter().enumerate().skip(1) {
            let tokens = tokens
                .into_iter()
                .map(|token| token.restore(&stores.interner))
                .collect::<Result<Vec<_>, _>>()?;
            if stores.tokens.intern(&tokens).raw() as usize != raw {
                return Err(StoreFormatError::Invalid("non-canonical token-list order"));
            }
        }
        for (raw, definition) in self.macros.into_iter().enumerate() {
            let meaning = MacroMeaning::new(
                crate::meaning::MeaningFlags::from_bits(definition.flags),
                stores.resolve_stored_token_list(TokenListId::new(definition.parameter_text)),
                stores.resolve_stored_token_list(TokenListId::new(definition.replacement_text)),
            );
            if stores.macros.intern_with_provenance(meaning, None).raw() as usize != raw {
                return Err(StoreFormatError::Invalid("macro order"));
            }
        }
        for (raw, glue) in self.glue.into_iter().enumerate().skip(1) {
            if stores.glue.intern(glue.restore()?).raw() as usize != raw {
                return Err(StoreFormatError::Invalid("non-canonical glue order"));
            }
        }
        for (raw, font) in self.fonts.into_iter().enumerate() {
            let identifier = font.identifier;
            let id = if raw == 0 {
                NULL_FONT
            } else {
                let id = stores
                    .fonts
                    .intern(font.restore())
                    .map_err(|_| StoreFormatError::Invalid("font count exceeds bank capacity"))?;
                if id.raw() as usize != raw {
                    return Err(StoreFormatError::Invalid("non-canonical font order"));
                }
                id
            };
            if let Some(symbol) = identifier {
                stores.fonts.set_identifier(
                    id,
                    stores
                        .interner
                        .symbol_at_slot(symbol)
                        .and_then(|symbol| stores.interner.resolve_stored(symbol))
                        .ok_or(StoreFormatError::Invalid("font identifier symbol"))?,
                );
            }
        }
        let mut node_ids = std::collections::BTreeMap::new();
        for list in self.node_lists {
            let nodes = list
                .nodes
                .into_iter()
                .map(|node| node.restore(&stores, &node_ids))
                .collect::<Result<Vec<_>, _>>()?;
            let id = stores.nodes.append(&nodes);
            node_ids.insert(list.key, id);
        }
        for entry in self.code_tables {
            entry.restore(&mut stores.code_tables)?;
        }
        stores.hyphenation = self.hyphenation.into();
        stores.prepared_mag = self.prepared_mag;
        stores.last_loaded_font = stores.resolve_stored_font(FontId::new(self.last_loaded_font));
        for entry in self.env {
            let dto_cell = crate::cell::CellId::from_raw(entry.cell)
                .ok_or(StoreFormatError::Invalid("unknown environment cell"))?;
            let bank = dto_cell.bank();
            let dto_index = dto_cell.index();
            let index = if bank == crate::cell::BankTag::Meaning {
                stores
                    .interner
                    .symbol_at_slot(dto_index)
                    .ok_or(StoreFormatError::Invalid("meaning symbol is not live"))?
                    .raw()
            } else {
                dto_index
            };
            let cell = crate::cell::CellId::new(bank, index);
            let word = match (cell.bank(), entry.value) {
                (crate::cell::BankTag::Box, FormatEnvValue::Box(key)) => {
                    let id = node_ids
                        .get(&key)
                        .copied()
                        .ok_or(StoreFormatError::Invalid("missing box node list"))?;
                    NodeListId::encode_box_word(Some(stores.prepare_box_value(id)))
                }
                (crate::cell::BankTag::Box, FormatEnvValue::Raw(_)) => {
                    return Err(StoreFormatError::Invalid("raw box environment value"));
                }
                (crate::cell::BankTag::CurrentFont, FormatEnvValue::Raw(word)) => {
                    restore_current_font_word(&stores, word)?
                }
                (_, FormatEnvValue::Raw(word)) => word,
                (_, FormatEnvValue::Box(_)) => {
                    return Err(StoreFormatError::Invalid("box value in non-box bank"));
                }
            };
            stores.env.restore_raw(cell, word);
        }
        Ok(stores)
    }
}

impl FormatListKey {
    fn capture(
        stores: &Stores,
        id: NodeListId,
        survivor_roots: &mut std::collections::BTreeMap<crate::ids::SurvivorRootId, u32>,
    ) -> Self {
        let (start, len) = match id.arena() {
            crate::ids::ArenaRef::Epoch => {
                let span = stores
                    .nodes
                    .span(id)
                    .expect("captured epoch node-list id must be live");
                (span.start, span.len)
            }
            crate::ids::ArenaRef::Survivor(_) => (id.start(), id.len()),
        };
        Self {
            survivor_root: match id.arena() {
                crate::ids::ArenaRef::Epoch => None,
                crate::ids::ArenaRef::Survivor(root) => Some(match survivor_roots.get(&root) {
                    Some(&detached) => detached,
                    None => {
                        let detached = u32::try_from(survivor_roots.len())
                            .expect("format survivor roots exceed u32");
                        survivor_roots.insert(root, detached);
                        detached
                    }
                }),
            },
            start,
            len,
        }
    }
}

fn capture_node_list(
    stores: &Stores,
    id: NodeListId,
    seen: &mut std::collections::BTreeSet<NodeListId>,
    visiting: &mut std::collections::BTreeSet<NodeListId>,
    survivor_roots: &mut std::collections::BTreeMap<crate::ids::SurvivorRootId, u32>,
    out: &mut Vec<FormatNodeList>,
) -> Result<(), StoreFormatError> {
    if seen.contains(&id) {
        return Ok(());
    }
    if !visiting.insert(id) {
        return Err(StoreFormatError::Invalid("cyclic node-list graph"));
    }
    let nodes = stores.nodes(id).to_vec();
    for node in &nodes {
        for child in node_child_ids(node) {
            capture_node_list(stores, child, seen, visiting, survivor_roots, out)?;
        }
    }
    visiting.remove(&id);
    seen.insert(id);
    let nodes = nodes
        .into_iter()
        .map(|node| FormatNode::capture(stores, node, survivor_roots))
        .collect();
    out.push(FormatNodeList {
        key: FormatListKey::capture(stores, id, survivor_roots),
        nodes,
    });
    Ok(())
}

fn node_child_ids(node: &Node) -> Vec<NodeListId> {
    let mut out = Vec::new();
    match node {
        Node::HList(box_node) | Node::VList(box_node) => out.push(box_node.children),
        Node::Glue {
            leader:
                Some(
                    crate::node::LeaderPayload::HList(box_node)
                    | crate::node::LeaderPayload::VList(box_node),
                ),
            ..
        } => out.push(box_node.children),
        Node::Unset(unset) => out.push(unset.children),
        Node::Disc {
            pre, post, replace, ..
        } => out.extend([*pre, *post, *replace]),
        Node::Ins { content, .. } | Node::Adjust(content) => out.push(*content),
        Node::MathNoad(noad) => {
            math_field_child(&noad.nucleus, &mut out);
            math_field_child(&noad.subscript, &mut out);
            math_field_child(&noad.superscript, &mut out);
        }
        Node::FractionNoad(fraction) => {
            out.extend([fraction.numerator, fraction.denominator]);
        }
        Node::MathChoice(choice) => out.extend([
            choice.display,
            choice.text,
            choice.script,
            choice.script_script,
        ]),
        Node::MathList(list) => out.push(list.content),
        _ => {}
    }
    out
}

fn math_field_child(field: &crate::math::MathField, out: &mut Vec<NodeListId>) {
    if let crate::math::MathField::SubBox(id) | crate::math::MathField::SubMlist(id) = field {
        out.push(*id);
    }
}

impl FormatToken {
    fn capture(stores: &Stores, token: Token) -> Self {
        match token {
            Token::Char { ch, cat } => Self::Char { ch, cat: cat as u8 },
            Token::Cs(symbol) => Self::Cs(stores.resolve_stored_symbol(symbol).raw()),
            Token::Param(slot) => Self::Param(slot),
            Token::Frozen(crate::token::FrozenToken::END_TEMPLATE) => Self::Frozen(0),
            Token::Frozen(crate::token::FrozenToken::END_V) => Self::Frozen(1),
            Token::Frozen(_) => unreachable!("invalid frozen token payload"),
        }
    }

    fn restore(self, interner: &crate::interner::Interner) -> Result<Token, StoreFormatError> {
        Ok(match self {
            Self::Char { ch, cat } => Token::Char {
                ch,
                cat: catcode(cat)?,
            },
            Self::Cs(raw) => Token::Cs(
                interner
                    .symbol_at_slot(raw)
                    .ok_or(StoreFormatError::Invalid("token symbol is not live"))?,
            ),
            Self::Param(slot) => Token::Param(slot),
            Self::Frozen(0) => Token::Frozen(crate::token::FrozenToken::END_TEMPLATE),
            Self::Frozen(1) => Token::Frozen(crate::token::FrozenToken::END_V),
            Self::Frozen(_) => return Err(StoreFormatError::Invalid("unknown frozen token")),
        })
    }
}

impl FormatGlue {
    fn capture(spec: GlueSpec) -> Self {
        Self {
            width: spec.width.raw(),
            stretch: spec.stretch.raw(),
            stretch_order: spec.stretch_order as u8,
            shrink: spec.shrink.raw(),
            shrink_order: spec.shrink_order as u8,
        }
    }

    fn restore(self) -> Result<GlueSpec, StoreFormatError> {
        Ok(GlueSpec {
            width: Scaled::from_raw(self.width),
            stretch: Scaled::from_raw(self.stretch),
            stretch_order: order(self.stretch_order)?,
            shrink: Scaled::from_raw(self.shrink),
            shrink_order: order(self.shrink_order)?,
        })
    }
}

impl FormatFont {
    fn capture(fonts: &FontStore, id: FontId) -> Self {
        let font = fonts.get(id);
        Self {
            name: font.name().to_owned(),
            path: font.path().to_owned(),
            content_hash: font.content_hash(),
            checksum: font.checksum(),
            design_size: font.design_size().raw(),
            size: font.size().raw(),
            parameters: font.parameters().iter().map(|v| v.raw()).collect(),
            characters: font.metrics().characters().to_vec(),
            lig_kern_program: font.metrics().lig_kern_program().to_vec(),
            right_boundary_char: font.metrics().right_boundary_char(),
            left_boundary_program: font.metrics().left_boundary_program(),
            extensible_recipes: font.metrics().extensible_recipes().to_vec(),
            identifier: fonts.identifier(id).map(crate::interner::SymbolId::raw),
        }
    }

    fn restore(self) -> LoadedFont {
        LoadedFont::new(
            self.name,
            self.path,
            self.content_hash,
            self.checksum,
            Scaled::from_raw(self.design_size),
            Scaled::from_raw(self.size),
            self.parameters.into_iter().map(Scaled::from_raw).collect(),
            FontMetrics::new(
                self.characters,
                self.lig_kern_program,
                self.right_boundary_char,
                self.left_boundary_program,
                self.extensible_recipes,
            ),
        )
    }

    fn metrics(&self) -> FontMetrics {
        FontMetrics::new(
            self.characters.clone(),
            self.lig_kern_program.clone(),
            self.right_boundary_char,
            self.left_boundary_program,
            self.extensible_recipes.clone(),
        )
    }
}

impl FormatCodeTables {
    fn restore(self, tables: &mut CodeTables) -> Result<(), StoreFormatError> {
        let ch = char::from_u32(self.code).ok_or(StoreFormatError::Invalid("codepoint"))?;
        tables.set_catcode(ch, catcode(self.catcode)?);
        tables.set_lccode(ch, self.lccode);
        tables.set_uccode(ch, self.uccode);
        tables.set_sfcode(ch, self.sfcode);
        tables.set_mathcode(ch, self.mathcode);
        tables.set_delcode(ch, self.delcode);
        Ok(())
    }
}

fn catcode(value: u8) -> Result<Catcode, StoreFormatError> {
    match value {
        0 => Ok(Catcode::Escape),
        1 => Ok(Catcode::BeginGroup),
        2 => Ok(Catcode::EndGroup),
        3 => Ok(Catcode::MathShift),
        4 => Ok(Catcode::AlignmentTab),
        5 => Ok(Catcode::EndLine),
        6 => Ok(Catcode::Parameter),
        7 => Ok(Catcode::Superscript),
        8 => Ok(Catcode::Subscript),
        9 => Ok(Catcode::Ignored),
        10 => Ok(Catcode::Space),
        11 => Ok(Catcode::Letter),
        12 => Ok(Catcode::Other),
        13 => Ok(Catcode::Active),
        14 => Ok(Catcode::Comment),
        15 => Ok(Catcode::Invalid),
        _ => Err(StoreFormatError::Invalid("catcode")),
    }
}

fn order(value: u8) -> Result<crate::glue::Order, StoreFormatError> {
    match value {
        0 => Ok(crate::glue::Order::Normal),
        1 => Ok(crate::glue::Order::Fil),
        2 => Ok(crate::glue::Order::Fill),
        3 => Ok(crate::glue::Order::Filll),
        _ => Err(StoreFormatError::Invalid("glue order")),
    }
}
