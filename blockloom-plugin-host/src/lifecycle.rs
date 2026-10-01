//! The order things happen in, and refusing stale replies.
//!
//! Discover, resolve, validate, register, open project, create world, start
//! run, stop run, close world, close project, unregister. Every callback and
//! job carries a [`Ticket`] stamped with the project, world and plugin
//! generation it began under; a reply whose ticket is no longer current is
//! dropped, so a late result can never revive a closed world.

use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Phase {
    Unregistered,
    Registered,
    ProjectOpen,
    WorldCreated,
    Running,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ticket {
    pub plugin: String,
    project: u64,
    world: u64,
    plugin_generation: u64,
}

#[derive(Debug)]
pub struct Lifecycle {
    phase: Phase,
    project: u64,
    world: u64,
    plugins: BTreeMap<String, u64>,
}

impl Default for Lifecycle {
    fn default() -> Self {
        Self {
            phase: Phase::Unregistered,
            project: 0,
            world: 0,
            plugins: BTreeMap::new(),
        }
    }
}

impl Lifecycle {
    pub fn phase(&self) -> Phase {
        self.phase
    }

    fn step(&mut self, from: Phase, to: Phase) -> Result<(), String> {
        if self.phase != from {
            return Err(format!(
                "cannot go to {to:?} from {:?} (expected {from:?})",
                self.phase
            ));
        }
        self.phase = to;
        Ok(())
    }

    /// Registers the plugins that passed validation.
    pub fn register(&mut self, ids: impl IntoIterator<Item = String>) -> Result<(), String> {
        self.step(Phase::Unregistered, Phase::Registered)?;
        for id in ids {
            *self.plugins.entry(id).or_insert(0) += 1;
        }
        Ok(())
    }

    pub fn open_project(&mut self) -> Result<(), String> {
        self.step(Phase::Registered, Phase::ProjectOpen)?;
        self.project += 1;
        Ok(())
    }

    pub fn create_world(&mut self) -> Result<(), String> {
        self.step(Phase::ProjectOpen, Phase::WorldCreated)?;
        self.world += 1;
        Ok(())
    }

    pub fn start_run(&mut self) -> Result<(), String> {
        self.step(Phase::WorldCreated, Phase::Running)
    }

    pub fn stop_run(&mut self) -> Result<(), String> {
        self.step(Phase::Running, Phase::WorldCreated)
    }

    /// Ends the world, and a run in it. Everything ticketed to this world
    /// goes stale at once.
    pub fn close_world(&mut self) -> Result<(), String> {
        if self.phase < Phase::WorldCreated {
            return Err(format!("no world to close ({:?})", self.phase));
        }
        self.phase = Phase::ProjectOpen;
        self.world += 1;
        Ok(())
    }

    pub fn close_project(&mut self) -> Result<(), String> {
        if self.phase < Phase::ProjectOpen {
            return Err(format!("no project to close ({:?})", self.phase));
        }
        self.phase = Phase::Registered;
        self.world += 1;
        self.project += 1;
        Ok(())
    }

    pub fn unregister(&mut self) -> Result<(), String> {
        self.step(Phase::Registered, Phase::Unregistered)?;
        for generation in self.plugins.values_mut() {
            *generation += 1;
        }
        Ok(())
    }

    /// The ticket a job for `plugin` carries from now on.
    pub fn ticket(&self, plugin: &str) -> Option<Ticket> {
        Some(Ticket {
            plugin: plugin.to_string(),
            project: self.project,
            world: self.world,
            plugin_generation: *self.plugins.get(plugin)?,
        })
    }

    pub fn is_current(&self, ticket: &Ticket) -> bool {
        self.phase != Phase::Unregistered
            && ticket.project == self.project
            && ticket.world == self.world
            && self.plugins.get(&ticket.plugin) == Some(&ticket.plugin_generation)
    }

    /// Restarts one plugin (a reload) without touching the others.
    pub fn restart_plugin(&mut self, plugin: &str) {
        if let Some(generation) = self.plugins.get_mut(plugin) {
            *generation += 1;
        }
    }
}

/// Jobs a plugin has in flight, so closing a world can cancel exactly them.
#[derive(Debug, Default)]
pub struct Jobs {
    next: u64,
    live: BTreeMap<u64, Ticket>,
}

impl Jobs {
    pub fn start(&mut self, ticket: Ticket) -> u64 {
        self.next += 1;
        self.live.insert(self.next, ticket);
        self.next
    }

    /// Completes a job. `Err` means its ticket is stale and the result must
    /// be discarded.
    pub fn finish(&mut self, lifecycle: &Lifecycle, id: u64) -> Result<(), String> {
        let ticket = self
            .live
            .remove(&id)
            .ok_or_else(|| format!("job {id} is not running"))?;
        if lifecycle.is_current(&ticket) {
            Ok(())
        } else {
            Err(format!(
                "job {id} of {} belongs to a closed world",
                ticket.plugin
            ))
        }
    }

    /// Drops every job the lifecycle no longer honours; returns them so the
    /// plugin can be told to cancel.
    pub fn cancel_stale(&mut self, lifecycle: &Lifecycle) -> Vec<u64> {
        let stale: Vec<u64> = self
            .live
            .iter()
            .filter(|(_, t)| !lifecycle.is_current(t))
            .map(|(id, _)| *id)
            .collect();
        for id in &stale {
            self.live.remove(id);
        }
        stale
    }

    pub fn len(&self) -> usize {
        self.live.len()
    }

    pub fn is_empty(&self) -> bool {
        self.live.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn running() -> Lifecycle {
        let mut l = Lifecycle::default();
        l.register(["a.b".to_string(), "c.d".to_string()]).unwrap();
        l.open_project().unwrap();
        l.create_world().unwrap();
        l.start_run().unwrap();
        l
    }

    #[test]
    fn phases_must_go_in_order() {
        let mut l = Lifecycle::default();
        assert!(l.open_project().is_err());
        l.register(["a.b".to_string()]).unwrap();
        assert!(l.start_run().is_err());
        assert!(l.register([]).is_err());
        l.open_project().unwrap();
        l.create_world().unwrap();
        l.start_run().unwrap();
        l.stop_run().unwrap();
        l.close_world().unwrap();
        l.close_project().unwrap();
        l.unregister().unwrap();
        assert_eq!(l.phase(), Phase::Unregistered);
    }

    #[test]
    fn closing_a_world_makes_its_tickets_stale() {
        let mut l = running();
        let ticket = l.ticket("a.b").unwrap();
        assert!(l.is_current(&ticket));
        l.close_world().unwrap();
        assert!(!l.is_current(&ticket));
        l.create_world().unwrap();
        assert!(!l.is_current(&ticket), "a new world is not the old one");
        assert!(l.is_current(&l.ticket("a.b").unwrap()));
    }

    #[test]
    fn restarting_one_plugin_leaves_the_others_current() {
        let mut l = running();
        let a = l.ticket("a.b").unwrap();
        let c = l.ticket("c.d").unwrap();
        l.restart_plugin("a.b");
        assert!(!l.is_current(&a));
        assert!(l.is_current(&c));
        assert!(l.ticket("not.registered").is_none());
    }

    #[test]
    fn late_job_results_are_discarded_and_stale_jobs_cancelled() {
        let mut l = running();
        let mut jobs = Jobs::default();
        let first = jobs.start(l.ticket("a.b").unwrap());
        let second = jobs.start(l.ticket("c.d").unwrap());
        jobs.finish(&l, first).unwrap();
        assert!(jobs.finish(&l, first).is_err(), "finishing twice");
        l.close_world().unwrap();
        assert_eq!(jobs.cancel_stale(&l), vec![second]);
        assert!(jobs.is_empty());
        let third = {
            l.create_world().unwrap();
            jobs.start(l.ticket("a.b").unwrap())
        };
        l.close_project().unwrap();
        assert!(jobs.finish(&l, third).is_err());
    }
}
