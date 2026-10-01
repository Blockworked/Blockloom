//! Dependency resolution: one version of each plugin per project.
//!
//! The whole graph is resolved before anything is activated or fetched for
//! real. Candidates are tried highest version first, except that a version
//! the lock already holds is preferred so resolving never upgrades behind the
//! author's back. A failure says which requirement could not be met and who
//! asked for it.

use crate::source::Source;
use blockloom_plugin_api::manifest::PluginManifest;
use blockloom_plugin_api::{Version, VersionReq};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// A concrete version that could be chosen.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub manifest: PluginManifest,
    pub source: Source,
    /// The package content hash.
    pub hash: String,
}

pub trait CandidateProvider {
    /// Every version of `id` that could be installed. `source` is the one the
    /// project's `plugins.json` pinned, which is then the only place to look.
    fn candidates(&self, id: &str, source: Option<&Source>) -> Result<Vec<Candidate>, String>;
}

/// A top-level dependency from `plugins.json`.
#[derive(Debug, Clone)]
pub struct Request {
    pub id: String,
    pub req: VersionReq,
    pub source: Option<Source>,
    pub features: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Options {
    pub engine: Version,
    /// Versions to keep when they still satisfy their requirements.
    pub locked: BTreeMap<String, Version>,
    /// Ids allowed to move off their locked version.
    pub upgrade: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ResolveError {
    NotFound {
        id: String,
        by: String,
    },
    NoMatch {
        id: String,
        req: VersionReq,
        by: String,
        available: Vec<Version>,
        engine_rejected: Vec<(Version, VersionReq)>,
    },
    Conflict {
        id: String,
        chosen: Version,
        req: VersionReq,
        by: String,
    },
    Cycle(Vec<String>),
    ConflictingProviders {
        service: String,
        first: String,
        second: String,
    },
    UnknownFeature {
        id: String,
        feature: String,
    },
    Source(String),
}

impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResolveError::NotFound { id, by } => {
                write!(f, "{id} was not found in any source (required by {by})")
            }
            ResolveError::NoMatch {
                id,
                req,
                by,
                available,
                engine_rejected,
            } => {
                write!(f, "no version of {id} satisfies {req} (required by {by})")?;
                if available.is_empty() && engine_rejected.is_empty() {
                    write!(f, "; no versions are available")?;
                } else if !available.is_empty() {
                    let list: Vec<String> = available.iter().map(|v| v.to_string()).collect();
                    write!(f, "; available: {}", list.join(", "))?;
                }
                for (version, engine) in engine_rejected {
                    write!(f, "; {id} {version} needs engine {engine}")?;
                }
                Ok(())
            }
            ResolveError::Conflict {
                id,
                chosen,
                req,
                by,
            } => write!(
                f,
                "{id} {chosen} was selected for another requirement, but {by} needs {req}; only one version of a plugin can be used"
            ),
            ResolveError::Cycle(path) => write!(f, "dependency cycle: {}", path.join(" -> ")),
            ResolveError::ConflictingProviders {
                service,
                first,
                second,
            } => write!(
                f,
                "{first} and {second} both provide \"{service}\"; remove one of them"
            ),
            ResolveError::UnknownFeature { id, feature } => {
                write!(f, "{id} has no feature \"{feature}\"")
            }
            ResolveError::Source(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for ResolveError {}

#[derive(Debug, Clone)]
pub struct Resolution {
    pub packages: BTreeMap<String, Candidate>,
    /// Dependencies before their dependents.
    pub order: Vec<String>,
}

impl Resolution {
    /// The ids `id` directly depends on in this resolution.
    pub fn dependencies_of(&self, id: &str) -> Vec<String> {
        self.packages
            .get(id)
            .map(|c| hard_edges(c, &self.packages))
            .unwrap_or_default()
    }
}

#[derive(Clone)]
struct Need {
    id: String,
    req: VersionReq,
    by: String,
    source: Option<Source>,
    features: Vec<String>,
}

#[derive(Clone, Default)]
struct State {
    chosen: BTreeMap<String, Candidate>,
    features: BTreeMap<String, BTreeSet<String>>,
    /// Optional dependencies, enforced only if something else selects them.
    soft: Vec<Need>,
}

struct Solver<'a> {
    provider: &'a dyn CandidateProvider,
    options: &'a Options,
    memo: BTreeMap<(String, Option<Source>), Vec<Candidate>>,
    best: Option<(usize, ResolveError)>,
}

fn label(manifest: &PluginManifest) -> String {
    format!("{} {}", manifest.id, manifest.version)
}

/// Edges that must be acyclic: required dependencies and enabled features'.
fn hard_edges(candidate: &Candidate, chosen: &BTreeMap<String, Candidate>) -> Vec<String> {
    let mut ids: BTreeSet<String> = candidate
        .manifest
        .dependencies
        .iter()
        .filter(|(_, d)| !d.optional)
        .map(|(id, _)| id.clone())
        .collect();
    for feature in candidate.manifest.features.values() {
        ids.extend(feature.dependencies.keys().cloned());
    }
    ids.into_iter()
        .filter(|id| chosen.contains_key(id))
        .collect()
}

impl Solver<'_> {
    fn note(&mut self, depth: usize, error: ResolveError) {
        if self.best.as_ref().is_none_or(|(d, _)| depth > *d) {
            self.best = Some((depth, error));
        }
    }

    fn candidates(&mut self, need: &Need) -> Result<Vec<Candidate>, ResolveError> {
        let key = (need.id.clone(), need.source.clone());
        if !self.memo.contains_key(&key) {
            let found = self
                .provider
                .candidates(&need.id, need.source.as_ref())
                .map_err(ResolveError::Source)?;
            self.memo.insert(key.clone(), found);
        }
        Ok(self.memo[&key].clone())
    }

    fn solve(&mut self, mut pending: Vec<Need>, mut state: State) -> Option<State> {
        let Some(need) = pending.pop() else {
            return self.finish(state);
        };
        if let Some(existing) = state.chosen.get(&need.id) {
            if !need.req.matches(&existing.manifest.version) {
                let error = ResolveError::Conflict {
                    id: need.id.clone(),
                    chosen: existing.manifest.version.clone(),
                    req: need.req.clone(),
                    by: need.by.clone(),
                };
                self.note(state.chosen.len(), error);
                return None;
            }
            // Already chosen: only a newly asked-for feature adds work.
            let candidate = existing.clone();
            if let Err(e) =
                self.enable_features(&candidate, &need.features, &mut state, &mut pending)
            {
                self.note(state.chosen.len(), e);
                return None;
            }
            return self.solve(pending, state);
        }
        let all = match self.candidates(&need) {
            Ok(all) => all,
            Err(e) => {
                self.note(state.chosen.len(), e);
                return None;
            }
        };
        if all.is_empty() {
            self.note(
                state.chosen.len(),
                ResolveError::NotFound {
                    id: need.id.clone(),
                    by: need.by.clone(),
                },
            );
            return None;
        }
        let mut engine_rejected = Vec::new();
        let mut usable: Vec<Candidate> = Vec::new();
        for c in &all {
            if !need.req.matches(&c.manifest.version) {
                continue;
            }
            if !c.manifest.engine.matches(&self.options.engine) {
                engine_rejected.push((c.manifest.version.clone(), c.manifest.engine.clone()));
                continue;
            }
            usable.push(c.clone());
        }
        let locked = self
            .options
            .locked
            .get(&need.id)
            .filter(|_| !self.options.upgrade.contains(&need.id))
            .cloned();
        usable.sort_by(|a, b| {
            let a_locked = locked.as_ref() == Some(&a.manifest.version);
            let b_locked = locked.as_ref() == Some(&b.manifest.version);
            b_locked
                .cmp(&a_locked)
                .then_with(|| b.manifest.version.cmp(&a.manifest.version))
        });
        if usable.is_empty() {
            let mut available: Vec<Version> =
                all.iter().map(|c| c.manifest.version.clone()).collect();
            available.sort();
            self.note(
                state.chosen.len(),
                ResolveError::NoMatch {
                    id: need.id.clone(),
                    req: need.req.clone(),
                    by: need.by.clone(),
                    available,
                    engine_rejected,
                },
            );
            return None;
        }
        for candidate in usable {
            let mut next = state.clone();
            let mut more = pending.clone();
            next.chosen.insert(need.id.clone(), candidate.clone());
            let by = label(&candidate.manifest);
            for (dep, spec) in &candidate.manifest.dependencies {
                let dep_need = Need {
                    id: dep.clone(),
                    req: spec.version.clone(),
                    by: by.clone(),
                    source: None,
                    features: vec![],
                };
                if spec.optional {
                    next.soft.push(dep_need);
                } else {
                    more.push(dep_need);
                }
            }
            if let Err(e) = self.enable_features(&candidate, &need.features, &mut next, &mut more) {
                self.note(next.chosen.len(), e);
                continue;
            }
            if let Some(done) = self.solve(more, next) {
                return Some(done);
            }
        }
        None
    }

    fn enable_features(
        &mut self,
        candidate: &Candidate,
        wanted: &[String],
        state: &mut State,
        pending: &mut Vec<Need>,
    ) -> Result<(), ResolveError> {
        let id = &candidate.manifest.id;
        for name in wanted {
            let feature = candidate.manifest.features.get(name).ok_or_else(|| {
                ResolveError::UnknownFeature {
                    id: id.clone(),
                    feature: name.clone(),
                }
            })?;
            if !state
                .features
                .entry(id.clone())
                .or_default()
                .insert(name.clone())
            {
                continue;
            }
            for (dep, spec) in &feature.dependencies {
                pending.push(Need {
                    id: dep.clone(),
                    req: spec.version.clone(),
                    by: format!("{} (feature {name})", label(&candidate.manifest)),
                    source: None,
                    features: vec![],
                });
            }
        }
        Ok(())
    }

    fn finish(&mut self, state: State) -> Option<State> {
        let depth = state.chosen.len();
        for soft in &state.soft {
            if let Some(chosen) = state.chosen.get(&soft.id)
                && !soft.req.matches(&chosen.manifest.version)
            {
                self.note(
                    depth,
                    ResolveError::Conflict {
                        id: soft.id.clone(),
                        chosen: chosen.manifest.version.clone(),
                        req: soft.req.clone(),
                        by: format!("{} (optional)", soft.by),
                    },
                );
                return None;
            }
        }
        let mut providers: BTreeMap<&str, &str> = BTreeMap::new();
        for (id, candidate) in &state.chosen {
            for service in &candidate.manifest.provides {
                if let Some(first) = providers.insert(service, id) {
                    self.note(
                        depth + 1,
                        ResolveError::ConflictingProviders {
                            service: service.clone(),
                            first: first.to_string(),
                            second: id.clone(),
                        },
                    );
                    return None;
                }
            }
        }
        Some(state)
    }
}

/// Dependencies first; errors with the cycle's path when there is one.
fn order(chosen: &BTreeMap<String, Candidate>) -> Result<Vec<String>, ResolveError> {
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        Visiting,
        Done,
    }
    fn visit(
        id: &str,
        chosen: &BTreeMap<String, Candidate>,
        marks: &mut BTreeMap<String, Mark>,
        path: &mut Vec<String>,
        out: &mut Vec<String>,
    ) -> Result<(), ResolveError> {
        match marks.get(id) {
            Some(Mark::Done) => return Ok(()),
            Some(Mark::Visiting) => {
                let start = path.iter().position(|p| p == id).unwrap_or(0);
                let mut cycle: Vec<String> = path[start..].to_vec();
                cycle.push(id.to_string());
                return Err(ResolveError::Cycle(cycle));
            }
            None => {}
        }
        marks.insert(id.to_string(), Mark::Visiting);
        path.push(id.to_string());
        for dep in hard_edges(&chosen[id], chosen) {
            visit(&dep, chosen, marks, path, out)?;
        }
        path.pop();
        marks.insert(id.to_string(), Mark::Done);
        out.push(id.to_string());
        Ok(())
    }
    let mut marks = BTreeMap::new();
    let mut out = Vec::new();
    for id in chosen.keys() {
        visit(id, chosen, &mut marks, &mut Vec::new(), &mut out)?;
    }
    Ok(out)
}

