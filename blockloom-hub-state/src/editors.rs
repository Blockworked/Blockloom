//! Starting, reusing and focusing editors.
//!
//! Opening a project reuses an editor that is idle at its Dashboard (same
//! installation, fresh presence) by dropping a request beside its presence
//! file; only when there is none, or it doesn't answer, is a new editor
//! process started.

use crate::{
    Error, Hub, PRESENCE_FRESH_SECS, Result, checkpoint, installs, now, oplock::OperationLock,
    process, write_json,
};
use blockloom_core::{hub as contract, sync};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

enum Reuse {
    /// The editor opened it (or is still at it when the wait ran out).
    Done(bool),
    /// The editor looked at the request and said no.
    Declined(String),
    /// Nobody took it; start an editor instead.
    Unanswered,
}

impl Hub {
    fn presence(&self, pid: u32) -> Option<contract::Presence> {
        let text = std::fs::read_to_string(contract::presence_path(&self.editors, pid)).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// The installation an editor says it is, whoever started it.
    pub fn presence_installation(&self, pid: u32) -> Option<String> {
        self.presence(pid)?.installation
    }

    /// An editor of `installation` that is waiting at its Dashboard.
    fn idle_editor(&self, installation: &str) -> Option<u32> {
        let read = std::fs::read_dir(&self.editors).ok()?;
        let mut best: Option<(u64, u32)> = None;
        for entry in read.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(pid) = name
                .strip_suffix(".json")
                .and_then(|n| n.parse::<u32>().ok())
            else {
                continue;
            };
            let Some(presence) = self.presence(pid) else {
                continue;
            };
            if !process::alive(pid) {
                // Left behind by an editor that is gone.
                let _ = std::fs::remove_file(entry.path());
                continue;
            }
            let fresh = sync::now_secs().saturating_sub(presence.heartbeat) <= PRESENCE_FRESH_SECS;
            let waiting = presence.project.is_none()
                && presence.installation.as_deref() == Some(installation)
                && !contract::request_path(&self.editors, pid).exists();
            if fresh && waiting && best.is_none_or(|(beat, _)| presence.heartbeat >= beat) {
                best = Some((presence.heartbeat, pid));
            }
        }
        best.map(|(_, pid)| pid)
    }

    fn send_request(
        &self,
        pid: u32,
        action: &str,
        path: Option<&Path>,
        project: Option<Value>,
    ) -> Result<String> {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let request = contract::Request {
            schema: 1,
            id: id.clone(),
            action: action.to_string(),
            path: path.map(|p| p.to_string_lossy().into_owned()),
            project,
            requested_at: sync::now_secs(),
        };
        let _ = std::fs::remove_file(contract::reply_path(&self.editors, pid));
        contract::write_file_atomic(
            &contract::request_path(&self.editors, pid),
            &serde_json::to_string(&request)?,
        )?;
        Ok(id)
    }

