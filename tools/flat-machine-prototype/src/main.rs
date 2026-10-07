//! Prototype of the minimal hot loop: one machine, u32 tokens, one u64 eqtb,
//! one token memory, 28-byte frames, a LIFO argument bank, first-touch
//! checkpoint undo. Runs the interp-microbench document for a subset of cases.
#![forbid(unsafe_code)]

mod checkpoint;
mod control;
mod input;
mod macros;
mod scan;
mod state;

use std::collections::HashMap;

// ---------------------------------------------------------------- tokens
const CAT_SHIFT: u32 = 21;
const CHR_MASK: u32 = (1 << CAT_SHIFT) - 1;
const CS_FLAG: u32 = 1 << 25;

const fn ct(cat: u8, ch: u32) -> u32 {
    ((cat as u32) << CAT_SHIFT) | ch
}
const fn word(cmd: u8, chr: u32) -> u64 {
    cmd as u64 | (chr as u64) << 32
}
#[inline(always)]
fn cmd(w: u64) -> u8 {
    w as u8
}
#[inline(always)]
fn chr(w: u64) -> u32 {
    (w >> 32) as u32
}

// Command codes. A character token's command is its category.
const RELAX: u8 = 0;
const LEFT_BRACE: u8 = 1;
const RIGHT_BRACE: u8 = 2;
const OUT_PARAM: u8 = 5; // only inside stored lists
const MAC_PARAM: u8 = 6;
const SPACER: u8 = 10;
const LETTER: u8 = 11;
const OTHER_CHAR: u8 = 12;
const MATCH: u8 = 13; // only inside stored lists
const END_MATCH: u8 = 14; // only inside stored lists
const STOP: u8 = 14;
const BEGIN_GROUP: u8 = 20;
const END_GROUP: u8 = 21;
const REGISTER: u8 = 22;
const ADVANCE: u8 = 23;
const TOKS_REGISTER: u8 = 24;
const LET: u8 = 25;
const DEF: u8 = 26;
const CHECKPOINT: u8 = 27;
const MAX_COMMAND: u8 = 100;
const UNDEFINED_CS: u8 = 101;
const EXPAND_AFTER: u8 = 102;
const IF_TEST: u8 = 105;
const FI_OR_ELSE: u8 = 106;
const CALL: u8 = 111;
const LONG_CALL: u8 = 112;
const OUTER_CALL: u8 = 113;
const LONG_OUTER_CALL: u8 = 114;

const LEFT_BRACE_LIMIT: u32 = ct(2, 0);
const RIGHT_BRACE_LIMIT: u32 = ct(3, 0);
const SPACE_TOKEN: u32 = ct(SPACER, b' ' as u32);
const OTHER_ZERO: u32 = ct(OTHER_CHAR, b'0' as u32);
const EQUALS_TOKEN: u32 = ct(OTHER_CHAR, b'=' as u32);
const END_MATCH_TOKEN: u32 = ct(END_MATCH, 0);

// ---------------------------------------------------------------- eqtb
const HASH_SIZE: usize = 1 << 12;
const FROZEN_BASE: usize = HASH_SIZE - 16;
const TOKS_BASE: usize = HASH_SIZE;
/// Slots below hold meanings (possibly list references); slots above hold raw values.
const RAW_BASE: usize = TOKS_BASE + 256;
const COUNT_BASE: usize = RAW_BASE;
const EQTB_SIZE: usize = COUNT_BASE + 256;
const LEVEL_ONE: u32 = 1;

// ---------------------------------------------------------------- token memory
/// List header: refs, touched epoch, len, info, origin.
const HDR: usize = 5;
const NPARAMS_MASK: u32 = 15;
const SIMPLE: u32 = 1 << 4;
const HAS_MATCH: u32 = 1 << 5;
const BODY_SHIFT: u32 = 8;
const FREE_CLASSES: usize = 128;
const NIL: u32 = u32::MAX;

// ---------------------------------------------------------------- input
const K_SOURCE: u8 = 0;
const K_MACRO: u8 = 1;
const LIFO: u8 = 8;
const K_PARAM: u8 = LIFO;
const K_BACKED_UP: u8 = LIFO | 1;
const NO_PARAMS: u32 = u32::MAX;
const NO_PIN: u32 = u32::MAX;

const NORMAL: u8 = 0;
const SKIPPING: u8 = 1;
const DEFINING: u8 = 2;
const MATCHING: u8 = 3;
const ABSORBING: u8 = 5;

const IF_CODE: u8 = 1;
const FI_CODE: u8 = 2;
const ELSE_CODE: u8 = 3;
const OR_CODE: u8 = 4;
const IF_INT_CODE: u32 = 2;

const BOTTOM_LEVEL: u8 = 0;
const SIMPLE_GROUP: u8 = 1;
const SEMI_SIMPLE_GROUP: u8 = 14;
const BOUNDARY: u32 = u32::MAX;
const LIST_UNDO: u32 = u32::MAX;

