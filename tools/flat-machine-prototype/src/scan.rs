//! Conditionals, numeric and keyword scanners, and token-list building.

use super::*;

impl Machine {
    pub(crate) fn conditional(&mut self, this_if: u32) {
        self.cond.push(Cond {
            limit: self.if_limit,
            cur_if: self.cur_if,
        });
        self.cur_if = this_if as u8;
        self.if_limit = IF_CODE;
        let save = self.cond.len();
        let b = match this_if {
            IF_INT_CODE => {
                let a = self.scan_int();
                let r = self.scan_relation();
                let c = self.scan_int();
                match r {
                    b'<' => a < c,
                    b'=' => a == c,
                    _ => a > c,
                }
            }
            _ => fatal("unimplemented conditional"),
        };
        if b {
            self.change_if_limit(ELSE_CODE, save);
            return;
        }
        let c = loop {
            let c = self.pass_text();
            if self.cond.len() == save {
                if c != OR_CODE {
                    break c;
                }
                fatal("extra \\or");
            } else if c == FI_CODE {
                self.pop_cond();
            }
        };
        if c == FI_CODE {
            self.pop_cond();
        } else {
            self.if_limit = FI_CODE;
        }
    }

    pub(crate) fn change_if_limit(&mut self, l: u8, save: usize) {
        if self.cond.len() == save {
            self.if_limit = l;
        } else {
            self.cond[save].limit = l;
        }
    }

    pub(crate) fn pop_cond(&mut self) {
        let c = match self.cond.pop() {
            Some(c) => c,
            None => fatal("condition stack underflow"),
        };
        self.if_limit = c.limit;
        self.cur_if = c.cur_if;
    }

    pub(crate) fn pass_text(&mut self) -> u8 {
        let saved = self.scanner_status;
        self.scanner_status = SKIPPING;
        let mut l = 0u32;
        let c = loop {
            let (_, w) = self.get_next();
            match cmd(w) {
                FI_OR_ELSE => {
                    if l == 0 {
                        break chr(w) as u8;
                    }
                    if chr(w) as u8 == FI_CODE {
                        l -= 1;
                    }
                }
                IF_TEST => l += 1,
                _ => {}
            }
        };
        self.scanner_status = saved;
        c
    }

    pub(crate) fn fi_or_else(&mut self, mut c: u8) {
        if c > self.if_limit {
            fatal("extra \\fi or insert_relax");
        }
        while c != FI_CODE {
            c = self.pass_text();
        }
        self.pop_cond();
    }

    // ------------------------------------------------------------ scanning

    pub(crate) fn scan_int(&mut self) -> i32 {
        let mut negative = false;
        let (mut t, mut w);
        loop {
            (t, w) = self.get_x_token();
            if cmd(w) == SPACER || t == ct(OTHER_CHAR, b'+' as u32) {
                continue;
            }
            if t == ct(OTHER_CHAR, b'-' as u32) {
                negative = !negative;
                continue;
            }
            break;
        }
        let v: i64 = if cmd(w) == REGISTER {
            let n = self.scan_register_num();
            self.eqtb[COUNT_BASE + n] as u32 as i32 as i64
        } else {
            let mut d = t.wrapping_sub(OTHER_ZERO);
            if d >= 10 {
                fatal("missing number");
            }
            let mut v = 0i64;
            loop {
                v = v * 10 + d as i64;
                if v > i32::MAX as i64 {
                    fatal("number too big");
                }
                (t, w) = self.get_x_token();
                d = t.wrapping_sub(OTHER_ZERO);
                if d >= 10 {
                    break;
                }
            }
            if cmd(w) != SPACER {
                let o = self.last_origin();
                self.back_input(t, o);
            }
            v
        };
        if negative { -v as i32 } else { v as i32 }
    }

    pub(crate) fn scan_register_num(&mut self) -> usize {
        let n = self.scan_int();
        if !(0..256).contains(&n) {
            fatal("bad register code");
        }
        n as usize
    }

    pub(crate) fn scan_relation(&mut self) -> u8 {
        let t = loop {
            let (t, w) = self.get_x_token();
            if cmd(w) != SPACER {
                break t;
            }
        };
        match t {
            x if x == ct(OTHER_CHAR, b'<' as u32) => b'<',
            x if x == ct(OTHER_CHAR, b'=' as u32) => b'=',
            x if x == ct(OTHER_CHAR, b'>' as u32) => b'>',
            _ => fatal("missing = inserted for \\ifnum"),
        }
    }

