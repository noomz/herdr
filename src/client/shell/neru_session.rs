//! The neru hints action's work off the client loop (`SPEC-neru-hints.md` §T.5).
//!
//! The client loop (`client::neru_hints`) owns the host mouse modes and the report routing.
//! This module owns what runs on worker threads, calibration and `neru hints`, and the steps
//! that read the hit map: the precheck and the targets. Workers talk to the loop only through
//! [`NeruHintsEvent`], so the loop never waits on neru (§V.6).

// The worker side runs only on Unix clients (`client::neru_hints`); Windows uses the request
// flag and the notice.
#![cfg_attr(windows, allow(dead_code))]

use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::neru_calibration::{
    calibrate, motion_pixels, NeruCalibrated, NeruCalibrationHost, NeruCalibrationOutcome, NeruCli,
    NeruCommandError, ScreenPoint, POINTER_NOT_RESTORED,
};
use super::neru_targets::{collect, to_targets, NeruCollectError, NeruScreenRect, NeruTargets};
use super::{
    ClientEndpointNoticeKey, ClientEndpointNoticeKind, ClientShellState,
    ClientVisibleEndpointNotice,
};

/// How long a neru notice stays on screen; the same as other unavailable notices.
const NOTICE_DURATION: Duration = Duration::from_secs(8);

/// The oldest neru with `hints --targets` (§C).
const MIN_NERU_VERSION: (u32, u32, u32) = (1, 58, 0);
/// How long the worker waits for the loop to turn on pixel reporting.
const START_REPORTS_TIMEOUT: Duration = Duration::from_secs(1);

/// What a worker sends the client loop.
pub(crate) enum NeruHintsEvent {
    /// Calibration is about to warp. The loop turns on pixel reporting, sends every host mouse
    /// report to `reports` from then on (§V.14), and answers on `ack`.
    StartReports {
        reports: mpsc::Sender<Vec<u8>>,
        ack: mpsc::SyncSender<()>,
    },
    /// Calibration ended, also after a worker panic. The loop stops routing reports to the
    /// calibrator and restores its modes, then hands this to
    /// [`ClientShellState::finish_neru_calibration`].
    Calibrated(NeruCalibrationDone),
    /// `neru hints` exited, with the message to report when it failed.
    HintsExited(Result<(), String>),
}

/// A finished calibration, with the grid it measured.
pub(crate) struct NeruCalibrationDone {
    /// `Err` holds the message for a failure before calibration started (neru missing or too
    /// old) or a worker that stopped.
    outcome: Result<NeruCalibrationOutcome, String>,
    grid: (u16, u16),
    extent_px: (u32, u32),
}

#[cfg(test)]
impl NeruCalibrationDone {
    pub(super) fn for_test(
        outcome: Result<NeruCalibrationOutcome, String>,
        grid: (u16, u16),
        extent_px: (u32, u32),
    ) -> Self {
        Self {
            outcome,
            grid,
            extent_px,
        }
    }
}

/// Why neru hints cannot run here, checked before any process starts (design section 8,
/// steps 1 to 5, §V.7). `env` reads the client's environment.
pub(crate) fn neru_unavailable_reason(
    macos: bool,
    remote_client: bool,
    env: impl Fn(&str) -> Option<String>,
    mouse_capture: bool,
    pixel_mouse: bool,
) -> Option<String> {
    if let Some(reason) = neru_host_unavailable_reason(macos, remote_client, env) {
        return Some(reason);
    }
    if !mouse_capture {
        return Some("neru hints needs mouse capture (ui.mouse_capture).".into());
    }
    if !pixel_mouse {
        return Some(
            "neru hints needs a terminal that reports pixel mouse positions (kitty, Ghostty)."
                .into(),
        );
    }
    None
}

/// Design section 8 steps 1 to 3: what the client's host decides for the whole session. A
/// client sets the `neru_hints` hello flag only when this is `None` (§V.23).
pub(crate) fn neru_host_unavailable_reason(
    macos: bool,
    remote_client: bool,
    env: impl Fn(&str) -> Option<String>,
) -> Option<String> {
    let set = |name: &str| env(name).is_some_and(|value| !value.is_empty());
    if !macos {
        return Some("neru hints needs macOS.".into());
    }
    if remote_client || set("SSH_CONNECTION") || set("SSH_TTY") {
        return Some(
            "neru hints is unavailable over SSH. Run herdr on the Mac that shows the terminal."
                .into(),
        );
    }
    let multiplexer = [
        ("TMUX", "tmux"),
        ("STY", "screen"),
        ("ZELLIJ", "zellij"),
        (crate::HERDR_ENV_VAR, "herdr"),
    ]
    .into_iter()
    .find(|(name, _)| set(name));
    multiplexer.map(|(_, name)| format!("neru hints is unavailable inside {name}."))
}

