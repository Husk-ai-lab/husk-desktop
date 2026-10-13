//! Keeping backends from outliving the shell.
//!
//! Plan: "if the shell crashes, the backends notice their parent is gone and exit". The docs
//! define no such contract in the backends yet, so the shell does what it can itself:
//!   * Windows: a Job Object with KILL_ON_JOB_CLOSE (children die with the shell, even on crash)
//!   * Linux:   PR_SET_PDEATHSIG (set in `supervisor` before exec)
//!   * macOS (and as a backstop everywhere): a pid file per child; the next launch kills a
//!     still-running leftover whose executable is the one we recorded, then starts clean.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sysinfo::{Pid, ProcessesToUpdate, System};

use crate::paths::Paths;

#[derive(Debug, Serialize, Deserialize)]
struct PidRecord {
    pid: u32,
    exe: PathBuf,
}

fn pid_file(paths: &Paths, id: &str) -> PathBuf {
    paths.run_dir().join(format!("{id}.pid"))
}

pub fn record(paths: &Paths, id: &str, pid: u32, exe: &Path) {
    let rec = PidRecord {
        pid,
        exe: exe.to_path_buf(),
    };
    if let Ok(b) = serde_json::to_vec(&rec) {
        std::fs::write(pid_file(paths, id), b).ok();
    }
}

pub fn forget(paths: &Paths, id: &str) {
    std::fs::remove_file(pid_file(paths, id)).ok();
}

/// Kill leftovers of a previous run that the shell did not get to stop. Returns how many were
/// killed. Only a process whose executable path equals the recorded one is touched, so a reused
/// pid belonging to some other program is left alone.
pub fn reap_stale(paths: &Paths) -> usize {
    let Ok(rd) = std::fs::read_dir(paths.run_dir()) else {
        return 0;
    };
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);
    let mut killed = 0;
    for e in rd.flatten() {
        let path = e.path();
        if path.extension().and_then(|x| x.to_str()) != Some("pid") {
            continue;
        }
        if let Some(rec) = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice::<PidRecord>(&b).ok())
        {
            if let Some(p) = sys.process(Pid::from_u32(rec.pid)) {
                if p.exe().map(|x| x == rec.exe).unwrap_or(false) {
                    tracing::warn!(
                        pid = rec.pid,
                        "killing leftover backend from a previous run"
                    );
                    p.kill();
                    killed += 1;
                }
            }
        }
        std::fs::remove_file(&path).ok();
    }
    killed
}

#[cfg(windows)]
pub mod job {
    //! One job object for the whole shell; every child is assigned to it.
    use std::sync::OnceLock;

    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    struct JobHandle(HANDLE);
    unsafe impl Send for JobHandle {}
    unsafe impl Sync for JobHandle {}

    static JOB: OnceLock<Option<JobHandle>> = OnceLock::new();

    fn job() -> Option<HANDLE> {
        JOB.get_or_init(|| unsafe {
            let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if h.is_null() {
                return None;
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = SetInformationJobObject(
                h,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            (ok != 0).then_some(JobHandle(h))
        })
        .as_ref()
        .map(|j| j.0)
    }

    /// Best effort: if the job cannot be created the pid-file backstop still applies.
    pub fn assign(process_handle: HANDLE) {
        if let Some(j) = job() {
            unsafe {
                AssignProcessToJobObject(j, process_handle);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_record_for_a_dead_pid_is_just_cleaned() {
        let t = tempfile::tempdir().unwrap();
        let paths = Paths::new(t.path().to_path_buf());
        paths.ensure_dirs().unwrap();
        record(
            &paths,
            "husk",
            4_000_000,
            Path::new("/nonexistent/husk-backend"),
        );
        assert_eq!(reap_stale(&paths), 0);
        assert!(!pid_file(&paths, "husk").exists());
    }

    #[test]
    fn a_reused_pid_with_a_different_executable_is_left_alone() {
        let t = tempfile::tempdir().unwrap();
        let paths = Paths::new(t.path().to_path_buf());
        paths.ensure_dirs().unwrap();
        // our own pid, but a different recorded executable: must not be killed
        record(
            &paths,
            "husk",
            std::process::id(),
            Path::new("/definitely/not/this/exe"),
        );
        assert_eq!(reap_stale(&paths), 0);
    }
}
