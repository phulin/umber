use std::collections::BTreeSet;
use std::fmt;

use super::opcodes::{
    BOP, DOWN1, EOP, FNT_DEF1, FNT_DEF4, FNT_NUM_0, FNT1, ID_BYTE, PADDING, POP, POST, POST_POST,
    PRE, PUSH, PUT_RULE, RIGHT1, SET_RULE, SET1, XXX1, XXX4,
};

#[cfg(test)]
mod tests;

/// Page metadata recovered from a DVI file's `post`/`bop` backpointer chain.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DviPage {
    /// Zero-based page ordinal in physical output order.
    pub index: usize,
    /// Byte offset of the page's `bop` opcode.
    pub bop_offset: usize,
    /// Byte offset immediately after the page's `eop`, when it can be found
    /// before the next backpointer-known boundary.
    pub eop_end: Option<usize>,
    /// TeX count registers written by `bop`.
    pub counts: [i32; 10],
    /// Previous `bop` pointer recorded in this page.
    pub previous_bop: i32,
}

/// DVI framing metadata recovered without scanning forward through pages.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DviFile {
    pub pages: Vec<DviPage>,
    pub post_offset: usize,
    commands: Vec<Vec<DviCommand>>,
}

/// One decoded DVI command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DviCommand {
    pub offset: usize,
    pub end: usize,
    pub opcode: u8,
    pub name: &'static str,
    pub text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DviDisasmError {
    MissingPostPost,
    InvalidPostPointer {
        offset: usize,
    },
    InvalidBopPointer {
        offset: i32,
    },
    Truncated {
        offset: usize,
        needed: usize,
    },
    BadOpcode {
        offset: usize,
        opcode: u8,
    },
    PageOutOfRange {
        page: usize,
        pages: usize,
    },
    BopCycle {
        offset: usize,
    },
    NonMonotonicBop {
        current: usize,
        previous: usize,
    },
    PageCountMismatch {
        declared: usize,
        actual: usize,
    },
    MissingEop {
        page: usize,
        bop_offset: usize,
    },
    CommandCrossesPageBoundary {
        offset: usize,
        end: usize,
        boundary: usize,
    },
}

impl fmt::Display for DviDisasmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingPostPost => f.write_str("DVI is missing a valid post_post trailer"),
            Self::InvalidPostPointer { offset } => {
                write!(
                    f,
                    "DVI post pointer {offset} does not point at a post opcode"
                )
            }
            Self::InvalidBopPointer { offset } => {
                write!(f, "DVI bop pointer {offset} is invalid")
            }
            Self::Truncated { offset, needed } => {
                write!(
                    f,
                    "DVI command at byte {offset} needs {needed} bytes beyond the available input"
                )
            }
            Self::BadOpcode { offset, opcode } => {
                write!(
                    f,
                    "DVI command at byte {offset} has undefined opcode {opcode}"
                )
            }
            Self::PageOutOfRange { page, pages } => {
                write!(f, "DVI page {} is out of range for {pages} pages", page + 1)
            }
            Self::BopCycle { offset } => {
                write!(f, "DVI bop backpointer graph cycles at byte {offset}")
            }
            Self::NonMonotonicBop { current, previous } => write!(
                f,
                "DVI bop at byte {current} points forward to byte {previous}"
            ),
            Self::PageCountMismatch { declared, actual } => write!(
                f,
                "DVI postamble declares {declared} pages but the bop chain contains {actual}"
            ),
            Self::MissingEop { page, bop_offset } => write!(
                f,
                "DVI page {} at byte {bop_offset} has no eop before its boundary",
                page + 1
            ),
            Self::CommandCrossesPageBoundary {
                offset,
                end,
                boundary,
            } => write!(
                f,
                "DVI command at byte {offset} ends at {end}, beyond page boundary {boundary}"
            ),
        }
    }
}

impl std::error::Error for DviDisasmError {}