/// Checks neru's version, then calibrates on this thread. `send` delivers events to the client
/// loop and returns false once the loop is gone. A [`NeruHintsEvent::Calibrated`] is always
/// sent, also when calibration panics, so the loop never keeps swallowing mouse reports.
pub(crate) fn run_neru_calibration(
    path: String,
    grid: (u16, u16),
    extent_px: (u32, u32),
    send: impl Fn(NeruHintsEvent) -> bool + Clone,
) {
    let mut done = DoneGuard {
        send: send.clone(),
        done: Some(NeruCalibrationDone {
            outcome: Err("neru hints stopped unexpectedly.".into()),
            grid,
            extent_px,
        }),
    };
    let cli = NeruCli::new(path);
    let outcome = match cli.version() {
        Err(error) => Err(error.to_string()),
        Ok(version) if version < MIN_NERU_VERSION => {
            let (major, minor, patch) = version;
            Err(format!(
                "neru {major}.{minor}.{patch} is too old. neru hints needs 1.58.0 or later."
            ))
        }
        Ok(_) => {
            let mut host = ChannelHost {
                cli,
                send,
                reports: None,
            };
            Ok(calibrate(&mut host, grid, extent_px))
        }
    };
    if let Some(done) = done.done.as_mut() {
        done.outcome = outcome;
    }
}

/// Starts hint mode and reports how `neru hints` exited.
pub(crate) fn run_neru_hints(path: String, targets_json: String) -> NeruHintsEvent {
    NeruHintsEvent::HintsExited(
        NeruCli::new(path)
            .hints(targets_json)
            .map_err(|error| error.to_string()),
    )
}

/// Sends the calibration result when the worker ends, on return or unwind.
struct DoneGuard<F: Fn(NeruHintsEvent) -> bool> {
    send: F,
    done: Option<NeruCalibrationDone>,
}

impl<F: Fn(NeruHintsEvent) -> bool> Drop for DoneGuard<F> {
    fn drop(&mut self) {
        if let Some(done) = self.done.take() {
            (self.send)(NeruHintsEvent::Calibrated(done));
        }
    }
}

/// Calibration's view of neru and the host: neru through its CLI, mouse reports through the
/// channel the client loop fills.
struct ChannelHost<F> {
    cli: NeruCli,
    send: F,
    reports: Option<mpsc::Receiver<Vec<u8>>>,
}

impl<F: Fn(NeruHintsEvent) -> bool> NeruCalibrationHost for ChannelHost<F> {
    fn query_window(&mut self) -> Result<NeruScreenRect, NeruCommandError> {
        self.cli.query_window()
    }

    fn query_cursor(&mut self) -> Result<ScreenPoint, NeruCommandError> {
        self.cli.query_cursor()
    }

    fn query_displays(&mut self) -> Result<Vec<NeruScreenRect>, NeruCommandError> {
        self.cli.query_displays()
    }

    fn move_mouse(&mut self, point: ScreenPoint) -> Result<(), NeruCommandError> {
        self.cli.move_mouse(point)
    }

    fn start_reports(&mut self) {
        let (reports, receiver) = mpsc::channel();
        let (ack, acked) = mpsc::sync_channel(1);
        self.reports = Some(receiver);
        if (self.send)(NeruHintsEvent::StartReports { reports, ack }) {
            // A loop that never answers leaves no reports; calibration then fails as NoReport.
            let _ = acked.recv_timeout(START_REPORTS_TIMEOUT);
        }
    }

    fn drain_reports(&mut self) {
        if let Some(reports) = &self.reports {
            while reports.try_recv().is_ok() {}
        }
    }

    fn next_motion(&mut self, timeout: Duration) -> Option<(u32, u32)> {
        next_motion(self.reports.as_ref()?, timeout)
    }
}

/// The first plain-motion report on `reports` within `timeout`. Other reports are skipped.
fn next_motion(reports: &mpsc::Receiver<Vec<u8>>, timeout: Duration) -> Option<(u32, u32)> {
    let deadline = Instant::now() + timeout;
    loop {
        let left = deadline.checked_duration_since(Instant::now())?;
        if let Some(pixels) = motion_pixels(&reports.recv_timeout(left).ok()?) {
            return Some(pixels);
        }
    }
}

