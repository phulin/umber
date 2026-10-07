//! The main-control switch and the assignment commands.

use super::*;

impl Machine {
    pub(crate) fn main_control(&mut self) {
        loop {
            let (_, w) = self.get_x_token();
            self.fuel -= 1;
            if self.fuel == 0 {
                self.refuel();
            }
            if self.attention != 0 {
                self.attend(w);
            }
            match cmd(w) {
                RELAX | SPACER => {}
                LETTER | OTHER_CHAR => self.chars += 1,
                LEFT_BRACE => self.new_save_level(SIMPLE_GROUP),
                RIGHT_BRACE => {
                    if self.cur_group != SIMPLE_GROUP {
                        fatal("too many }'s");
                    }
                    self.unsave();
                }
                BEGIN_GROUP => self.new_save_level(SEMI_SIMPLE_GROUP),
                END_GROUP => {
                    if self.cur_group != SEMI_SIMPLE_GROUP {
                        fatal("extra \\endgroup");
                    }
                    self.unsave();
                }
                LET => {
                    self.do_let();
                    self.after_assignment();
                }
                DEF => {
                    self.do_def();
                    self.after_assignment();
                }
                REGISTER | ADVANCE => {
                    self.do_register_command(w);
                    self.after_assignment();
                }
                TOKS_REGISTER => {
                    self.do_toks();
                    self.after_assignment();
                }
                CHECKPOINT => self.take_checkpoint(),
                STOP => return,
                _ => fatal("unimplemented command"),
            }
        }
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn attend(&mut self, _w: u64) {
        fatal("interrupts and tracing unimplemented");
    }

    #[inline(always)]
    pub(crate) fn after_assignment(&mut self) {
        if self.attention != 0 {
            fatal("\\afterassignment unimplemented");
        }
    }

    pub(crate) fn do_let(&mut self) {
        let p = self.get_r_token();
        let (mut t, mut w);
        loop {
            (t, w) = self.get_next();
            if cmd(w) != SPACER {
                break;
            }
        }
        if t == EQUALS_TOKEN {
            (_, w) = self.get_next();
            if cmd(w) == SPACER {
                (_, w) = self.get_next();
            }
        }
        if (CALL..=LONG_OUTER_CALL).contains(&cmd(w)) {
            self.add_ref(chr(w) as usize);
        }
        self.eq_define(p, w);
    }

    pub(crate) fn do_def(&mut self) {
        let p = self.get_r_token();
        let o = self.last_origin();
        let l = self.scan_toks(true, o);
        self.eq_define(p, word(CALL, l));
    }

    pub(crate) fn do_register_command(&mut self, w: u64) {
        let q = cmd(w);
        if q != REGISTER {
            let (_, w2) = self.get_x_token();
            if cmd(w2) != REGISTER {
                fatal("you can't use that after \\advance");
            }
        }
        let l = COUNT_BASE + self.scan_register_num();
        if q == REGISTER {
            self.scan_optional_equals();
        } else {
            self.scan_keyword(b"by");
        }
        let mut v = self.scan_int();
        if q == ADVANCE {
            v = v.wrapping_add(self.eqtb[l] as u32 as i32);
        }
        self.eq_word_define(l, v as u32 as u64);
    }

    pub(crate) fn do_toks(&mut self) {
        let p = TOKS_BASE + self.scan_register_num();
        self.scan_optional_equals();
        let (t, w) = loop {
            let (t, w) = self.get_x_token();
            if cmd(w) != SPACER && cmd(w) != RELAX {
                break (t, w);
            }
        };
        if cmd(w) != LEFT_BRACE {
            fatal("unimplemented toks right-hand side");
        }
        let o = self.last_origin();
        self.back_input(t, o);
        let l = self.scan_toks(false, o);
        if self.mem[l as usize + 2] == 0 {
            self.free_list(l as usize);
            self.eq_define(p, 0);
        } else {
            self.eq_define(p, word(CALL, l));
        }
    }
}
