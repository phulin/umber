//! Concrete math and arithmetic operands.
use super::super::*;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::main_control) enum MathShiftPairing {
    Unpaired,
    Paired,
    ProbeDisplayEnd,
}
#[derive(Clone, Copy, Debug)]
pub(in crate::main_control) enum ArithmeticTarget {
    IntegerRegister(u16),
    DimensionRegister(u16),
    GlueRegister { index: u16, mu: bool },
    IntegerParameter(u16),
    DimensionParameter(u16),
    GlueParameter { index: u16, mu: bool },
}

#[derive(Clone, Copy, Debug)]
pub(in crate::main_control) enum ArithmeticOperand {
    Integer(i32),
    Dimension(Scaled),
    Glue(GlueSpec),
}