#[derive(Clone, Copy, Default, Debug, PartialEq)]
struct Frame {
    loc: u32,
    end: u32,
    /// List header for stored lists; first LIFO word for LIFO frames.
    start: u32,
    /// Index of this macro's region start in `ps`, followed by (start, end) pairs.
    params: u32,
    name: u32,
    origin: u32,
    kind: u8,
}

#[derive(Clone, Copy, PartialEq, Debug)]
struct Save {
    p: u32,
    level: u32,
    word: u64,
}

#[derive(Clone, Copy)]
struct Undo {
    slot: u32,
    level: u32,
    epoch: u32,
    word: u64,
}

#[derive(Clone, Copy, PartialEq, Debug)]
struct Cond {
    limit: u8,
    cur_if: u8,
}

struct Checkpoint {
    mark: u32,
    cur: Frame,
    stack: Vec<Frame>,
    lifo: Vec<u64>,
    ps: Vec<u32>,
    save: Vec<Save>,
    cond: Vec<Cond>,
    _frozen: Vec<u32>,
    cur_level: u32,
    cur_group: u8,
    cur_boundary: u32,
    if_limit: u8,
    cur_if: u8,
    align_state: i32,
    snapshot: Option<(Vec<u64>, Vec<u64>, Vec<u32>)>,
}

struct Machine {
    cur: Frame,
    stack: Vec<Frame>,
    mem: Vec<u32>,
    /// Arguments and backed-up tokens: token | origin << 32.
    lifo: Vec<u64>,
    ps: Vec<u32>,
    lifo_pin: u32,
    dead_floor: u32,
    align_state: i32,
    scanner_status: u8,
    par_token: u32,
    eqtb: Vec<u64>,
    /// epoch << 32 | level
    meta: Vec<u64>,
    cur_meta: u64,
    cur_level: u32,
    cur_group: u8,
    cur_boundary: u32,
    save: Vec<Save>,
    epoch: u32,
    undo: Vec<Undo>,
    free: Vec<u32>,
    mark: u32,
    deferred: Vec<u32>,
    cond: Vec<Cond>,
    if_limit: u8,
    cur_if: u8,
    fuel: u64,
    refuels: u64,
    trace: u32,
    /// Nonzero when an interrupt, tracing, \afterassignment or \globaldefs needs attention.
    attention: u32,
    chars: u64,
    names: HashMap<String, usize>,
    checkpoint: Option<Box<Checkpoint>>,
    verify: bool,
}

#[cold]
#[inline(never)]
fn fatal(msg: &str) -> ! {
    panic!("{msg}")
}

impl Machine {
    fn new() -> Self {
        let mut m = Machine {
            cur: Frame::default(),
            stack: Vec::with_capacity(256),
            mem: Vec::with_capacity(1 << 16),
            lifo: Vec::with_capacity(1024),
            ps: Vec::with_capacity(256),
            lifo_pin: NO_PIN,
            dead_floor: NO_PIN,
            align_state: 1_000_000,
            scanner_status: NORMAL,
            par_token: 0,
            eqtb: vec![word(UNDEFINED_CS, 0); EQTB_SIZE],
            meta: vec![LEVEL_ONE as u64; EQTB_SIZE],
            cur_meta: LEVEL_ONE as u64,
            cur_level: LEVEL_ONE,
            cur_group: BOTTOM_LEVEL,
            cur_boundary: 0,
            save: Vec::with_capacity(256),
            epoch: 0,
            undo: Vec::new(),
            free: vec![NIL; FREE_CLASSES],
            mark: 0,
            deferred: Vec::new(),
            cond: Vec::with_capacity(64),
            if_limit: 0,
            cur_if: 0,
            fuel: 1 << 20,
            refuels: 0,
            trace: 0,
            attention: 0,
            chars: 0,
            names: HashMap::new(),
            checkpoint: None,
            verify: false,
        };
        for p in TOKS_BASE..EQTB_SIZE {
            m.eqtb[p] = 0;
        }
        m.primitive("relax", RELAX, 256);
        m.primitive("begingroup", BEGIN_GROUP, 0);
        m.primitive("endgroup", END_GROUP, 0);
        m.primitive("count", REGISTER, 0);
        m.primitive("advance", ADVANCE, 0);
        m.primitive("toks", TOKS_REGISTER, 0);
        m.primitive("let", LET, 0);
        m.primitive("def", DEF, 0);
        m.primitive("expandafter", EXPAND_AFTER, 0);
        m.primitive("ifnum", IF_TEST, IF_INT_CODE);
        m.primitive("fi", FI_OR_ELSE, FI_CODE as u32);
        m.primitive("end", STOP, 0);
        m.primitive("checkpoint", CHECKPOINT, 0);
        m.par_token = CS_FLAG + m.lookup("par") as u32;
        m
    }

