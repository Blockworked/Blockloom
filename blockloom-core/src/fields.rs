//! Field ids: what the frontend calls each value slot on an instruction, and
//! the lookups blockstitch resolves a drag or a typed edit through.

use crate::blocks::{BlockDef, InputValueType, InstructionKind};
use crate::value::Value;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// One addressable value slot on an [`InstructionKind`]. Travels as its
/// `Display`/`FromStr` string form, which is what the frontend sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FieldId {
    MoveSteps,
    GoToX,
    GoToY,
    GoToZ,
    ChangeByAmount,
    GlideSeconds,
    GlideX,
    GlideY,
    GlideZ,
    TurnDegrees,
    RotationDegrees,
    ScaleFactor,
    ImpulseX,
    ImpulseY,
    ImpulseZ,
    VelocityX,
    VelocityY,
    VelocityZ,
    GravityX,
    GravityY,
    GravityZ,
    SayText,
    ColorText,
    WaitDuration,
    WaitUntilCondition,
    Condition,
    RepeatCount,
    SetVariableValue,
    ChangeVariableValue,
    ReturnValue,
    CallArg(usize),
}

impl FieldId {
    /// Only a loop count is whole-numbers-only; every coordinate, duration
    /// and factor in the engine is fractional.
    pub fn requires_integer(self) -> bool {
        matches!(self, FieldId::RepeatCount)
    }

    /// Slots that hold text rather than a number, so a blank restores to an
    /// empty string instead of `0`.
    fn is_text(self) -> bool {
        matches!(self, FieldId::SayText | FieldId::ColorText)
    }

    /// Slots that hold a boolean, so a blank restores to the empty hexagon.
    fn is_bool(self) -> bool {
        matches!(self, FieldId::Condition | FieldId::WaitUntilCondition)
    }
}

impl std::fmt::Display for FieldId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FieldId::MoveSteps => write!(f, "MoveSteps"),
            FieldId::GoToX => write!(f, "GoToX"),
            FieldId::GoToY => write!(f, "GoToY"),
            FieldId::GoToZ => write!(f, "GoToZ"),
            FieldId::ChangeByAmount => write!(f, "ChangeByAmount"),
            FieldId::GlideSeconds => write!(f, "GlideSeconds"),
            FieldId::GlideX => write!(f, "GlideX"),
            FieldId::GlideY => write!(f, "GlideY"),
            FieldId::GlideZ => write!(f, "GlideZ"),
            FieldId::TurnDegrees => write!(f, "TurnDegrees"),
            FieldId::RotationDegrees => write!(f, "RotationDegrees"),
            FieldId::ScaleFactor => write!(f, "ScaleFactor"),
            FieldId::ImpulseX => write!(f, "ImpulseX"),
            FieldId::ImpulseY => write!(f, "ImpulseY"),
            FieldId::ImpulseZ => write!(f, "ImpulseZ"),
            FieldId::VelocityX => write!(f, "VelocityX"),
            FieldId::VelocityY => write!(f, "VelocityY"),
            FieldId::VelocityZ => write!(f, "VelocityZ"),
            FieldId::GravityX => write!(f, "GravityX"),
            FieldId::GravityY => write!(f, "GravityY"),
            FieldId::GravityZ => write!(f, "GravityZ"),
            FieldId::SayText => write!(f, "SayText"),
            FieldId::ColorText => write!(f, "ColorText"),
            FieldId::WaitDuration => write!(f, "WaitDuration"),
            FieldId::WaitUntilCondition => write!(f, "WaitUntilCondition"),
            FieldId::Condition => write!(f, "Condition"),
            FieldId::RepeatCount => write!(f, "RepeatCount"),
            FieldId::SetVariableValue => write!(f, "SetVariableValue"),
            FieldId::ChangeVariableValue => write!(f, "ChangeVariableValue"),
            FieldId::ReturnValue => write!(f, "ReturnValue"),
            FieldId::CallArg(i) => write!(f, "CallArg:{i}"),
        }
    }
}

impl FromStr for FieldId {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(index) = s.strip_prefix("CallArg:") {
            return index.parse().map(FieldId::CallArg).map_err(|_| ());
        }
        Ok(match s {
            "MoveSteps" => FieldId::MoveSteps,
            "GoToX" => FieldId::GoToX,
            "GoToY" => FieldId::GoToY,
            "GoToZ" => FieldId::GoToZ,
            "ChangeByAmount" => FieldId::ChangeByAmount,
            "GlideSeconds" => FieldId::GlideSeconds,
            "GlideX" => FieldId::GlideX,
            "GlideY" => FieldId::GlideY,
            "GlideZ" => FieldId::GlideZ,
            "TurnDegrees" => FieldId::TurnDegrees,
            "RotationDegrees" => FieldId::RotationDegrees,
            "ScaleFactor" => FieldId::ScaleFactor,
            "ImpulseX" => FieldId::ImpulseX,
            "ImpulseY" => FieldId::ImpulseY,
            "ImpulseZ" => FieldId::ImpulseZ,
            "VelocityX" => FieldId::VelocityX,
            "VelocityY" => FieldId::VelocityY,
            "VelocityZ" => FieldId::VelocityZ,
            "GravityX" => FieldId::GravityX,
            "GravityY" => FieldId::GravityY,
            "GravityZ" => FieldId::GravityZ,
            "SayText" => FieldId::SayText,
            "ColorText" => FieldId::ColorText,
            "WaitDuration" => FieldId::WaitDuration,
            "WaitUntilCondition" => FieldId::WaitUntilCondition,
            "Condition" => FieldId::Condition,
            "RepeatCount" => FieldId::RepeatCount,
            "SetVariableValue" => FieldId::SetVariableValue,
            "ChangeVariableValue" => FieldId::ChangeVariableValue,
            "ReturnValue" => FieldId::ReturnValue,
            _ => return Err(()),
        })
    }
}

