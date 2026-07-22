use std::collections::BTreeSet;

use super::{FontLimits, FontParseError, OpenTypeTag, VariationCoordinate, VariationInstance};
use crate::VariationSelection;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VariationAxis {
    pub tag: OpenTypeTag,
    pub minimum: i32,
    pub default: i32,
    pub maximum: i32,
    pub name_id: u16,
    pub hidden: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NamedVariationInstance {
    pub subfamily_name_id: u16,
    pub postscript_name_id: Option<u16>,
    pub coordinates: Vec<VariationCoordinate>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct VariationModel {
    pub axes: Vec<VariationAxis>,
    pub named_instances: Vec<NamedVariationInstance>,
}

impl VariationModel {
    pub fn parse(data: Option<&[u8]>, limits: FontLimits) -> Result<Self, FontParseError> {
        let Some(data) = data else {
            return Ok(Self::default());
        };
        if read_u32(data, 0)? != 0x0001_0000 {
            return Err(FontParseError::UnsupportedVariationTable);
        }
        let axes_offset = usize::from(read_u16(data, 4)?);
        let axis_count = usize::from(read_u16(data, 8)?);
        let axis_size = usize::from(read_u16(data, 10)?);
        let instance_count = usize::from(read_u16(data, 12)?);
        let instance_size = usize::from(read_u16(data, 14)?);
        if axis_count > limits.max_variation_axes {
            return Err(FontParseError::LimitExceeded {
                resource: "variation axes",
                limit: limits.max_variation_axes,
                attempted: axis_count,
            });
        }
        if axis_size < 20 || instance_size < 4 + axis_count.saturating_mul(4) {
            return Err(FontParseError::InvalidVariationTable);
        }

        let mut tags = BTreeSet::new();
        let mut axes = Vec::with_capacity(axis_count);
        for index in 0..axis_count {
            let offset = checked_record(axes_offset, index, axis_size, data.len())?;
            let tag = OpenTypeTag::new(read_array(data, offset)?);
            let minimum = read_i32(data, offset + 4)?;
            let default = read_i32(data, offset + 8)?;
            let maximum = read_i32(data, offset + 12)?;
            if minimum > default || default > maximum || !tags.insert(tag) {
                return Err(FontParseError::InvalidVariationTable);
            }
            axes.push(VariationAxis {
                tag,
                minimum,
                default,
                maximum,
                hidden: read_u16(data, offset + 16)? & 1 != 0,
                name_id: read_u16(data, offset + 18)?,
            });
        }

        let instances_offset = axes_offset
            .checked_add(
                axis_count
                    .checked_mul(axis_size)
                    .ok_or(FontParseError::ArithmeticOverflow)?,
            )
            .ok_or(FontParseError::ArithmeticOverflow)?;
        let mut names = BTreeSet::new();
        let mut named_instances = Vec::with_capacity(instance_count);
        for index in 0..instance_count {
            let offset = checked_record(instances_offset, index, instance_size, data.len())?;
            let subfamily_name_id = read_u16(data, offset)?;
            if !names.insert(subfamily_name_id) {
                return Err(FontParseError::DuplicateNamedVariationInstance(
                    subfamily_name_id,
                ));
            }
            let mut coordinates = Vec::with_capacity(axis_count);
            for (axis_index, axis) in axes.iter().enumerate() {
                let value = read_i32(data, offset + 4 + axis_index * 4)?;
                if value < axis.minimum || value > axis.maximum {
                    return Err(FontParseError::InvalidVariationTable);
                }
                coordinates.push(VariationCoordinate {
                    tag: axis.tag,
                    value,
                });
            }
            let postscript_offset = 4 + axis_count * 4;
            let postscript_name_id = (instance_size >= postscript_offset + 2)
                .then(|| read_u16(data, offset + postscript_offset))
                .transpose()?
                .filter(|name_id| *name_id != 0xffff);
            named_instances.push(NamedVariationInstance {
                subfamily_name_id,
                postscript_name_id,
                coordinates,
            });
        }
        Ok(Self {
            axes,
            named_instances,
        })
    }

    pub fn resolve(
        &self,
        selection: &VariationSelection,
    ) -> Result<VariationSelection, FontParseError> {
        match selection.instance() {
            VariationInstance::Default => Ok(selection.clone()),
            VariationInstance::Named(name_id) => {
                let instance = self
                    .named_instances
                    .iter()
                    .find(|instance| instance.subfamily_name_id == name_id)
                    .ok_or(FontParseError::UnknownNamedVariationInstance(name_id))?;
                Ok(selection
                    .clone()
                    .with_resolved_coordinates(instance.coordinates.clone()))
            }
            VariationInstance::Coordinates => {
                for coordinate in selection.coordinates() {
                    let axis = self
                        .axes
                        .iter()
                        .find(|axis| axis.tag == coordinate.tag)
                        .ok_or(FontParseError::UnknownVariationAxis(coordinate.tag))?;
                    if coordinate.value < axis.minimum || coordinate.value > axis.maximum {
                        return Err(FontParseError::VariationOutOfRange(coordinate.tag));
                    }
                }
                Ok(selection.clone())
            }
        }
    }
}

fn checked_record(
    base: usize,
    index: usize,
    size: usize,
    len: usize,
) -> Result<usize, FontParseError> {
    let offset = base
        .checked_add(
            index
                .checked_mul(size)
                .ok_or(FontParseError::ArithmeticOverflow)?,
        )
        .ok_or(FontParseError::ArithmeticOverflow)?;
    let end = offset
        .checked_add(size)
        .ok_or(FontParseError::ArithmeticOverflow)?;
    if end > len {
        return Err(FontParseError::InvalidVariationTable);
    }
    Ok(offset)
}

fn read_array(data: &[u8], offset: usize) -> Result<[u8; 4], FontParseError> {
    data.get(offset..offset + 4)
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or(FontParseError::InvalidVariationTable)
}

fn read_u16(data: &[u8], offset: usize) -> Result<u16, FontParseError> {
    Ok(u16::from_be_bytes(read_array_n(data, offset)?))
}

fn read_u32(data: &[u8], offset: usize) -> Result<u32, FontParseError> {
    Ok(u32::from_be_bytes(read_array_n(data, offset)?))
}

fn read_i32(data: &[u8], offset: usize) -> Result<i32, FontParseError> {
    Ok(i32::from_be_bytes(read_array_n(data, offset)?))
}

fn read_array_n<const N: usize>(data: &[u8], offset: usize) -> Result<[u8; N], FontParseError> {
    data.get(offset..offset + N)
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or(FontParseError::InvalidVariationTable)
}
