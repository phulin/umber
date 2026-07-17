//! Web2C-compatible classic BibTeX string-pool accounting.
//!
//! The reference assigns a string number when a value is first inserted into
//! `str_pool`; hash-table ilks may differ, but they share that number.  This
//! type models that lifetime directly rather than deriving a summary from
//! unrelated compiler containers.

use std::collections::BTreeMap;

/// Stable identity of a string in one classic BibTeX pool.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PoolStringId(pub u32);

/// Charged string-pool usage, corresponding to Web2C's `str_ptr` and
/// `pool_ptr` summary counters.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StringPoolUsage {
    strings: usize,
    characters: usize,
}

impl StringPoolUsage {
    #[must_use]
    pub const fn strings(self) -> usize {
        self.strings
    }

    #[must_use]
    pub const fn characters(self) -> usize {
        self.characters
    }
}

/// Independent hard bounds for a charged pool.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StringPoolLimits {
    strings: usize,
    characters: usize,
}

impl StringPoolLimits {
    #[must_use]
    pub const fn new(strings: usize, characters: usize) -> Self {
        Self {
            strings,
            characters,
        }
    }

    #[must_use]
    pub const fn unlimited() -> Self {
        Self::new(usize::MAX, usize::MAX)
    }
}

/// A pool insertion would exceed one of its separately charged bounds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StringPoolLimit {
    Strings,
    Characters,
}

/// A monotonic, job-lifetime string pool.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClassicStringPool {
    values: Vec<String>,
    identities: BTreeMap<String, PoolStringId>,
    usage: StringPoolUsage,
    limits: StringPoolLimits,
}

impl ClassicStringPool {
    /// Starts an empty pool. The reserved Web2C `str_start[0]` slot has no
    /// string number and is therefore intentionally not charged.
    #[must_use]
    pub fn new(limits: StringPoolLimits) -> Self {
        Self {
            values: Vec::new(),
            identities: BTreeMap::new(),
            usage: StringPoolUsage::default(),
            limits,
        }
    }

    /// Creates the pool after BibTeX 0.99d's `pre_def_certain_strings`.
    ///
    /// These are the actual Web2C bootstrap values, in source order.  The
    /// pool still interns by value, so `preamble` and the duplicated `width$`
    /// definition retain their earlier identities just as `str_lookup` does.
    #[must_use]
    pub fn web2c() -> Self {
        let mut pool = Self::new(StringPoolLimits::unlimited());
        for value in WEB2C_BOOTSTRAP_STRINGS {
            pool.intern(value).expect("fixed Web2C bootstrap fits");
        }
        pool
    }

    /// Inserts `value` if necessary and returns its stable pool identity.
    pub fn intern(&mut self, value: &str) -> Result<PoolStringId, StringPoolLimit> {
        if let Some(id) = self.identities.get(value) {
            return Ok(*id);
        }
        if self.usage.strings >= self.limits.strings {
            return Err(StringPoolLimit::Strings);
        }
        if value.len() > self.limits.characters.saturating_sub(self.usage.characters) {
            return Err(StringPoolLimit::Characters);
        }
        let id = PoolStringId(self.values.len() as u32);
        let owned = value.to_owned();
        self.usage.strings += 1;
        self.usage.characters += owned.len();
        self.identities.insert(owned.clone(), id);
        self.values.push(owned);
        Ok(id)
    }

    #[must_use]
    pub const fn usage(&self) -> StringPoolUsage {
        self.usage
    }

    #[must_use]
    pub fn value(&self, id: PoolStringId) -> Option<&str> {
        self.values.get(id.0 as usize).map(String::as_str)
    }
}

const WEB2C_BOOTSTRAP_STRINGS: &[&str] = &[
    ".aux",
    ".bbl",
    ".blg",
    ".bst",
    ".bib",
    "texinputs:",
    "texbib:",
    "\\citation",
    "\\bibdata",
    "\\bibstyle",
    "\\@input",
    "entry",
    "execute",
    "function",
    "integers",
    "iterate",
    "macro",
    "read",
    "reverse",
    "sort",
    "strings",
    "comment",
    "preamble",
    "string",
    "=",
    ">",
    "<",
    "+",
    "-",
    "*",
    ":=",
    "add.period$",
    "call.type$",
    "change.case$",
    "chr.to.int$",
    "cite$",
    "duplicate$",
    "empty$",
    "format.name$",
    "if$",
    "int.to.chr$",
    "int.to.str$",
    "missing$",
    "newline$",
    "num.names$",
    "pop$",
    "preamble$",
    "purify$",
    "quote$",
    "skip$",
    "stack$",
    "substring$",
    "swap$",
    "text.length$",
    "text.prefix$",
    "top$",
    "type$",
    "warning$",
    "width$",
    "while$",
    "width$",
    "write$",
    "",
    "default.type",
    "i",
    "j",
    "oe",
    "OE",
    "ae",
    "AE",
    "aa",
    "AA",
    "o",
    "O",
    "l",
    "L",
    "ss",
    "crossref",
    "sort.key$",
    "entry.max$",
    "global.max$",
];