    fn await_reply(&self, pid: u32, id: &str, folder: Option<&Path>) -> Reuse {
        let request = contract::request_path(&self.editors, pid);
        let reply = contract::reply_path(&self.editors, pid);
        let take_by = Instant::now() + self.reuse_timeout;
        let give_up = Instant::now() + self.reuse_timeout.max(self.open_timeout);
        loop {
            if let Ok(text) = std::fs::read_to_string(&reply)
                && let Ok(answer) = serde_json::from_str::<contract::Reply>(&text)
                && answer.id == id
            {
                let _ = std::fs::remove_file(&reply);
                return if answer.ok {
                    Reuse::Done(true)
                } else {
                    Reuse::Declined(
                        answer
                            .error
                            .unwrap_or_else(|| "The editor couldn't open it.".into()),
                    )
                };
            }
            if folder.is_some_and(|f| self.lock_owned_by(pid, Some(f))) {
                return Reuse::Done(true);
            }
            if !process::alive(pid) {
                return Reuse::Unanswered;
            }
            let waiting = request.exists();
            if waiting && Instant::now() >= take_by {
                let _ = std::fs::remove_file(&request);
                return Reuse::Unanswered;
            }
            if !waiting && Instant::now() >= give_up {
                return Reuse::Done(false);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Who holds a project: the folder given, one under a parent folder, or
    /// (with none given) any project the library lists.
    fn lock_owned_by(&self, pid: u32, project: Option<&Path>) -> bool {
        let mut candidates: Vec<PathBuf> = vec![];
        match project {
            Some(p) if p.join("project.blockloom").exists() || p.join(".blockloom").exists() => {
                candidates.push(p.to_path_buf())
            }
            Some(p) if p.is_dir() => {
                if let Ok(read) = std::fs::read_dir(p) {
                    candidates.extend(read.flatten().map(|e| e.path()).filter(|e| e.is_dir()));
                }
            }
            Some(_) => {}
            None => candidates.extend(self.remembered_paths().into_iter().map(PathBuf::from)),
        }
        candidates.iter().any(|folder| {
            crate::read_json(&sync::lock_path(folder))
                .is_ok_and(|lock| lock["pid"].as_u64() == Some(pid as u64))
        })
    }

    fn remembered_paths(&self) -> Vec<String> {
        self.projects()
            .map(|rows| {
                rows.iter()
                    .filter_map(|r| r["path"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn spawn(
        &self,
        identity: &str,
        arguments: &[String],
        project: Option<&Path>,
    ) -> Result<(Child, PathBuf)> {
        let installation = self.installation(identity, true)?;
        let directory = self.slot(identity)?;
        let tool = |name: &str| directory.join("tools").join(name);
        let mut command = Command::new(directory.join(installs::editor_name()));
        command.args(arguments).current_dir(&directory);
        command.env("BLOCKLOOM_INSTALLATION", identity);
        let rust = tool("rust");
        let mut bins = vec![rust.join("bin")];
        command.env("RUSTUP_TOOLCHAIN", &rust);
        let tools = installation["tools"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        for (name, variable) in [
            ("java", "JAVA_HOME"),
            ("android-sdk", "BLOCKLOOM_HUB_ANDROID_SDK"),
            ("android-ndk", "BLOCKLOOM_HUB_ANDROID_NDK"),
        ] {
            // Clear inherited Hub paths when this installation omits a tool.
            if variable.starts_with("BLOCKLOOM_HUB_") {
                command.env_remove(variable);
            }
            if tools.contains_key(name) {
                let root = tool(name);
                command.env(variable, &root);
                match name {
                    "java" => bins.push(root.join("bin")),
                    "android-sdk" => {
                        command
                            .env("ANDROID_HOME", &root)
                            .env("ANDROID_SDK_ROOT", &root);
                    }
                    _ => {
                        command
                            .env("ANDROID_NDK_HOME", &root)
                            .env("ANDROID_NDK_ROOT", &root);
                    }
                }
            }
        }
        let inherited = std::env::var_os("PATH").unwrap_or_default();
        bins.extend(std::env::split_paths(&inherited));
        if let Ok(path) = std::env::join_paths(bins) {
            command.env("PATH", path);
        }
        let logs = self.root.join("logs");
        std::fs::create_dir_all(&logs)?;
        let log = logs.join(format!("{identity}-editor.log"));
        let output = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log)?;
        let errors = output.try_clone()?;
        command
            .stdin(Stdio::null())
            .stdout(Stdio::from(output))
            .stderr(Stdio::from(errors));
        let mut child = command.spawn().map_err(|e| {
            Error::new(format!(
                "Couldn't start the editor ({}): {e}",
                directory.display()
            ))
        })?;
        if let Some(token) = process::token(child.id()) {
            let mut records = self.running_editors();
            records.push(json!({
                "pid": child.id(), "token": token, "installation": identity,
                "path": project.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
                "launched_at": now(),
            }));
            if let Err(error) = write_json(
                &self.root.join("running.json"),
                &json!({"editors": records}),
            ) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        }
        Ok((child, log))
    }

    /// Waits until the editor owns the project, not merely until it spawned.
    /// `Ok(false)` when it is still starting at the deadline.
    fn confirm_open(&self, child: &mut Child, project: Option<&Path>, log: &Path) -> Result<bool> {
        let deadline = Instant::now() + self.open_timeout;
        loop {
            if child.try_wait()?.is_some() {
                return Err(Error::new(format!(
                    "The editor closed before it opened the project. See {}",
                    log.display()
                )));
            }
            if self.lock_owned_by(child.id(), project) {
                return Ok(true);
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    /// Opens a project in its editor: an idle one when there is one,
    /// otherwise a new process.
    pub fn launch(&self, project: &Path) -> Result<Value> {
        let (info, identity, bound, reuse) = {
            let _lock = OperationLock::acquire(&self.root)?;
            let info = self.project(project)?;
            let path = PathBuf::from(info["path"].as_str().unwrap_or(""));
            self.require_idle(&path)?;
            let mut identity = info["installation"].as_str().map(str::to_string);
            let mut bound = false;
            if identity.is_none() {
                let chosen = self.default_installation()?.ok_or_else(|| {
                    Error::new("Install an editor first. Get a release on the Installations tab.")
                })?;
                checkpoint()?;
                write_json(
                    &path.join(".blockloom").join("hub.json"),
                    &json!({"schema": 1, "installation": chosen}),
                )?;
                identity = Some(chosen);
                bound = true;
            }
            let identity = identity.unwrap_or_default();
            let reuse = match self.idle_editor(&identity) {
                Some(pid) => {
                    self.installation(&identity, true)?;
                    let id = self.send_request(pid, "open", Some(&path), None)?;
                    Some((pid, id))
                }
                None => None,
            };
            (info, identity, bound, reuse)
        };
        let path = PathBuf::from(info["path"].as_str().unwrap_or(""));
        if let Some((pid, id)) = reuse {
            match self.await_reply(pid, &id, Some(&path)) {
                Reuse::Done(opened) => {
                    return Ok(
                        json!({"pid": pid, "installation": identity, "path": path.to_string_lossy(),
                                     "opened": opened, "bound": bound, "reused": true}),
                    );
                }
                Reuse::Declined(error) => return Err(Error::new(error)),
                Reuse::Unanswered => {}
            }
        }
        let (mut child, log) = {
            let _lock = OperationLock::acquire(&self.root)?;
            self.require_idle(&path)?;
            self.spawn(
                &identity,
                &["--project".into(), path.to_string_lossy().into_owned()],
                Some(&path),
            )?
        };
        // The lock is only for starting the editor, not for waiting on it.
        let opened = self.confirm_open(&mut child, Some(&path), &log)?;
        Ok(
            json!({"pid": child.id(), "installation": identity, "path": path.to_string_lossy(),
                  "opened": opened, "bound": bound, "reused": false}),
        )
    }

    /// Asks the editor holding a project to come to the front. The editor
    /// deletes the request when it takes it, so one that is gone within a few
    /// seconds was heard.
    pub fn focus(&self, project: &Path) -> Result<Value> {
        let info = self.project(project)?;
        let path = PathBuf::from(info["path"].as_str().unwrap_or(""));
        let owner = self
            .project_owner(&path)
            .ok_or_else(|| Error::new("No editor has this project open. Open it instead"))?;
        let request = path.join(".blockloom").join("focus.json");
        write_json(
            &request,
            &json!({"requested_at": sync::now_secs(), "pid": owner["pid"]}),
        )?;
        let mut acknowledged = false;
        let deadline = Instant::now() + self.focus_timeout;
        while Instant::now() < deadline {
            if !request.exists() {
                acknowledged = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        if !acknowledged && !self.focus_timeout.is_zero() {
            let _ = std::fs::remove_file(&request);
        }
        Ok(json!({"path": path.to_string_lossy(), "owner": owner, "acknowledged": acknowledged}))
    }

    /// Creates a project with the editor's own rules: an idle editor is asked
    /// to, otherwise a new one is started with the request.
    pub fn new_project(
        &self,
        name: &str,
        installation: Option<&str>,
        location: Option<&str>,
        mode: &str,
        sample: Option<&str>,
    ) -> Result<Value> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::new("Give the project a name"));
        }
        if mode != "TwoD" && mode != "ThreeD" {
            return Err(Error::new("Dimension must be TwoD or ThreeD"));
        }
        let identity = match installation {
            Some(id) => id.to_string(),
            None => self.default_installation()?.ok_or_else(|| {
                Error::new("Install an editor first. Get a release on the Installations tab.")
            })?,
        };
        let folder = location.map(|l| {
            PathBuf::from(l.replacen(
                '~',
                &dirs::home_dir().unwrap_or_default().to_string_lossy(),
                1,
            ))
        });
        let mut spec = json!({"name": name, "mode": mode});
        if let Some(folder) = &folder {
            spec["location"] = json!(folder.to_string_lossy());
        }
        if let Some(sample) = sample {
            spec["sample"] = json!(sample);
        }
        let reuse = {
            let _lock = OperationLock::acquire(&self.root)?;
            self.installation(&identity, true)?;
            match self.idle_editor(&identity) {
                Some(pid) => Some((
                    pid,
                    self.send_request(pid, "create", None, Some(spec.clone()))?,
                )),
                None => None,
            }
        };
        if let Some((pid, id)) = reuse {
            match self.await_reply(pid, &id, folder.as_deref()) {
                Reuse::Done(opened) => {
                    return Ok(
                        json!({"pid": pid, "installation": identity, "opened": opened,
                                     "name": name, "reused": true}),
                    );
                }
                Reuse::Declined(error) => return Err(Error::new(error)),
                Reuse::Unanswered => {}
            }
        }
        let mut args: Vec<String> = vec![
            "--new-project".into(),
            name.into(),
            "--mode".into(),
            mode.into(),
        ];
        if let Some(folder) = &folder {
            args.extend(["--location".into(), folder.to_string_lossy().into_owned()]);
        }
        if let Some(sample) = sample {
            args.extend(["--sample".into(), sample.into()]);
        }
        let (mut child, log) = {
            let _lock = OperationLock::acquire(&self.root)?;
            self.installation(&identity, true)?;
            self.spawn(&identity, &args, None)?
        };
        let opened = self.confirm_open(&mut child, folder.as_deref(), &log)?;
        Ok(
            json!({"pid": child.id(), "installation": identity, "opened": opened,
                  "name": name, "reused": false}),
        )
    }
}
