use tex_arith::Scaled;

// TeX82 map: `tex.web`'s `movement` and `prune_movements` procedures.  The
// newest-to-oldest search, its six `info` states, and the order in which y/z
// (or w/x) hits restrict intervening entries are semantic: changing any of
// them changes later opcode reuse.  `emit_explicit_movement` is the final
// `Generate a down or right command` fragment, including TeX's exact signed
// one/two/three/four-byte thresholds and two's-complement byte order.
//
// Umber policy: a page is staged in one growable byte vector, so every prior
// opcode remains patchable; TeX's `dvi_gone` rejection for already-flushed
// ring-buffer bytes is consequently unnecessary.  The two independent
// MovementStack values in DviWriter are TeX's right and down stacks.

const Y0_OFFSET: u8 = 161 - 157;
const Z0_OFFSET: u8 = 166 - 157;
const Y1_OFFSET: u8 = 162 - 157;
const Z1_OFFSET: u8 = 167 - 157;

const ONE_BYTE_LIMIT: i64 = 0o200;
const TWO_BYTE_LIMIT: i64 = 0o100000;
const THREE_BYTE_LIMIT: i64 = 0o40000000;

#[derive(Clone, Debug, Default)]
pub(super) struct MovementStack {
    entries: Vec<MovementEntry>,
}

impl MovementStack {
    pub(super) fn clear(&mut self) {
        self.entries.clear();
    }

    pub(super) fn prune_movements(&mut self, save_loc: usize) {
        while self
            .entries
            .last()
            .is_some_and(|entry| entry.location >= save_loc)
        {
            self.entries.pop();
        }
    }

    pub(super) fn movement(&mut self, bytes: &mut Vec<u8>, w: Scaled, o: u8) {
        let q = self.entries.len();
        self.entries.push(MovementEntry {
            width: w.raw(),
            location: bytes.len(),
            info: MovementInfo::YzOk,
        });

        if let Some((p, info)) = self.find_hit(q, w.raw(), bytes) {
            self.entries[q].info = info;
            if info == MovementInfo::YHere {
                bytes.push(o + Y0_OFFSET); // y0 or w0
                self.restrict_intervening_y(q, p);
            } else {
                bytes.push(o + Z0_OFFSET); // z0 or x0
                self.restrict_intervening_z(q, p);
            }
            return;
        }

        self.entries[q].info = MovementInfo::YzOk;
        emit_explicit_movement(bytes, w.raw(), o);
    }

    fn find_hit(&mut self, q: usize, w: i32, bytes: &mut [u8]) -> Option<(usize, MovementInfo)> {
        let mut mstate = MovementState::None;
        for p in (0..q).rev() {
            let info = self.entries[p].info;
            if self.entries[p].width == w {
                match (mstate, info) {
                    (
                        MovementState::None | MovementState::Z,
                        MovementInfo::YzOk | MovementInfo::YOk,
                    ) => {
                        bytes[self.entries[p].location] += Y1_OFFSET;
                        self.entries[p].info = MovementInfo::YHere;
                        return Some((p, MovementInfo::YHere));
                    }
                    (
                        MovementState::None | MovementState::Y,
                        MovementInfo::YzOk | MovementInfo::ZOk,
                    ) => {
                        bytes[self.entries[p].location] += Z1_OFFSET;
                        self.entries[p].info = MovementInfo::ZHere;
                        return Some((p, MovementInfo::ZHere));
                    }
                    (MovementState::None, MovementInfo::YHere | MovementInfo::ZHere)
                    | (MovementState::Y, MovementInfo::ZHere)
                    | (MovementState::Z, MovementInfo::YHere) => return Some((p, info)),
                    _ => {}
                }
            } else {
                match (mstate, info) {
                    (MovementState::None, MovementInfo::YHere) => {
                        mstate = MovementState::Y;
                    }
                    (MovementState::None, MovementInfo::ZHere) => {
                        mstate = MovementState::Z;
                    }
                    (MovementState::Y, MovementInfo::ZHere)
                    | (MovementState::Z, MovementInfo::YHere) => break,
                    _ => {}
                }
            }
        }
        None
    }

    fn restrict_intervening_y(&mut self, q: usize, p: usize) {
        for entry in &mut self.entries[p + 1..q] {
            match entry.info {
                MovementInfo::YzOk => entry.info = MovementInfo::ZOk,
                MovementInfo::YOk => entry.info = MovementInfo::DFixed,
                _ => {}
            }
        }
    }

    fn restrict_intervening_z(&mut self, q: usize, p: usize) {
        for entry in &mut self.entries[p + 1..q] {
            match entry.info {
                MovementInfo::YzOk => entry.info = MovementInfo::YOk,
                MovementInfo::ZOk => entry.info = MovementInfo::DFixed,
                _ => {}
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MovementEntry {
    width: i32,
    location: usize,
    info: MovementInfo,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MovementInfo {
    YHere,
    ZHere,
    YzOk,
    YOk,
    ZOk,
    DFixed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MovementState {
    None,
    Y,
    Z,
}

fn emit_explicit_movement(bytes: &mut Vec<u8>, w: i32, o: u8) {
    let abs_w = i64::from(w).abs();
    if abs_w >= THREE_BYTE_LIMIT {
        bytes.push(o + 3); // down4 or right4
        bytes.extend_from_slice(&w.to_be_bytes());
    } else if abs_w >= TWO_BYTE_LIMIT {
        bytes.push(o + 2); // down3 or right3
        let w = if w < 0 { w + 0o100000000 } else { w };
        bytes.push((w / 0o200000) as u8);
        bytes.push(((w % 0o200000) / 0o400) as u8);
        bytes.push((w % 0o400) as u8);
    } else if abs_w >= ONE_BYTE_LIMIT {
        bytes.push(o + 1); // down2 or right2
        let w = if w < 0 { w + 0o200000 } else { w };
        bytes.push((w / 0o400) as u8);
        bytes.push((w % 0o400) as u8);
    } else {
        bytes.push(o); // down1 or right1
        let w = if w < 0 { w + 0o400 } else { w };
        bytes.push((w % 0o400) as u8);
    }
}
