//! Compiling a canvas into a flat program.
//!
//! A nested instruction tree can't be suspended mid-body without either
//! recursion or a path cursor, so every strand is flattened into one `Vec` of
//! [`Step`]s with jumps. A running script is then just a program counter and a
//! small frame stack - suspending it is free, which is what makes `wait` and
//! per-frame yielding work.

use crate::animation::TweenEasing;
use crate::blocks::{ActorGraph, Instruction, InstructionKind};
use crate::components::CameraView;
use crate::scene::{Axis, BodyKind};
use crate::sound::SoundBus;
use crate::ui::{UiAnchor, UiKind, UiProp, UiTheme};
use crate::value::Value;
use std::collections::{HashMap, HashSet};

/// What starts a script.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Trigger {
    /// The green flag.
    Started,
    KeyPressed(String),
    Clicked,
    /// An empty `with` means "anything".
    Collision {
        with: String,
    },
    Message(String),
    /// A fresh clone starting up, in the clone itself.
    Cloned,
    /// A `Once` clip finished. Empty matches any clip ending.
    AnimationEnded {
        clip: String,
    },
    /// The actor's own particles spawned, died or hit something.
    Particles(crate::vfx::ParticleEvent),
    /// A clip reached a frame marker. Empty matches any marker.
    AnimationMarker {
        marker: String,
    },
    /// The actor walked into a room. Empty matches any room.
    EnteredRoom {
        room: String,
    },
    /// The named input action went down.
    ActionPressed(String),
    /// A finger touched the screen.
    Touched,
    /// An interface element was clicked, by its id.
    UiEvent {
        id: String,
        event: String,
    },
    UiClicked(String),
    /// An input element was changed, by its id.
    UiChanged(String),
}

impl Trigger {
    /// True for a trigger the interface fires. A strand one of these started
    /// keeps running while the game is paused - otherwise a pause menu's own
    /// buttons would be dead - and it runs on the wall clock, so a blink on
    /// a paused menu still blinks.
    pub fn is_ui(&self) -> bool {
        matches!(
            self,
            Trigger::UiEvent { .. } | Trigger::UiClicked(_) | Trigger::UiChanged(_)
        )
    }
}

/// One entry point: a header strand's trigger and where its body starts.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub trigger: Trigger,
    pub strand_id: String,
    pub pc: usize,
}

/// How a loop decides whether to go round again.
#[derive(Debug, Clone, PartialEq)]
pub enum LoopKind {
    /// Counted once, on entry.
    Repeat(Value),
    Forever,
    /// Re-checked before every iteration.
    While(Value),
}

/// A leaf instruction: it changes the world and never affects control flow.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Move(Value),
    GoTo([Value; 3]),
    NavigateTo {
        target: [Value; 3],
        speed: Value,
    },
    ChangePosition {
        axis: Axis,
        by: Value,
    },
    Turn {
        axis: Axis,
        degrees: Value,
    },
    SetRotation {
        axis: Axis,
        degrees: Value,
    },
    PointTowards(String),
    SetScale(Value),
    TweenScale {
        factor: Value,
        seconds: Value,
        easing: TweenEasing,
    },
    TweenRotation {
        axis: Axis,
        degrees: Value,
        seconds: Value,
        easing: TweenEasing,
    },
    TweenColor {
        color: Value,
        seconds: Value,
        easing: TweenEasing,
    },
    StopTweens,
    PlayAnimation {
        clip: Value,
        speed: Value,
    },
    StopAnimation,
    SetAnimationSpeed(Value),
    FireAnimationTrigger(Value),
    SetRigSlot {
        slot: Value,
        attachment: Value,
    },
    SetSlotTint {
        slot: Value,
        color: Value,
    },
    SetIkTarget {
        constraint: Value,
        x: Value,
        y: Value,
    },
    SetSpriteDial {
        dial: crate::blocks::SpriteDial,
        value: Value,
    },
    SetExposure(Value),
    SetLightIntensity(Value),
    SetEmissiveStrength(Value),
    SetHdrOutput(bool),
    SetPeakBrightness(Value),
    EnableVolume {
        volume: Value,
        enabled: bool,
    },
    SetVolumeWeight {
        volume: Value,
        weight: Value,
    },
    CaptureProbes,
    SetShadowDistance(Value),
    SetLightShadows(bool),
    SetRayTracing(bool),
    SetGiBounces(Value),
    SetGiSamples(Value),
    SetFogDensity(Value),
    SetAurora(Value),
    StrikeLightning([Value; 3]),
    SetLightningRate(Value),
    SetWind {
        property: crate::wind::WindProperty,
        value: Value,
    },
    SetClouds {
        property: crate::clouds::CloudProperty,
        value: Value,
    },
    SetWater {
        property: crate::water::WaterProperty,
        value: Value,
    },
    PaintTile {
        map: Value,
        tile: Value,
        x: Value,
        y: Value,
        z: Value,
    },
    SetParallax {
        layer: Value,
        axis: crate::tilemap::ParallaxAxis,
        value: Value,
    },
    SetCloudLayer {
        layer: Value,
        property: crate::cloud_layers::CloudLayerProperty,
        value: Value,
    },
    SetCloudDrift([Value; 3]),
    SetBody(BodyKind),
    ApplyImpulse([Value; 3]),
    SetVelocity([Value; 3]),
    SetGravity([Value; 3]),
    SetDensity(Value),
    SetMass(Value),
    SetTrigger(bool),
    SetCollisionLayer(Value),
    SetCollisionMask(Value),
    BurstParticles(Value),
    SetEmitterDial {
        dial: crate::blocks::EmitterDial,
        value: Value,
    },
    SetTrailEnabled(bool),
    SetEmitterPlaying(bool),
    Say(Value),
    SetVisible(bool),
    SetColor(Value),
    PlaySound {
        sound: Value,
        volume: Value,
        pitch: Value,
        loop_: bool,
        bus: SoundBus,
    },
    /// An empty target plays at the running actor's own place.
    PlaySoundAt {
        sound: Value,
        volume: Value,
        pitch: Value,
        loop_: bool,
        bus: SoundBus,
        target: Value,
    },
    /// An empty sound stops every voice at once.
    StopSound {
        sound: Value,
    },
    SetSoundVolume {
        sound: Value,
        volume: Value,
    },
    SetSoundPitch {
        sound: Value,
        pitch: Value,
    },
    SetBusVolume {
        bus: SoundBus,
        volume: Value,
    },
    SetComponentField {
        component: String,
        field: String,
        value: Value,
    },
    SetCameraView(CameraView),
    SetCameraPitch(Value),
    SetCameraFov(Value),
    AttachComponent(String),
    DetachComponent(String),
    /// An empty target takes the actor off whatever it was hanging from.
    SetParent(Value),
    /// An empty `of` clones the running actor.
    CreateClone(String),
    CreateActor {
        name: Value,
        position: [Value; 3],
    },
    /// An empty target deletes the running actor.
    DeleteActor(Value),
    Broadcast(String),
    /// Grabs or frees the pointer; window-global, like gravity.
    SetMouseLocked(bool),
    /// Rumbles connected gamepads: 0-100 strength for seconds.
    RumbleGamepad {
        strength: Value,
        duration: Value,
    },
    /// Adds one binding to an action for the rest of the run.
    BindAction {
        action: Value,
        binding: Value,
    },
    /// Forgets every binding an action has for the rest of the run.
    ClearActionBindings {
        action: Value,
    },
    /// Makes or updates one interface element. Boxed because it names nine
    /// slots where no other block names more than four, and every `Step` in
    /// a program is as big as the biggest one.
    ShowElement(Box<ShowElement>),
    SetUiProp {
        prop: UiProp,
        id: Value,
        value: Value,
    },
    /// An empty id, or the `all` flag, hides everything.
    HideElement {
        id: Value,
        all: bool,
    },
    DeleteElement(Value),
    /// Hands the keyboard to a text input. An empty id, or one that names
    /// anything else, takes it back instead.
    SetFocus(Value),
    SetUiTheme(UiTheme),
    SetPaused(bool),
    SaveVariable {
        name: String,
        clear: bool,
    },
    SetVariable {
        name: String,
        value: Value,
    },
    ChangeVariable {
        name: String,
        value: Value,
    },
    AddToList {
        value: Value,
        name: String,
    },
    DeleteOfList {
        index: Value,
        name: String,
    },
    DeleteAllOfList {
        name: String,
    },
    ShiftList {
        name: String,
        amount: Value,
    },
    InsertIntoList {
        value: Value,
        index: Value,
        name: String,
    },
    ReplaceItemOfList {
        index: Value,
        name: String,
        value: Value,
    },
    ReverseList {
        name: String,
    },
    SetDictValue {
        key: Value,
        name: String,
        value: Value,
    },
    DeleteDictKey {
        key: Value,
        name: String,
    },
    DeleteAllOfDict {
        name: String,
    },
    LoadJsonIntoDict {
        json: Value,
        name: String,
    },
    LoadJsonIntoList {
        json: Value,
        name: String,
    },
}

