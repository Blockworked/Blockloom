//! Work a plugin spreads over many ticks.
//!
//! A module starts a job with `jobs.start {name, args, event?, priority?}` and
//! gets an id. The world then calls the module's op `job.<name>` once per slice
//! ([`crate::world::WorldPlugins::run_jobs`]) with `{job, name, args, state,
//! slice, budget_ms}`; the answer is `{"done": bool, "progress": 0..1,
//! "state": <kept for the next slice>, "result": <when done>}` and may carry
//! `effects` like any other op. A plugin asks `jobs.status`, `jobs.list`,
//! `jobs.take` (a finished job's result, once) or `jobs.cancel`; a job that
//! names an `event` raises it with `[id, status]` when it ends, which is how a
//! hat waits for it. A job belongs to the world that started it: dropping the
//! world drops its jobs, and a reload cancels the reloaded plugin's.

use serde_json::{Value, json};
use std::collections::BTreeMap;

/// Jobs one plugin may have running or waiting to be taken.
pub const MAX_LIVE_PER_PLUGIN: usize = 256;
/// Finished jobs kept for a reader, per plugin; the oldest go first.
pub const MAX_FINISHED_PER_PLUGIN: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum JobStatus {
    Running,
    Done,
    Failed,
    Cancelled,
}

impl JobStatus {
    pub fn name(self) -> &'static str {
        match self {
            JobStatus::Running => "running",
            JobStatus::Done => "done",
            JobStatus::Failed => "failed",
            JobStatus::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    pub id: u64,
    pub plugin: String,
    pub name: String,
    pub args: Value,
    pub state: Value,
    pub slice: u64,
    pub status: JobStatus,
    pub progress: f64,
    pub result: Option<Value>,
    pub error: Option<String>,
    pub event: Option<String>,
    pub priority: i32,
    /// Set until the module has been told of a cancellation.
    pub notify_cancel: bool,
}

/// A job's end, for the world to raise its event and report a failure.
#[derive(Debug, Clone, PartialEq)]
pub struct Ended {
    pub id: u64,
    pub plugin: String,
    pub status: JobStatus,
    pub event: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Default)]
pub struct JobTable {
    next: u64,
    jobs: BTreeMap<u64, Job>,
    /// The id the last slice ran, so each round starts after it.
    cursor: u64,
    ended: Vec<Ended>,
}

impl JobTable {
    pub fn start(
        &mut self,
        plugin: &str,
        name: &str,
        args: Value,
        event: Option<String>,
        priority: i32,
    ) -> Result<u64, String> {
        if name.is_empty() || name.len() > 80 || name.contains(char::is_whitespace) {
            return Err("a job name is 1 to 80 characters without spaces".to_string());
        }
        let live = self
            .jobs
            .values()
            .filter(|j| j.plugin == plugin && j.status == JobStatus::Running)
            .count();
        if live >= MAX_LIVE_PER_PLUGIN {
            return Err(format!("over {MAX_LIVE_PER_PLUGIN} running jobs"));
        }
        self.next += 1;
        self.jobs.insert(
            self.next,
            Job {
                id: self.next,
                plugin: plugin.to_string(),
                name: name.to_string(),
                args,
                state: Value::Null,
                slice: 0,
                status: JobStatus::Running,
                progress: 0.0,
                result: None,
                error: None,
                event,
                priority,
                notify_cancel: false,
            },
        );
        Ok(self.next)
    }

    pub fn get(&self, plugin: &str, id: u64) -> Option<&Job> {
        self.jobs.get(&id).filter(|j| j.plugin == plugin)
    }

    pub fn cancel(&mut self, plugin: &str, id: u64) -> bool {
        let Some(job) = self.jobs.get_mut(&id).filter(|j| j.plugin == plugin) else {
            return false;
        };
        if job.status == JobStatus::Running {
            job.status = JobStatus::Cancelled;
            job.notify_cancel = true;
            self.ended.push(ended_of(job));
        }
        true
    }