/// Resolves `requests` into one version of every plugin they need.
pub fn resolve(
    requests: &[Request],
    provider: &dyn CandidateProvider,
    options: &Options,
) -> Result<Resolution, ResolveError> {
    let mut solver = Solver {
        provider,
        options,
        memo: BTreeMap::new(),
        best: None,
    };
    let pending: Vec<Need> = requests
        .iter()
        .rev()
        .map(|r| Need {
            id: r.id.clone(),
            req: r.req.clone(),
            by: "the project".to_string(),
            source: r.source.clone(),
            features: r.features.clone(),
        })
        .collect();
    let Some(state) = solver.solve(pending, State::default()) else {
        return Err(solver.best.map(|(_, e)| e).unwrap_or(ResolveError::Source(
            "the requirements could not be satisfied".to_string(),
        )));
    };
    let order = order(&state.chosen)?;
    Ok(Resolution {
        packages: state.chosen,
        order,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Default)]
    struct Fake(BTreeMap<String, Vec<Candidate>>);

    impl Fake {
        fn add(
            &mut self,
            id: &str,
            version: &str,
            deps: serde_json::Value,
            extra: serde_json::Value,
        ) {
            let mut value = json!({
                "format": 1, "id": id, "name": id, "version": version,
                "engine": ">=0.0.1", "dependencies": deps,
            });
            if let (Some(base), Some(extra)) = (value.as_object_mut(), extra.as_object()) {
                base.extend(extra.clone());
            }
            let manifest: PluginManifest = serde_json::from_value(value).unwrap();
            self.0.entry(id.to_string()).or_default().push(Candidate {
                manifest,
                source: Source::Registry("main".into()),
                hash: format!("{id}{version}"),
            });
        }
    }

    impl CandidateProvider for Fake {
        fn candidates(&self, id: &str, _: Option<&Source>) -> Result<Vec<Candidate>, String> {
            Ok(self.0.get(id).cloned().unwrap_or_default())
        }
    }

    fn options() -> Options {
        Options {
            engine: Version::new(0, 0, 1),
            locked: BTreeMap::new(),
            upgrade: BTreeSet::new(),
        }
    }

    fn request(id: &str, req: &str) -> Request {
        Request {
            id: id.to_string(),
            req: req.parse().unwrap(),
            source: None,
            features: vec![],
        }
    }

    fn versions(r: &Resolution) -> Vec<String> {
        r.packages
            .values()
            .map(|c| format!("{}@{}", c.manifest.id, c.manifest.version))
            .collect()
    }

    #[test]
    fn picks_the_highest_compatible_versions_dependencies_first() {
        let mut fake = Fake::default();
        fake.add("com.x.a", "1.0.0", json!({"com.x.b": "^1"}), json!({}));
        fake.add("com.x.b", "1.0.0", json!({}), json!({}));
        fake.add("com.x.b", "1.4.0", json!({}), json!({}));
        fake.add("com.x.b", "2.0.0", json!({}), json!({}));
        let r = resolve(&[request("com.x.a", "*")], &fake, &options()).unwrap();
        assert_eq!(versions(&r), ["com.x.a@1.0.0", "com.x.b@1.4.0"]);
        assert_eq!(r.order, ["com.x.b", "com.x.a"]);
        assert_eq!(r.dependencies_of("com.x.a"), ["com.x.b"]);
    }

    #[test]
    fn backtracks_to_an_older_version_to_satisfy_a_shared_dependency() {
        let mut fake = Fake::default();
        fake.add("com.x.a", "2.0.0", json!({"com.x.c": "^2"}), json!({}));
        fake.add("com.x.a", "1.0.0", json!({"com.x.c": "^1"}), json!({}));
        fake.add("com.x.b", "1.0.0", json!({"com.x.c": "^1"}), json!({}));
        fake.add("com.x.c", "1.0.0", json!({}), json!({}));
        fake.add("com.x.c", "2.0.0", json!({}), json!({}));
        let r = resolve(
            &[request("com.x.b", "*"), request("com.x.a", "*")],
            &fake,
            &options(),
        )
        .unwrap();
        assert_eq!(
            versions(&r),
            ["com.x.a@1.0.0", "com.x.b@1.0.0", "com.x.c@1.0.0"]
        );
    }

    #[test]
    fn locked_versions_are_kept_unless_told_to_move() {
        let mut fake = Fake::default();
        fake.add("com.x.a", "1.0.0", json!({}), json!({}));
        fake.add("com.x.a", "1.5.0", json!({}), json!({}));
        let mut opts = options();
        opts.locked.insert("com.x.a".into(), Version::new(1, 0, 0));
        let kept = resolve(&[request("com.x.a", "^1")], &fake, &opts).unwrap();
        assert_eq!(versions(&kept), ["com.x.a@1.0.0"]);
        opts.upgrade.insert("com.x.a".into());
        let moved = resolve(&[request("com.x.a", "^1")], &fake, &opts).unwrap();
        assert_eq!(versions(&moved), ["com.x.a@1.5.0"]);
        // A lock that no longer satisfies the requirement is not kept.
        opts.upgrade.clear();
        let narrowed = resolve(&[request("com.x.a", "^1.2")], &fake, &opts).unwrap();
        assert_eq!(versions(&narrowed), ["com.x.a@1.5.0"]);
    }

    #[test]
    fn an_unsatisfiable_requirement_explains_itself() {
        let mut fake = Fake::default();
        fake.add("com.x.a", "1.0.0", json!({"com.x.b": "^3"}), json!({}));
        fake.add("com.x.b", "1.0.0", json!({}), json!({}));
        fake.add("com.x.b", "2.0.0", json!({}), json!({}));
        let error = resolve(&[request("com.x.a", "*")], &fake, &options()).unwrap_err();
        let text = error.to_string();
        assert!(text.contains("com.x.b") && text.contains("^3"), "{text}");
        assert!(text.contains("com.x.a 1.0.0"), "{text}");
        assert!(text.contains("1.0.0, 2.0.0"), "{text}");
    }

    #[test]
    fn a_missing_package_is_not_found() {
        let fake = Fake::default();
        let error = resolve(&[request("com.x.a", "*")], &fake, &options()).unwrap_err();
        assert!(matches!(error, ResolveError::NotFound { .. }));
        assert!(error.to_string().contains("the project"));
    }

    #[test]
    fn engine_incompatible_versions_are_skipped_and_reported() {
        let mut fake = Fake::default();
        fake.add("com.x.a", "1.0.0", json!({}), json!({"engine": ">=9.0.0"}));
        let error = resolve(&[request("com.x.a", "*")], &fake, &options()).unwrap_err();
        assert!(
            error.to_string().contains("needs engine >=9.0.0"),
            "{error}"
        );
        fake.add("com.x.a", "0.9.0", json!({}), json!({}));
        let r = resolve(&[request("com.x.a", "*")], &fake, &options()).unwrap();
        assert_eq!(versions(&r), ["com.x.a@0.9.0"]);
    }

    #[test]
    fn cycles_are_rejected_with_their_path() {
        let mut fake = Fake::default();
        fake.add("com.x.a", "1.0.0", json!({"com.x.b": "*"}), json!({}));
        fake.add("com.x.b", "1.0.0", json!({"com.x.a": "*"}), json!({}));
        let error = resolve(&[request("com.x.a", "*")], &fake, &options()).unwrap_err();
        let ResolveError::Cycle(path) = error else {
            panic!("expected a cycle");
        };
        assert_eq!(path, ["com.x.a", "com.x.b", "com.x.a"]);
    }

    #[test]
    fn two_providers_of_one_service_conflict() {
        let mut fake = Fake::default();
        fake.add(
            "com.x.a",
            "1.0.0",
            json!({}),
            json!({"provides": ["terrain"]}),
        );
        fake.add(
            "com.x.b",
            "1.0.0",
            json!({}),
            json!({"provides": ["terrain"]}),
        );
        let error = resolve(
            &[request("com.x.a", "*"), request("com.x.b", "*")],
            &fake,
            &options(),
        )
        .unwrap_err();
        assert!(
            matches!(error, ResolveError::ConflictingProviders { .. }),
            "{error}"
        );
    }

    #[test]
    fn optional_dependencies_constrain_but_do_not_pull() {
        let mut fake = Fake::default();
        fake.add(
            "com.x.a",
            "1.0.0",
            json!({"com.x.b": {"version": "^1", "optional": true}}),
            json!({}),
        );
        fake.add("com.x.b", "2.0.0", json!({}), json!({}));
        let alone = resolve(&[request("com.x.a", "*")], &fake, &options()).unwrap();
        assert_eq!(versions(&alone), ["com.x.a@1.0.0"]);
        let error = resolve(
            &[request("com.x.a", "*"), request("com.x.b", "*")],
            &fake,
            &options(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("optional"), "{error}");
    }

    #[test]
    fn features_add_dependencies_only_when_asked() {
        let mut fake = Fake::default();
        fake.add(
            "com.x.a",
            "1.0.0",
            json!({}),
            json!({"features": {"fx": {"dependencies": {"com.x.fx": "^1"}}}}),
        );
        fake.add("com.x.fx", "1.0.0", json!({}), json!({}));
        let plain = resolve(&[request("com.x.a", "*")], &fake, &options()).unwrap();
        assert_eq!(plain.packages.len(), 1);
        let mut with = request("com.x.a", "*");
        with.features = vec!["fx".into()];
        assert_eq!(
            resolve(&[with], &fake, &options()).unwrap().packages.len(),
            2
        );
        let mut bad = request("com.x.a", "*");
        bad.features = vec!["nope".into()];
        assert!(matches!(
            resolve(&[bad], &fake, &options()).unwrap_err(),
            ResolveError::UnknownFeature { .. }
        ));
    }
}