/// What a `show` block asks for, before any of it is evaluated. The slots
/// are read in the order they are written here, which is the order the row
/// reads in - and the order a compiled program has to keep.
#[derive(Debug, Clone, PartialEq)]
pub struct ShowElement {
    pub kind: UiKind,
    pub id: Value,
    pub content: Value,
    /// A slider's ends; `None` for every other kind, which has no row for
    /// them and so asks the world nothing.
    pub range: Option<[Value; 2]>,
    /// A slider's starting number; the rest carry their own flag in `flag`
    /// instead.
    pub value: Option<Value>,
    pub anchor: UiAnchor,
    pub offset: [Value; 2],
    pub size: [Value; 2],
    pub parent: Value,
    /// A panel's modal, a toggle's on. Meaningless for the rest.
    pub flag: bool,
}

/// One step of a compiled program.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Action(Action),
    /// Falls through when `condition` holds, jumps to `to` when it doesn't.
    JumpUnless {
        condition: Value,
        to: usize,
    },
    Jump {
        to: usize,
    },
    /// Pushes a loop frame (or re-checks an existing one, when jumped back
    /// to) and jumps past `end` when the loop is done.
    LoopBegin {
        kind: LoopKind,
        end: usize,
    },
    /// The back edge. Yields the script for this frame.
    LoopEnd {
        begin: usize,
    },
    /// Leaves the nearest enclosing loop, within this invocation.
    Break,
    /// Jumps to the nearest enclosing loop's back edge.
    Continue,
    /// Suspends for a number of seconds.
    Wait(Value),
    /// Suspends until the condition holds, re-checked once a frame.
    WaitUntil(Value),
    Glide {
        seconds: Value,
        target: [Value; 3],
        easing: TweenEasing,
    },
    TweenScale {
        factor: Value,
        seconds: Value,
        easing: TweenEasing,
    },
    TweenRotation {
        axis: Axis,
        degrees: Value,
        seconds: Value,
        easing: TweenEasing,
    },
    TweenColor {
        color: Value,
        seconds: Value,
        easing: TweenEasing,
    },
    Call {
        block_id: String,
        args: Vec<Value>,
    },
    /// Runs a reporter-shaped custom block that can suspend, storing its
    /// value in the strand's `temp` slot. The args are already free of any
    /// suspendable call, so binding them never suspends - the body does.
    Invoke {
        block_id: String,
        args: Vec<Value>,
        temp: usize,
    },
    Return(Value),
    /// `stop all`.
    StopAll,
    /// End of a body: return from a call, or finish the script.
    End,
}

/// A reporter result slot, as a variable read. The `~` prefix is reserved for
/// the runtime: the editor never writes it, so no project variable collides.
pub fn temp_var(temp: usize) -> Value {
    Value::Var {
        name: format!("~t{temp}"),
    }
}

/// True for a `~tN` slot made by [`temp_var`].
pub fn temp_index(name: &str) -> Option<usize> {
    name.strip_prefix("~t")?.parse().ok()
}

/// An actor's whole canvas, compiled.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Program {
    pub steps: Vec<Step>,
    /// Header strands that a trigger can start.
    pub entries: Vec<Entry>,
    /// Custom block id -> where its body starts.
    pub blocks: HashMap<String, usize>,
}

/// Compiles every strand on `graph`. A strand with no header block is inert -
/// it's persisted, but nothing can ever start it, so it isn't compiled.
pub fn compile(graph: &ActorGraph) -> Program {
    let mut program = Program::default();
    for strand in &graph.strands {
        let Some(header) = strand.instructions.first() else {
            continue;
        };
        let trigger = match &header.kind {
            InstructionKind::WhenStarted => Some(Trigger::Started),
            InstructionKind::WhenKeyPressed { key } => {
                Some(Trigger::KeyPressed(crate::sense::normalize_key(key)))
            }
            InstructionKind::WhenClicked => Some(Trigger::Clicked),
            InstructionKind::WhenCollision { with } => Some(Trigger::Collision {
                with: with.trim().to_string(),
            }),
            InstructionKind::WhenMessage { name } => {
                Some(Trigger::Message(name.trim().to_string()))
            }
            InstructionKind::WhenCloned => Some(Trigger::Cloned),
            InstructionKind::WhenAnimationEnds { clip } => Some(Trigger::AnimationEnded {
                clip: clip.trim().to_string(),
            }),
            InstructionKind::WhenParticles { event } => Some(Trigger::Particles(*event)),
            InstructionKind::WhenAnimationMarker { marker } => Some(Trigger::AnimationMarker {
                marker: marker.trim().to_string(),
            }),
            InstructionKind::WhenEnterRoom { room } => Some(Trigger::EnteredRoom {
                room: room.trim().to_string(),
            }),
            InstructionKind::WhenActionPressed { action } => Some(Trigger::ActionPressed(
                crate::input::normalize_action(action).to_lowercase(),
            )),
            InstructionKind::WhenTouched => Some(Trigger::Touched),
            InstructionKind::WhenUiEvent { element, event } => Some(Trigger::UiEvent {
                id: element.trim().into(),
                event: event.clone(),
            }),
            InstructionKind::WhenUiClicked { element } => {
                Some(Trigger::UiClicked(element.trim().to_string()))
            }
            InstructionKind::WhenUiChanged { element } => {
                Some(Trigger::UiChanged(element.trim().to_string()))
            }
            InstructionKind::BlockHeader { .. } => None,
            // Not a header at all: a loose stack nothing can start.
            _ => continue,
        };
        let pc = program.steps.len();
        emit_body(&mut program.steps, &strand.instructions[1..]);
        program.steps.push(Step::End);
        match (&header.kind, trigger) {
            (InstructionKind::BlockHeader { block_id }, _) => {
                program.blocks.insert(block_id.clone(), pc);
            }
            (_, Some(trigger)) => program.entries.push(Entry {
                trigger,
                strand_id: strand.id.clone(),
                pc,
            }),
            (_, None) => {}
        }
    }
    // A reporter that waits suspends its caller, so its calls become `Invoke`
    // steps with the value in a temp slot - the same program both halves run.
    let bounds: HashMap<String, usize> = graph
        .block_defs
        .iter()
        .map(|def| (def.id.clone(), def.input_names().count()))
        .collect();
    lift_suspendable_calls(&mut program, &bounds);
    program
}

fn emit_body(steps: &mut Vec<Step>, body: &[Instruction]) {
    for instruction in body {
        emit(steps, &instruction.kind);
    }
}

// ─── Suspendable reporters ────────────────────────────────────────────────
// A reporter-shaped custom block that waits suspends its caller, so a call to
// one can't run to completion in place. Its calls become `Invoke` steps with
// the value in a temp slot, in the order `resolve` would have walked them.

fn step_is_suspending(step: &Step) -> bool {
    matches!(
        step,
        Step::Wait(_)
            | Step::WaitUntil(_)
            | Step::Glide { .. }
            | Step::TweenScale { .. }
            | Step::TweenRotation { .. }
            | Step::TweenColor { .. }
    )
}

fn calls_in_value(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Call { block_id, args, .. } => {
            out.push(block_id.clone());
            for arg in args {
                calls_in_value(arg, out);
            }
        }
        Value::Op { args, .. } => {
            for arg in args {
                calls_in_value(arg, out);
            }
        }
        _ => {}
    }
}