    /// Cancels everything `plugin` is running, as a reload or removal does.
    pub fn cancel_plugin(&mut self, plugin: &str) -> usize {
        let ids: Vec<u64> = self
            .jobs
            .values()
            .filter(|j| j.plugin == plugin && j.status == JobStatus::Running)
            .map(|j| j.id)
            .collect();
        for id in &ids {
            self.cancel(plugin, *id);
        }
        ids.len()
    }

    /// A finished job's result, once; the job is forgotten.
    pub fn take(&mut self, plugin: &str, id: u64) -> Option<Job> {
        match self.jobs.get(&id) {
            Some(j) if j.plugin == plugin && j.status != JobStatus::Running => {
                self.jobs.remove(&id)
            }
            _ => None,
        }
    }

    pub fn list(&self, plugin: &str) -> Vec<&Job> {
        self.jobs.values().filter(|j| j.plugin == plugin).collect()
    }

    pub fn running(&self) -> usize {
        self.jobs
            .values()
            .filter(|j| j.status == JobStatus::Running)
            .count()
    }

    /// The next running job to give a slice, highest priority first and then
    /// round-robin by id, skipping `skip` (jobs already sliced this round).
    pub fn next_runnable(&mut self, skip: &[u64]) -> Option<Job> {
        let best = self
            .jobs
            .values()
            .filter(|j| j.status == JobStatus::Running && !skip.contains(&j.id))
            .max_by_key(|j| (j.priority, std::cmp::Reverse(wrap(j.id, self.cursor))))?;
        self.cursor = best.id;
        Some(best.clone())
    }

    /// Cancelled jobs whose module has not been told yet.
    pub fn take_cancel_notices(&mut self) -> Vec<Job> {
        self.jobs
            .values_mut()
            .filter(|j| j.notify_cancel)
            .map(|j| {
                j.notify_cancel = false;
                j.clone()
            })
            .collect()
    }

    /// Records a slice's answer.
    pub fn answered(&mut self, id: u64, answer: &Value) {
        let Some(job) = self.jobs.get_mut(&id) else {
            return;
        };
        // A job cancelled during its own slice stays cancelled.
        if job.status != JobStatus::Running {
            return;
        }
        job.slice += 1;
        if let Some(p) = answer.get("progress").and_then(Value::as_f64) {
            job.progress = p.clamp(0.0, 1.0);
        }
        if let Some(state) = answer.get("state") {
            job.state = state.clone();
        }
        if let Some(error) = answer.get("error").and_then(Value::as_str) {
            job.status = JobStatus::Failed;
            job.error = Some(error.to_string());
        } else if answer.get("done").and_then(Value::as_bool) == Some(true) {
            job.status = JobStatus::Done;
            job.progress = 1.0;
            job.result = answer.get("result").cloned();
        }
        if job.status != JobStatus::Running {
            self.ended.push(ended_of(job));
        }
    }

    /// A slice that could not run (the module failed or is gone).
    pub fn failed(&mut self, id: u64, error: String) {
        if let Some(job) = self.jobs.get_mut(&id)
            && job.status == JobStatus::Running
        {
            job.status = JobStatus::Failed;
            job.error = Some(error);
            self.ended.push(ended_of(job));
        }
    }

    pub fn drain_ended(&mut self) -> Vec<Ended> {
        std::mem::take(&mut self.ended)
    }

    /// Drops the oldest finished jobs of each plugin past the keep limit.
    pub fn prune(&mut self) {
        let mut finished: BTreeMap<String, Vec<u64>> = BTreeMap::new();
        for j in self
            .jobs
            .values()
            .filter(|j| j.status != JobStatus::Running)
        {
            finished.entry(j.plugin.clone()).or_default().push(j.id);
        }
        for ids in finished.values() {
            for id in ids
                .iter()
                .take(ids.len().saturating_sub(MAX_FINISHED_PER_PLUGIN))
            {
                self.jobs.remove(id);
            }
        }
    }
}

