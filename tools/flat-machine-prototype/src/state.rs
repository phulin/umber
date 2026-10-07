//! Token-list references, eqtb assignment and the save stack.

use super::*;

impl Machine {
    #[inline(always)]
    pub(crate) fn add_ref(&mut self, l: usize) {
        if self.mem[l + 1] != self.epoch {
            self.log_list(l);
        }
        self.mem[l] += 1;
    }

    #[inline(always)]
    pub(crate) fn del_ref(&mut self, l: usize) {
        if self.mem[l + 1] != self.epoch {
            self.log_list(l);
        }
        let r = self.mem[l] - 1;
        self.mem[l] = r;
        if r == 0 {
            self.free_list(l);
        }
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn log_list(&mut self, l: usize) {
        self.undo.push(Undo {
            slot: l as u32,
            level: LIST_UNDO,
            epoch: self.mem[l + 1],
            word: self.mem[l] as u64,
        });
        self.mem[l + 1] = self.epoch;
    }

    #[inline(never)]
    pub(crate) fn free_list(&mut self, l: usize) {
        let size = HDR + self.mem[l + 2] as usize;
        if (l as u32) < self.mark {
            // The live checkpoint still needs it.
            self.deferred.push(l as u32);
            return;
        }
        if l + size == self.mem.len() {
            self.mem.truncate(l);
            return;
        }
        if size < FREE_CLASSES {
            self.mem[l] = self.free[size];
            self.free[size] = l as u32;
        }
    }

    // ------------------------------------------------------------ eqtb

    #[inline(always)]
    pub(crate) fn eq_destroy(&mut self, w: u64) {
        if (CALL..=LONG_OUTER_CALL).contains(&cmd(w)) {
            self.del_ref(chr(w) as usize);
        }
    }

    pub(crate) fn eq_define(&mut self, p: usize, w: u64) {
        let old = self.eqtb[p];
        if old == w {
            self.eq_destroy(w);
            return;
        }
        let m = self.meta[p];
        if m == self.cur_meta {
            self.eq_destroy(old);
        } else {
            self.define_slow(p, old, m);
        }
        self.eqtb[p] = w;
    }

    #[inline(never)]
    pub(crate) fn define_slow(&mut self, p: usize, old: u64, m: u64) {
        if m as u32 == self.cur_level {
            self.eq_destroy(old);
        } else if self.cur_level > LEVEL_ONE {
            self.save.push(Save {
                p: p as u32,
                level: m as u32,
                word: old,
            });
        }
        if (m >> 32) as u32 != self.epoch {
            self.log_eqtb(p, m);
        }
        self.meta[p] = self.cur_meta;
    }

    pub(crate) fn eq_word_define(&mut self, p: usize, w: u64) {
        let old = self.eqtb[p];
        if old == w {
            return;
        }
        let m = self.meta[p];
        if m != self.cur_meta {
            if m as u32 != self.cur_level {
                self.save.push(Save {
                    p: p as u32,
                    level: m as u32,
                    word: old,
                });
            }
            if (m >> 32) as u32 != self.epoch {
                self.log_eqtb(p, m);
            }
            self.meta[p] = self.cur_meta;
        }
        self.eqtb[p] = w;
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn log_eqtb(&mut self, p: usize, m: u64) {
        self.undo.push(Undo {
            slot: p as u32,
            level: m as u32,
            epoch: (m >> 32) as u32,
            word: self.eqtb[p],
        });
    }

    pub(crate) fn new_save_level(&mut self, group: u8) {
        if self.cur_level == u16::MAX as u32 {
            fatal("grouping levels");
        }
        self.save.push(Save {
            p: BOUNDARY,
            level: self.cur_boundary,
            word: self.cur_group as u64,
        });
        self.cur_boundary = (self.save.len() - 1) as u32;
        self.cur_group = group;
        self.cur_level += 1;
        self.cur_meta += 1;
    }

    pub(crate) fn unsave(&mut self) {
        self.cur_level -= 1;
        self.cur_meta -= 1;
        loop {
            let e = match self.save.pop() {
                Some(e) => e,
                None => fatal("save stack underflow"),
            };
            if e.p == BOUNDARY {
                self.cur_group = e.word as u8;
                self.cur_boundary = e.level;
                return;
            }
            let p = e.p as usize;
            let m = self.meta[p];
            let meaning = p < RAW_BASE;
            if m as u32 == LEVEL_ONE {
                if meaning {
                    self.eq_destroy(e.word);
                }
            } else {
                if meaning {
                    self.eq_destroy(self.eqtb[p]);
                }
                if (m >> 32) as u32 != self.epoch {
                    self.log_eqtb(p, m);
                }
                self.eqtb[p] = e.word;
                self.meta[p] = (self.epoch as u64) << 32 | e.level as u64;
            }
        }
    }
}
