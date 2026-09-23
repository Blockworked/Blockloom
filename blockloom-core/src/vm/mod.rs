//! The block VM: it compiles an actor's canvas into a flat program and runs
//! every script cooperatively, one slice per fixed step.
//!
//! Nothing here touches a renderer. Running a script produces [`Effect`]s -
//! "move this actor forward", "push that one" - which the host applies to
//! whatever it draws with. `blockloom-runtime` is that host; the tests in
//! this module are another.
//!
//! Scripts yield the way Scratch's do: at a `wait`, and once per loop
//! iteration. A `forever` loop therefore advances one step per fixed tick
//! instead of hanging the process, and that is the only scheduling rule to
//! know.

mod effect;
mod exec;
mod program;

pub use effect::Effect;
pub use exec::{
    ActorDicts, ActorLists, ActorVariables, DictValues, Dicts, Event, ListValues, Lists,
    MAX_REPORTER_DEPTH, STEP_BUDGET, VariableSnapshot, VariableValues, Variables, Vm,
};
pub use program::{Action, Entry, LoopKind, Program, ShowElement, Step, Trigger, compile};