    pub(crate) fn scan_keyword(&mut self, kw: &[u8]) -> bool {
        let mut held = [(0u32, 0u32); 8];
        let mut k = 0;
        while k < kw.len() {
            let (t, w) = self.get_x_token();
            let c = kw[k] as u32;
            let ch = t & CHR_MASK;
            if t < CS_FLAG && (ch == c || ch + 32 == c) {
                held[k] = (t, self.last_origin());
                k += 1;
            } else if cmd(w) != SPACER || k > 0 {
                let o = self.last_origin();
                self.back_input(t, o);
                for &(t, o) in held[..k].iter().rev() {
                    self.back_input(t, o);
                }
                return false;
            }
        }
        true
    }

    pub(crate) fn scan_optional_equals(&mut self) {
        let (t, _) = loop {
            let (t, w) = self.get_x_token();
            if cmd(w) != SPACER {
                break (t, w);
            }
        };
        if t != EQUALS_TOKEN {
            let o = self.last_origin();
            self.back_input(t, o);
        }
    }

    pub(crate) fn scan_left_brace(&mut self) {
        let w = loop {
            let (_, w) = self.get_x_token();
            if cmd(w) != SPACER && cmd(w) != RELAX {
                break w;
            }
        };
        if cmd(w) != LEFT_BRACE {
            fatal("missing { inserted");
        }
    }

    pub(crate) fn get_r_token(&mut self) -> usize {
        loop {
            let (t, _) = self.get_next();
            if t == SPACE_TOKEN {
                continue;
            }
            if t < CS_FLAG || (t - CS_FLAG) as usize >= FROZEN_BASE {
                fatal("missing control sequence inserted");
            }
            return (t - CS_FLAG) as usize;
        }
    }

    /// Builds a token list at the memory tail, then moves it into a freed
    /// block of the same size when one exists.
    pub(crate) fn scan_toks(&mut self, macro_def: bool, origin: u32) -> u32 {
        let saved = self.scanner_status;
        self.scanner_status = if macro_def { DEFINING } else { ABSORBING };
        let l = self.mem.len();
        self.mem.extend_from_slice(&[1, self.epoch, 0, 0, origin]);
        let mut info = 0;
        let mut n = 0u32;
        let mut hash_brace = 0;
        if macro_def {
            let mut delimited = false;
            loop {
                let (mut t, w) = self.get_next();
                if t < RIGHT_BRACE_LIMIT {
                    if cmd(w) == RIGHT_BRACE {
                        fatal("missing { inserted");
                    }
                    break;
                }
                if cmd(w) == MAC_PARAM {
                    let (t2, w2) = self.get_next();
                    if cmd(w2) == LEFT_BRACE {
                        hash_brace = t2;
                        self.mem.push(t2);
                        delimited = true;
                        break;
                    }
                    if n == 9 {
                        fatal("you already have nine parameters");
                    }
                    n += 1;
                    if t2 != OTHER_ZERO + n {
                        fatal("parameters must be numbered consecutively");
                    }
                    t = ct(MATCH, n);
                } else {
                    delimited = true;
                }
                self.mem.push(t);
            }
            self.mem.push(END_MATCH_TOKEN);
            info = n;
            if n > 0 || delimited {
                info |= HAS_MATCH;
            }
            if !delimited {
                info |= SIMPLE;
            }
        } else {
            self.scan_left_brace();
        }
        let body = (self.mem.len() - l - HDR) as u32;
        let mut unbalance = 0u32;
        loop {
            let (mut t, w) = self.get_next();
            if t < RIGHT_BRACE_LIMIT {
                if cmd(w) < RIGHT_BRACE {
                    unbalance += 1;
                } else {
                    if unbalance == 0 {
                        break;
                    }
                    unbalance -= 1;
                }
            } else if macro_def && cmd(w) == MAC_PARAM {
                let (t2, w2) = self.get_next();
                if cmd(w2) != MAC_PARAM {
                    let d = t2.wrapping_sub(OTHER_ZERO);
                    if d == 0 || d > n {
                        fatal("illegal parameter number");
                    }
                    t = ct(OUT_PARAM, d);
                } else {
                    t = t2;
                }
            }
            self.mem.push(t);
        }
        if hash_brace != 0 {
            self.mem.push(hash_brace);
        }
        self.scanner_status = saved;
        self.finish_list(l, info | body << BODY_SHIFT)
    }

    pub(crate) fn finish_list(&mut self, l: usize, info: u32) -> u32 {
        let len = self.mem.len() - l - HDR;
        self.mem[l + 2] = len as u32;
        self.mem[l + 3] = info;
        let size = HDR + len;
        if size < FREE_CLASSES {
            let b = self.free[size];
            if b != NIL {
                self.free[size] = self.mem[b as usize];
                self.mem.copy_within(l..l + size, b as usize);
                self.mem.truncate(l);
                return b;
            }
        }
        l as u32
    }
}
