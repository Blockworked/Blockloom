//! The live variables of one run, kept in slots. Names are interned once into
//! [`VarId`]s and each actor's variables sit in a [`ScopeId`]'s slot vector,
//! so a VM read with both ids in hand hashes nothing. The by-name API is what
//! scripts, compiled logic and the host use, and follows the same rules.

use crate::project::Project;
use crate::value::Evaluated;
use rustc_hash::FxHashMap;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// One scope's variables, by name.
pub type VariableValues = HashMap<String, Evaluated>;
/// Every actor's own variables, by actor id.
pub type ActorVariables = HashMap<String, VariableValues>;

/// A snapshot of every variable in play.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VariableSnapshot {
    /// The project's shared variables.
    pub globals: VariableValues,
    /// Each actor's own, by actor id.
    pub actors: ActorVariables,
}

/// An interned variable name. Ids are never reused or forgotten, even across
/// loads, so one held by compiled code stays good.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct VarId(u32);

/// One actor's variables. Valid until that actor is forgotten or the store is
/// loaded again; a forgotten scope's id is never handed out again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ScopeId(u32);

type Slots = Vec<Option<Evaluated>>;

#[derive(Debug, Default)]
struct VariableState {
    ids: FxHashMap<String, VarId>,
    names: Vec<String>,
    globals: Slots,
    scope_ids: FxHashMap<String, ScopeId>,
    /// Actor id and slots per scope; `None` once the actor is forgotten.
    scopes: Vec<Option<(String, Slots)>>,
}

impl VariableState {
    fn intern(&mut self, name: &str) -> VarId {
        if let Some(&id) = self.ids.get(name) {
            return id;
        }
        let id = VarId(self.names.len() as u32);
        self.names.push(name.to_string());
        self.ids.insert(name.to_string(), id);
        id
    }

    fn slots(&self, scope: ScopeId) -> Option<&Slots> {
        self.scopes[scope.0 as usize]
            .as_ref()
            .map(|(_, slots)| slots)
    }

    fn slots_mut(&mut self, scope: ScopeId) -> Option<&mut Slots> {
        self.scopes[scope.0 as usize]
            .as_mut()
            .map(|(_, slots)| slots)
    }

    /// `actor`'s scope, made empty if it has none.
    fn scope_or_new(&mut self, actor: &str) -> ScopeId {
        if let Some(&scope) = self.scope_ids.get(actor) {
            return scope;
        }
        let scope = ScopeId(self.scopes.len() as u32);
        self.scopes.push(Some((actor.to_string(), Vec::new())));
        self.scope_ids.insert(actor.to_string(), scope);
        scope
    }

    fn values(&mut self, values: HashMap<String, Evaluated>) -> Slots {
        let mut slots = Slots::new();
        for (name, value) in values {
            set(&mut slots, self.intern(&name), value);
        }
        slots
    }

    fn read(&self, scope: Option<ScopeId>, var: VarId) -> Evaluated {
        scope
            .and_then(|scope| get(self.slots(scope)?, var))
            .or_else(|| get(&self.globals, var))
            .cloned()
            .unwrap_or(Evaluated::Number(0.0))
    }

    /// The actor's slot, else the global, else a new actor slot.
    fn write(&mut self, actor: &str, scope: Option<ScopeId>, var: VarId, value: Evaluated) {
        if let Some(slot) = scope
            .and_then(|scope| self.slots_mut(scope))
            .and_then(|slots| get_mut(slots, var))
        {
            *slot = value;
            return;
        }
        if let Some(slot) = get_mut(&mut self.globals, var) {
            *slot = value;
            return;
        }
        let scope = match scope {
            Some(scope) if self.slots(scope).is_some() => scope,
            _ => self.scope_or_new(actor),
        };
        if let Some(slots) = self.slots_mut(scope) {
            set(slots, var, value);
        }
    }
}

fn get(slots: &Slots, var: VarId) -> Option<&Evaluated> {
    slots.get(var.0 as usize)?.as_ref()
}

fn get_mut(slots: &mut Slots, var: VarId) -> Option<&mut Evaluated> {
    slots.get_mut(var.0 as usize)?.as_mut()
}

fn set(slots: &mut Slots, var: VarId, value: Evaluated) {
    let index = var.0 as usize;
    if slots.len() <= index {
        slots.resize(index + 1, None);
    }
    slots[index] = Some(value);
}

/// The live variables of one run. The VM and compiled logic hold clones of
/// this handle, so either scheduler reads and writes the same slots.
#[derive(Debug, Clone, Default)]
pub struct Variables(Rc<RefCell<VariableState>>);

impl Variables {
    pub fn load(&self, project: &Project) {
        let mut state = self.0.borrow_mut();
        state.scope_ids.clear();
        state.scopes.clear();
        let globals = project
            .globals
            .iter()
            .map(|variable| (variable.name.clone(), variable.value.clone()))
            .collect();
        state.globals = state.values(globals);
        for actor in &project.actors {
            let slots = state.values(actor.graph.variable_values());
            let scope = state.scope_or_new(&actor.id);
            if let Some(own) = state.slots_mut(scope) {
                *own = slots;
            }
        }
    }

