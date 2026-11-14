//! Host mouse modes around neru calibration (`SPEC-neru-hints.md` §T.4, §V.8, §V.12).
//!
//! Both functions write to the host terminal, so the main loop calls them, on the thread that
//! writes frames. A calibration worker that wrote modes itself could split a frame's escape
//! sequence.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

use super::state::ClientState;
use super::terminal_setup::{effective_sgr_pixel_mouse, write_mouse_capture};

/// Turns on any-motion SGR pixel reporting for calibration.
///
/// Until [`restore_neru_calibration_modes`] runs, the main loop's other mode writers (the redraw
/// refresh, the `MouseCapture` handler, `apply_reload`) must update client state only and not
/// write modes. A write there would turn pixel reporting off mid-calibration. Restore reads that
/// state.
pub(super) fn enter_neru_calibration_modes<W: io::Write>(
    writer: &mut W,
    host_mouse_capture_active: &AtomicBool,
    host_sgr_pixels_active: &AtomicBool,
) -> io::Result<()> {
    // Store the flags before the host can send a pixel report, so the input thread never
    // reads one as a cell report. The calibrator drains every report queued before its first
    // warp (§V.14), including cell reports read as pixels in this gap.
    host_mouse_capture_active.store(true, Ordering::Release);
    host_sgr_pixels_active.store(true, Ordering::Release);
    write_mouse_capture(writer, true, true)
}

/// Re-sends the full mode set the client needs now, after calibration succeeds or fails
/// (§V.8, §V.12). It recomputes the set instead of replaying a snapshot, because a pane can
/// request pixel mode while calibration runs.
pub(super) fn restore_neru_calibration_modes<W: io::Write>(
    writer: &mut W,
    state: &ClientState,
    host_mouse_capture_active: &AtomicBool,
    host_sgr_pixels_active: &AtomicBool,
) -> io::Result<()> {
    let enabled = state.mouse_capture_active;
    let sgr_pixels = effective_sgr_pixel_mouse(
        enabled,
        state.endpoint_sgr_pixels_requested,
        state.pixel_mouse_available(),
    );
    let written = write_mouse_capture(writer, enabled, sgr_pixels);
    host_mouse_capture_active.store(enabled, Ordering::Release);
    host_sgr_pixels_active.store(sgr_pixels, Ordering::Release);
    written
}

#[cfg(test)]
mod tests {
    use super::*;

    const SGR_ON: &str = "\x1b[?1006h";
    const PIXELS_ON: &str = "\x1b[?1016h";
    const PIXELS_OFF: &str = "\x1b[?1016l";
    const ANY_MOTION_ON: &str = "\x1b[?1003h";

    fn shell_state(pixel_requested: bool) -> ClientState {
        let mut state = ClientState::test_new();
        state.mouse_capture_active = true;
        state.pixel_geometry_exact = true;
        state.host_sgr_pixel_mouse = Some(true);
        state.endpoint_sgr_pixels_requested = pixel_requested;
        state
    }

    fn written(write: impl FnOnce(&mut Vec<u8>) -> io::Result<()>) -> String {
        let mut bytes = Vec::new();
        write(&mut bytes).expect("writing to a Vec succeeds");
        String::from_utf8(bytes).expect("mode sequences are ASCII")
    }

    #[test]
    fn calibration_turns_on_any_motion_sgr_pixel_reports_and_flags_them_first() {
        let capture = AtomicBool::new(false);
        let pixels = AtomicBool::new(false);
        let out = written(|w| enter_neru_calibration_modes(w, &capture, &pixels));
        let pixels_on = out.rfind(PIXELS_ON).expect("pixel mode on");
        assert!(out.rfind(ANY_MOTION_ON).is_some_and(|at| at < pixels_on));
        assert!(out.rfind(SGR_ON).is_some_and(|at| at < pixels_on));
        assert!(capture.load(Ordering::Acquire));
        assert!(pixels.load(Ordering::Acquire));
    }

    #[test]
    fn restore_resends_sgr_after_turning_pixel_mode_off() {
        let capture = AtomicBool::new(true);
        let pixels = AtomicBool::new(true);
        let state = shell_state(false);
        let out = written(|w| restore_neru_calibration_modes(w, &state, &capture, &pixels));
        let last_off = out.rfind(PIXELS_OFF).expect("pixel mode turned off");
        assert!(
            out[last_off..].contains(SGR_ON),
            "SGR re-sent after ?1016l: {out:?}"
        );
        assert!(out[last_off..].contains(ANY_MOTION_ON));
        assert!(!out.contains(PIXELS_ON));
        assert!(capture.load(Ordering::Acquire));
        assert!(!pixels.load(Ordering::Acquire));
    }

    #[test]
    fn restore_keeps_pixel_mode_a_pane_requested() {
        let capture = AtomicBool::new(true);
        let pixels = AtomicBool::new(true);
        let state = shell_state(true);
        let out = written(|w| restore_neru_calibration_modes(w, &state, &capture, &pixels));
        assert!(out.ends_with(PIXELS_ON), "{out:?}");
        assert!(pixels.load(Ordering::Acquire));
    }

    #[test]
    fn restore_turns_capture_off_when_the_client_had_it_off() {
        let capture = AtomicBool::new(true);
        let pixels = AtomicBool::new(true);
        let mut state = shell_state(false);
        state.mouse_capture_active = false;
        let out = written(|w| restore_neru_calibration_modes(w, &state, &capture, &pixels));
        assert!(!out.contains(ANY_MOTION_ON));
        assert!(!capture.load(Ordering::Acquire));
        assert!(!pixels.load(Ordering::Acquire));
    }
}
