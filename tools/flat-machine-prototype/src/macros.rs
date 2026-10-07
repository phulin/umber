//! Macro activation and argument scanning into the LIFO bank.

use super::*;

impl Machine {
    pub(crate) fn macro_call(&mut self, t: u32, w: u64) {
        let def = chr(w) as usize;
        let h = &self.mem[def..def + HDR];
        let (len, info) = (h[2], h[3]);
        let origin = self.last_origin();
        if self.trace != 0 {
            fatal("tracing unimplemented");
        }
        let body = (def + HDR) as u32 + (info >> BODY_SHIFT);
        let end = (def + HDR) as u32 + len;
        if info & HAS_MATCH == 0 {
            if body == end {
                return;
            }
            self.pop_exhausted();
            self.stack.push(self.cur);
            self.cur = Frame {
                loc: body,
                end,
                start: def as u32,
                params: NO_PARAMS,
                name: t,
                origin,
                kind: K_MACRO,
            };
            self.add_ref(def);
            return;
        }
        let params = self.scan_args(t, w, def, info);
        self.pop_exhausted();
        self.unpin(params);
        if body == end {
            let p = params as usize;
            self.lifo.truncate(self.ps[p] as usize);
            self.ps.truncate(p);
            return;
        }
        self.stack.push(self.cur);
        self.cur = Frame {
            loc: body,
            end,
            start: def as u32,
            params,
            name: t,
            origin,
            kind: K_MACRO,
        };
        self.add_ref(def);
    }

    pub(crate) fn scan_args(&mut self, t: u32, w: u64, def: usize, info: u32) -> u32 {
        let base = self.ps.len() as u32;
        let start = self.lifo.len() as u32;
        self.ps.push(start);
        self.lifo_pin = start;
        self.dead_floor = NO_PIN;
        let saved = self.scanner_status;
        self.scanner_status = MATCHING;
        let long = cmd(w) == LONG_CALL || cmd(w) == LONG_OUTER_CALL;
        let _ = t;
        if info & SIMPLE != 0 {
            for _ in 0..info & NPARAMS_MASK {
                self.scan_undelimited(long);
            }
        } else {
            self.scan_delimited(def, long);
        }
        self.scanner_status = saved;
        base
    }

    /// Slides freshly scanned arguments down over LIFO regions released
    /// while they were scanned.
    #[inline(always)]
    pub(crate) fn unpin(&mut self, base: u32) {
        let pin = self.lifo_pin;
        self.lifo_pin = NO_PIN;
        if self.dead_floor < pin {
            self.slide(base, pin);
        }
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn slide(&mut self, base: u32, pin: u32) {
        let floor = self.dead_floor;
        let delta = pin - floor;
        let len = self.lifo.len();
        self.lifo.copy_within(pin as usize..len, floor as usize);
        self.lifo.truncate(len - delta as usize);
        for b in &mut self.ps[base as usize..] {
            *b -= delta;
        }
    }

    #[inline(always)]
    pub(crate) fn push_arg(&mut self, t: u32) {
        let o = self.last_origin();
        self.lifo.push(t as u64 | (o as u64) << 32);
    }

    pub(crate) fn scan_undelimited(&mut self, long: bool) {
        let start = loop {
            let (t, _) = self.get_next();
            if t == SPACE_TOKEN {
                continue;
            }
            if t < RIGHT_BRACE_LIMIT {
                if t >= LEFT_BRACE_LIMIT {
                    fatal("argument of macro has an extra }");
                }
                let start = self.lifo.len() as u32;
                let mut unbalance = 1u32;
                loop {
                    let (t, _) = self.get_next();
                    if t == self.par_token && !long {
                        fatal("runaway argument");
                    }
                    if t < RIGHT_BRACE_LIMIT {
                        if t < LEFT_BRACE_LIMIT {
                            unbalance += 1;
                        } else {
                            unbalance -= 1;
                            if unbalance == 0 {
                                break;
                            }
                        }
                    }
                    self.push_arg(t);
                }
                break start;
            }
            if t == self.par_token && !long {
                fatal("runaway argument");
            }
            let start = self.lifo.len() as u32;
            self.push_arg(t);
            break start;
        };
        self.ps.push(start);
        self.ps.push(self.lifo.len() as u32);
    }

    /// TeX's general matcher (§392-§397) over the stored parameter text.
    pub(crate) fn scan_delimited(&mut self, def: usize, long: bool) {
        let mut r = def + HDR;
        loop {
            let tr = self.mem[r];
            let is_match = |t: u32| t >> CAT_SHIFT == MATCH as u32;
            let s = if is_match(tr) {
                r += 1;
                r
            } else {
                usize::MAX
            };
            let start = self.lifo.len() as u32;
            let mut m = 0u32;
            'cont: loop {
                let (t, _) = self.get_next();
                if t == self.mem[r] {
                    r += 1;
                    let n = self.mem[r];
                    if is_match(n) || n == END_MATCH_TOKEN {
                        if t < LEFT_BRACE_LIMIT {
                            self.align_state -= 1;
                        }
                        break 'cont;
                    }
                    continue 'cont;
                }
                if s != r {
                    if s == usize::MAX {
                        fatal("use doesn't match its definition");
                    }
                    let mut tt = s;
                    loop {
                        let d = self.mem[tt];
                        self.push_arg(d);
                        m += 1;
                        let (mut u, mut v) = (tt + 1, s);
                        loop {
                            if u == r {
                                if t != self.mem[v] {
                                    break;
                                }
                                r = v + 1;
                                continue 'cont;
                            }
                            if self.mem[u] != self.mem[v] {
                                break;
                            }
                            u += 1;
                            v += 1;
                        }
                        tt += 1;
                        if tt == r {
                            break;
                        }
                    }
                    r = s;
                }
                if t == self.par_token && !long {
                    fatal("runaway argument");
                }
                if t < RIGHT_BRACE_LIMIT {
                    if t >= LEFT_BRACE_LIMIT {
                        fatal("argument of macro has an extra }");
                    }
                    self.push_arg(t);
                    let mut unbalance = 1u32;
                    loop {
                        let (t, _) = self.get_next();
                        if t == self.par_token && !long {
                            fatal("runaway argument");
                        }
                        self.push_arg(t);
                        if t < RIGHT_BRACE_LIMIT {
                            if t < LEFT_BRACE_LIMIT {
                                unbalance += 1;
                            } else {
                                unbalance -= 1;
                                if unbalance == 0 {
                                    break;
                                }
                            }
                        }
                    }
                } else {
                    let n = self.mem[r];
                    if t == SPACE_TOKEN && (is_match(n) || n == END_MATCH_TOKEN) {
                        continue 'cont;
                    }
                    self.push_arg(t);
                }
                m += 1;
                let n = self.mem[r];
                if !is_match(n) && n != END_MATCH_TOKEN {
                    continue 'cont;
                }
                break 'cont;
            }
            if s != usize::MAX {
                let mut a = start;
                let mut b = self.lifo.len() as u32;
                let last = self.lifo.last().map_or(0, |&x| x as u32);
                if m == 1 && last < RIGHT_BRACE_LIMIT && b > a {
                    a += 1;
                    b -= 1;
                }
                self.ps.push(a);
                self.ps.push(b);
            }
            if self.mem[r] == END_MATCH_TOKEN {
                return;
            }
        }
    }
}
