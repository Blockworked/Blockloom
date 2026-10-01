//! Ordering the runtime hooks of every active plugin.
//!
//! Stages run in a fixed order. Inside one stage, `before`/`after`
//! constraints are resolved into one order, ties broken by plugin id then
//! name so two machines agree. A cycle is refused rather than guessed at.

use blockloom_plugin_api::schema::{HookSchema, Stage};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct HookRef {
    pub plugin: String,
    pub name: String,
    pub stage: Stage,
}

impl HookRef {
    pub fn qualified(&self) -> String {
        format!("{}/{}", self.plugin, self.name)
    }
}

/// The order hooks run in, stage by stage.
pub fn order_hooks<'a>(
    hooks: impl IntoIterator<Item = (&'a str, &'a HookSchema)>,
) -> Result<Vec<HookRef>, String> {
    let all: Vec<(&str, &HookSchema)> = hooks.into_iter().collect();
    let by_name: BTreeMap<String, &HookSchema> = all
        .iter()
        .map(|(plugin, hook)| (format!("{plugin}/{}", hook.name), *hook))
        .collect();
    let mut result = Vec::new();
    for stage in Stage::ALL {
        let members: Vec<HookRef> = all
            .iter()
            .filter(|(_, h)| h.stage == stage)
            .map(|(plugin, h)| HookRef {
                plugin: plugin.to_string(),
                name: h.name.clone(),
                stage,
            })
            .collect();
        // edges: a must run before b
        let mut after: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut indegree: BTreeMap<String, usize> = BTreeMap::new();
        for m in &members {
            indegree.entry(m.qualified()).or_insert(0);
        }
        for m in &members {
            let me = m.qualified();
            let hook = by_name[&me];
            let resolve = |other: &str| -> String {
                if other.contains('/') {
                    other.to_string()
                } else {
                    format!("{}/{other}", m.plugin)
                }
            };
            let mut edges = Vec::new();
            for other in &hook.after {
                edges.push((resolve(other), me.clone()));
            }
            for other in &hook.before {
                edges.push((me.clone(), resolve(other)));
            }
            for (first, second) in edges {
                let (Some(a), Some(b)) = (by_name.get(&first), by_name.get(&second)) else {
                    continue; // ordered against a plugin that is not active
                };
                if a.stage != b.stage {
                    return Err(format!(
                        "{first} ({:?}) and {second} ({:?}) are in different stages; stages run in a fixed order",
                        a.stage, b.stage
                    ));
                }
                if after.entry(first).or_default().insert(second.clone()) {
                    *indegree.entry(second).or_insert(0) += 1;
                }
            }
        }
        let mut ready: BTreeSet<&HookRef> = members
            .iter()
            .filter(|m| indegree[&m.qualified()] == 0)
            .collect();
        let mut emitted = 0;
        while let Some(next) = ready.iter().next().cloned() {
            ready.remove(next);
            emitted += 1;
            result.push(next.clone());
            for follower in after.get(&next.qualified()).into_iter().flatten() {
                let count = indegree.get_mut(follower).expect("known hook");
                *count -= 1;
                if *count == 0
                    && let Some(m) = members.iter().find(|m| &m.qualified() == follower)
                {
                    ready.insert(m);
                }
            }
        }
        if emitted != members.len() {
            let stuck: Vec<String> = members
                .iter()
                .map(HookRef::qualified)
                .filter(|q| indegree[q] > 0)
                .collect();
            return Err(format!(
                "hook ordering cycle in stage {stage:?} among {}",
                stuck.join(", ")
            ));
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hook(name: &str, stage: Stage, before: &[&str], after: &[&str]) -> HookSchema {
        HookSchema {
            name: name.to_string(),
            stage,
            before: before.iter().map(|s| s.to_string()).collect(),
            after: after.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn names(order: &[HookRef]) -> Vec<String> {
        order.iter().map(HookRef::qualified).collect()
    }

    #[test]
    fn stages_run_in_their_fixed_order_and_ties_are_deterministic() {
        let a = hook("late", Stage::Presentation, &[], &[]);
        let b = hook("early", Stage::Input, &[], &[]);
        let c = hook("sim", Stage::FixedSimulation, &[], &[]);
        let d = hook("sim", Stage::FixedSimulation, &[], &[]);
        let order = order_hooks([("p.b", &a), ("p.a", &b), ("p.z", &c), ("p.y", &d)]).unwrap();
        assert_eq!(
            names(&order),
            ["p.a/early", "p.y/sim", "p.z/sim", "p.b/late"]
        );
    }

    #[test]
    fn constraints_reorder_within_a_stage_across_plugins() {
        let a = hook("a", Stage::PostPhysics, &[], &["q.b/b"]);
        let b = hook("b", Stage::PostPhysics, &[], &[]);
        let order = order_hooks([("p.a", &a), ("q.b", &b)]).unwrap();
        assert_eq!(names(&order), ["q.b/b", "p.a/a"]);
        let before = hook("a", Stage::PostPhysics, &["q.b/b"], &[]);
        let order = order_hooks([("p.a", &before), ("q.b", &b)]).unwrap();
        assert_eq!(names(&order), ["p.a/a", "q.b/b"]);
    }

    #[test]
    fn cycles_and_cross_stage_constraints_are_refused() {
        let a = hook("a", Stage::Input, &[], &["b"]);
        let b = hook("b", Stage::Input, &[], &["a"]);
        assert!(
            order_hooks([("p.x", &a), ("p.x", &b)])
                .unwrap_err()
                .contains("cycle")
        );
        let c = hook("c", Stage::Input, &["p.x/d"], &[]);
        let d = hook("d", Stage::Presentation, &[], &[]);
        assert!(
            order_hooks([("p.x", &c), ("p.x", &d)])
                .unwrap_err()
                .contains("different stages")
        );
    }

    #[test]
    fn ordering_against_an_inactive_plugin_is_ignored() {
        let a = hook("a", Stage::Input, &["gone.plugin/x"], &[]);
        assert_eq!(order_hooks([("p.x", &a)]).unwrap().len(), 1);
    }
}
