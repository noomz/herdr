//! The neru hints client action on the client loop (`SPEC-neru-hints.md` §T.5).
//!
//! The loop checks availability, then a worker checks neru's version and calibrates
//! (`shell::run_neru_calibration`). While calibration has pixel reporting on, every host mouse
//! report goes to the calibrator and none reaches the shell (§V.14). When calibration ends, the
//! loop restores its mouse modes, builds the targets from the current hit map (§V.5), and a
//! second worker runs `neru hints`. The loop never waits on neru (§V.6).

use std::io::{self, Write};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;

use tracing::warn;

use super::events::ClientLoopEvent;
use super::neru_modes::{enter_neru_calibration_modes, restore_neru_calibration_modes};
use super::shell::{self, NeruHintsEvent};
use super::state::ClientState;
use super::terminal_geometry::ioctl_extent;
use crate::raw_input::RawInputEvent;

#[derive(Default)]
pub(super) struct NeruHints {
    phase: Phase,
}

#[derive(Default)]
enum Phase {
    #[default]
    Idle,
    /// A worker runs calibration. `reports` is set once pixel reporting is on.
    Calibrating {
        reports: Option<mpsc::Sender<Vec<u8>>>,
    },
    /// `neru hints` is starting.
    Hints,
}

impl NeruHints {
    /// Runs the action for a press of its keybinding.
    pub(super) fn start(
        &mut self,
        state: &mut ClientState,
        remote_client: bool,
        event_tx: &tokio::sync::mpsc::Sender<ClientLoopEvent>,
    ) {
        let repaint = self.try_start(state, remote_client, event_tx);
        present_chrome(state, repaint);
    }

    fn try_start(
        &mut self,
        state: &mut ClientState,
        remote_client: bool,
        event_tx: &tokio::sync::mpsc::Sender<ClientLoopEvent>,
    ) -> bool {
        let pixel_mouse = state.pixel_mouse_available();
        let mouse_capture = state.mouse_capture_active;
        let reported_size = state.reported_size;
        let Some(shell) = state.shell.as_mut() else {
            return false;
        };
        let refusal = if matches!(self.phase, Phase::Idle) {
            shell::neru_unavailable_reason(
                crate::platform::supports_neru_hints(),
                remote_client,
                |name| std::env::var(name).ok(),
                mouse_capture,
                pixel_mouse,
            )
            .or_else(|| shell.neru_precheck().err())
        } else {
            Some("neru hints is already running.".to_owned())
        };
        if let Some(message) = refusal {
            return shell.show_neru_notice(message);
        }
        let Some((grid, extent_px)) = ioctl_extent().filter(|(grid, _)| *grid == reported_size)
        else {
            return shell
                .show_neru_notice("Calibration failed: the terminal did not report its size.");
        };
        let path = shell.neru_path().to_owned();
        let event_tx = event_tx.clone();
        self.phase = Phase::Calibrating { reports: None };
        std::thread::spawn(move || {
            shell::run_neru_calibration(path, grid, extent_px, move |event| {
                event_tx
                    .blocking_send(ClientLoopEvent::NeruHints(event))
                    .is_ok()
            });
        });
        false
    }

    /// True while host mouse reports belong to the calibrator (§V.14).
    pub(super) fn owns_mouse_reports(&self) -> bool {
        matches!(&self.phase, Phase::Calibrating { reports: Some(_) })
    }

    /// Hands a host pixel report to the calibrator. True when it took the report, which then
    /// must not reach the shell (§V.14).
    pub(super) fn take_pixel_report(&self, report: &[u8]) -> bool {
        let Phase::Calibrating {
            reports: Some(reports),
        } = &self.phase
        else {
            return false;
        };
        // A calibrator that already finished has dropped its end; the report is still swallowed.
        let _ = reports.send(report.to_vec());
        true
    }

    /// Handles a worker's event.
    pub(super) fn handle(
        &mut self,
        event: NeruHintsEvent,
        state: &mut ClientState,
        host_mouse_capture_active: &AtomicBool,
        host_sgr_pixels_active: &AtomicBool,
        event_tx: &tokio::sync::mpsc::Sender<ClientLoopEvent>,
    ) {
        let repaint = self.handle_event(
            event,
            state,
            host_mouse_capture_active,
            host_sgr_pixels_active,
            event_tx,
        );
        present_chrome(state, repaint);
    }

