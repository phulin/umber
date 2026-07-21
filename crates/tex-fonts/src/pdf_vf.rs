//! Bounded, host-neutral parsing of TeX virtual-font programs.

use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

const PRE: u8 = 247;
const VF_ID: u8 = 202;
const LONG_CHAR: u8 = 242;
const FNT_DEF1: u8 = 243;
const POST: u8 = 248;

/// pdfTeX section 32e's maximum nested virtual-font expansion depth.
pub const PDFTEX_VF_MAX_RECURSION: usize = 10;

/// Configurable parser capacities. Every value is checked before the parser
/// performs the allocation or appends the represented item.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VfLimits {
    pub max_input_bytes: usize,
    pub max_local_fonts: usize,
    pub max_packets: usize,
    pub max_packet_bytes: usize,
    pub max_total_packet_bytes: usize,
    pub max_total_commands: usize,
    pub max_total_special_bytes: usize,
    pub max_stack_depth: usize,
}

impl Default for VfLimits {
    fn default() -> Self {
        Self {
            max_input_bytes: 16 * 1024 * 1024,
            max_local_fonts: 4_096,
            max_packets: 65_536,
            // These two defaults match pdftex.web section 32e.
            max_packet_bytes: 10_000,
            max_stack_depth: 100,
            max_total_packet_bytes: 16 * 1024 * 1024,
            max_total_commands: 1_000_000,
            max_total_special_bytes: 8 * 1024 * 1024,
        }
    }
}

/// Stable identity of the exact VF bytes.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct VfProgramIdentity([u8; 32]);

impl VfProgramIdentity {
    #[must_use]
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }
}

/// One local font declared by the VF preamble.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VfLocalFont {
    pub number: i32,
    pub checksum: u32,
    /// A fix-word relative to the containing virtual font's design size.
    pub scaled_size: i32,
    /// An absolute fix-word in printer's points.
    pub design_size: i32,
    pub area: Vec<u8>,
    pub name: Vec<u8>,
}

impl VfLocalFont {
    #[must_use]
    pub fn logical_name(&self) -> Vec<u8> {
        let mut logical_name = Vec::with_capacity(self.area.len() + self.name.len());
        logical_name.extend_from_slice(&self.area);
        logical_name.extend_from_slice(&self.name);
        logical_name
    }
}

/// One normalized command from a character packet's embedded DVI program.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VfCommand {
    SetCharacter {
        code: u32,
        move_cursor: bool,
    },
    Rule {
        height: i32,
        width: i32,
        move_cursor: bool,
    },
    Nop,
    Push,
    Pop,
    MoveRight(i32),
    MoveW,
    SetW(i32),
    MoveX,
    SetX(i32),
    MoveDown(i32),
    MoveY,
    SetY(i32),
    MoveZ,
    SetZ(i32),
    SelectFont(i32),
    Special(Vec<u8>),
}

/// A character edge that recursive lowering may need to resolve.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct VfCharacterReference {
    pub local_font: i32,
    pub character: u32,
}

/// Precomputed packet facts that do not require executing dimensions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VfPacketMetadata {
    pub max_stack_depth: usize,
    pub character_references: Vec<VfCharacterReference>,
}

/// One virtual-character definition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VfPacket {
    pub character: u32,
    pub tfm_width: i32,
    pub commands: Vec<VfCommand>,
    pub metadata: VfPacketMetadata,
}

/// A validated immutable VF program. It contains logical font names only and
/// owns no host resource or live engine handles.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VfProgram {
    identity: VfProgramIdentity,
    comment: Vec<u8>,
    checksum: u32,
    design_size: i32,
    local_fonts: Vec<VfLocalFont>,
    packets: BTreeMap<u32, VfPacket>,
}

impl VfProgram {
    pub fn parse(bytes: &[u8]) -> Result<Self, VfParseError> {
        Self::parse_with_limits(bytes, VfLimits::default())
    }