fn calls_in_step(step: &Step) -> Vec<String> {
    let mut out = Vec::new();
    match step {
        Step::Action(action) => {
            for value in action_values(action) {
                calls_in_value(value, &mut out);
            }
        }
        Step::JumpUnless { condition, .. } => calls_in_value(condition, &mut out),
        Step::LoopBegin { kind, .. } => match kind {
            LoopKind::Repeat(count) => calls_in_value(count, &mut out),
            LoopKind::While(condition) => calls_in_value(condition, &mut out),
            LoopKind::Forever => {}
        },
        Step::Wait(duration) => calls_in_value(duration, &mut out),
        Step::WaitUntil(condition) => calls_in_value(condition, &mut out),
        Step::Glide {
            seconds, target, ..
        } => {
            calls_in_value(seconds, &mut out);
            for value in target {
                calls_in_value(value, &mut out);
            }
        }
        Step::TweenScale {
            factor, seconds, ..
        } => {
            calls_in_value(factor, &mut out);
            calls_in_value(seconds, &mut out);
        }
        Step::TweenRotation {
            degrees, seconds, ..
        } => {
            calls_in_value(degrees, &mut out);
            calls_in_value(seconds, &mut out);
        }
        Step::TweenColor { color, seconds, .. } => {
            calls_in_value(color, &mut out);
            calls_in_value(seconds, &mut out);
        }
        Step::Call { block_id, args } => {
            out.push(block_id.clone());
            for arg in args {
                calls_in_value(arg, &mut out);
            }
        }
        Step::Invoke { block_id, args, .. } => {
            out.push(block_id.clone());
            for arg in args {
                calls_in_value(arg, &mut out);
            }
        }
        Step::Return(value) => calls_in_value(value, &mut out),
        _ => {}
    }
    out
}

// Every value slot a step reads, in the order the VM evaluates them.
fn action_values(action: &Action) -> Vec<&Value> {
    match action {
        Action::Move(value)
        | Action::SetScale(value)
        | Action::SetExposure(value)
        | Action::SetLightIntensity(value)
        | Action::SetEmissiveStrength(value)
        | Action::SetPeakBrightness(value)
        | Action::SetShadowDistance(value)
        | Action::SetGiBounces(value)
        | Action::SetGiSamples(value)
        | Action::SetFogDensity(value)
        | Action::SetAurora(value)
        | Action::SetLightningRate(value)
        | Action::SetWind { value, .. }
        | Action::SetClouds { value, .. }
        | Action::SetWater { value, .. }
        | Action::Say(value)
        | Action::SetColor(value)
        | Action::StopSound { sound: value }
        | Action::DeleteElement(value)
        | Action::SetFocus(value)
        | Action::SetParent(value)
        | Action::DeleteActor(value) => vec![value],
        Action::BurstParticles(value) | Action::SetEmitterDial { value, .. } => vec![value],
        Action::SetSpriteDial { value, .. } | Action::FireAnimationTrigger(value) => vec![value],
        Action::SetRigSlot { slot, attachment } => vec![slot, attachment],
        Action::SetSlotTint { slot, color } => vec![slot, color],
        Action::SetIkTarget { constraint, x, y } => vec![constraint, x, y],
        Action::GoTo(target)
        | Action::ApplyImpulse(target)
        | Action::SetVelocity(target)
        | Action::SetGravity(target)
        | Action::StrikeLightning(target)
        | Action::SetCloudDrift(target) => target.iter().collect(),
        Action::NavigateTo { target, speed } => {
            let mut values: Vec<&Value> = target.iter().collect();
            values.push(speed);
            values
        }
        Action::ChangePosition { by, .. }
        | Action::Turn { degrees: by, .. }
        | Action::SetRotation { degrees: by, .. }
        | Action::SetDensity(by)
        | Action::SetMass(by)
        | Action::SetCameraPitch(by)
        | Action::SetAnimationSpeed(by)
        | Action::SetCameraFov(by) => vec![by],
        Action::TweenScale {
            factor, seconds, ..
        } => vec![factor, seconds],
        Action::TweenRotation {
            degrees, seconds, ..
        } => vec![degrees, seconds],
        Action::TweenColor { color, seconds, .. } => vec![color, seconds],
        Action::PlayAnimation { clip, speed } => vec![clip, speed],
        Action::EnableVolume { volume, .. } => vec![volume],
        Action::SetVolumeWeight { volume, weight } => vec![volume, weight],
        Action::SetCloudLayer { layer, value, .. } => vec![layer, value],
        Action::PaintTile { map, tile, x, y, z } => vec![map, tile, x, y, z],
        Action::SetParallax { layer, value, .. } => vec![layer, value],
        Action::PlaySound {
            sound,
            volume,
            pitch,
            ..
        } => vec![sound, volume, pitch],
        Action::PlaySoundAt {
            sound,
            volume,
            pitch,
            target,
            ..
        } => vec![sound, volume, pitch, target],
        Action::SetSoundVolume { sound, volume } => vec![sound, volume],
        Action::SetSoundPitch { sound, pitch } => vec![sound, pitch],
        Action::SetBusVolume { volume, .. } => vec![volume],
        Action::SetComponentField { value, .. } => vec![value],
        Action::ShowElement(spec) => {
            let mut values = vec![&spec.id, &spec.content];
            if let Some([low, high]) = &spec.range {
                values.push(low);
                values.push(high);
            }
            if let Some(value) = &spec.value {
                values.push(value);
            }
            values.push(&spec.offset[0]);
            values.push(&spec.offset[1]);
            values.push(&spec.size[0]);
            values.push(&spec.size[1]);
            values.push(&spec.parent);
            values
        }
        Action::SetUiProp { id, value, .. } => vec![id, value],
        Action::HideElement { id, .. } => vec![id],
        Action::CreateActor { name, position } => {
            let mut values = vec![name];
            values.extend(position.iter());
            values
        }
        Action::RumbleGamepad { strength, duration } => vec![strength, duration],
        Action::BindAction { action, binding } => vec![action, binding],
        Action::SetVariable { value, .. } | Action::ChangeVariable { value, .. } => vec![value],
        Action::AddToList { value, .. } => vec![value],
        Action::DeleteOfList { index, .. } => vec![index],
        Action::ShiftList { amount, .. } => vec![amount],
        Action::InsertIntoList { value, index, .. } => vec![value, index],
        Action::ReplaceItemOfList { index, value, .. } => vec![index, value],
        Action::SetDictValue { key, value, .. } => vec![key, value],
        Action::DeleteDictKey { key, .. } => vec![key],
        Action::LoadJsonIntoDict { json, .. } | Action::LoadJsonIntoList { json, .. } => {
            vec![json]
        }
        _ => Vec::new(),
    }
}

