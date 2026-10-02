//! Background builds keep platform queries and cancellation off the build worker.

use crate::{Backend, commands};
use blockloom_core::{android, build, build_control::BuildControl};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BuildParams {
    pub path: String,
    pub target: Option<String>,
    pub fast: Option<bool>,
    pub hdr: Option<bool>,
    pub store_pass: Option<String>,
    pub key_pass: Option<String>,
    #[serde(default)]
    pub remember_passwords: bool,
    pub device: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct BuildStatus {
    pub id: u64,
    pub state: String,
    pub step: String,
    pub detail: String,
    pub built: Option<build::Build>,
    pub error: Option<String>,
    pub device: Option<String>,
}

struct Job {
    status: BuildStatus,
    control: BuildControl,
    worker: Option<std::thread::JoinHandle<()>>,
}

#[derive(Default)]
struct Jobs {
    job: Option<Job>,
    previous: VecDeque<BuildStatus>,
    next_id: u64,
}

#[derive(Clone, Default)]
pub(crate) struct BuildJobs(Arc<Mutex<Jobs>>);

impl BuildJobs {
    pub fn start(&self, backend: &Backend, params: BuildParams) -> Result<BuildStatus, String> {
        let mut current = self.0.lock().map_err(|e| e.to_string())?;
        if current
            .job
            .as_ref()
            .is_some_and(|job| job.status.state == "running")
        {
            return Err("A build is already running. Wait for it or cancel it first.".to_string());
        }
        let request = commands::prepare_build_game(&backend.state, params.clone())?;
        current.next_id += 1;
        let id = current.next_id;
        if let Some(previous) = current.job.take() {
            current.previous.push_back(previous.status);
            if current.previous.len() > 16 {
                current.previous.pop_front();
            }
        }
        let status = BuildStatus {
            id,
            state: "running".into(),
            step: "Preparing build".into(),
            detail: String::new(),
            built: None,
            error: None,
            device: params.device.clone(),
        };
        let control = BuildControl::default();
        current.job = Some(Job {
            status: status.clone(),
            control: control.clone(),
            worker: None,
        });
        let jobs = self.clone();
        let backend = backend.clone();
        let worker = std::thread::Builder::new()
            .name("blockloom-build".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    control.run(|| {
                        let built =
                            commands::run_build_game(&backend.state, &backend.app, request)?;
                        if let Some(device) = &params.device {
                            android::install_apk(
                                Path::new(&built.binary),
                                &built.application_id,
                                Some(device),
                            )?;
                        }
                        Ok(built)
                    })
                }))
                .unwrap_or_else(|_| Err("The build worker stopped unexpectedly.".to_string()));
                let mut current = jobs.0.lock().unwrap();
                let job = current.job.as_mut().unwrap();
                if control.cancelled() {
                    job.status.state = "cancelled".into();
                    job.status.step = "Cancelled".into();
                } else {
                    match result {
                        Ok(built) => {
                            job.status.state = "complete".into();
                            job.status.step = if params.device.is_some() {
                                "Running on device"
                            } else {
                                "Build complete"
                            }
                            .into();
                            job.status.built = Some(built);
                        }
                        Err(error) => {
                            job.status.state = "failed".into();
                            job.status.step = "Build failed".into();
                            job.status.error = Some(error);
                        }
                    }
                }
                job.status.detail.clear();
            })
            .map_err(|e| {
                current.job = None;
                format!("Couldn't start build worker: {e}")
            })?;
        current.job.as_mut().unwrap().worker = Some(worker);
        Ok(status)
    }

    pub fn status(&self, id: u64) -> Result<BuildStatus, String> {
        let current = self.0.lock().map_err(|e| e.to_string())?;
        let Some(job) = current.job.as_ref().filter(|job| job.status.id == id) else {
            return current
                .previous
                .iter()
                .find(|status| status.id == id)
                .cloned()
                .ok_or_else(|| "This build job is no longer available.".to_string());
        };
        let mut status = job.status.clone();
        if status.state == "running" {
            let (step, detail) = job.control.progress();
            if !step.is_empty() {
                status.step = step;
            }
            status.detail = detail;
            if job.control.cancelled() {
                status.step = "Cancelling...".into();
            }
        }
        Ok(status)
    }

    pub fn cancel(&self, id: u64) -> Result<(), String> {
        let current = self.0.lock().map_err(|e| e.to_string())?;
        let Some(job) = current.job.as_ref().filter(|job| job.status.id == id) else {
            return if current.previous.iter().any(|status| status.id == id) {
                Ok(())
            } else {
                Err("This build job is no longer available.".to_string())
            };
        };
        if job.status.state == "running" {
            job.control.cancel();
        }
        Ok(())
    }

    pub fn wait(&self, id: u64) -> Result<build::Build, String> {
        loop {
            let status = self.status(id)?;
            match status.state.as_str() {
                "complete" => {
                    return status
                        .built
                        .ok_or_else(|| "Build completed without an output.".to_string());
                }
                "failed" => {
                    return Err(status.error.unwrap_or_else(|| "Build failed.".to_string()));
                }
                "cancelled" => return Err("Build cancelled.".to_string()),
                _ => std::thread::sleep(std::time::Duration::from_millis(100)),
            }
        }
    }
    pub fn shutdown(&self) {
        let worker = {
            let mut current = self.0.lock().unwrap();
            current.job.as_mut().and_then(|job| {
                job.control.cancel();
                job.worker.take()
            })
        };
        // Let the worker terminate its child tools before the editor exits.
        if let Some(worker) = worker {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{AppState, History};

    fn backend() -> Backend {
        Backend {
            state: Arc::new(Mutex::new(AppState {
                library: vec![],
                session_id: "build-test".into(),
                open: None,
                selected_actor: None,
                history: History::new(50),
                invalid_field_buffers: Default::default(),
                runtime: None,
                running: false,
                paused: false,
                status: None,
                interface_edit: None,
                interface_design: None,
                interface_layout: None,
                log: vec![],
                log_total: 0,
                preview_enabled: false,
                preview_headless: false,
                preview_port: None,
                pointer_locked: false,
                ray_tracing: None,
                embedded: None,
                scene_view: Default::default(),
                picked_tile: None,
            })),
            app: crate::AppHandle::new(|_| {}),
            builds: BuildJobs::default(),
        }
    }

    fn running_job(id: u64) -> Job {
        Job {
            status: BuildStatus {
                id,
                state: "running".into(),
                step: "Preparing build".into(),
                detail: String::new(),
                built: None,
                error: None,
                device: Some("emulator-5554".into()),
            },
            control: BuildControl::default(),
            worker: None,
        }
    }

    #[test]
    fn status_and_cancel_do_not_wait_for_project_state() {
        let backend = backend();
        backend.builds.0.lock().unwrap().job = Some(running_job(7));
        let _state = backend.state.lock().unwrap();
        let response = backend
            .dispatch("build_job_status", serde_json::json!({"id": 7}))
            .unwrap();
        assert_eq!(response["state"], "running");
        backend
            .dispatch("cancel_build_job", serde_json::json!({"id": 7}))
            .unwrap();
        let response = backend
            .dispatch("build_job_status", serde_json::json!({"id": 7}))
            .unwrap();
        assert_eq!(response["step"], "Cancelling...");
    }

    #[test]
    fn a_previous_result_survives_a_new_build() {
        let jobs = BuildJobs::default();
        let mut previous = running_job(1).status;
        previous.state = "cancelled".into();
        {
            let mut current = jobs.0.lock().unwrap();
            current.previous.push_back(previous);
            current.job = Some(running_job(2));
        }
        assert_eq!(jobs.status(1).unwrap().state, "cancelled");
        assert!(jobs.wait(1).unwrap_err().contains("cancelled"));
        jobs.cancel(1).unwrap();
        assert!(
            !jobs
                .0
                .lock()
                .unwrap()
                .job
                .as_ref()
                .unwrap()
                .control
                .cancelled()
        );
    }

    #[test]
    fn stale_ids_and_overlapping_builds_are_refused() {
        let backend = backend();
        backend.builds.0.lock().unwrap().job = Some(running_job(2));
        assert!(backend.builds.cancel(1).is_err());
        assert!(backend.builds.status(1).is_err());
        assert!(
            !backend
                .builds
                .0
                .lock()
                .unwrap()
                .job
                .as_ref()
                .unwrap()
                .control
                .cancelled()
        );
        let params: BuildParams =
            serde_json::from_value(serde_json::json!({"path": "/tmp/build"})).unwrap();
        assert!(
            backend
                .builds
                .start(&backend, params)
                .unwrap_err()
                .contains("already running")
        );
        backend.shutdown();
        assert!(
            backend
                .builds
                .0
                .lock()
                .unwrap()
                .job
                .as_ref()
                .unwrap()
                .control
                .cancelled()
        );
    }
}