    pub fn parse_with_limits(bytes: &[u8], limits: VfLimits) -> Result<Self, VfParseError> {
        if bytes.len() > limits.max_input_bytes {
            return Err(VfParseError::InputTooLarge);
        }
        let identity = VfProgramIdentity(Sha256::digest(bytes).into());
        let mut cursor = Cursor::new(bytes);
        if cursor.u8()? != PRE || cursor.u8()? != VF_ID {
            return Err(VfParseError::InvalidPreamble);
        }
        let comment_length = usize::from(cursor.u8()?);
        let comment = cursor.take(comment_length)?.to_vec();
        let checksum = cursor.u32()?;
        let design_size = cursor.i32()?;

        let mut local_fonts = Vec::new();
        let mut local_font_numbers = BTreeSet::new();
        let mut packets = BTreeMap::new();
        let mut saw_packet = false;
        let mut total_packet_bytes = 0usize;
        let mut packet_budget = PacketBudget::new(limits);

        loop {
            let opcode = cursor.u8().map_err(|error| match error {
                VfParseError::Truncated => VfParseError::MissingPostamble,
                other => other,
            })?;
            match opcode {
                FNT_DEF1..=246 => {
                    if saw_packet {
                        return Err(VfParseError::FontDefinitionAfterPacket);
                    }
                    if local_fonts.len() >= limits.max_local_fonts {
                        return Err(VfParseError::TooManyLocalFonts);
                    }
                    let number = cursor.font_number(opcode - FNT_DEF1 + 1)?;
                    if !local_font_numbers.insert(number) {
                        return Err(VfParseError::DuplicateLocalFont(number));
                    }
                    let checksum = cursor.u32()?;
                    let scaled_size = cursor.i32()?;
                    if !(1..(1 << 24)).contains(&scaled_size) {
                        return Err(VfParseError::InvalidLocalFontSize);
                    }
                    let design_size = cursor.i32()?;
                    let area_length = usize::from(cursor.u8()?);
                    let name_length = usize::from(cursor.u8()?);
                    let area = cursor.take(area_length)?.to_vec();
                    let name = cursor.take(name_length)?.to_vec();
                    local_fonts.push(VfLocalFont {
                        number,
                        checksum,
                        scaled_size,
                        design_size,
                        area,
                        name,
                    });
                }
                0..=LONG_CHAR => {
                    saw_packet = true;
                    if packets.len() >= limits.max_packets {
                        return Err(VfParseError::TooManyPackets);
                    }
                    let (packet_length, character, tfm_width) = if opcode == LONG_CHAR {
                        let length = cursor.u32()?;
                        let length =
                            usize::try_from(length).map_err(|_| VfParseError::PacketTooLarge)?;
                        (length, cursor.u32()?, cursor.i32()?)
                    } else {
                        (
                            usize::from(opcode),
                            u32::from(cursor.u8()?),
                            i32::try_from(cursor.unsigned(3)?).expect("u24 fits in i32"),
                        )
                    };
                    if packet_length > limits.max_packet_bytes {
                        return Err(VfParseError::PacketTooLarge);
                    }
                    total_packet_bytes = total_packet_bytes
                        .checked_add(packet_length)
                        .filter(|total| *total <= limits.max_total_packet_bytes)
                        .ok_or(VfParseError::TotalPacketBytesExceeded)?;
                    let packet_bytes = cursor.take(packet_length)?;
                    let packet = parse_packet(
                        character,
                        tfm_width,
                        packet_bytes,
                        local_fonts.first().map(|font| font.number),
                        &local_font_numbers,
                        &mut packet_budget,
                    )?;
                    if packets.insert(character, packet).is_some() {
                        return Err(VfParseError::DuplicateCharacter(character));
                    }
                }
                POST => {
                    while !cursor.is_empty() {
                        if cursor.u8()? != POST {
                            return Err(VfParseError::TrailingData);
                        }
                    }
                    if !bytes.len().is_multiple_of(4) {
                        return Err(VfParseError::InvalidPostamblePadding);
                    }
                    break;
                }
                other => return Err(VfParseError::InvalidTopLevelCommand(other)),
            }
        }

        Ok(Self {
            identity,
            comment,
            checksum,
            design_size,
            local_fonts,
            packets,
        })
    }

    #[must_use]
    pub const fn identity(&self) -> VfProgramIdentity {
        self.identity
    }

