//! Input frames, token delivery, back-up and expansion entry.

use super::*;

impl Machine {
    #[inline(always)]
    pub(crate) fn get_next(&mut self) -> (u32, u64) {
        loop {
            let loc = self.cur.loc;
            if loc < self.cur.end {
                self.cur.loc = loc + 1;
                let t = if self.cur.kind & LIFO == 0 {
                    self.mem[loc as usize]
                } else {
                    self.lifo[loc as usize] as u32
                };
                if t >= CS_FLAG {
                    let w = self.eqtb[(t - CS_FLAG) as usize];
                    if cmd(w) < OUTER_CALL {
                        return (t, w);
                    }
                    return self.special_cs(t, w);
                }
                let cat = (t >> CAT_SHIFT) as u8;
                match cat {
                    LEFT_BRACE => self.align_state += 1,
                    RIGHT_BRACE => self.align_state -= 1,
                    OUT_PARAM => {
                        self.insert_param(t & CHR_MASK);
                        continue;
                    }
                    _ => {}
                }
                return (t, word(cat, t & CHR_MASK));
            }
            self.end_frame();
        }
    }

    /// The origin of the token the last `get_next` delivered.
    #[inline(always)]
    pub(crate) fn last_origin(&self) -> u32 {
        if self.cur.kind & LIFO == 0 {
            self.cur.origin
        } else {
            (self.lifo[(self.cur.loc - 1) as usize] >> 32) as u32
        }
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn special_cs(&mut self, t: u32, w: u64) -> (u32, u64) {
        // Outer macros, \endtemplate, \noexpand markers and halts land here.
        if self.scanner_status != NORMAL {
            fatal("forbidden control sequence");
        }
        (t, w)
    }

    #[inline(never)]
    pub(crate) fn insert_param(&mut self, n: u32) {
        let base = self.cur.params as usize + 1 + 2 * (n as usize - 1);
        let (a, b) = (self.ps[base], self.ps[base + 1]);
        if a < b {
            self.stack.push(self.cur);
            self.cur = Frame {
                loc: a,
                end: b,
                start: a,
                params: NO_PARAMS,
                name: 0,
                origin: 0,
                kind: K_PARAM,
            };
        }
    }

    #[inline(never)]
    pub(crate) fn end_frame(&mut self) {
        let f = self.cur;
        match f.kind {
            K_MACRO => {
                if f.params != NO_PARAMS {
                    let p = f.params as usize;
                    self.release_lifo(self.ps[p]);
                    self.ps.truncate(p);
                }
                self.del_ref(f.start as usize);
            }
            K_BACKED_UP => self.release_lifo(f.start),
            K_PARAM => {}
            _ => fatal("end of source"),
        }
        self.cur = match self.stack.pop() {
            Some(f) => f,
            None => fatal("input stack underflow"),
        };
    }

    #[inline(always)]
    pub(crate) fn release_lifo(&mut self, start: u32) {
        if self.lifo_pin == NO_PIN {
            self.lifo.truncate(start as usize);
        } else {
            self.dead_floor = self.dead_floor.min(start);
        }
    }

    /// Pops frames that are already exhausted, as TeX does before pushing.
    #[inline(always)]
    pub(crate) fn pop_exhausted(&mut self) {
        while self.cur.loc >= self.cur.end && self.cur.kind != K_SOURCE {
            self.end_frame();
        }
    }

    pub(crate) fn back_input(&mut self, t: u32, origin: u32) {
        self.pop_exhausted();
        if t < RIGHT_BRACE_LIMIT {
            if t < LEFT_BRACE_LIMIT {
                self.align_state -= 1;
            } else {
                self.align_state += 1;
            }
        }
        let k = self.lifo.len() as u32;
        self.lifo.push(t as u64 | (origin as u64) << 32);
        self.stack.push(self.cur);
        self.cur = Frame {
            loc: k,
            end: k + 1,
            start: k,
            params: NO_PARAMS,
            name: 0,
            origin: 0,
            kind: K_BACKED_UP,
        };
    }

    #[inline(always)]
    pub(crate) fn get_x_token(&mut self) -> (u32, u64) {
        loop {
            let (t, w) = self.get_next();
            if cmd(w) <= MAX_COMMAND {
                return (t, w);
            }
            self.expand_one(t, w);
        }
    }

    #[inline(never)]
    pub(crate) fn expand_one(&mut self, t: u32, w: u64) {
        self.fuel -= 1;
        if self.fuel == 0 {
            self.refuel();
        }
        if cmd(w) >= CALL {
            self.macro_call(t, w);
        } else {
            self.expand(w);
        }
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn refuel(&mut self) {
        // Samples cancellation and charges the host budget.
        self.fuel = 1 << 20;
        self.refuels += 1;
    }

    pub(crate) fn expand(&mut self, w: u64) {
        match cmd(w) {
            EXPAND_AFTER => {
                let (t1, _) = self.get_next();
                let o1 = self.last_origin();
                let (t2, w2) = self.get_next();
                if cmd(w2) > MAX_COMMAND {
                    self.expand_one(t2, w2);
                } else {
                    let o2 = self.last_origin();
                    self.back_input(t2, o2);
                }
                self.back_input(t1, o1);
            }
            IF_TEST => self.conditional(chr(w)),
            FI_OR_ELSE => self.fi_or_else(chr(w) as u8),
            UNDEFINED_CS => fatal("undefined control sequence"),
            _ => fatal("unimplemented expandable command"),
        }
    }
}
