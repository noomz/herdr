use std::path::Path;

use super::App;

/// Agent launcher capture: when a pane's agent process sits behind a launcher
/// command (`zsh -> node .../ccs.js work -> claude`), session restore must
/// replay the resume through that launcher (`ccs work --resume <id>`) so
/// launcher-managed environment, account, and provider setup apply. Capture
/// runs only on the agent-session report path, never in render or detection.
impl App {
    /// Synthesize and record a launcher-wrapped resume command for a reported
    /// agent session. No-op unless the launcher is listed in
    /// `session.resume_launcher_allowlist` and the live process ancestry
    /// resolves to exactly one allowlisted launcher command.
    pub(crate) fn maybe_synthesize_launcher_resume(
        &mut self,
        pane_id: crate::layout::PaneId,
        source: &str,
        agent_label: &str,
        session_ref: &crate::agent_resume::AgentSessionRef,
        seq: Option<u64>,
    ) {
        if self.resume_launcher_allowlist.is_empty() {
            return;
        }
        let Some(base_plan) = crate::agent_resume::plan(source, agent_label, session_ref) else {
            return;
        };
        let Some((_, pane)) = self.find_pane(pane_id) else {
            return;
        };
        let terminal_id = pane.attached_terminal_id.clone();
        let Some(runtime) = self.terminal_runtimes.get(&terminal_id) else {
            return;
        };
        let Some(shell_pid) = runtime.child_pid() else {
            return;
        };
        let Some(job) = crate::detect::foreground_job(shell_pid) else {
            return;
        };
        // The reporter claims an agent; capture only when process detection
        // agrees, so a stale hook cannot attach a launcher to another agent.
        let Some((detected, _, agent_pid)) = crate::detect::identify_agent_process_in_job(&job)
        else {
            return;
        };
        if crate::detect::agent_label(detected) != agent_label {
            return;
        }
        let Some(ancestors) = crate::platform::process_ancestors_until(agent_pid, shell_pid) else {
            return;
        };
        // Only a single launcher process between the pane shell and the agent
        // is replayable as one command; deeper or absent chains keep the
        // native resume plan.
        let [launcher] = ancestors.as_slice() else {
            return;
        };
        let Some(raw_argv) = launcher.argv.as_deref() else {
            return;
        };
        let Some(normalized) =
            crate::agent_resume::normalize_launcher_argv(raw_argv, &resolve_path_shim)
        else {
            return;
        };
        let prefix = crate::agent_resume::launcher_command_prefix(&normalized);
        let Some(command) = prefix.first() else {
            return;
        };
        if !self
            .resume_launcher_allowlist
            .iter()
            .any(|allowed| allowed == command)
        {
            return;
        }
        let Some(argv) = crate::agent_resume::launcher_resume_argv(prefix, &base_plan.argv) else {
            return;
        };
        tracing::debug!(
            pane = pane_id.raw(),
            agent = agent_label,
            command = %command,
            "recording launcher-wrapped agent resume"
        );
        self.handle_internal_event(crate::events::AppEvent::AgentResumeReported {
            pane_id,
            source: source.to_string(),
            agent_label: agent_label.to_string(),
            seq,
            argv,
        });
    }
}

/// Resolve an invoked executable or script path to the name of the PATH entry
/// that owns it, following symlinks (`/opt/homebrew/bin/ccs` and
/// `/opt/homebrew/lib/node_modules/@kaitranntt/ccs/dist/ccs.js` both resolve
/// to `ccs`). Returns `None` when no PATH entry resolves to the target.
fn resolve_path_shim(target: &Path) -> Option<String> {
    let target = std::fs::canonicalize(target).ok()?;
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(canonical) = std::fs::canonicalize(entry.path()) else {
                continue;
            };
            if canonical == target {
                return Some(entry.file_name().to_string_lossy().into_owned());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {

    #[test]
    fn resolve_path_shim_finds_symlink_owner() {
        let dir = std::env::temp_dir().join(format!(
            "herdr-shim-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let real = dir.join("real-cli.js");
        std::fs::write(&real, "// cli").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&real, dir.join("shim-cli")).unwrap();
        }
        #[cfg(not(unix))]
        {
            std::fs::copy(&real, dir.join("shim-cli")).unwrap();
        }
        let target = std::fs::canonicalize(&real).unwrap();
        let found = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .find(|entry| {
                std::fs::canonicalize(entry.path()).ok().as_deref() == Some(target.as_path())
            })
            .map(|entry| entry.file_name().to_string_lossy().into_owned());
        // Both the real file and its symlink canonicalize to the target; the
        // shim lookup may return either entry name.
        assert!(matches!(
            found.as_deref(),
            Some("shim-cli") | Some("real-cli.js")
        ));
        std::fs::remove_dir_all(&dir).ok();
    }
}