/// Orders ids so the one right after `cursor` comes first.
fn wrap(id: u64, cursor: u64) -> u64 {
    if id > cursor {
        id - cursor
    } else {
        u64::MAX / 2 + id
    }
}

fn ended_of(job: &Job) -> Ended {
    Ended {
        id: job.id,
        plugin: job.plugin.clone(),
        status: job.status,
        event: job.event.clone(),
        error: job.error.clone(),
    }
}

pub fn job_json(job: &Job) -> Value {
    let mut out = json!({
        "job": job.id,
        "name": job.name,
        "status": job.status.name(),
        "progress": job.progress,
        "slices": job.slice,
    });
    if let Some(result) = &job.result {
        out["result"] = result.clone();
    }
    if let Some(error) = &job.error {
        out["error"] = json!(error);
    }
    out
}

/// The `jobs.*` services over a table.
pub fn call(
    table: &std::sync::Mutex<JobTable>,
    plugin: &str,
    verb: &str,
    input: &Value,
) -> Option<Result<Value, String>> {
    let id = || {
        input
            .get("job")
            .and_then(Value::as_u64)
            .ok_or_else(|| "give a `job` id".to_string())
    };
    let mut table = table.lock().unwrap_or_else(|e| e.into_inner());
    Some(match verb {
        "start" => (|| {
            let name = input
                .get("name")
                .and_then(Value::as_str)
                .ok_or("give a job `name`")?;
            let args = input.get("args").cloned().unwrap_or(Value::Null);
            let event = input
                .get("event")
                .and_then(Value::as_str)
                .map(str::to_string);
            let priority = input.get("priority").and_then(Value::as_i64).unwrap_or(0) as i32;
            let id = table.start(plugin, name, args, event, priority)?;
            Ok(json!({"job": id}))
        })(),
        "status" => id().map(|id| match table.get(plugin, id) {
            Some(job) => job_json(job),
            None => json!({"job": id, "status": "unknown"}),
        }),
        "cancel" => id().map(|id| json!({"found": table.cancel(plugin, id)})),
        "take" => id().map(|id| match table.take(plugin, id) {
            Some(job) => job_json(&job),
            None => json!({"job": id, "status": "unknown"}),
        }),
        "list" => {
            Ok(json!({"jobs": table.list(plugin).into_iter().map(job_json).collect::<Vec<_>>()}))
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jobs_run_in_priority_then_round_robin_order() {
        let mut t = JobTable::default();
        let a = t.start("p", "a", Value::Null, None, 0).unwrap();
        let b = t.start("p", "b", Value::Null, None, 0).unwrap();
        let hi = t.start("p", "hi", Value::Null, None, 5).unwrap();
        assert_eq!(t.next_runnable(&[]).unwrap().id, hi);
        // Skipping the hot one, the rest go in turn, wrapping.
        assert_eq!(t.next_runnable(&[hi]).unwrap().id, a);
        assert_eq!(t.next_runnable(&[hi, a]).unwrap().id, b);
        assert_eq!(t.next_runnable(&[hi, a, b]), None);
        let mut fair = JobTable::default();
        let x = fair.start("p", "x", Value::Null, None, 0).unwrap();
        let y = fair.start("p", "y", Value::Null, None, 0).unwrap();
        let first = fair.next_runnable(&[]).unwrap().id;
        let second = fair.next_runnable(&[]).unwrap().id;
        assert_eq!((first, second), (x, y));
        assert_eq!(fair.next_runnable(&[]).unwrap().id, x);
    }

    #[test]
    fn an_answer_moves_a_job_to_done_with_its_result_and_event() {
        let mut t = JobTable::default();
        let id = t
            .start("p", "gen", json!({"n": 3}), Some("ready".into()), 0)
            .unwrap();
        t.answered(id, &json!({"progress": 0.5, "state": {"i": 2}}));
        let job = t.get("p", id).unwrap();
        assert_eq!(
            (job.status, job.slice, job.progress),
            (JobStatus::Running, 1, 0.5)
        );
        assert_eq!(job.state["i"], 2);
        assert!(t.drain_ended().is_empty());
        t.answered(id, &json!({"done": true, "result": {"cells": 9}}));
        let ended = t.drain_ended();
        assert_eq!(ended.len(), 1);
        assert_eq!(ended[0].event.as_deref(), Some("ready"));
        assert_eq!(t.get("p", id).unwrap().result, Some(json!({"cells": 9})));
        // Another plugin cannot see or take it; the owner takes it once.
        assert!(t.get("q", id).is_none());
        assert!(t.take("q", id).is_none());
        assert_eq!(t.take("p", id).unwrap().status, JobStatus::Done);
        assert!(t.take("p", id).is_none());
    }

    #[test]
    fn cancel_and_failure_end_a_job_once_and_late_answers_are_ignored() {
        let mut t = JobTable::default();
        let a = t.start("p", "a", Value::Null, None, 0).unwrap();
        assert!(t.cancel("p", a));
        t.answered(a, &json!({"done": true, "result": 1}));
        assert_eq!(t.get("p", a).unwrap().status, JobStatus::Cancelled);
        assert_eq!(t.take_cancel_notices().len(), 1);
        assert!(t.take_cancel_notices().is_empty());
        let b = t.start("p", "b", Value::Null, None, 0).unwrap();
        t.answered(b, &json!({"error": "no cells"}));
        assert_eq!(t.get("p", b).unwrap().error.as_deref(), Some("no cells"));
        assert_eq!(t.drain_ended().len(), 2);
        assert!(!t.cancel("p", 99));
        let c = t.start("p", "c", Value::Null, None, 0).unwrap();
        let d = t.start("p", "d", Value::Null, None, 0).unwrap();
        assert_eq!(t.cancel_plugin("p"), 2);
        assert_eq!(t.running(), 0);
        let _ = (c, d);
    }

    #[test]
    fn names_and_counts_are_limited_and_finished_jobs_are_pruned() {
        let mut t = JobTable::default();
        assert!(t.start("p", "has space", Value::Null, None, 0).is_err());
        assert!(t.start("p", "", Value::Null, None, 0).is_err());
        for _ in 0..MAX_LIVE_PER_PLUGIN {
            t.start("p", "x", Value::Null, None, 0).unwrap();
        }
        assert!(t.start("p", "x", Value::Null, None, 0).is_err());
        t.start("q", "x", Value::Null, None, 0).unwrap();
        for id in 1..=MAX_LIVE_PER_PLUGIN as u64 {
            t.answered(id, &json!({"done": true}));
        }
        t.prune();
        assert_eq!(t.list("p").len(), MAX_FINISHED_PER_PLUGIN);
        assert_eq!(t.list("q").len(), 1);
    }

    #[test]
    fn the_service_verbs_cover_start_status_cancel_take_and_list() {
        let table = std::sync::Mutex::new(JobTable::default());
        let ask = |verb: &str, input: Value| call(&table, "p", verb, &input).unwrap();
        let id = ask("start", json!({"name": "gen", "args": [1]})).unwrap()["job"]
            .as_u64()
            .unwrap();
        assert_eq!(
            ask("status", json!({"job": id})).unwrap()["status"],
            "running"
        );
        assert_eq!(
            ask("list", json!({})).unwrap()["jobs"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(ask("start", json!({})).is_err());
        assert!(ask("status", json!({})).is_err());
        assert_eq!(ask("cancel", json!({"job": id})).unwrap()["found"], true);
        assert_eq!(
            ask("take", json!({"job": id})).unwrap()["status"],
            "cancelled"
        );
        assert_eq!(
            ask("status", json!({"job": id})).unwrap()["status"],
            "unknown"
        );
        assert!(call(&table, "p", "nonsense", &json!({})).is_none());
    }
}
