use tex_arith::Scaled;

// TeX82 map: `tex.web`'s `movement` and `prune_movements` procedures.  The
// newest-to-oldest search, its six `info` states, and the order in which y/z
// (or w/x) hits restrict intervening entries are semantic: changing any of
// them changes later opcode reuse.  `emit_explicit_movement` is the final
// `Generate a down or right command` fragment (tex.web §611), including the
// DVI operands' signed one/two/three/four-byte widths and two's-complement
// byte order.
//
// Umber policy: a page is staged in one growable byte vector, so every prior
// opcode remains patchable; TeX's `dvi_gone` rejection for already-flushed
// ring-buffer bytes is consequently unnecessary.  The two independent
// MovementStack values in DviBodyCompiler are TeX's right and down stacks.

const Y0_OFFSET: u8 = 161 - 157;
const Z0_OFFSET: u8 = 166 - 157;
const Y1_OFFSET: u8 = 162 - 157;
const Z1_OFFSET: u8 = 167 - 157;

const THREE_BYTE_MIN: i32 = -0o40000000;
const THREE_BYTE_MAX: i32 = 0o37777777;

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
    if (i32::from(i8::MIN)..=i32::from(i8::MAX)).contains(&w) {
        bytes.push(o); // down1 or right1
        bytes.push(w as u8);
    } else if (i32::from(i16::MIN)..=i32::from(i16::MAX)).contains(&w) {
        bytes.push(o + 1); // down2 or right2
        bytes.extend_from_slice(&(w as i16).to_be_bytes());
    } else if (THREE_BYTE_MIN..=THREE_BYTE_MAX).contains(&w) {
        bytes.push(o + 2); // down3 or right3
        let encoded = w.to_be_bytes();
        bytes.extend_from_slice(&encoded[1..]);
    } else {
        bytes.push(o + 3); // down4 or right4
        bytes.extend_from_slice(&w.to_be_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dvi::opcodes::RIGHT1;

    fn entry(width: i32, location: usize, info: MovementInfo) -> MovementEntry {
        MovementEntry {
            width,
            location,
            info,
        }
    }

    /// TeX.web §§611-615: exhaust the movement search's three scan states,
    /// six entry states, both register rewrites, both crossed-register stops,
    /// both restriction tables, and the inclusive prune boundary.
    #[test]
    fn movement_search_reuse_and_prune_matrix() {
        let infos = [
            MovementInfo::YHere,
            MovementInfo::ZHere,
            MovementInfo::YzOk,
            MovementInfo::YOk,
            MovementInfo::ZOk,
            MovementInfo::DFixed,
        ];
        let states = [MovementState::None, MovementState::Y, MovementState::Z];
        for state in states {
            for info in infos {
                let mut entries = vec![entry(7, 0, info)];
                match state {
                    MovementState::None => {}
                    MovementState::Y => entries.push(entry(8, 1, MovementInfo::YHere)),
                    MovementState::Z => entries.push(entry(8, 1, MovementInfo::ZHere)),
                }
                let q = entries.len();
                let mut stack = MovementStack { entries };
                let mut bytes = vec![RIGHT1; q];
                let hit = stack.find_hit(q, 7, &mut bytes).map(|(_, info)| info);
                let expected = match (state, info) {
                    (MovementState::None, MovementInfo::YzOk | MovementInfo::YOk)
                    | (MovementState::Z, MovementInfo::YzOk | MovementInfo::YOk) => {
                        Some(MovementInfo::YHere)
                    }
                    (MovementState::None, MovementInfo::ZOk)
                    | (MovementState::Y, MovementInfo::YzOk | MovementInfo::ZOk) => {
                        Some(MovementInfo::ZHere)
                    }
                    (MovementState::None, MovementInfo::YHere | MovementInfo::ZHere)
                    | (MovementState::Y, MovementInfo::ZHere)
                    | (MovementState::Z, MovementInfo::YHere) => Some(info),
                    _ => None,
                };
                assert_eq!(hit, expected, "scan={state:?}, info={info:?}");
            }
        }

        for intervening in [
            [MovementInfo::YHere, MovementInfo::ZHere],
            [MovementInfo::ZHere, MovementInfo::YHere],
        ] {
            let mut stack = MovementStack {
                entries: vec![
                    entry(7, 0, MovementInfo::YzOk),
                    entry(8, 1, intervening[0]),
                    entry(9, 2, intervening[1]),
                ],
            };
            assert_eq!(stack.find_hit(3, 7, &mut [RIGHT1; 3]), None);
        }

        let mut y = MovementStack {
            entries: infos
                .into_iter()
                .enumerate()
                .map(|(i, info)| entry(0, i, info))
                .collect(),
        };
        y.restrict_intervening_y(6, 0);
        assert_eq!(
            y.entries.iter().map(|entry| entry.info).collect::<Vec<_>>(),
            [
                MovementInfo::YHere,
                MovementInfo::ZHere,
                MovementInfo::ZOk,
                MovementInfo::DFixed,
                MovementInfo::ZOk,
                MovementInfo::DFixed
            ]
        );
        let mut z = MovementStack {
            entries: infos
                .into_iter()
                .enumerate()
                .map(|(i, info)| entry(0, i, info))
                .collect(),
        };
        z.restrict_intervening_z(6, 0);
        assert_eq!(
            z.entries.iter().map(|entry| entry.info).collect::<Vec<_>>(),
            [
                MovementInfo::YHere,
                MovementInfo::ZHere,
                MovementInfo::YOk,
                MovementInfo::YOk,
                MovementInfo::DFixed,
                MovementInfo::DFixed
            ]
        );

        let mut stack = MovementStack {
            entries: vec![
                entry(1, 3, MovementInfo::YzOk),
                entry(2, 4, MovementInfo::YzOk),
                entry(3, 5, MovementInfo::YzOk),
            ],
        };
        stack.prune_movements(4);
        assert_eq!(stack.entries, [entry(1, 3, MovementInfo::YzOk)]);
    }
}