impl DviFile {
    /// Recovers pages by following the final `bop` pointer from the postamble.
    ///
    /// This is deliberately not a forward scan through page bodies, so callers
    /// can still locate a divergent page when bytes inside an earlier page are
    /// corrupt or have changed length.
    pub fn parse(bytes: &[u8]) -> Result<Self, DviDisasmError> {
        let post_offset = post_offset_from_trailer(bytes)?;
        if bytes.get(post_offset) != Some(&POST) {
            return Err(DviDisasmError::InvalidPostPointer {
                offset: post_offset,
            });
        }
        let declared_pages = usize::from(read_u16(bytes, post_offset + 27)?);
        let mut pointer = read_i32(bytes, post_offset + 1)?;
        let mut reversed = Vec::new();
        let mut visited = BTreeSet::new();
        let mut later_offset = post_offset;
        while pointer >= 0 {
            let offset = usize::try_from(pointer)
                .map_err(|_| DviDisasmError::InvalidBopPointer { offset: pointer })?;
            if !visited.insert(offset) {
                return Err(DviDisasmError::BopCycle { offset });
            }
            if offset >= later_offset {
                return Err(DviDisasmError::NonMonotonicBop {
                    current: later_offset,
                    previous: offset,
                });
            }
            if reversed.len() == declared_pages {
                return Err(DviDisasmError::PageCountMismatch {
                    declared: declared_pages,
                    actual: reversed.len() + 1,
                });
            }
            if bytes.get(offset) != Some(&BOP) {
                return Err(DviDisasmError::InvalidBopPointer { offset: pointer });
            }
            let mut counts = [0; 10];
            for (slot, count) in counts.iter_mut().enumerate() {
                *count = read_i32(bytes, offset + 1 + slot * 4)?;
            }
            let previous_bop = read_i32(bytes, offset + 41)?;
            reversed.push(DviPage {
                index: 0,
                bop_offset: offset,
                eop_end: None,
                counts,
                previous_bop,
            });
            later_offset = offset;
            pointer = previous_bop;
        }

        if reversed.len() != declared_pages {
            return Err(DviDisasmError::PageCountMismatch {
                declared: declared_pages,
                actual: reversed.len(),
            });
        }

        reversed.reverse();
        let offsets: Vec<usize> = reversed.iter().map(|page| page.bop_offset).collect();
        let mut commands = Vec::with_capacity(reversed.len());
        for (index, page) in reversed.iter_mut().enumerate() {
            page.index = index;
            let boundary = offsets.get(index + 1).copied().unwrap_or(post_offset);
            let page_commands = decode_page_commands(bytes, page.bop_offset, boundary, index)?;
            page.eop_end = page_commands.last().map(|command| command.end);
            commands.push(page_commands);
        }
        Ok(Self {
            pages: reversed,
            post_offset,
            commands,
        })
    }

    pub fn page_for_offset(&self, offset: usize) -> Option<&DviPage> {
        self.pages.iter().find(|page| {
            let end = self
                .pages
                .get(page.index + 1)
                .map_or(self.post_offset, |next| next.bop_offset);
            page.bop_offset <= offset && offset < end
        })
    }

    pub fn disassemble_page(&self, page: usize) -> Result<String, DviDisasmError> {
        let page_meta = self.page(page)?;
        let mut out = String::new();
        out.push_str(&format!(
            "page {} count0={} bop={}\n",
            page + 1,
            page_meta.counts[0],
            page_meta.bop_offset
        ));
        for command in &self.commands[page] {
            out.push_str(&command.text);
            out.push('\n');
        }
        Ok(out)
    }

    pub fn command_at_or_before(
        &self,
        page: usize,
        offset: usize,
    ) -> Result<Option<DviCommand>, DviDisasmError> {
        self.page(page)?;
        let mut latest = None;
        for command in &self.commands[page] {
            if command.offset <= offset && offset < command.end {
                return Ok(Some(command.clone()));
            }
            if command.offset <= offset {
                latest = Some(command.clone());
            }
        }
        Ok(latest)
    }