impl ClientShellState {
    /// True once per press of the neru hints keybinding.
    pub(crate) fn take_neru_hints_request(&mut self) -> bool {
        std::mem::take(&mut self.neru_hints_requested)
    }

    /// The binary `neru.path` names.
    pub(crate) fn neru_path(&self) -> &str {
        &self.config.neru_path
    }

    /// Design section 8, step 6: a popup takes every click, and no controls means nothing to
    /// label. Checked before any process starts.
    pub(crate) fn neru_precheck(&self) -> Result<(), String> {
        match collect(&self.hits, &self.neru_layer_state()) {
            Err(NeruCollectError::PopupOpen) => Err("Close the popup first.".into()),
            Ok(controls) if controls.is_empty() => Err("No clickable controls on screen.".into()),
            Ok(_) => Ok(()),
        }
    }

    /// Reports a neru hints message as a client notice. Returns true when it needs a repaint.
    /// Every press reports its reason (§V.16), so this skips the once-per-boot filter that
    /// `push_endpoint_notice` applies. A timed-out endpoint's notice still takes precedence.
    pub(crate) fn show_neru_notice(&mut self, message: impl Into<String>) -> bool {
        if self.timed_out_endpoint.is_some() {
            return false;
        }
        let message = message.into();
        self.visible_endpoint_notice = Some(ClientVisibleEndpointNotice {
            key: ClientEndpointNoticeKey {
                boot_id: "neru".into(),
                kind: ClientEndpointNoticeKind::Unavailable,
                code: format!("neru_hints:{message}"),
            },
            title: "neru hints".into(),
            body: message,
            deadline: Instant::now() + NOTICE_DURATION,
        });
        true
    }

    /// Turns a finished calibration into the targets JSON for `neru hints`, or `None` after
    /// reporting why there is none. `grid` and `extent_px` are the client's size now: the
    /// targets come from the current hit map, and a size change since calibration started
    /// means the measured cell pitch no longer holds (§V.4, §V.5).
    pub(crate) fn finish_neru_calibration(
        &mut self,
        done: NeruCalibrationDone,
        grid: (u16, u16),
        extent_px: (u32, u32),
    ) -> (Option<String>, bool) {
        let mut repaint = false;
        let outcome = match done.outcome {
            Ok(outcome) => outcome,
            Err(message) => return (None, self.show_neru_notice(message)),
        };
        let restored = outcome.pointer_restored;
        if !restored {
            repaint |= self.show_neru_notice(POINTER_NOT_RESTORED);
        }
        // The notice slot holds one message, so a later notice repeats the restore note (§V.8).
        let note = |message: String| match restored {
            true => message,
            false => format!("{message} {POINTER_NOT_RESTORED}"),
        };
        let calibrated = match outcome.result {
            Ok(calibrated) => calibrated,
            Err(error) => {
                return (
                    None,
                    repaint | self.show_neru_notice(note(error.to_string())),
                )
            }
        };
        if (grid, extent_px) != (done.grid, done.extent_px) {
            let message = "Calibration failed: the terminal resized. Try again.";
            return (None, repaint | self.show_neru_notice(note(message.into())));
        }
        // A notice pushed above draws a toast over the chrome. Compose first, so the hit map has
        // the toast and no target sits under it (§V.1, design section 4.1).
        let Some(targets) = self.neru_targets_now(&calibrated, grid, repaint) else {
            return (
                None,
                repaint | self.show_neru_notice(note("Close the popup first.".into())),
            );
        };
        if targets.sent == 0 {
            let message = "No clickable controls on screen.";
            return (None, repaint | self.show_neru_notice(note(message.into())));
        }
        if targets.sent == targets.total {
            return (Some(targets.json), repaint);
        }
        let message = format!(
            "neru hints: sent {} of {} controls.",
            targets.sent, targets.total
        );
        repaint |= self.show_neru_notice(note(message));
        match self.neru_targets_now(&calibrated, grid, repaint) {
            Some(targets) if targets.sent > 0 => (Some(targets.json), repaint),
            _ => (None, repaint),
        }
    }

    /// The targets for the current state, composed first when a notice changed the screen.
    /// `None` when a popup is open.
    fn neru_targets_now(
        &mut self,
        calibrated: &NeruCalibrated,
        grid: (u16, u16),
        recompose: bool,
    ) -> Option<NeruTargets> {
        if recompose {
            let _ = self.compose(grid.0, grid.1);
        }
        let controls = collect(&self.hits, &self.neru_layer_state()).ok()?;
        Some(to_targets(
            &controls,
            &calibrated.calibration,
            &calibrated.displays,
        ))
    }
}

#[cfg(test)]
mod tests;