    fn lookup(&mut self, name: &str) -> usize {
        let next = self.names.len() + 1;
        let p = *self.names.entry(name.to_owned()).or_insert(next);
        assert!(p < FROZEN_BASE);
        p
    }

    fn primitive(&mut self, name: &str, c: u8, ch: u32) {
        let p = self.lookup(name);
        self.eqtb[p] = word(c, ch);
    }

    fn tokenize(&mut self, s: &str) -> Vec<u32> {
        fn cat(c: u8) -> u8 {
            match c {
                b'\\' => 0,
                b'{' => 1,
                b'}' => 2,
                b'#' => 6,
                b' ' | b'\n' => 10,
                b'a'..=b'z' | b'A'..=b'Z' => 11,
                _ => 12,
            }
        }
        let b = s.as_bytes();
        let mut out = Vec::new();
        let mut skip_blanks = true;
        let mut i = 0;
        while i < b.len() {
            let c = b[i];
            match cat(c) {
                0 => {
                    let start = i + 1;
                    i = start;
                    while i < b.len() && cat(b[i]) == LETTER {
                        i += 1;
                    }
                    if i == start {
                        i += 1;
                        skip_blanks = false;
                    } else {
                        skip_blanks = true;
                    }
                    let p = self.lookup(&s[start..i]);
                    out.push(CS_FLAG + p as u32);
                }
                SPACER => {
                    if !skip_blanks {
                        out.push(SPACE_TOKEN);
                        skip_blanks = true;
                    }
                    i += 1;
                }
                k => {
                    out.push(ct(k, c as u32));
                    skip_blanks = false;
                    i += 1;
                }
            }
        }
        out
    }

    fn push_source(&mut self, toks: &[u32]) {
        let l = self.mem.len();
        self.mem.extend_from_slice(&[1, self.epoch, 0, 0, 1]);
        self.mem.extend_from_slice(toks);
        let l = self.finish_list(l, 0) as usize;
        let len = self.mem[l + 2];
        self.cur = Frame {
            loc: (l + HDR) as u32,
            end: (l + HDR) as u32 + len,
            start: l as u32,
            params: NO_PARAMS,
            name: 0,
            origin: 1,
            kind: K_SOURCE,
        };
    }
}

fn snippet(case: &str) -> &'static str {
    match case {
        "base" => "",
        "relax" => r"\relax",
        "mzero" => r"\mzero",
        "mbody" => r"\mbody",
        "mbody64" => r"\longrelax",
        "mone" => r"\mone x",
        "mone_brace" => r"\mone{xy}",
        "mtwo" => r"\mtwo\relax\relax",
        "mdelim" => r"\mdelim a.b\stop",
        "let" => r"\let\x\relax",
        "def" => r"\def\x{abc}",
        "group" => r"\begingroup\endgroup",
        "brace64" => {
            "{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}"
        }
        "group_let" => r"\begingroup\let\y\relax\endgroup",
        "advance" => r"\advance\count2 by 1 ",
        "count" => r"\count2=5 ",
        "toks" => r"\toks2={abc}",
        _ => panic!("unknown case {case}"),
    }
}

fn document(snippet: &str, iterations: u64) -> String {
    [
        r"\def\mzero{}\def\mone#1{}\def\mtwo#1#2{#2#1}\def\mdelim#1.#2\stop{}".to_owned(),
        format!(
            r"\def\mbody{{\relax\relax}}\let\x\relax\def\longrelax{{{}}}",
            r"\relax".repeat(64)
        ),
        format!(
            r"\def\iter{{\ifnum\count255<{iterations} \advance\count255 by 1 {snippet}\expandafter\iter\fi}}\checkpoint\iter"
        ),
        r"\end".to_owned(),
        String::new(),
    ]
    .join("\n")
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let case = args.get(1).map_or("base", String::as_str);
    let iterations: u64 = args.get(2).map_or(1000, |s| s.parse().expect("iterations"));
    let verify = args.iter().any(|a| a == "--verify");
    let mut m = Machine::new();
    m.verify = verify;
    let text = document(snippet(case), iterations);
    let toks = m.tokenize(&text);
    m.push_source(&toks);
    m.main_control();
    assert_eq!(m.eqtb[COUNT_BASE + 255] as u32 as u64, iterations);
    assert_eq!(m.cur_level, LEVEL_ONE);
    assert!(m.cond.is_empty() && m.stack.is_empty() && m.lifo.is_empty() && m.ps.is_empty());
    if verify {
        let first = m.fingerprint();
        let undo = m.undo.len();
        m.restore();
        m.main_control();
        assert!(first == m.fingerprint(), "replay from checkpoint diverged");
        println!(
            "verify ok: {case} undo={undo} mem={} deferred={}",
            m.mem.len(),
            m.deferred.len()
        );
    } else {
        println!(
            "{case} {iterations} chars={} refuels={} mem={}",
            m.chars,
            m.refuels,
            m.mem.len()
        );
    }
}