    fn page(&self, page: usize) -> Result<&DviPage, DviDisasmError> {
        self.pages.get(page).ok_or(DviDisasmError::PageOutOfRange {
            page,
            pages: self.pages.len(),
        })
    }
}

pub fn disassemble_page(bytes: &[u8], page: usize) -> Result<String, DviDisasmError> {
    DviFile::parse(bytes)?.disassemble_page(page)
}

pub fn command_at_or_before(
    bytes: &[u8],
    page: usize,
    offset: usize,
) -> Result<Option<DviCommand>, DviDisasmError> {
    DviFile::parse(bytes)?.command_at_or_before(page, offset)
}

pub fn opcode_name(opcode: u8) -> &'static str {
    match opcode {
        0..=127 => "setchar",
        SET1..=131 => match opcode {
            128 => "set1",
            129 => "set2",
            130 => "set3",
            _ => "set4",
        },
        SET_RULE => "setrule",
        133..=136 => match opcode {
            133 => "put1",
            134 => "put2",
            135 => "put3",
            _ => "put4",
        },
        PUT_RULE => "putrule",
        138 => "nop",
        BOP => "bop",
        EOP => "eop",
        PUSH => "push",
        POP => "pop",
        RIGHT1..=146 => match opcode {
            143 => "right1",
            144 => "right2",
            145 => "right3",
            _ => "right4",
        },
        147 => "w0",
        148..=151 => match opcode {
            148 => "w1",
            149 => "w2",
            150 => "w3",
            _ => "w4",
        },
        152 => "x0",
        153..=156 => match opcode {
            153 => "x1",
            154 => "x2",
            155 => "x3",
            _ => "x4",
        },
        DOWN1..=160 => match opcode {
            157 => "down1",
            158 => "down2",
            159 => "down3",
            _ => "down4",
        },
        161 => "y0",
        162..=165 => match opcode {
            162 => "y1",
            163 => "y2",
            164 => "y3",
            _ => "y4",
        },
        166 => "z0",
        167..=170 => match opcode {
            167 => "z1",
            168 => "z2",
            169 => "z3",
            _ => "z4",
        },
        FNT_NUM_0..=234 => "fntnum",
        FNT1..=238 => match opcode {
            235 => "fnt1",
            236 => "fnt2",
            237 => "fnt3",
            _ => "fnt4",
        },
        XXX1..=XXX4 => match opcode {
            239 => "xxx1",
            240 => "xxx2",
            241 => "xxx3",
            _ => "xxx4",
        },
        FNT_DEF1..=FNT_DEF4 => match opcode {
            243 => "fntdef1",
            244 => "fntdef2",
            245 => "fntdef3",
            _ => "fntdef4",
        },
        PRE => "pre",
        POST => "post",
        POST_POST => "postpost",
        _ => "undefined",
    }
}

fn decode_page_commands(
    bytes: &[u8],
    start: usize,
    boundary: usize,
    page: usize,
) -> Result<Vec<DviCommand>, DviDisasmError> {
    let mut commands = Vec::new();
    let mut offset = start;
    while offset < boundary {
        let command = decode_command(bytes, offset)?;
        let end = command.end;
        if end > boundary {
            return Err(DviDisasmError::CommandCrossesPageBoundary {
                offset,
                end,
                boundary,
            });
        }
        let is_eop = command.opcode == EOP;
        commands.push(command);
        offset = end;
        if is_eop {
            return Ok(commands);
        }
    }
    Err(DviDisasmError::MissingEop {
        page,
        bop_offset: start,
    })
}

