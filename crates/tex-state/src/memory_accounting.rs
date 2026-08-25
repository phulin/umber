//! Exact constant-time TeX main-memory accounting.

use core::cell::Cell;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct LiveWords {
    tex82_variable: usize,
    tex82_dynamic: usize,
    etex_variable: usize,
    etex_dynamic: usize,
}

/// Generation-local aggregate fed only by real allocation and release events.
#[derive(Clone, Debug, Default)]
pub(crate) struct MemoryAccounting {
    live: Rc<Cell<LiveWords>>,
}

impl MemoryAccounting {
    pub(crate) fn allocate_shared_dynamic(&self, words: usize) {
        self.adjust((0, words), (0, words), true);
    }

    pub(crate) fn release_shared_dynamic(&self, words: usize) {
        self.adjust((0, words), (0, words), false);
    }

    pub(crate) fn allocate_nodes(&self, tex82: (usize, usize), etex: (usize, usize)) {
        self.adjust(tex82, etex, true);
    }

    pub(crate) fn release_nodes(&self, tex82: (usize, usize), etex: (usize, usize)) {
        self.adjust(tex82, etex, false);
    }

    #[must_use]
    pub(crate) fn words(&self, etex_node_sizes: bool) -> (usize, usize) {
        let live = self.live.get();
        if etex_node_sizes {
            (live.etex_variable, live.etex_dynamic)
        } else {
            (live.tex82_variable, live.tex82_dynamic)
        }
    }

    fn adjust(&self, tex82: (usize, usize), etex: (usize, usize), allocate: bool) {
        let mut live = self.live.get();
        let update = |value: usize, delta: usize| {
            if allocate {
                value
                    .checked_add(delta)
                    .expect("TeX memory accounting overflow")
            } else {
                value
                    .checked_sub(delta)
                    .expect("released more TeX memory than was live")
            }
        };
        live.tex82_variable = update(live.tex82_variable, tex82.0);
        live.tex82_dynamic = update(live.tex82_dynamic, tex82.1);
        live.etex_variable = update(live.etex_variable, etex.0);
        live.etex_dynamic = update(live.etex_dynamic, etex.1);
        self.live.set(live);
    }
}
