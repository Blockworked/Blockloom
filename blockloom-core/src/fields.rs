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
    NavigateX,
    NavigateY,
    NavigateZ,
    NavigateSpeed,
    ChangeByAmount,
    GlideSeconds,
    GlideX,
    GlideY,
    GlideZ,
    TurnDegrees,
    RotationDegrees,
    CameraPitchDegrees,
    CameraFovDegrees,
    ScaleFactor,
    ExposureEv,
    LightIntensity,
    ImpulseX,
    ImpulseY,
    ImpulseZ,
    VelocityX,
    VelocityY,
    VelocityZ,
    GravityX,
    GravityY,
    GravityZ,
    Density,
    Mass,
    TriggerLayer,
    TriggerMask,
    SayText,
    ColorText,
    ParticleCount,
    EmitterValue,
    SoundAsset,
    SoundVolume,
    SoundPitch,
    SoundTarget,
    ComponentFieldValue,
    ParentTarget,
    NewActorName,
    NewActorX,
    NewActorY,
    NewActorZ,
    DeleteTarget,
    // The interface slots. Every `show` block spells the shared five the
    // same way, so one id each covers every kind.
    UiId,
    UiContent,
    UiX,
    UiY,
    UiWidth,
    UiHeight,
    UiParent,
    UiMin,
    UiMax,
    UiValue,
    UiTarget,
    UiPropValue,
    WaitDuration,
    WaitUntilCondition,
    Condition,
    RepeatCount,
    SetVariableValue,
    ChangeVariableValue,
    AddToListValue,
    DeleteOfListIndex,
    ShiftListAmount,
    InsertIntoListValue,
    InsertIntoListIndex,
    ReplaceItemOfListIndex,
    ReplaceItemOfListValue,
    SetDictKey,
    SetDictValue,
    DeleteDictKey,
    LoadJsonIntoDictText,
    LoadJsonIntoListText,
    RumbleStrength,
    RumbleDuration,
    ActionName,
    ActionBinding,
    ReturnValue,
    CallArg(usize),
}

impl FieldId {
    /// Only a loop count is whole-numbers-only; every coordinate, duration
    /// and factor in the engine is fractional.
    pub fn requires_integer(self) -> bool {
        matches!(
            self,
            FieldId::RepeatCount
                | FieldId::DeleteOfListIndex
                | FieldId::ShiftListAmount
                | FieldId::InsertIntoListIndex
                | FieldId::ReplaceItemOfListIndex
        )
    }