fn decode_command(bytes: &[u8], offset: usize) -> Result<DviCommand, DviDisasmError> {
    let opcode = *bytes
        .get(offset)
        .ok_or(DviDisasmError::Truncated { offset, needed: 1 })?;
    if opcode >= 250 {
        return Err(DviDisasmError::BadOpcode { offset, opcode });
    }
    let name = opcode_name(opcode);
    let mut end = offset + 1;
    let text = match opcode {
        0..=127 => format!("{offset}: setchar{opcode}"),
        SET1..=131 | 133..=136 | FNT1..=238 => {
            let width = operand_width(opcode);
            let value = read_unsigned(bytes, end, width)?;
            end += width;
            format!("{offset}: {name} {value}")
        }
        SET_RULE | PUT_RULE => {
            let height = read_i32(bytes, end)?;
            let width = read_i32(bytes, end + 4)?;
            end += 8;
            format!("{offset}: {name} height={height} width={width}")
        }
        138 | EOP | PUSH | POP | 147 | 152 | 161 | 166 => {
            format!("{offset}: {name}")
        }
        BOP => {
            let mut counts = [0; 10];
            for (slot, count) in counts.iter_mut().enumerate() {
                *count = read_i32(bytes, end + slot * 4)?;
            }
            let previous = read_i32(bytes, end + 40)?;
            end += 44;
            format!("{offset}: bop {:?} previous={previous}", counts)
        }
        RIGHT1..=146 | 148..=151 | 153..=160 | 162..=165 | 167..=170 => {
            let width = movement_width(opcode);
            let value = read_signed(bytes, end, width)?;
            end += width;
            format!("{offset}: {name} {value}")
        }
        FNT_NUM_0..=234 => format!("{offset}: fntnum{}", opcode - FNT_NUM_0),
        XXX1..=XXX4 => {
            let width = usize::from(opcode - XXX1 + 1);
            let len = read_unsigned(bytes, end, width)?;
            end += width;
            let len = usize::try_from(len).map_err(|_| DviDisasmError::Truncated {
                offset: end,
                needed: usize::MAX,
            })?;
            let payload = take(bytes, end, len)?;
            end += len;
            format!(
                "{offset}: {name} '{}'{}",
                printable(payload),
                if payload.len() == len {
                    ""
                } else {
                    " (truncated)"
                }
            )
        }
        FNT_DEF1..=FNT_DEF4 => {
            let width = usize::from(opcode - FNT_DEF1 + 1);
            let font = read_unsigned(bytes, end, width)?;
            end += width;
            let checksum = read_u32(bytes, end)?;
            let scale = read_u32(bytes, end + 4)?;
            let design = read_u32(bytes, end + 8)?;
            let area_len = usize::from(*take(bytes, end + 12, 1)?.first().ok_or(
                DviDisasmError::Truncated {
                    offset: end + 12,
                    needed: 1,
                },
            )?);
            let name_len = usize::from(*take(bytes, end + 13, 1)?.first().ok_or(
                DviDisasmError::Truncated {
                    offset: end + 13,
                    needed: 1,
                },
            )?);
            end += 14;
            let area = take(bytes, end, area_len)?;
            end += area_len;
            let font_name = take(bytes, end, name_len)?;
            end += name_len;
            format!(
                "{offset}: {name} {font}: {}{} checksum={checksum} scale={scale} design={design}",
                printable(area),
                printable(font_name)
            )
        }
        PRE => {
            let id = *take(bytes, end, 1)?
                .first()
                .ok_or(DviDisasmError::Truncated {
                    offset: end,
                    needed: 1,
                })?;
            let num = read_i32(bytes, end + 1)?;
            let den = read_i32(bytes, end + 5)?;
            let mag = read_i32(bytes, end + 9)?;
            let len = usize::from(*take(bytes, end + 13, 1)?.first().ok_or(
                DviDisasmError::Truncated {
                    offset: end + 13,
                    needed: 1,
                },
            )?);
            end += 14;
            let comment = take(bytes, end, len)?;
            end += len;
            format!(
                "{offset}: pre id={id} num={num} den={den} mag={mag} '{}'",
                printable(comment)
            )
        }
        POST => {
            let final_bop = read_i32(bytes, end)?;
            let num = read_i32(bytes, end + 4)?;
            let den = read_i32(bytes, end + 8)?;
            let mag = read_i32(bytes, end + 12)?;
            let max_height = read_i32(bytes, end + 16)?;
            let max_width = read_i32(bytes, end + 20)?;
            let stack = read_u16(bytes, end + 24)?;
            let pages = read_u16(bytes, end + 26)?;
            end += 28;
            format!(
                "{offset}: post final_bop={final_bop} num={num} den={den} mag={mag} max_height={max_height} max_width={max_width} stack={stack} pages={pages}"
            )
        }
        POST_POST => {
            let post = read_i32(bytes, end)?;
            let id = *take(bytes, end + 4, 1)?
                .first()
                .ok_or(DviDisasmError::Truncated {
                    offset: end + 4,
                    needed: 1,
                })?;
            end += 5;
            while bytes.get(end) == Some(&PADDING) {
                end += 1;
            }
            format!("{offset}: postpost post={post} id={id}")
        }
        _ => return Err(DviDisasmError::BadOpcode { offset, opcode }),
    };
    Ok(DviCommand {
        offset,
        end,
        opcode,
        name,
        text,
    })
}