    #[must_use]
    pub fn comment(&self) -> &[u8] {
        &self.comment
    }

    #[must_use]
    pub const fn checksum(&self) -> u32 {
        self.checksum
    }

    #[must_use]
    pub const fn design_size(&self) -> i32 {
        self.design_size
    }

    #[must_use]
    pub fn local_fonts(&self) -> &[VfLocalFont] {
        &self.local_fonts
    }

    #[must_use]
    pub fn packet(&self, character: u32) -> Option<&VfPacket> {
        self.packets.get(&character)
    }

    #[must_use]
    pub fn packets(&self) -> impl ExactSizeIterator<Item = &VfPacket> {
        self.packets.values()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VfParseError {
    InputTooLarge,
    InvalidPreamble,
    Truncated,
    MissingPostamble,
    InvalidTopLevelCommand(u8),
    FontDefinitionAfterPacket,
    TooManyLocalFonts,
    DuplicateLocalFont(i32),
    InvalidLocalFontSize,
    TooManyPackets,
    PacketTooLarge,
    TotalPacketBytesExceeded,
    DuplicateCharacter(u32),
    InvalidPacketCommand(u8),
    TooManyCommands,
    SpecialBytesExceeded,
    UndefinedLocalFont(i32),
    NoCurrentFont,
    StackUnderflow,
    StackOverflow,
    UnbalancedStack,
    TrailingData,
    InvalidPostamblePadding,
}

impl std::fmt::Display for VfParseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "invalid TeX virtual font: {self:?}")
    }
}

impl std::error::Error for VfParseError {}

fn parse_packet(
    character: u32,
    tfm_width: i32,
    bytes: &[u8],
    default_font: Option<i32>,
    font_numbers: &BTreeSet<i32>,
    budget: &mut PacketBudget,
) -> Result<VfPacket, VfParseError> {
    let mut current_font = default_font;
    let mut commands = Vec::new();
    let mut references = Vec::new();
    let mut cursor = Cursor::new(bytes);
    let mut stack_depth = 0usize;
    let mut max_stack_depth = 0usize;

    while !cursor.is_empty() {
        budget.total_commands = budget
            .total_commands
            .checked_add(1)
            .filter(|total| *total <= budget.limits.max_total_commands)
            .ok_or(VfParseError::TooManyCommands)?;
        let opcode = cursor.u8()?;
        let command = match opcode {
            0..=127 => character_command(u32::from(opcode), true, current_font, &mut references)?,
            128..=131 => character_command(
                cursor.unsigned(opcode - 127)?,
                true,
                current_font,
                &mut references,
            )?,
            132 | 137 => VfCommand::Rule {
                height: cursor.i32()?,
                width: cursor.i32()?,
                move_cursor: opcode == 132,
            },
            133..=136 => character_command(
                cursor.unsigned(opcode - 132)?,
                false,
                current_font,
                &mut references,
            )?,
            138 => VfCommand::Nop,
            141 => {
                stack_depth = stack_depth
                    .checked_add(1)
                    .filter(|depth| *depth <= budget.limits.max_stack_depth)
                    .ok_or(VfParseError::StackOverflow)?;
                max_stack_depth = max_stack_depth.max(stack_depth);
                VfCommand::Push
            }
            142 => {
                stack_depth = stack_depth
                    .checked_sub(1)
                    .ok_or(VfParseError::StackUnderflow)?;
                VfCommand::Pop
            }
            143..=146 => VfCommand::MoveRight(cursor.signed(opcode - 142)?),
            147 => VfCommand::MoveW,
            148..=151 => VfCommand::SetW(cursor.signed(opcode - 147)?),
            152 => VfCommand::MoveX,
            153..=156 => VfCommand::SetX(cursor.signed(opcode - 152)?),
            157..=160 => VfCommand::MoveDown(cursor.signed(opcode - 156)?),
            161 => VfCommand::MoveY,
            162..=165 => VfCommand::SetY(cursor.signed(opcode - 161)?),
            166 => VfCommand::MoveZ,
            167..=170 => VfCommand::SetZ(cursor.signed(opcode - 166)?),
            171..=234 => {
                let number = i32::from(opcode - 171);
                select_font(number, font_numbers, &mut current_font)?
            }
            235..=238 => {
                let number = cursor.font_number(opcode - 234)?;
                select_font(number, font_numbers, &mut current_font)?
            }
            239..=242 => {
                let length = cursor.unsigned(opcode - 238)?;
                let length =
                    usize::try_from(length).map_err(|_| VfParseError::SpecialBytesExceeded)?;
                budget.total_special_bytes = budget
                    .total_special_bytes
                    .checked_add(length)
                    .filter(|total| *total <= budget.limits.max_total_special_bytes)
                    .ok_or(VfParseError::SpecialBytesExceeded)?;
                VfCommand::Special(cursor.take(length)?.to_vec())
            }
            other => return Err(VfParseError::InvalidPacketCommand(other)),
        };
        commands.push(command);
    }
    if stack_depth != 0 {
        return Err(VfParseError::UnbalancedStack);
    }
    Ok(VfPacket {
        character,
        tfm_width,
        commands,
        metadata: VfPacketMetadata {
            max_stack_depth,
            character_references: references,
        },
    })
}