    pub fn read(&self, actor: &str, name: &str) -> Evaluated {
        let state = self.0.borrow();
        let Some(&var) = state.ids.get(name) else {
            return Evaluated::Number(0.0);
        };
        state.read(state.scope_ids.get(actor).copied(), var)
    }

    /// Writes an actor slot first, then a global, and otherwise declares an
    /// actor slot. This is the variable rule blocks have always used.
    pub fn write(&self, actor: &str, name: &str, value: Evaluated) {
        let mut state = self.0.borrow_mut();
        let var = state.intern(name);
        let scope = state.scope_ids.get(actor).copied();
        state.write(actor, scope, var, value);
    }

    /// Gives `to` its own copy of `from`'s variables, as they stand. A clone
    /// starts life with whatever its template had counted up to, and changes
    /// either way after that.
    pub fn copy_actor(&self, from: &str, to: &str) {
        let mut state = self.0.borrow_mut();
        let copied = state
            .scope_ids
            .get(from)
            .and_then(|&scope| state.slots(scope))
            .cloned()
            .unwrap_or_default();
        let scope = state.scope_or_new(to);
        if let Some(slots) = state.slots_mut(scope) {
            *slots = copied;
        }
    }

    /// Forgets an actor's own variables. A deleted actor is gone for the rest
    /// of the run, and so is what it was remembering.
    pub fn forget_actor(&self, actor: &str) {
        let mut state = self.0.borrow_mut();
        if let Some(scope) = state.scope_ids.remove(actor) {
            state.scopes[scope.0 as usize] = None;
        }
    }

    pub fn snapshot(&self) -> VariableSnapshot {
        let state = self.0.borrow();
        let named = |slots: &Slots| {
            slots
                .iter()
                .enumerate()
                .filter_map(|(index, value)| {
                    Some((state.names[index].clone(), value.as_ref()?.clone()))
                })
                .collect()
        };
        VariableSnapshot {
            globals: named(&state.globals),
            actors: state
                .scopes
                .iter()
                .flatten()
                .map(|(actor, slots)| (actor.clone(), named(slots)))
                .collect(),
        }
    }

    // ─── By id, for the VM ──────────────────────────────────────────────────

    pub(crate) fn intern(&self, name: &str) -> VarId {
        self.0.borrow_mut().intern(name)
    }

    pub(crate) fn scope(&self, actor: &str) -> Option<ScopeId> {
        self.0.borrow().scope_ids.get(actor).copied()
    }

    pub(crate) fn read_slot(&self, scope: Option<ScopeId>, var: VarId) -> Evaluated {
        self.0.borrow().read(scope, var)
    }

    /// [`Variables::write`] by id. `scope` is what the caller last resolved
    /// for `actor`, and the answer is its scope after the write, which a
    /// declaration may just have made.
    pub(crate) fn write_slot(
        &self,
        actor: &str,
        scope: Option<ScopeId>,
        var: VarId,
        value: Evaluated,
    ) -> Option<ScopeId> {
        let mut state = self.0.borrow_mut();
        state.write(actor, scope, var, value);
        match scope {
            Some(scope) if state.slots(scope).is_some() => Some(scope),
            _ => state.scope_ids.get(actor).copied(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(value: f64) -> Evaluated {
        Evaluated::Number(value)
    }

    #[test]
    fn an_actor_slot_shadows_a_global_and_a_new_name_lands_on_the_actor() {
        let variables = Variables::default();
        {
            let mut state = variables.0.borrow_mut();
            let var = state.intern("score");
            set(&mut state.globals, var, n(1.0));
        }
        variables.copy_actor("nobody", "a");
        variables.write("a", "score", n(2.0));
        assert_eq!(variables.read("b", "score"), n(2.0));
        variables.write("a", "lives", n(3.0));
        assert_eq!(variables.read("a", "lives"), n(3.0));
        assert_eq!(variables.read("b", "lives"), n(0.0));
        let snapshot = variables.snapshot();
        assert_eq!(snapshot.globals.get("score"), Some(&n(2.0)));
        assert_eq!(snapshot.actors["a"].get("lives"), Some(&n(3.0)));
        assert!(!snapshot.actors["a"].contains_key("score"));
    }

    #[test]
    fn a_forgotten_scope_is_never_handed_out_again() {
        let variables = Variables::default();
        variables.write("a", "x", n(1.0));
        let old = variables.scope("a").unwrap();
        variables.forget_actor("a");
        let var = variables.intern("x");
        assert_eq!(variables.read_slot(Some(old), var), n(0.0));
        let fresh = variables.write_slot("a", Some(old), var, n(5.0));
        assert_ne!(fresh, Some(old));
        assert_eq!(variables.read("a", "x"), n(5.0));
        assert_eq!(variables.read_slot(Some(old), var), n(0.0));
    }

    #[test]
    fn a_copy_is_its_own_from_then_on() {
        let variables = Variables::default();
        variables.write("a", "hits", n(4.0));
        variables.copy_actor("a", "~1");
        variables.write("~1", "hits", n(5.0));
        assert_eq!(variables.read("a", "hits"), n(4.0));
        assert_eq!(variables.read("~1", "hits"), n(5.0));
    }
}