    fn handle_event(
        &mut self,
        event: NeruHintsEvent,
        state: &mut ClientState,
        host_mouse_capture_active: &AtomicBool,
        host_sgr_pixels_active: &AtomicBool,
        event_tx: &tokio::sync::mpsc::Sender<ClientLoopEvent>,
    ) -> bool {
        match event {
            NeruHintsEvent::StartReports { reports, ack } => {
                if let Phase::Calibrating { reports: slot } = &mut self.phase {
                    state.neru_calibrating = true;
                    let mut stdout = io::stdout();
                    if let Err(error) = enter_neru_calibration_modes(
                        &mut stdout,
                        host_mouse_capture_active,
                        host_sgr_pixels_active,
                    )
                    .and_then(|()| stdout.flush())
                    {
                        warn!(%error, "neru hints could not turn on pixel mouse reports");
                    }
                    *slot = Some(reports);
                }
                let _ = ack.send(());
                false
            }
            NeruHintsEvent::Calibrated(done) => {
                let modes_entered = self.owns_mouse_reports();
                self.phase = Phase::Idle;
                if modes_entered {
                    state.neru_calibrating = false;
                    let mut stdout = io::stdout();
                    if let Err(error) = restore_neru_calibration_modes(
                        &mut stdout,
                        state,
                        host_mouse_capture_active,
                        host_sgr_pixels_active,
                    )
                    .and_then(|()| stdout.flush())
                    {
                        warn!(%error, "neru hints could not restore mouse reports");
                    }
                }
                // A grid that no longer matches the hit map reads as a resize.
                let (grid, extent_px) = ioctl_extent()
                    .filter(|(grid, _)| *grid == state.reported_size)
                    .unwrap_or((state.reported_size, (0, 0)));
                let Some(shell) = state.shell.as_mut() else {
                    return false;
                };
                let (targets, repaint) = shell.finish_neru_calibration(done, grid, extent_px);
                if let Some(targets) = targets {
                    self.phase = Phase::Hints;
                    let path = shell.neru_path().to_owned();
                    let event_tx = event_tx.clone();
                    std::thread::spawn(move || {
                        let exited = shell::run_neru_hints(path, targets);
                        let _ = event_tx.blocking_send(ClientLoopEvent::NeruHints(exited));
                    });
                }
                repaint
            }
            NeruHintsEvent::HintsExited(result) => {
                self.phase = Phase::Idle;
                match (result, state.shell.as_mut()) {
                    (Err(message), Some(shell)) => shell.show_neru_notice(message),
                    _ => false,
                }
            }
        }
    }
}

/// Draws a changed notice. neru hints changes only chrome, never pane content.
fn present_chrome(state: &mut ClientState, repaint: bool) {
    let (columns, rows) = state.reported_size;
    if let Some(frame) = repaint
        .then(|| state.shell.as_mut()?.compose(columns, rows))
        .flatten()
    {
        state.present_frozen_chrome(frame);
    }
}

/// Host input with its mouse reports removed. Keys typed during calibration still reach the
/// shell; mouse reports do not (§V.14).
pub(super) fn without_mouse_reports(events: Vec<RawInputEvent>) -> Vec<RawInputEvent> {
    events
        .into_iter()
        .filter(|event| !matches!(event, RawInputEvent::Mouse(_)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixel_reports_go_to_the_calibrator_only_once_reporting_is_on() {
        let mut hints = NeruHints::default();
        assert!(!hints.take_pixel_report(b"\x1b[<35;1;1M"), "idle");

        hints.phase = Phase::Calibrating { reports: None };
        assert!(
            !hints.take_pixel_report(b"\x1b[<35;1;1M"),
            "before pixel reporting is on, reports stay with the shell"
        );

        let (reports, calibrator) = mpsc::channel();
        hints.phase = Phase::Calibrating {
            reports: Some(reports),
        };
        assert!(hints.owns_mouse_reports());
        assert!(hints.take_pixel_report(b"\x1b[<35;440;465M"));
        assert_eq!(calibrator.try_recv().unwrap(), b"\x1b[<35;440;465M");

        drop(calibrator);
        assert!(
            hints.take_pixel_report(b"\x1b[<35;1;1M"),
            "the warp-back report is swallowed after the calibrator returns"
        );
    }

    #[test]
    fn a_request_while_neru_hints_runs_does_what_a_second_press_does() {
        // A request (`herdr neru-hints`) and a key press both reach `start` through the
        // request flag (§V.21), so a second one during calibration or `neru hints` is refused.
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(1);
        for phase in [Phase::Calibrating { reports: None }, Phase::Hints] {
            let mut state = ClientState::test_new();
            let mut shell = crate::client::ClientShellState::new(
                crate::client::ClientShellConfig::from_config(&crate::config::Config::default()),
            );
            let mut outcome = shell::ClientShellInput::default();
            shell.request_neru_hints(
                &crate::client::endpoint::ClientEndpointId::Local,
                &mut outcome,
            );
            state.shell = Some(shell);
            let mut hints = NeruHints { phase };

            let shell = state.shell.as_mut().unwrap();
            assert!(shell.take_neru_hints_request());
            hints.start(&mut state, false, &event_tx);

            assert_eq!(
                state.shell.as_ref().unwrap().visible_notice_body(),
                Some("neru hints is already running.")
            );
            assert!(!matches!(hints.phase, Phase::Idle));
        }
    }

    #[test]
    fn mode_writers_leave_the_host_modes_to_calibration() {
        use std::sync::atomic::Ordering;
        let mut state = ClientState::test_new();
        state.mouse_capture_active = false;
        state.shell_mouse_capture_preference = true;
        state.neru_calibrating = true;
        let capture = AtomicBool::new(true);
        let pixels = AtomicBool::new(true);

        super::super::shell_runtime::clear_endpoint_host_effects(&mut state, &capture, &pixels);

        assert!(state.mouse_capture_active, "client state still updates");
        assert!(
            capture.load(Ordering::Acquire),
            "calibration flags untouched"
        );
        assert!(pixels.load(Ordering::Acquire));
    }

    #[test]
    fn calibration_strips_mouse_reports_and_keeps_keys() {
        let events = crate::raw_input::parse_framed_input(b"\x1b[<35;10;5Ma");
        assert!(events
            .iter()
            .any(|event| matches!(event, RawInputEvent::Mouse(_))));
        let kept = without_mouse_reports(events);
        assert_eq!(kept.len(), 1, "{kept:?}");
        assert!(!matches!(kept[0], RawInputEvent::Mouse(_)));
    }
}