fn suspendable_blocks(program: &Program) -> HashSet<String> {
    let mut ranges: HashMap<&str, (usize, usize)> = HashMap::new();
    for (id, &start) in &program.blocks {
        let mut end = start;
        for (pc, step) in program.steps.iter().enumerate().skip(start) {
            end = pc;
            if matches!(step, Step::End) {
                break;
            }
        }
        ranges.insert(id.as_str(), (start, end));
    }
    let mut suspendable: HashSet<String> = HashSet::new();
    for (id, (start, end)) in &ranges {
        if program.steps[*start..=*end].iter().any(step_is_suspending) {
            suspendable.insert(id.to_string());
        }
    }
    loop {
        let mut changed = false;
        for (id, (start, end)) in &ranges {
            if suspendable.contains(*id) {
                continue;
            }
            let reaches = program.steps[*start..=*end]
                .iter()
                .flat_map(calls_in_step)
                .any(|called| suspendable.contains(&called));
            if reaches {
                suspendable.insert(id.to_string());
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    suspendable
}

fn lift_value(
    value: Value,
    blocks: &HashMap<String, usize>,
    suspendable: &HashSet<String>,
    bounds: &HashMap<String, usize>,
    next_temp: &mut usize,
    invokes: &mut Vec<Step>,
) -> Value {
    match value {
        Value::Call {
            block_id,
            args,
            branches,
            saved,
        } => {
            if !blocks.contains_key(&block_id) {
                return Value::Call {
                    block_id,
                    args,
                    branches,
                    saved,
                };
            }
            let bound = bounds.get(&block_id).copied().unwrap_or(0).min(args.len());
            let mut lifted = Vec::with_capacity(args.len());
            for (index, arg) in args.into_iter().enumerate() {
                if index < bound {
                    lifted.push(lift_value(
                        arg,
                        blocks,
                        suspendable,
                        bounds,
                        next_temp,
                        invokes,
                    ));
                } else {
                    lifted.push(arg);
                }
            }
            if suspendable.contains(&block_id) {
                let temp = *next_temp;
                *next_temp += 1;
                invokes.push(Step::Invoke {
                    block_id: block_id.clone(),
                    args: lifted,
                    temp,
                });
                temp_var(temp)
            } else {
                Value::Call {
                    block_id,
                    args: lifted,
                    branches,
                    saved,
                }
            }
        }
        Value::Op { op, args, saved } => {
            let lifted = args
                .into_iter()
                .map(|arg| lift_value(arg, blocks, suspendable, bounds, next_temp, invokes))
                .collect();
            Value::Op {
                op,
                args: lifted,
                saved,
            }
        }
        other => other,
    }
}

struct LiftCtx<'a> {
    blocks: &'a HashMap<String, usize>,
    suspendable: &'a HashSet<String>,
    bounds: &'a HashMap<String, usize>,
    next_temp: &'a mut usize,
    invokes: &'a mut Vec<Step>,
}

fn lift_one(value: Value, ctx: &mut LiftCtx) -> Value {
    lift_value(
        value,
        ctx.blocks,
        ctx.suspendable,
        ctx.bounds,
        ctx.next_temp,
        ctx.invokes,
    )
}

fn lift_action(action: Action, ctx: &mut LiftCtx) -> Action {
    match action {
        Action::Move(v) => Action::Move(lift_one(v, ctx)),
        Action::GoTo(mut t) => {
            for v in &mut t {
                *v = lift_one(std::mem::replace(v, Value::Bool), ctx);
            }
            Action::GoTo(t)
        }
        Action::NavigateTo { target, speed } => {
            let mut t = target;
            for v in &mut t {
                *v = lift_one(std::mem::replace(v, Value::Bool), ctx);
            }
            Action::NavigateTo {
                target: t,
                speed: lift_one(speed, ctx),
            }
        }
        Action::ChangePosition { axis, by } => Action::ChangePosition {
            axis,
            by: lift_one(by, ctx),
        },
        Action::Turn { axis, degrees } => Action::Turn {
            axis,
            degrees: lift_one(degrees, ctx),
        },
        Action::SetRotation { axis, degrees } => Action::SetRotation {
            axis,
            degrees: lift_one(degrees, ctx),
        },
        Action::SetScale(v) => Action::SetScale(lift_one(v, ctx)),
        Action::SetExposure(v) => Action::SetExposure(lift_one(v, ctx)),
        Action::SetLightIntensity(v) => Action::SetLightIntensity(lift_one(v, ctx)),
        Action::SetEmissiveStrength(v) => Action::SetEmissiveStrength(lift_one(v, ctx)),
        Action::SetPeakBrightness(v) => Action::SetPeakBrightness(lift_one(v, ctx)),
        Action::SetShadowDistance(v) => Action::SetShadowDistance(lift_one(v, ctx)),
        Action::SetGiBounces(v) => Action::SetGiBounces(lift_one(v, ctx)),
        Action::SetGiSamples(v) => Action::SetGiSamples(lift_one(v, ctx)),
        Action::SetFogDensity(v) => Action::SetFogDensity(lift_one(v, ctx)),
        Action::SetAurora(v) => Action::SetAurora(lift_one(v, ctx)),
        Action::SetLightningRate(v) => Action::SetLightningRate(lift_one(v, ctx)),
        Action::StrikeLightning(mut t) => {
            for v in &mut t {
                *v = lift_one(std::mem::replace(v, Value::Bool), ctx);
            }
            Action::StrikeLightning(t)
        }
        Action::SetWind { property, value } => Action::SetWind {
            property,
            value: lift_one(value, ctx),
        },
        Action::SetClouds { property, value } => Action::SetClouds {
            property,
            value: lift_one(value, ctx),
        },
        Action::SetWater { property, value } => Action::SetWater {
            property,
            value: lift_one(value, ctx),
        },
        Action::SetCloudDrift(mut t) => {
            for v in &mut t {
                *v = lift_one(std::mem::replace(v, Value::Bool), ctx);
            }
            Action::SetCloudDrift(t)
        }
        Action::EnableVolume { volume, enabled } => Action::EnableVolume {
            volume: lift_one(volume, ctx),
            enabled,
        },
        Action::SetVolumeWeight { volume, weight } => Action::SetVolumeWeight {
            volume: lift_one(volume, ctx),
            weight: lift_one(weight, ctx),
        },
        Action::SetCloudLayer {
            layer,
            property,
            value,
        } => Action::SetCloudLayer {
            layer: lift_one(layer, ctx),
            property,
            value: lift_one(value, ctx),
        },
        Action::PaintTile { map, tile, x, y, z } => Action::PaintTile {
            map: lift_one(map, ctx),
            tile: lift_one(tile, ctx),
            x: lift_one(x, ctx),
            y: lift_one(y, ctx),
            z: lift_one(z, ctx),
        },
        Action::SetParallax { layer, axis, value } => Action::SetParallax {
            layer: lift_one(layer, ctx),
            axis,
            value: lift_one(value, ctx),
        },
        Action::ApplyImpulse(mut t) => {
            for v in &mut t {
                *v = lift_one(std::mem::replace(v, Value::Bool), ctx);
            }
            Action::ApplyImpulse(t)
        }
        Action::SetVelocity(mut t) => {
            for v in &mut t {
                *v = lift_one(std::mem::replace(v, Value::Bool), ctx);
            }
            Action::SetVelocity(t)
        }
        Action::SetGravity(mut t) => {
            for v in &mut t {
                *v = lift_one(std::mem::replace(v, Value::Bool), ctx);
            }
            Action::SetGravity(t)
        }
        Action::SetDensity(v) => Action::SetDensity(lift_one(v, ctx)),
        Action::SetMass(v) => Action::SetMass(lift_one(v, ctx)),
        Action::BurstParticles(v) => Action::BurstParticles(lift_one(v, ctx)),
        Action::SetEmitterDial { dial, value } => Action::SetEmitterDial {
            dial,
            value: lift_one(value, ctx),
        },
        Action::SetSpriteDial { dial, value } => Action::SetSpriteDial {
            dial,
            value: lift_one(value, ctx),
        },
        Action::FireAnimationTrigger(v) => Action::FireAnimationTrigger(lift_one(v, ctx)),
        Action::SetRigSlot { slot, attachment } => Action::SetRigSlot {
            slot: lift_one(slot, ctx),
            attachment: lift_one(attachment, ctx),
        },
        Action::SetSlotTint { slot, color } => Action::SetSlotTint {
            slot: lift_one(slot, ctx),
            color: lift_one(color, ctx),
        },
        Action::SetIkTarget { constraint, x, y } => Action::SetIkTarget {
            constraint: lift_one(constraint, ctx),
            x: lift_one(x, ctx),
            y: lift_one(y, ctx),
        },
        Action::Say(v) => Action::Say(lift_one(v, ctx)),
        Action::SetColor(v) => Action::SetColor(lift_one(v, ctx)),
        Action::PlaySound {
            sound,
            volume,
            pitch,
            loop_,
            bus,
        } => Action::PlaySound {
            sound: lift_one(sound, ctx),
            volume: lift_one(volume, ctx),
            pitch: lift_one(pitch, ctx),
            loop_,
            bus,
        },
        Action::PlaySoundAt {
            sound,
            volume,
            pitch,
            loop_,
            bus,
            target,
        } => Action::PlaySoundAt {
            sound: lift_one(sound, ctx),
            volume: lift_one(volume, ctx),
            pitch: lift_one(pitch, ctx),
            loop_,
            bus,
            target: lift_one(target, ctx),
        },
        Action::StopSound { sound } => Action::StopSound {
            sound: lift_one(sound, ctx),
        },
        Action::SetSoundVolume { sound, volume } => Action::SetSoundVolume {
            sound: lift_one(sound, ctx),
            volume: lift_one(volume, ctx),
        },
        Action::SetSoundPitch { sound, pitch } => Action::SetSoundPitch {
            sound: lift_one(sound, ctx),
            pitch: lift_one(pitch, ctx),
        },
        Action::SetBusVolume { bus, volume } => Action::SetBusVolume {
            bus,
            volume: lift_one(volume, ctx),
        },
        Action::SetComponentField {
            component,
            field,
            value,
        } => Action::SetComponentField {
            component,
            field,
            value: lift_one(value, ctx),
        },
        Action::SetCameraPitch(v) => Action::SetCameraPitch(lift_one(v, ctx)),
        Action::SetCameraFov(v) => Action::SetCameraFov(lift_one(v, ctx)),
        Action::SetAnimationSpeed(v) => Action::SetAnimationSpeed(lift_one(v, ctx)),
        Action::TweenScale {
            factor,
            seconds,
            easing,
        } => Action::TweenScale {
            factor: lift_one(factor, ctx),
            seconds: lift_one(seconds, ctx),
            easing,
        },
        Action::TweenRotation {
            axis,
            degrees,
            seconds,
            easing,
        } => Action::TweenRotation {
            axis,
            degrees: lift_one(degrees, ctx),
            seconds: lift_one(seconds, ctx),
            easing,
        },
        Action::TweenColor {
            color,
            seconds,
            easing,
        } => Action::TweenColor {
            color: lift_one(color, ctx),
            seconds: lift_one(seconds, ctx),
            easing,
        },
        Action::PlayAnimation { clip, speed } => Action::PlayAnimation {
            clip: lift_one(clip, ctx),
            speed: lift_one(speed, ctx),
        },
        Action::SetParent(v) => Action::SetParent(lift_one(v, ctx)),
        Action::CreateActor { name, position } => {
            let mut p = position;
            for v in &mut p {
                *v = lift_one(std::mem::replace(v, Value::Bool), ctx);
            }
            Action::CreateActor {
                name: lift_one(name, ctx),
                position: p,
            }
        }
        Action::DeleteActor(v) => Action::DeleteActor(lift_one(v, ctx)),
        Action::RumbleGamepad { strength, duration } => Action::RumbleGamepad {
            strength: lift_one(strength, ctx),
            duration: lift_one(duration, ctx),
        },
        Action::BindAction { action, binding } => Action::BindAction {
            action: lift_one(action, ctx),
            binding: lift_one(binding, ctx),
        },
        Action::ShowElement(mut spec) => {
            spec.id = lift_one(std::mem::replace(&mut spec.id, Value::Bool), ctx);
            spec.content = lift_one(std::mem::replace(&mut spec.content, Value::Bool), ctx);
            if let Some([low, high]) = spec.range.take() {
                let low = lift_one(low, ctx);
                let high = lift_one(high, ctx);
                spec.range = Some([low, high]);
            }
            if let Some(value) = spec.value.take() {
                spec.value = Some(lift_one(value, ctx));
            }
            spec.offset[0] = lift_one(std::mem::replace(&mut spec.offset[0], Value::Bool), ctx);
            spec.offset[1] = lift_one(std::mem::replace(&mut spec.offset[1], Value::Bool), ctx);
            spec.size[0] = lift_one(std::mem::replace(&mut spec.size[0], Value::Bool), ctx);
            spec.size[1] = lift_one(std::mem::replace(&mut spec.size[1], Value::Bool), ctx);
            spec.parent = lift_one(std::mem::replace(&mut spec.parent, Value::Bool), ctx);
            Action::ShowElement(spec)
        }
        Action::SetUiProp { prop, id, value } => Action::SetUiProp {
            prop,
            id: lift_one(id, ctx),
            value: lift_one(value, ctx),
        },
        Action::HideElement { id, all } => Action::HideElement {
            id: lift_one(id, ctx),
            all,
        },
        Action::DeleteElement(v) => Action::DeleteElement(lift_one(v, ctx)),
        Action::SetFocus(v) => Action::SetFocus(lift_one(v, ctx)),
        Action::SetVariable { name, value } => Action::SetVariable {
            name,
            value: lift_one(value, ctx),
        },
        Action::ChangeVariable { name, value } => Action::ChangeVariable {
            name,
            value: lift_one(value, ctx),
        },
        Action::AddToList { value, name } => Action::AddToList {
            value: lift_one(value, ctx),
            name,
        },
        Action::DeleteOfList { index, name } => Action::DeleteOfList {
            index: lift_one(index, ctx),
            name,
        },
        Action::ShiftList { name, amount } => Action::ShiftList {
            name,
            amount: lift_one(amount, ctx),
        },
        Action::InsertIntoList { value, index, name } => Action::InsertIntoList {
            value: lift_one(value, ctx),
            index: lift_one(index, ctx),
            name,
        },
        Action::ReplaceItemOfList { index, name, value } => Action::ReplaceItemOfList {
            index: lift_one(index, ctx),
            name,
            value: lift_one(value, ctx),
        },
        Action::SetDictValue { key, name, value } => Action::SetDictValue {
            key: lift_one(key, ctx),
            name,
            value: lift_one(value, ctx),
        },
        Action::DeleteDictKey { key, name } => Action::DeleteDictKey {
            key: lift_one(key, ctx),
            name,
        },
        Action::LoadJsonIntoDict { json, name } => Action::LoadJsonIntoDict {
            json: lift_one(json, ctx),
            name,
        },
        Action::LoadJsonIntoList { json, name } => Action::LoadJsonIntoList {
            json: lift_one(json, ctx),
            name,
        },
        other => other,
    }
}

fn lift_step(
    step: Step,
    blocks: &HashMap<String, usize>,
    suspendable: &HashSet<String>,
    bounds: &HashMap<String, usize>,
    next_temp: &mut usize,
) -> Vec<Step> {
    let mut invokes = Vec::new();
    let mut ctx = LiftCtx {
        blocks,
        suspendable,
        bounds,
        next_temp,
        invokes: &mut invokes,
    };
    let rewritten = match step {
        Step::Action(action) => Step::Action(lift_action(action, &mut ctx)),
        Step::JumpUnless { condition, to } => Step::JumpUnless {
            condition: lift_one(condition, &mut ctx),
            to,
        },
        Step::LoopBegin { kind, end } => {
            let kind = match kind {
                LoopKind::Repeat(count) => LoopKind::Repeat(lift_one(count, &mut ctx)),
                LoopKind::While(condition) => LoopKind::While(lift_one(condition, &mut ctx)),
                LoopKind::Forever => LoopKind::Forever,
            };
            Step::LoopBegin { kind, end }
        }
        Step::Wait(duration) => Step::Wait(lift_one(duration, &mut ctx)),
        Step::WaitUntil(condition) => Step::WaitUntil(lift_one(condition, &mut ctx)),
        Step::Glide {
            seconds,
            target,
            easing,
        } => {
            let seconds = lift_one(seconds, &mut ctx);
            let mut t = target;
            for v in &mut t {
                *v = lift_one(std::mem::replace(v, Value::Bool), &mut ctx);
            }
            Step::Glide {
                seconds,
                target: t,
                easing,
            }
        }
        Step::TweenScale {
            factor,
            seconds,
            easing,
        } => Step::TweenScale {
            factor: lift_one(factor, &mut ctx),
            seconds: lift_one(seconds, &mut ctx),
            easing,
        },
        Step::TweenRotation {
            axis,
            degrees,
            seconds,
            easing,
        } => Step::TweenRotation {
            axis,
            degrees: lift_one(degrees, &mut ctx),
            seconds: lift_one(seconds, &mut ctx),
            easing,
        },
        Step::TweenColor {
            color,
            seconds,
            easing,
        } => Step::TweenColor {
            color: lift_one(color, &mut ctx),
            seconds: lift_one(seconds, &mut ctx),
            easing,
        },
        Step::Call { block_id, args } => {
            if !blocks.contains_key(&block_id) {
                Step::Call { block_id, args }
            } else {
                let bound = bounds.get(&block_id).copied().unwrap_or(0).min(args.len());
                let mut lifted = Vec::with_capacity(args.len());
                for (index, arg) in args.into_iter().enumerate() {
                    if index < bound {
                        lifted.push(lift_one(arg, &mut ctx));
                    } else {
                        lifted.push(arg);
                    }
                }
                Step::Call {
                    block_id,
                    args: lifted,
                }
            }
        }
        Step::Return(value) => Step::Return(lift_one(value, &mut ctx)),
        other => other,
    };
    invokes.push(rewritten);
    invokes
}

fn lift_suspendable_calls(program: &mut Program, bounds: &HashMap<String, usize>) {
    let suspendable = suspendable_blocks(program);
    if suspendable.is_empty() {
        return;
    }
    let used = program
        .steps
        .iter()
        .flat_map(calls_in_step)
        .any(|called| suspendable.contains(&called));
    if !used {
        return;
    }
    let mut next_temp = 0;
    let mut expanded: Vec<Vec<Step>> = Vec::with_capacity(program.steps.len());
    for step in std::mem::take(&mut program.steps) {
        expanded.push(lift_step(
            step,
            &program.blocks,
            &suspendable,
            bounds,
            &mut next_temp,
        ));
    }
    let mut mapping = HashMap::new();
    let mut next = 0;
    for (old, group) in expanded.iter().enumerate() {
        mapping.insert(old, next);
        next += group.len();
    }
    let remap = |old: usize| -> usize { *mapping.get(&old).unwrap_or(&old) };
    let mut steps = Vec::with_capacity(next);
    for group in expanded {
        for step in group {
            steps.push(match step {
                Step::Jump { to } => Step::Jump { to: remap(to) },
                Step::JumpUnless { condition, to } => Step::JumpUnless {
                    condition,
                    to: remap(to),
                },
                Step::LoopBegin { kind, end } => Step::LoopBegin {
                    kind,
                    end: remap(end),
                },
                Step::LoopEnd { begin } => Step::LoopEnd {
                    begin: remap(begin),
                },
                other => other,
            });
        }
    }
    for entry in &mut program.entries {
        entry.pc = remap(entry.pc);
    }
    for start in program.blocks.values_mut() {
        *start = remap(*start);
    }
    program.steps = steps;
}

fn emit(steps: &mut Vec<Step>, kind: &InstructionKind) {
    use InstructionKind as K;
    match kind {
        // Headers only ever appear at index 0 of a strand, which the caller
        // already stripped; a stray one is skipped rather than run.
        K::WhenStarted
        | K::WhenKeyPressed { .. }
        | K::WhenActionPressed { .. }
        | K::WhenTouched
        | K::WhenClicked
        | K::WhenCollision { .. }
        | K::WhenMessage { .. }
        | K::WhenCloned
        | K::WhenAnimationEnds { .. }
        | K::WhenParticles { .. }
        | K::WhenAnimationMarker { .. }
        | K::WhenEnterRoom { .. }
        | K::WhenUiEvent { .. }
        | K::WhenUiClicked { .. }
        | K::WhenUiChanged { .. }
        | K::BlockHeader { .. } => {}

        K::Move { steps: amount } => steps.push(Step::Action(Action::Move(amount.clone()))),
        K::GoTo { x, y, z } => steps.push(Step::Action(Action::GoTo([
            x.clone(),
            y.clone(),
            z.clone(),
        ]))),
        K::NavigateTo { x, y, z, speed } => steps.push(Step::Action(Action::NavigateTo {
            target: [x.clone(), y.clone(), z.clone()],
            speed: speed.clone(),
        })),
        K::ChangePosition { axis, by } => steps.push(Step::Action(Action::ChangePosition {
            axis: *axis,
            by: by.clone(),
        })),
        K::Glide {
            seconds,
            x,
            y,
            z,
            easing,
        } => steps.push(Step::Glide {
            seconds: seconds.clone(),
            target: [x.clone(), y.clone(), z.clone()],
            easing: *easing,
        }),
        K::TweenScale {
            factor,
            seconds,
            easing,
        } => steps.push(Step::TweenScale {
            factor: factor.clone(),
            seconds: seconds.clone(),
            easing: *easing,
        }),
        K::TweenRotation {
            axis,
            degrees,
            seconds,
            easing,
        } => steps.push(Step::TweenRotation {
            axis: *axis,
            degrees: degrees.clone(),
            seconds: seconds.clone(),
            easing: *easing,
        }),
        K::TweenColor {
            color,
            seconds,
            easing,
        } => steps.push(Step::TweenColor {
            color: color.clone(),
            seconds: seconds.clone(),
            easing: *easing,
        }),
        K::StopTweens => steps.push(Step::Action(Action::StopTweens)),
        K::PlayAnimation { clip, speed } => steps.push(Step::Action(Action::PlayAnimation {
            clip: clip.clone(),
            speed: speed.clone(),
        })),
        K::StopAnimation => steps.push(Step::Action(Action::StopAnimation)),
        K::SetAnimationSpeed { speed } => {
            steps.push(Step::Action(Action::SetAnimationSpeed(speed.clone())))
        }
        K::FireAnimationTrigger { name } => {
            steps.push(Step::Action(Action::FireAnimationTrigger(name.clone())))
        }
        K::SetRigSlot { slot, attachment } => steps.push(Step::Action(Action::SetRigSlot {
            slot: slot.clone(),
            attachment: attachment.clone(),
        })),
        K::SetSlotTint { slot, color } => steps.push(Step::Action(Action::SetSlotTint {
            slot: slot.clone(),
            color: color.clone(),
        })),
        K::SetIkTarget { constraint, x, y } => steps.push(Step::Action(Action::SetIkTarget {
            constraint: constraint.clone(),
            x: x.clone(),
            y: y.clone(),
        })),
        K::SetSpriteDial { dial, value } => steps.push(Step::Action(Action::SetSpriteDial {
            dial: *dial,
            value: value.clone(),
        })),
        K::Turn { axis, degrees } => steps.push(Step::Action(Action::Turn {
            axis: *axis,
            degrees: degrees.clone(),
        })),
        K::SetRotation { axis, degrees } => steps.push(Step::Action(Action::SetRotation {
            axis: *axis,
            degrees: degrees.clone(),
        })),
        K::PointTowards { target } => {
            steps.push(Step::Action(Action::PointTowards(target.clone())))
        }
        K::SetScale { factor } => steps.push(Step::Action(Action::SetScale(factor.clone()))),
        K::SetExposure { ev } => steps.push(Step::Action(Action::SetExposure(ev.clone()))),
        K::SetLightIntensity { intensity } => {
            steps.push(Step::Action(Action::SetLightIntensity(intensity.clone())))
        }
        K::SetEmissiveStrength { strength } => {
            steps.push(Step::Action(Action::SetEmissiveStrength(strength.clone())))
        }
        K::SetHdrOutput { enabled } => steps.push(Step::Action(Action::SetHdrOutput(*enabled))),
        K::SetPeakBrightness { nits } => {
            steps.push(Step::Action(Action::SetPeakBrightness(nits.clone())))
        }
        K::EnableVolume { enabled, volume } => steps.push(Step::Action(Action::EnableVolume {
            volume: volume.clone(),
            enabled: *enabled,
        })),
        K::SetVolumeWeight { volume, weight } => {
            steps.push(Step::Action(Action::SetVolumeWeight {
                volume: volume.clone(),
                weight: weight.clone(),
            }))
        }
        K::CaptureProbes => steps.push(Step::Action(Action::CaptureProbes)),
        K::SetShadowDistance { distance } => {
            steps.push(Step::Action(Action::SetShadowDistance(distance.clone())))
        }
        K::SetLightShadows { enabled } => {
            steps.push(Step::Action(Action::SetLightShadows(*enabled)))
        }
        K::SetRayTracing { enabled } => steps.push(Step::Action(Action::SetRayTracing(*enabled))),
        K::SetGiBounces { bounces } => {
            steps.push(Step::Action(Action::SetGiBounces(bounces.clone())))
        }
        K::SetGiSamples { samples } => {
            steps.push(Step::Action(Action::SetGiSamples(samples.clone())))
        }
        K::SetFogDensity { density } => {
            steps.push(Step::Action(Action::SetFogDensity(density.clone())))
        }
        K::SetAurora { kp } => steps.push(Step::Action(Action::SetAurora(kp.clone()))),
        K::StrikeLightning { x, y, z } => steps.push(Step::Action(Action::StrikeLightning([
            x.clone(),
            y.clone(),
            z.clone(),
        ]))),
        K::SetLightningRate { rate } => {
            steps.push(Step::Action(Action::SetLightningRate(rate.clone())))
        }
        K::SetWind { property, value } => steps.push(Step::Action(Action::SetWind {
            property: *property,
            value: value.clone(),
        })),
        K::SetClouds { property, value } => steps.push(Step::Action(Action::SetClouds {
            property: *property,
            value: value.clone(),
        })),
        K::SetWater { property, value } => steps.push(Step::Action(Action::SetWater {
            property: *property,
            value: value.clone(),
        })),
        K::SetCloudLayer {
            layer,
            property,
            value,
        } => steps.push(Step::Action(Action::SetCloudLayer {
            layer: layer.clone(),
            property: *property,
            value: value.clone(),
        })),
        K::PaintTile { map, tile, x, y, z } => steps.push(Step::Action(Action::PaintTile {
            map: map.clone(),
            tile: tile.clone(),
            x: x.clone(),
            y: y.clone(),
            z: z.clone(),
        })),
        K::SetParallax { layer, axis, value } => steps.push(Step::Action(Action::SetParallax {
            layer: layer.clone(),
            axis: *axis,
            value: value.clone(),
        })),
        K::SetCloudDrift { x, y, z } => steps.push(Step::Action(Action::SetCloudDrift([
            x.clone(),
            y.clone(),
            z.clone(),
        ]))),
        K::SetBody { body } => steps.push(Step::Action(Action::SetBody(*body))),
        K::ApplyImpulse { x, y, z } => steps.push(Step::Action(Action::ApplyImpulse([
            x.clone(),
            y.clone(),
            z.clone(),
        ]))),
        K::SetVelocity { x, y, z } => steps.push(Step::Action(Action::SetVelocity([
            x.clone(),
            y.clone(),
            z.clone(),
        ]))),
        K::SetGravity { x, y, z } => steps.push(Step::Action(Action::SetGravity([
            x.clone(),
            y.clone(),
            z.clone(),
        ]))),
        K::SetDensity { density } => steps.push(Step::Action(Action::SetDensity(density.clone()))),
        K::SetMass { mass } => steps.push(Step::Action(Action::SetMass(mass.clone()))),
        K::SetTrigger { trigger } => steps.push(Step::Action(Action::SetTrigger(*trigger))),
        K::SetCollisionLayer { layer } => {
            steps.push(Step::Action(Action::SetCollisionLayer(layer.clone())))
        }
        K::SetCollisionMask { mask } => {
            steps.push(Step::Action(Action::SetCollisionMask(mask.clone())))
        }
        K::BurstParticles { count } => {
            steps.push(Step::Action(Action::BurstParticles(count.clone())))
        }
        K::SetEmitterDial { dial, value } => steps.push(Step::Action(Action::SetEmitterDial {
            dial: *dial,
            value: value.clone(),
        })),
        K::SetTrailEnabled { enabled } => {
            steps.push(Step::Action(Action::SetTrailEnabled(*enabled)))
        }
        K::SetEmitterPlaying { playing } => {
            steps.push(Step::Action(Action::SetEmitterPlaying(*playing)))
        }
        K::Say { text } => steps.push(Step::Action(Action::Say(text.clone()))),
        K::SetVisible { visible } => steps.push(Step::Action(Action::SetVisible(*visible))),
        K::SetColor { color } => steps.push(Step::Action(Action::SetColor(color.clone()))),
        K::PlaySound {
            sound,
            volume,
            pitch,
            loop_,
            bus,
        } => steps.push(Step::Action(Action::PlaySound {
            sound: sound.clone(),
            volume: volume.clone(),
            pitch: pitch.clone(),
            loop_: *loop_,
            bus: *bus,
        })),
        K::PlaySoundAt {
            sound,
            volume,
            pitch,
            loop_,
            bus,
            target,
        } => steps.push(Step::Action(Action::PlaySoundAt {
            sound: sound.clone(),
            volume: volume.clone(),
            pitch: pitch.clone(),
            loop_: *loop_,
            bus: *bus,
            target: target.clone(),
        })),
        K::StopSound { sound } => steps.push(Step::Action(Action::StopSound {
            sound: sound.clone(),
        })),
        K::SetSoundVolume { sound, volume } => steps.push(Step::Action(Action::SetSoundVolume {
            sound: sound.clone(),
            volume: volume.clone(),
        })),
        K::SetSoundPitch { sound, pitch } => steps.push(Step::Action(Action::SetSoundPitch {
            sound: sound.clone(),
            pitch: pitch.clone(),
        })),
        K::SetBusVolume { bus, volume } => steps.push(Step::Action(Action::SetBusVolume {
            bus: *bus,
            volume: volume.clone(),
        })),
        K::SetComponentField {
            component,
            field,
            value,
        } => steps.push(Step::Action(Action::SetComponentField {
            component: component.clone(),
            field: field.clone(),
            value: value.clone(),
        })),
        K::SetCameraView { view } => steps.push(Step::Action(Action::SetCameraView(*view))),
        K::SetCameraPitch { degrees } => {
            steps.push(Step::Action(Action::SetCameraPitch(degrees.clone())))
        }
        K::SetCameraFov { fov } => steps.push(Step::Action(Action::SetCameraFov(fov.clone()))),
        K::AttachComponent { component } => steps.push(Step::Action(Action::AttachComponent(
            component.trim().to_string(),
        ))),
        K::DetachComponent { component } => steps.push(Step::Action(Action::DetachComponent(
            component.trim().to_string(),
        ))),
        K::SetParent { parent } => steps.push(Step::Action(Action::SetParent(parent.clone()))),
        K::CreateClone { of } => {
            steps.push(Step::Action(Action::CreateClone(of.trim().to_string())))
        }
        K::CreateActor { name, x, y, z } => steps.push(Step::Action(Action::CreateActor {
            name: name.clone(),
            position: [x.clone(), y.clone(), z.clone()],
        })),
        K::DeleteActor { target } => steps.push(Step::Action(Action::DeleteActor(target.clone()))),
        K::Broadcast { name } => steps.push(Step::Action(Action::Broadcast(name.clone()))),
        K::SetMouseLocked { locked } => steps.push(Step::Action(Action::SetMouseLocked(*locked))),
        K::RumbleGamepad { strength, duration } => {
            steps.push(Step::Action(Action::RumbleGamepad {
                strength: strength.clone(),
                duration: duration.clone(),
            }))
        }
        K::BindAction { action, binding } => steps.push(Step::Action(Action::BindAction {
            action: action.clone(),
            binding: binding.clone(),
        })),
        K::ClearActionBindings { action } => {
            steps.push(Step::Action(Action::ClearActionBindings {
                action: action.clone(),
            }))
        }

        K::ShowPanel {
            element: id,
            title,
            modal,
            anchor,
            x,
            y,
            width,
            height,
            parent,
        } => steps.push(show(
            UiKind::Panel,
            id,
            title,
            None,
            *anchor,
            x,
            y,
            width,
            height,
            parent,
            *modal,
        )),
        K::ShowLabel {
            element: id,
            text,
            anchor,
            x,
            y,
            width,
            height,
            parent,
        } => steps.push(show(
            UiKind::Label,
            id,
            text,
            None,
            *anchor,
            x,
            y,
            width,
            height,
            parent,
            false,
        )),
        K::ShowButton {
            element: id,
            label,
            anchor,
            x,
            y,
            width,
            height,
            parent,
        } => steps.push(show(
            UiKind::Button,
            id,
            label,
            None,
            *anchor,
            x,
            y,
            width,
            height,
            parent,
            false,
        )),
        K::ShowImage {
            element: id,
            asset,
            anchor,
            x,
            y,
            width,
            height,
            parent,
        } => steps.push(show(
            UiKind::Image,
            id,
            asset,
            None,
            *anchor,
            x,
            y,
            width,
            height,
            parent,
            false,
        )),
        K::ShowInput {
            element: id,
            placeholder,
            anchor,
            x,
            y,
            width,
            height,
            parent,
        } => steps.push(show(
            UiKind::Input,
            id,
            placeholder,
            None,
            *anchor,
            x,
            y,
            width,
            height,
            parent,
            false,
        )),
        K::ShowSlider {
            element: id,
            min,
            max,
            value,
            anchor,
            x,
            y,
            width,
            height,
            parent,
        } => steps.push(Step::Action(Action::ShowElement(Box::new(ShowElement {
            kind: UiKind::Slider,
            id: id.clone(),
            // A slider has no caption of its own; its ends and its number
            // are its content.
            content: Value::text(""),
            range: Some([min.clone(), max.clone()]),
            value: Some(value.clone()),
            anchor: *anchor,
            offset: [x.clone(), y.clone()],
            size: [width.clone(), height.clone()],
            parent: parent.clone(),
            flag: false,
        })))),
        K::ShowToggle {
            element: id,
            label,
            on,
            anchor,
            x,
            y,
            width,
            height,
            parent,
        } => steps.push(show(
            UiKind::Toggle,
            id,
            label,
            None,
            *anchor,
            x,
            y,
            width,
            height,
            parent,
            *on,
        )),
        K::ShowWidget {
            kind,
            element,
            text,
            anchor,
            x,
            y,
            width,
            height,
            parent,
        } => steps.push(show(
            *kind, element, text, None, *anchor, x, y, width, height, parent, false,
        )),
        K::ShowList {
            element: id,
            anchor,
            x,
            y,
            width,
            height,
            parent,
        } => steps.push(show(
            UiKind::List,
            id,
            &Value::text(""),
            None,
            *anchor,
            x,
            y,
            width,
            height,
            parent,
            false,
        )),
        K::SetUiTheme { theme } => steps.push(Step::Action(Action::SetUiTheme(*theme))),
        K::BindUi { element, value } => steps.push(Step::Action(Action::SetUiProp {
            prop: crate::ui::UiProp::Bind,
            id: element.clone(),
            value: value.clone(),
        })),
        K::SetUiItems { element, value } => steps.push(Step::Action(Action::SetUiProp {
            prop: crate::ui::UiProp::Items,
            id: element.clone(),
            value: value.clone(),
        })),
        K::ScrollUi { element, value } => steps.push(Step::Action(Action::SetUiProp {
            prop: crate::ui::UiProp::Scroll,
            id: element.clone(),
            value: value.clone(),
        })),
        K::SetElementTheme { element, value } => steps.push(Step::Action(Action::SetUiProp {
            prop: crate::ui::UiProp::Theme,
            id: element.clone(),
            value: value.clone(),
        })),
        K::SetUiProp {
            prop,
            element,
            value,
        } => steps.push(Step::Action(Action::SetUiProp {
            prop: *prop,
            id: element.clone(),
            value: value.clone(),
        })),
        K::HideElement { element } => steps.push(Step::Action(Action::HideElement {
            id: element.clone(),
            all: false,
        })),
        K::HideAllUi => steps.push(Step::Action(Action::HideElement {
            id: Value::text(""),
            all: true,
        })),
        K::DeleteElement { element } => {
            steps.push(Step::Action(Action::DeleteElement(element.clone())))
        }
        K::FocusElement { element } => steps.push(Step::Action(Action::SetFocus(element.clone()))),
        K::ClearFocus => steps.push(Step::Action(Action::SetFocus(Value::text("")))),
        K::PauseGame => steps.push(Step::Action(Action::SetPaused(true))),
        K::ResumeGame => steps.push(Step::Action(Action::SetPaused(false))),
        K::SaveVariable { name } => steps.push(Step::Action(Action::SaveVariable {
            name: name.clone(),
            clear: false,
        })),
        K::ClearSavedVariable { name } => steps.push(Step::Action(Action::SaveVariable {
            name: name.clone(),
            clear: true,
        })),
        K::SetVariable { name, value } => steps.push(Step::Action(Action::SetVariable {
            name: name.clone(),
            value: value.clone(),
        })),
        K::ChangeVariable { name, value } => steps.push(Step::Action(Action::ChangeVariable {
            name: name.clone(),
            value: value.clone(),
        })),
        K::AddToList { value, name } => steps.push(Step::Action(Action::AddToList {
            value: value.clone(),
            name: name.clone(),
        })),
        K::DeleteOfList { index, name } => steps.push(Step::Action(Action::DeleteOfList {
            index: index.clone(),
            name: name.clone(),
        })),
        K::DeleteAllOfList { name } => {
            steps.push(Step::Action(Action::DeleteAllOfList { name: name.clone() }))
        }
        K::ShiftList { name, amount } => steps.push(Step::Action(Action::ShiftList {
            name: name.clone(),
            amount: amount.clone(),
        })),
        K::InsertIntoList { value, index, name } => {
            steps.push(Step::Action(Action::InsertIntoList {
                value: value.clone(),
                index: index.clone(),
                name: name.clone(),
            }))
        }
        K::ReplaceItemOfList { index, name, value } => {
            steps.push(Step::Action(Action::ReplaceItemOfList {
                index: index.clone(),
                name: name.clone(),
                value: value.clone(),
            }))
        }
        K::ReverseList { name } => {
            steps.push(Step::Action(Action::ReverseList { name: name.clone() }))
        }
        K::SetDictValue { key, name, value } => steps.push(Step::Action(Action::SetDictValue {
            key: key.clone(),
            name: name.clone(),
            value: value.clone(),
        })),
        K::DeleteDictKey { key, name } => steps.push(Step::Action(Action::DeleteDictKey {
            key: key.clone(),
            name: name.clone(),
        })),
        K::DeleteAllOfDict { name } => {
            steps.push(Step::Action(Action::DeleteAllOfDict { name: name.clone() }))
        }
        K::LoadJsonIntoDict { json, name } => steps.push(Step::Action(Action::LoadJsonIntoDict {
            json: json.clone(),
            name: name.clone(),
        })),
        K::LoadJsonIntoList { json, name } => steps.push(Step::Action(Action::LoadJsonIntoList {
            json: json.clone(),
            name: name.clone(),
        })),
        K::Wait { duration } => steps.push(Step::Wait(duration.clone())),
        K::WaitUntil { condition } => steps.push(Step::WaitUntil(condition.clone())),
        K::CallBlock { block_id, args } => steps.push(Step::Call {
            block_id: block_id.clone(),
            args: args.clone(),
        }),
        K::Return { value } => steps.push(Step::Return(value.clone())),
        K::StopAll => steps.push(Step::StopAll),
        K::EscapeLoop => steps.push(Step::Break),
        K::ContinueLoop => steps.push(Step::Continue),

        K::If { condition, body } => {
            let jump = steps.len();
            steps.push(Step::JumpUnless {
                condition: condition.clone(),
                to: 0,
            });
            emit_body(steps, body);
            let after = steps.len();
            steps[jump] = Step::JumpUnless {
                condition: condition.clone(),
                to: after,
            };
        }
        K::IfElse {
            condition,
            then_body,
            else_body,
        } => {
            let jump = steps.len();
            steps.push(Step::JumpUnless {
                condition: condition.clone(),
                to: 0,
            });
            emit_body(steps, then_body);
            let skip_else = steps.len();
            steps.push(Step::Jump { to: 0 });
            let else_start = steps.len();
            emit_body(steps, else_body);
            let after = steps.len();
            steps[jump] = Step::JumpUnless {
                condition: condition.clone(),
                to: else_start,
            };
            steps[skip_else] = Step::Jump { to: after };
        }
        K::Repeat { count, body } => emit_loop(steps, LoopKind::Repeat(count.clone()), body),
        K::Forever { body } => emit_loop(steps, LoopKind::Forever, body),
        K::While { condition, body } => emit_loop(steps, LoopKind::While(condition.clone()), body),
    }
}

/// The kinds whose row is `id`, one caption and the shared placement.
/// A slider is spelled out in full above instead, since its row asks the
/// world for three more things.
#[allow(clippy::too_many_arguments)]
fn show(
    kind: UiKind,
    id: &Value,
    content: &Value,
    value: Option<Value>,
    anchor: UiAnchor,
    x: &Value,
    y: &Value,
    width: &Value,
    height: &Value,
    parent: &Value,
    flag: bool,
) -> Step {
    Step::Action(Action::ShowElement(Box::new(ShowElement {
        kind,
        id: id.clone(),
        content: content.clone(),
        range: None,
        value,
        anchor,
        offset: [x.clone(), y.clone()],
        size: [width.clone(), height.clone()],
        parent: parent.clone(),
        flag,
    })))
}

fn emit_loop(steps: &mut Vec<Step>, kind: LoopKind, body: &[Instruction]) {
    let begin = steps.len();
    steps.push(Step::LoopBegin {
        kind: kind.clone(),
        end: 0,
    });
    emit_body(steps, body);
    let end = steps.len();
    steps.push(Step::LoopEnd { begin });
    steps[begin] = Step::LoopBegin { kind, end };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::Strand;

    fn graph_with(instructions: Vec<Instruction>) -> ActorGraph {
        let mut graph = ActorGraph::new();
        graph
            .strands
            .push(Strand::with_instructions(0, 0, instructions));
        graph
    }

    #[test]
    fn a_headerless_strand_compiles_to_nothing() {
        let program = compile(&graph_with(vec![Instruction::new(InstructionKind::Move {
            steps: Value::number(1.0),
        })]));
        assert!(program.entries.is_empty());
        assert!(program.steps.is_empty());
    }

    #[test]
    fn an_if_jumps_past_its_body() {
        let program = compile(&graph_with(vec![
            Instruction::new(InstructionKind::WhenStarted),
            Instruction::new(InstructionKind::If {
                condition: Value::Bool,
                body: vec![Instruction::new(InstructionKind::Move {
                    steps: Value::number(1.0),
                })],
            }),
            Instruction::new(InstructionKind::StopAll),
        ]));
        assert_eq!(program.entries[0].trigger, Trigger::Started);
        assert_eq!(program.entries[0].pc, 0);
        match &program.steps[0] {
            // Past the one body step, onto the StopAll.
            Step::JumpUnless { to, .. } => assert_eq!(*to, 2),
            other => panic!("expected a JumpUnless, got {other:?}"),
        }
        assert_eq!(program.steps[3], Step::End);
    }

    #[test]
    fn a_loops_back_edge_points_at_its_head_and_its_head_past_its_tail() {
        let program = compile(&graph_with(vec![
            Instruction::new(InstructionKind::WhenStarted),
            Instruction::new(InstructionKind::Forever {
                body: vec![Instruction::new(InstructionKind::Move {
                    steps: Value::number(1.0),
                })],
            }),
        ]));
        assert_eq!(
            program.steps[0],
            Step::LoopBegin {
                kind: LoopKind::Forever,
                end: 2
            }
        );
        assert_eq!(program.steps[2], Step::LoopEnd { begin: 0 });
    }

    #[test]
    fn a_custom_blocks_body_is_registered_by_id_not_as_an_entry() {
        let program = compile(&graph_with(vec![
            Instruction::new(InstructionKind::BlockHeader {
                block_id: "b1".to_string(),
            }),
            Instruction::new(InstructionKind::Return {
                value: Value::number(3.0),
            }),
        ]));
        assert!(program.entries.is_empty());
        assert_eq!(program.blocks.get("b1"), Some(&0));
    }
}