    /// Slots that hold text rather than a number, so a blank restores to an
    /// empty string instead of `0`.
    fn is_text(self) -> bool {
        matches!(
            self,
            FieldId::SayText
                | FieldId::ColorText
                | FieldId::SoundAsset
                | FieldId::SoundTarget
                | FieldId::ParentTarget
                | FieldId::NewActorName
                | FieldId::DeleteTarget
                | FieldId::ActionName
                | FieldId::ActionBinding
                | FieldId::UiId
                | FieldId::UiContent
                | FieldId::UiParent
                | FieldId::UiTarget
        )
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
            FieldId::TriggerLayer => write!(f, "TriggerLayer"),
            FieldId::TriggerMask => write!(f, "TriggerMask"),
            FieldId::GoToX => write!(f, "GoToX"),
            FieldId::GoToY => write!(f, "GoToY"),
            FieldId::GoToZ => write!(f, "GoToZ"),
            FieldId::NavigateX => write!(f, "NavigateX"),
            FieldId::NavigateY => write!(f, "NavigateY"),
            FieldId::NavigateZ => write!(f, "NavigateZ"),
            FieldId::NavigateSpeed => write!(f, "NavigateSpeed"),
            FieldId::ChangeByAmount => write!(f, "ChangeByAmount"),
            FieldId::GlideSeconds => write!(f, "GlideSeconds"),
            FieldId::GlideX => write!(f, "GlideX"),
            FieldId::GlideY => write!(f, "GlideY"),
            FieldId::GlideZ => write!(f, "GlideZ"),
            FieldId::TurnDegrees => write!(f, "TurnDegrees"),
            FieldId::RotationDegrees => write!(f, "RotationDegrees"),
            FieldId::CameraPitchDegrees => write!(f, "CameraPitchDegrees"),
            FieldId::CameraFovDegrees => write!(f, "CameraFovDegrees"),
            FieldId::ScaleFactor => write!(f, "ScaleFactor"),
            FieldId::ExposureEv => write!(f, "ExposureEv"),
            FieldId::LightIntensity => write!(f, "LightIntensity"),
            FieldId::ImpulseX => write!(f, "ImpulseX"),
            FieldId::ImpulseY => write!(f, "ImpulseY"),
            FieldId::ImpulseZ => write!(f, "ImpulseZ"),
            FieldId::VelocityX => write!(f, "VelocityX"),
            FieldId::VelocityY => write!(f, "VelocityY"),
            FieldId::VelocityZ => write!(f, "VelocityZ"),
            FieldId::GravityX => write!(f, "GravityX"),
            FieldId::GravityY => write!(f, "GravityY"),
            FieldId::GravityZ => write!(f, "GravityZ"),
            FieldId::Density => write!(f, "Density"),
            FieldId::Mass => write!(f, "Mass"),
            FieldId::SayText => write!(f, "SayText"),
            FieldId::ColorText => write!(f, "ColorText"),
            FieldId::ParticleCount => write!(f, "ParticleCount"),
            FieldId::EmitterValue => write!(f, "EmitterValue"),
            FieldId::SoundAsset => write!(f, "SoundAsset"),
            FieldId::SoundVolume => write!(f, "SoundVolume"),
            FieldId::SoundPitch => write!(f, "SoundPitch"),
            FieldId::SoundTarget => write!(f, "SoundTarget"),
            FieldId::ComponentFieldValue => write!(f, "ComponentFieldValue"),
            FieldId::ParentTarget => write!(f, "ParentTarget"),
            FieldId::NewActorName => write!(f, "NewActorName"),
            FieldId::NewActorX => write!(f, "NewActorX"),
            FieldId::NewActorY => write!(f, "NewActorY"),
            FieldId::NewActorZ => write!(f, "NewActorZ"),
            FieldId::DeleteTarget => write!(f, "DeleteTarget"),
            FieldId::UiId => write!(f, "UiId"),
            FieldId::UiContent => write!(f, "UiContent"),
            FieldId::UiX => write!(f, "UiX"),
            FieldId::UiY => write!(f, "UiY"),
            FieldId::UiWidth => write!(f, "UiWidth"),
            FieldId::UiHeight => write!(f, "UiHeight"),
            FieldId::UiParent => write!(f, "UiParent"),
            FieldId::UiMin => write!(f, "UiMin"),
            FieldId::UiMax => write!(f, "UiMax"),
            FieldId::UiValue => write!(f, "UiValue"),
            FieldId::UiTarget => write!(f, "UiTarget"),
            FieldId::UiPropValue => write!(f, "UiPropValue"),
            FieldId::WaitDuration => write!(f, "WaitDuration"),
            FieldId::WaitUntilCondition => write!(f, "WaitUntilCondition"),
            FieldId::Condition => write!(f, "Condition"),
            FieldId::RepeatCount => write!(f, "RepeatCount"),
            FieldId::SetVariableValue => write!(f, "SetVariableValue"),
            FieldId::ChangeVariableValue => write!(f, "ChangeVariableValue"),
            FieldId::AddToListValue => write!(f, "AddToListValue"),
            FieldId::DeleteOfListIndex => write!(f, "DeleteOfListIndex"),
            FieldId::ShiftListAmount => write!(f, "ShiftListAmount"),
            FieldId::InsertIntoListValue => write!(f, "InsertIntoListValue"),
            FieldId::InsertIntoListIndex => write!(f, "InsertIntoListIndex"),
            FieldId::ReplaceItemOfListIndex => write!(f, "ReplaceItemOfListIndex"),
            FieldId::ReplaceItemOfListValue => write!(f, "ReplaceItemOfListValue"),
            FieldId::SetDictKey => write!(f, "SetDictKey"),
            FieldId::SetDictValue => write!(f, "SetDictValue"),
            FieldId::DeleteDictKey => write!(f, "DeleteDictKey"),
            FieldId::LoadJsonIntoDictText => write!(f, "LoadJsonIntoDictText"),
            FieldId::LoadJsonIntoListText => write!(f, "LoadJsonIntoListText"),
            FieldId::RumbleStrength => write!(f, "RumbleStrength"),
            FieldId::RumbleDuration => write!(f, "RumbleDuration"),
            FieldId::ActionName => write!(f, "ActionName"),
            FieldId::ActionBinding => write!(f, "ActionBinding"),
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
            "TriggerLayer" => FieldId::TriggerLayer,
            "TriggerMask" => FieldId::TriggerMask,
            "GoToX" => FieldId::GoToX,
            "GoToY" => FieldId::GoToY,
            "GoToZ" => FieldId::GoToZ,
            "NavigateX" => FieldId::NavigateX,
            "NavigateY" => FieldId::NavigateY,
            "NavigateZ" => FieldId::NavigateZ,
            "NavigateSpeed" => FieldId::NavigateSpeed,
            "ChangeByAmount" => FieldId::ChangeByAmount,
            "GlideSeconds" => FieldId::GlideSeconds,
            "GlideX" => FieldId::GlideX,
            "GlideY" => FieldId::GlideY,
            "GlideZ" => FieldId::GlideZ,
            "TurnDegrees" => FieldId::TurnDegrees,
            "RotationDegrees" => FieldId::RotationDegrees,
            "CameraPitchDegrees" => FieldId::CameraPitchDegrees,
            "CameraFovDegrees" => FieldId::CameraFovDegrees,
            "ScaleFactor" => FieldId::ScaleFactor,
            "ExposureEv" => FieldId::ExposureEv,
            "LightIntensity" => FieldId::LightIntensity,
            "ImpulseX" => FieldId::ImpulseX,
            "ImpulseY" => FieldId::ImpulseY,
            "ImpulseZ" => FieldId::ImpulseZ,
            "VelocityX" => FieldId::VelocityX,
            "VelocityY" => FieldId::VelocityY,
            "VelocityZ" => FieldId::VelocityZ,
            "GravityX" => FieldId::GravityX,
            "GravityY" => FieldId::GravityY,
            "GravityZ" => FieldId::GravityZ,
            "Density" => FieldId::Density,
            "Mass" => FieldId::Mass,
            "SayText" => FieldId::SayText,
            "ColorText" => FieldId::ColorText,
            "ParticleCount" => FieldId::ParticleCount,
            "EmitterValue" => FieldId::EmitterValue,
            "SoundAsset" => FieldId::SoundAsset,
            "SoundVolume" => FieldId::SoundVolume,
            "SoundPitch" => FieldId::SoundPitch,
            "SoundTarget" => FieldId::SoundTarget,
            "ComponentFieldValue" => FieldId::ComponentFieldValue,
            "ParentTarget" => FieldId::ParentTarget,
            "NewActorName" => FieldId::NewActorName,
            "NewActorX" => FieldId::NewActorX,
            "NewActorY" => FieldId::NewActorY,
            "NewActorZ" => FieldId::NewActorZ,
            "DeleteTarget" => FieldId::DeleteTarget,
            "UiId" => FieldId::UiId,
            "UiContent" => FieldId::UiContent,
            "UiX" => FieldId::UiX,
            "UiY" => FieldId::UiY,
            "UiWidth" => FieldId::UiWidth,
            "UiHeight" => FieldId::UiHeight,
            "UiParent" => FieldId::UiParent,
            "UiMin" => FieldId::UiMin,
            "UiMax" => FieldId::UiMax,
            "UiValue" => FieldId::UiValue,
            "UiTarget" => FieldId::UiTarget,
            "UiPropValue" => FieldId::UiPropValue,
            "WaitDuration" => FieldId::WaitDuration,
            "WaitUntilCondition" => FieldId::WaitUntilCondition,
            "Condition" => FieldId::Condition,
            "RepeatCount" => FieldId::RepeatCount,
            "SetVariableValue" => FieldId::SetVariableValue,
            "ChangeVariableValue" => FieldId::ChangeVariableValue,
            "AddToListValue" => FieldId::AddToListValue,
            "DeleteOfListIndex" => FieldId::DeleteOfListIndex,
            "ShiftListAmount" => FieldId::ShiftListAmount,
            "InsertIntoListValue" => FieldId::InsertIntoListValue,
            "InsertIntoListIndex" => FieldId::InsertIntoListIndex,
            "ReplaceItemOfListIndex" => FieldId::ReplaceItemOfListIndex,
            "ReplaceItemOfListValue" => FieldId::ReplaceItemOfListValue,
            "SetDictKey" => FieldId::SetDictKey,
            "SetDictValue" => FieldId::SetDictValue,
            "DeleteDictKey" => FieldId::DeleteDictKey,
            "LoadJsonIntoDictText" => FieldId::LoadJsonIntoDictText,
            "LoadJsonIntoListText" => FieldId::LoadJsonIntoListText,
            "RumbleStrength" => FieldId::RumbleStrength,
            "RumbleDuration" => FieldId::RumbleDuration,
            "ActionName" => FieldId::ActionName,
            "ActionBinding" => FieldId::ActionBinding,
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
        (K::NavigateTo { x, .. }, F::NavigateX) => Some(x),
        (K::NavigateTo { y, .. }, F::NavigateY) => Some(y),
        (K::NavigateTo { z, .. }, F::NavigateZ) => Some(z),
        (K::NavigateTo { speed, .. }, F::NavigateSpeed) => Some(speed),
        (K::ChangePosition { by, .. }, F::ChangeByAmount) => Some(by),
        (K::Glide { seconds, .. }, F::GlideSeconds) => Some(seconds),
        (K::Glide { x, .. }, F::GlideX) => Some(x),
        (K::Glide { y, .. }, F::GlideY) => Some(y),
        (K::Glide { z, .. }, F::GlideZ) => Some(z),
        (K::Turn { degrees, .. }, F::TurnDegrees) => Some(degrees),
        (K::SetRotation { degrees, .. }, F::RotationDegrees) => Some(degrees),
        (K::SetCameraPitch { degrees }, F::CameraPitchDegrees) => Some(degrees),
        (K::SetCameraFov { fov }, F::CameraFovDegrees) => Some(fov),
        (K::SetScale { factor }, F::ScaleFactor) => Some(factor),
        (K::SetExposure { ev }, F::ExposureEv) => Some(ev),
        (K::SetLightIntensity { intensity }, F::LightIntensity) => Some(intensity),
        (K::ApplyImpulse { x, .. }, F::ImpulseX) => Some(x),
        (K::ApplyImpulse { y, .. }, F::ImpulseY) => Some(y),
        (K::ApplyImpulse { z, .. }, F::ImpulseZ) => Some(z),
        (K::SetVelocity { x, .. }, F::VelocityX) => Some(x),
        (K::SetVelocity { y, .. }, F::VelocityY) => Some(y),
        (K::SetVelocity { z, .. }, F::VelocityZ) => Some(z),
        (K::SetGravity { x, .. }, F::GravityX) => Some(x),
        (K::SetGravity { y, .. }, F::GravityY) => Some(y),
        (K::SetGravity { z, .. }, F::GravityZ) => Some(z),
        (K::SetDensity { density }, F::Density) => Some(density),
        (K::SetMass { mass }, F::Mass) => Some(mass),
        (K::SetCollisionLayer { layer }, F::TriggerLayer) => Some(layer),
        (K::SetCollisionMask { mask }, F::TriggerMask) => Some(mask),
        (K::Say { text }, F::SayText) => Some(text),
        (K::SetColor { color }, F::ColorText) => Some(color),
        (K::BurstParticles { count }, F::ParticleCount) => Some(count),
        (K::SetEmitterDial { value, .. }, F::EmitterValue) => Some(value),
        (K::PlaySound { sound, .. }, F::SoundAsset)
        | (K::PlaySoundAt { sound, .. }, F::SoundAsset)
        | (K::StopSound { sound }, F::SoundAsset)
        | (K::SetSoundVolume { sound, .. }, F::SoundAsset)
        | (K::SetSoundPitch { sound, .. }, F::SoundAsset) => Some(sound),
        (K::PlaySound { volume, .. }, F::SoundVolume)
        | (K::PlaySoundAt { volume, .. }, F::SoundVolume)
        | (K::SetSoundVolume { volume, .. }, F::SoundVolume)
        | (K::SetBusVolume { volume, .. }, F::SoundVolume) => Some(volume),
        (K::PlaySound { pitch, .. }, F::SoundPitch)
        | (K::PlaySoundAt { pitch, .. }, F::SoundPitch)
        | (K::SetSoundPitch { pitch, .. }, F::SoundPitch) => Some(pitch),
        (K::PlaySoundAt { target, .. }, F::SoundTarget) => Some(target),
        (K::SetComponentField { value, .. }, F::ComponentFieldValue) => Some(value),
        (K::SetParent { parent }, F::ParentTarget) => Some(parent),
        (K::CreateActor { name, .. }, F::NewActorName) => Some(name),
        (K::CreateActor { x, .. }, F::NewActorX) => Some(x),
        (K::CreateActor { y, .. }, F::NewActorY) => Some(y),
        (K::CreateActor { z, .. }, F::NewActorZ) => Some(z),
        (K::DeleteActor { target }, F::DeleteTarget) => Some(target),
        // The interface blocks. Their shared slots answer to one id each,
        // whichever `show` block is asking.
        (K::ShowPanel { element, .. }, F::UiId)
        | (K::ShowLabel { element, .. }, F::UiId)
        | (K::ShowButton { element, .. }, F::UiId)
        | (K::ShowImage { element, .. }, F::UiId)
        | (K::ShowInput { element, .. }, F::UiId)
        | (K::ShowSlider { element, .. }, F::UiId)
        | (K::ShowToggle { element, .. }, F::UiId)
        | (K::ShowList { element, .. }, F::UiId) => Some(element),
        (K::ShowPanel { title: slot, .. }, F::UiContent)
        | (K::ShowLabel { text: slot, .. }, F::UiContent)
        | (K::ShowButton { label: slot, .. }, F::UiContent)
        | (K::ShowImage { asset: slot, .. }, F::UiContent)
        | (
            K::ShowInput {
                placeholder: slot, ..
            },
            F::UiContent,
        )
        | (K::ShowToggle { label: slot, .. }, F::UiContent) => Some(slot),
        (K::ShowPanel { x, .. }, F::UiX)
        | (K::ShowLabel { x, .. }, F::UiX)
        | (K::ShowButton { x, .. }, F::UiX)
        | (K::ShowImage { x, .. }, F::UiX)
        | (K::ShowInput { x, .. }, F::UiX)
        | (K::ShowSlider { x, .. }, F::UiX)
        | (K::ShowToggle { x, .. }, F::UiX)
        | (K::ShowList { x, .. }, F::UiX) => Some(x),
        (K::ShowPanel { y, .. }, F::UiY)
        | (K::ShowLabel { y, .. }, F::UiY)
        | (K::ShowButton { y, .. }, F::UiY)
        | (K::ShowImage { y, .. }, F::UiY)
        | (K::ShowInput { y, .. }, F::UiY)
        | (K::ShowSlider { y, .. }, F::UiY)
        | (K::ShowToggle { y, .. }, F::UiY)
        | (K::ShowList { y, .. }, F::UiY) => Some(y),
        (K::ShowPanel { width, .. }, F::UiWidth)
        | (K::ShowLabel { width, .. }, F::UiWidth)
        | (K::ShowButton { width, .. }, F::UiWidth)
        | (K::ShowImage { width, .. }, F::UiWidth)
        | (K::ShowInput { width, .. }, F::UiWidth)
        | (K::ShowSlider { width, .. }, F::UiWidth)
        | (K::ShowToggle { width, .. }, F::UiWidth)
        | (K::ShowList { width, .. }, F::UiWidth) => Some(width),
        (K::ShowPanel { height, .. }, F::UiHeight)
        | (K::ShowLabel { height, .. }, F::UiHeight)
        | (K::ShowButton { height, .. }, F::UiHeight)
        | (K::ShowImage { height, .. }, F::UiHeight)
        | (K::ShowInput { height, .. }, F::UiHeight)
        | (K::ShowSlider { height, .. }, F::UiHeight)
        | (K::ShowToggle { height, .. }, F::UiHeight)
        | (K::ShowList { height, .. }, F::UiHeight) => Some(height),
        (K::ShowPanel { parent, .. }, F::UiParent)
        | (K::ShowLabel { parent, .. }, F::UiParent)
        | (K::ShowButton { parent, .. }, F::UiParent)
        | (K::ShowImage { parent, .. }, F::UiParent)
        | (K::ShowInput { parent, .. }, F::UiParent)
        | (K::ShowSlider { parent, .. }, F::UiParent)
        | (K::ShowToggle { parent, .. }, F::UiParent)
        | (K::ShowList { parent, .. }, F::UiParent) => Some(parent),
        (K::ShowSlider { min, .. }, F::UiMin) => Some(min),
        (K::ShowSlider { max, .. }, F::UiMax) => Some(max),
        (K::ShowSlider { value, .. }, F::UiValue) => Some(value),
        (K::SetUiProp { element, .. }, F::UiTarget)
        | (K::HideElement { element }, F::UiTarget)
        | (K::DeleteElement { element }, F::UiTarget)
        | (K::FocusElement { element }, F::UiTarget) => Some(element),
        (K::SetUiProp { value, .. }, F::UiPropValue) => Some(value),
        (K::Wait { duration }, F::WaitDuration) => Some(duration),
        (K::WaitUntil { condition }, F::WaitUntilCondition) => Some(condition),
        (K::If { condition, .. }, F::Condition) => Some(condition),
        (K::IfElse { condition, .. }, F::Condition) => Some(condition),
        (K::While { condition, .. }, F::Condition) => Some(condition),
        (K::Repeat { count, .. }, F::RepeatCount) => Some(count),
        (K::SetVariable { value, .. }, F::SetVariableValue) => Some(value),
        (K::ChangeVariable { value, .. }, F::ChangeVariableValue) => Some(value),
        (K::AddToList { value, .. }, F::AddToListValue) => Some(value),
        (K::DeleteOfList { index, .. }, F::DeleteOfListIndex) => Some(index),
        (K::ShiftList { amount, .. }, F::ShiftListAmount) => Some(amount),
        (K::InsertIntoList { value, .. }, F::InsertIntoListValue) => Some(value),
        (K::InsertIntoList { index, .. }, F::InsertIntoListIndex) => Some(index),
        (K::ReplaceItemOfList { index, .. }, F::ReplaceItemOfListIndex) => Some(index),
        (K::ReplaceItemOfList { value, .. }, F::ReplaceItemOfListValue) => Some(value),
        (K::SetDictValue { key, .. }, F::SetDictKey) => Some(key),
        (K::SetDictValue { value, .. }, F::SetDictValue) => Some(value),
        (K::DeleteDictKey { key, .. }, F::DeleteDictKey) => Some(key),
        (K::LoadJsonIntoDict { json, .. }, F::LoadJsonIntoDictText) => Some(json),
        (K::LoadJsonIntoList { json, .. }, F::LoadJsonIntoListText) => Some(json),
        (K::RumbleGamepad { strength, .. }, F::RumbleStrength) => Some(strength),
        (K::RumbleGamepad { duration, .. }, F::RumbleDuration) => Some(duration),
        (K::BindAction { action, .. }, F::ActionName)
        | (K::ClearActionBindings { action }, F::ActionName) => Some(action),
        (K::BindAction { binding, .. }, F::ActionBinding) => Some(binding),
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
            FieldId::Density,
            FieldId::Mass,
            FieldId::ExposureEv,
            FieldId::LightIntensity,
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