/// The value tree `field` names on `kind`, so a drag or a typed edit reaches
/// the right slot. `None` when the pair doesn't match up (a stale frontend
/// location, or a field id from another block).
pub fn value_slot_mut(kind: &mut InstructionKind, field: FieldId) -> Option<&mut Value> {
    use FieldId as F;
    use InstructionKind as K;
    match (kind, field) {
        (K::Move { steps }, F::MoveSteps) => Some(steps),
        (K::GoTo { x, .. }, F::GoToX) => Some(x),
        (K::GoTo { y, .. }, F::GoToY) => Some(y),
        (K::GoTo { z, .. }, F::GoToZ) => Some(z),
        (K::ChangePosition { by, .. }, F::ChangeByAmount) => Some(by),
        (K::Glide { seconds, .. }, F::GlideSeconds) => Some(seconds),
        (K::Glide { x, .. }, F::GlideX) => Some(x),
        (K::Glide { y, .. }, F::GlideY) => Some(y),
        (K::Glide { z, .. }, F::GlideZ) => Some(z),
        (K::Turn { degrees, .. }, F::TurnDegrees) => Some(degrees),
        (K::SetRotation { degrees, .. }, F::RotationDegrees) => Some(degrees),
        (K::SetScale { factor }, F::ScaleFactor) => Some(factor),
        (K::ApplyImpulse { x, .. }, F::ImpulseX) => Some(x),
        (K::ApplyImpulse { y, .. }, F::ImpulseY) => Some(y),
        (K::ApplyImpulse { z, .. }, F::ImpulseZ) => Some(z),
        (K::SetVelocity { x, .. }, F::VelocityX) => Some(x),
        (K::SetVelocity { y, .. }, F::VelocityY) => Some(y),
        (K::SetVelocity { z, .. }, F::VelocityZ) => Some(z),
        (K::SetGravity { x, .. }, F::GravityX) => Some(x),
        (K::SetGravity { y, .. }, F::GravityY) => Some(y),
        (K::SetGravity { z, .. }, F::GravityZ) => Some(z),
        (K::Say { text }, F::SayText) => Some(text),
        (K::SetColor { color }, F::ColorText) => Some(color),
        (K::Wait { duration }, F::WaitDuration) => Some(duration),
        (K::WaitUntil { condition }, F::WaitUntilCondition) => Some(condition),
        (K::If { condition, .. }, F::Condition) => Some(condition),
        (K::IfElse { condition, .. }, F::Condition) => Some(condition),
        (K::While { condition, .. }, F::Condition) => Some(condition),
        (K::Repeat { count, .. }, F::RepeatCount) => Some(count),
        (K::SetVariable { value, .. }, F::SetVariableValue) => Some(value),
        (K::ChangeVariable { value, .. }, F::ChangeVariableValue) => Some(value),
        (K::Return { value }, F::ReturnValue) => Some(value),
        (K::CallBlock { args, .. }, F::CallArg(i)) => args.get_mut(i),
        _ => None,
    }
}

/// The blank a top-level field restores to when a value block is dragged out
/// of it and nothing was shadowed. `None` means a plain zero.
pub fn blank_field_value(
    kind: &InstructionKind,
    field: FieldId,
    blocks: &[BlockDef],
) -> Option<Value> {
    // A call site's declared types live on its `BlockDef`, not here.
    if let (InstructionKind::CallBlock { block_id, .. }, FieldId::CallArg(index)) = (kind, field) {
        let declared = blocks
            .iter()
            .find(|def| &def.id == block_id)?
            .input_types()
            .nth(index)?;
        return match declared {
            InputValueType::Bool => Some(Value::Bool),
            InputValueType::Any => None,
        };
    }
    if field.is_bool() {
        return Some(Value::Bool);
    }
    if field.is_text() {
        return Some(Value::text(""));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_ids_round_trip_through_their_string_form() {
        for field in [
            FieldId::MoveSteps,
            FieldId::GlideZ,
            FieldId::Condition,
            FieldId::CallArg(3),
        ] {
            assert_eq!(field.to_string().parse::<FieldId>(), Ok(field));
        }
        assert_eq!("Nonsense".parse::<FieldId>(), Err(()));
    }

    #[test]
    fn a_text_slot_blanks_to_empty_text_and_a_number_slot_to_a_plain_zero() {
        let say = InstructionKind::Say {
            text: Value::text("hi"),
        };
        assert_eq!(
            blank_field_value(&say, FieldId::SayText, &[]),
            Some(Value::text(""))
        );
        let mv = InstructionKind::Move {
            steps: Value::number(10.0),
        };
        assert_eq!(blank_field_value(&mv, FieldId::MoveSteps, &[]), None);
    }

    #[test]
    fn value_slots_resolve_per_axis_field() {
        let mut go_to = InstructionKind::GoTo {
            x: Value::number(1.0),
            y: Value::number(2.0),
            z: Value::number(3.0),
        };
        assert_eq!(
            value_slot_mut(&mut go_to, FieldId::GoToY),
            Some(&mut Value::number(2.0))
        );
        assert_eq!(value_slot_mut(&mut go_to, FieldId::MoveSteps), None);
    }
}
