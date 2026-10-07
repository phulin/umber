//! First-touch checkpoint capture, restore and replay fingerprints.

use super::*;

impl Machine {
    pub(crate) fn take_checkpoint(&mut self) {
        self.epoch += 1;
        self.cur_meta = (self.epoch as u64) << 32 | self.cur_level as u64;
        self.mark = self.mem.len() as u32;
        let frozen = std::mem::replace(&mut self.free, vec![NIL; FREE_CLASSES]);
        self.undo.clear();
        self.deferred.clear();
        let snapshot = self
            .verify
            .then(|| (self.eqtb.clone(), self.meta.clone(), self.mem.clone()));
        self.checkpoint = Some(Box::new(Checkpoint {
            mark: self.mark,
            cur: self.cur,
            stack: self.stack.clone(),
            lifo: self.lifo.clone(),
            ps: self.ps.clone(),
            save: self.save.clone(),
            cond: self.cond.clone(),
            _frozen: frozen,
            cur_level: self.cur_level,
            cur_group: self.cur_group,
            cur_boundary: self.cur_boundary,
            if_limit: self.if_limit,
            cur_if: self.cur_if,
            align_state: self.align_state,
            snapshot,
        }));
    }

    pub(crate) fn restore(&mut self) {
        let cp = self.checkpoint.take().expect("no checkpoint");
        while let Some(u) = self.undo.pop() {
            let s = u.slot as usize;
            if u.level == LIST_UNDO {
                self.mem[s] = u.word as u32;
                self.mem[s + 1] = u.epoch;
            } else {
                self.eqtb[s] = u.word;
                self.meta[s] = (u.epoch as u64) << 32 | u.level as u64;
            }
        }
        self.mem.truncate(cp.mark as usize);
        self.free = vec![NIL; FREE_CLASSES];
        self.deferred.clear();
        self.cur = cp.cur;
        self.stack.clone_from(&cp.stack);
        self.lifo.clone_from(&cp.lifo);
        self.ps.clone_from(&cp.ps);
        self.save.clone_from(&cp.save);
        self.cond.clone_from(&cp.cond);
        self.cur_level = cp.cur_level;
        self.cur_group = cp.cur_group;
        self.cur_boundary = cp.cur_boundary;
        self.if_limit = cp.if_limit;
        self.cur_if = cp.cur_if;
        self.align_state = cp.align_state;
        self.scanner_status = NORMAL;
        self.cur_meta = (self.epoch as u64) << 32 | self.cur_level as u64;
        if let Some((eqtb, meta, mem)) = &cp.snapshot {
            assert!(&self.eqtb == eqtb, "eqtb differs after restore");
            assert!(
                &self.meta == meta,
                "eqtb levels/epochs differ after restore"
            );
            assert!(&self.mem == mem, "token memory differs after restore");
        }
        self.checkpoint = Some(cp);
    }

    /// Meaning-level fingerprint: eqtb words with list references replaced by contents.
    pub(crate) fn fingerprint(&self) -> Vec<Vec<u32>> {
        let mut out = Vec::new();
        for p in 0..EQTB_SIZE {
            let w = self.eqtb[p];
            if p < RAW_BASE && (CALL..=LONG_OUTER_CALL).contains(&cmd(w)) {
                let l = chr(w) as usize;
                let len = self.mem[l + 2] as usize;
                let mut v = vec![cmd(w) as u32, self.mem[l + 3]];
                v.extend_from_slice(&self.mem[l + HDR..l + HDR + len]);
                out.push(v);
            } else {
                out.push(vec![w as u32, (w >> 32) as u32, self.meta[p] as u32]);
            }
        }
        out
    }
}