struct PacketBudget {
    limits: VfLimits,
    total_commands: usize,
    total_special_bytes: usize,
}

impl PacketBudget {
    const fn new(limits: VfLimits) -> Self {
        Self {
            limits,
            total_commands: 0,
            total_special_bytes: 0,
        }
    }
}

fn character_command(
    code: u32,
    move_cursor: bool,
    current_font: Option<i32>,
    references: &mut Vec<VfCharacterReference>,
) -> Result<VfCommand, VfParseError> {
    let local_font = current_font.ok_or(VfParseError::NoCurrentFont)?;
    references.push(VfCharacterReference {
        local_font,
        character: code,
    });
    Ok(VfCommand::SetCharacter { code, move_cursor })
}

fn select_font(
    number: i32,
    font_numbers: &BTreeSet<i32>,
    current_font: &mut Option<i32>,
) -> Result<VfCommand, VfParseError> {
    if !font_numbers.contains(&number) {
        return Err(VfParseError::UndefinedLocalFont(number));
    }
    *current_font = Some(number);
    Ok(VfCommand::SelectFont(number))
}

struct Cursor<'a> {
    bytes: &'a [u8],
    index: usize,
}

impl<'a> Cursor<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, index: 0 }
    }

    fn is_empty(&self) -> bool {
        self.index == self.bytes.len()
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], VfParseError> {
        let end = self
            .index
            .checked_add(length)
            .ok_or(VfParseError::Truncated)?;
        let bytes = self
            .bytes
            .get(self.index..end)
            .ok_or(VfParseError::Truncated)?;
        self.index = end;
        Ok(bytes)
    }

    fn u8(&mut self) -> Result<u8, VfParseError> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, VfParseError> {
        Ok(u32::from_be_bytes(
            self.take(4)?.try_into().expect("length checked"),
        ))
    }

    fn i32(&mut self) -> Result<i32, VfParseError> {
        Ok(i32::from_be_bytes(
            self.take(4)?.try_into().expect("length checked"),
        ))
    }

    fn unsigned(&mut self, length: u8) -> Result<u32, VfParseError> {
        let mut value = 0u32;
        for byte in self.take(usize::from(length))? {
            value = value * 256 + u32::from(*byte);
        }
        Ok(value)
    }

    fn signed(&mut self, length: u8) -> Result<i32, VfParseError> {
        let bytes = self.take(usize::from(length))?;
        let mut value = if bytes[0] & 0x80 == 0 { 0i32 } else { -1i32 };
        for byte in bytes {
            value = value.wrapping_mul(256).wrapping_add(i32::from(*byte));
        }
        Ok(value)
    }

    fn font_number(&mut self, length: u8) -> Result<i32, VfParseError> {
        if length == 4 {
            self.signed(length)
        } else {
            Ok(i32::try_from(self.unsigned(length)?).expect("u24 fits in i32"))
        }
    }
}

#[cfg(test)]
mod tests;