fn post_offset_from_trailer(bytes: &[u8]) -> Result<usize, DviDisasmError> {
    let mut index = bytes.len();
    while index > 0 && bytes[index - 1] == PADDING {
        index -= 1;
    }
    if index < 6 || bytes[index - 1] != ID_BYTE || bytes[index - 6] != POST_POST {
        return Err(DviDisasmError::MissingPostPost);
    }
    let pointer = read_u32(bytes, index - 5)?;
    usize::try_from(pointer).map_err(|_| DviDisasmError::InvalidPostPointer { offset: usize::MAX })
}

fn operand_width(opcode: u8) -> usize {
    match opcode {
        128 | 133 | 235 => 1,
        129 | 134 | 236 => 2,
        130 | 135 | 237 => 3,
        131 | 136 | 238 => 4,
        _ => 0,
    }
}

fn movement_width(opcode: u8) -> usize {
    match opcode {
        143 | 148 | 153 | 157 | 162 | 167 => 1,
        144 | 149 | 154 | 158 | 163 | 168 => 2,
        145 | 150 | 155 | 159 | 164 | 169 => 3,
        146 | 151 | 156 | 160 | 165 | 170 => 4,
        _ => 0,
    }
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, DviDisasmError> {
    let slice = take(bytes, offset, 2)?;
    Ok(u16::from_be_bytes([slice[0], slice[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, DviDisasmError> {
    let slice = take(bytes, offset, 4)?;
    Ok(u32::from_be_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn read_i32(bytes: &[u8], offset: usize) -> Result<i32, DviDisasmError> {
    let slice = take(bytes, offset, 4)?;
    Ok(i32::from_be_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn read_unsigned(bytes: &[u8], offset: usize, width: usize) -> Result<u32, DviDisasmError> {
    let slice = take(bytes, offset, width)?;
    let mut value = 0u32;
    for byte in slice {
        value = (value << 8) | u32::from(*byte);
    }
    Ok(value)
}

fn read_signed(bytes: &[u8], offset: usize, width: usize) -> Result<i32, DviDisasmError> {
    let value = read_unsigned(bytes, offset, width)?;
    let value = i64::from(value);
    let sign_bit = 1i64 << (width * 8 - 1);
    let full = 1i64 << (width * 8);
    if value & sign_bit == 0 {
        Ok(i32::try_from(value).map_err(|_| DviDisasmError::Truncated {
            offset,
            needed: width,
        })?)
    } else {
        Ok(
            i32::try_from(value - full).map_err(|_| DviDisasmError::Truncated {
                offset,
                needed: width,
            })?,
        )
    }
}

fn take(bytes: &[u8], offset: usize, len: usize) -> Result<&[u8], DviDisasmError> {
    let end = offset.checked_add(len).ok_or(DviDisasmError::Truncated {
        offset,
        needed: len,
    })?;
    bytes.get(offset..end).ok_or(DviDisasmError::Truncated {
        offset,
        needed: len,
    })
}

fn printable(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .chars()
        .flat_map(char::escape_default)
        .collect()
}
