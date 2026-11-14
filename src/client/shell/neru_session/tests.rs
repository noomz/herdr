use std::cell::RefCell;
use std::rc::Rc;

use super::*;

fn reason(env: &[(&str, &str)], mouse_capture: bool, pixel_mouse: bool) -> Option<String> {
    reason_on(true, false, env, mouse_capture, pixel_mouse)
}

fn reason_on(
    macos: bool,
    remote_client: bool,
    env: &[(&str, &str)],
    mouse_capture: bool,
    pixel_mouse: bool,
) -> Option<String> {
    neru_unavailable_reason(
        macos,
        remote_client,
        |name| {
            env.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        },
        mouse_capture,
        pixel_mouse,
    )
}

#[test]
fn a_local_mac_client_with_pixel_mouse_is_available() {
    assert_eq!(reason(&[("TERM", "xterm-kitty")], true, true), None);
}

#[test]
fn unavailable_cases_give_the_design_messages_in_order() {
    assert_eq!(
        reason_on(false, true, &[("TMUX", "/tmp/t")], false, false).as_deref(),
        Some("neru hints needs macOS.")
    );
    let ssh = Some(
        "neru hints is unavailable over SSH. Run herdr on the Mac that shows the terminal.".into(),
    );
    assert_eq!(reason(&[("SSH_CONNECTION", "1 2 3 4")], true, true), ssh);
    assert_eq!(reason(&[("SSH_TTY", "/dev/ttys001")], true, true), ssh);
    assert_eq!(
        reason_on(true, true, &[], true, true),
        ssh,
        "herdr --remote"
    );
    for (name, multiplexer) in [
        ("TMUX", "tmux"),
        ("STY", "screen"),
        ("ZELLIJ", "zellij"),
        (crate::HERDR_ENV_VAR, "herdr"),
    ] {
        assert_eq!(
            reason(&[(name, "1")], true, true),
            Some(format!("neru hints is unavailable inside {multiplexer}.")),
        );
    }
    assert_eq!(
        reason(&[("TMUX", "")], true, true),
        None,
        "an empty variable is unset"
    );
    assert_eq!(
        reason(&[], false, false).as_deref(),
        Some("neru hints needs mouse capture (ui.mouse_capture).")
    );
    assert_eq!(
        reason(&[], true, false).as_deref(),
        Some("neru hints needs a terminal that reports pixel mouse positions (kitty, Ghostty).")
    );
}

#[test]
fn next_motion_skips_other_reports_and_times_out() {
    let (send, reports) = mpsc::channel();
    send.send(b"\x1b[<0;10;10M".to_vec()).unwrap();
    send.send(b"a".to_vec()).unwrap();
    send.send(b"\x1b[<35;440;465M".to_vec()).unwrap();
    assert_eq!(
        next_motion(&reports, Duration::from_millis(50)),
        Some((440, 465))
    );
    assert_eq!(next_motion(&reports, Duration::from_millis(10)), None);
    drop(send);
    assert_eq!(next_motion(&reports, Duration::from_millis(10)), None);
}

/// Collects what a worker sends the loop.
fn recorder() -> (
    impl Fn(NeruHintsEvent) -> bool + Clone,
    Rc<RefCell<Vec<NeruHintsEvent>>>,
) {
    let events = Rc::new(RefCell::new(Vec::new()));
    let sink = events.clone();
    (
        move |event| {
            sink.borrow_mut().push(event);
            true
        },
        events,
    )
}

fn calibration_message(event: &NeruHintsEvent) -> &str {
    match event {
        NeruHintsEvent::Calibrated(NeruCalibrationDone {
            outcome: Err(message),
            ..
        }) => message,
        _ => panic!("expected a failed calibration"),
    }
}

#[test]
fn a_missing_neru_ends_calibration_before_any_mode_change() {
    let (send, events) = recorder();
    run_neru_calibration(
        "/nonexistent/herdr-test/neru".into(),
        (80, 24),
        (640, 384),
        send,
    );
    let events = events.borrow();
    assert_eq!(events.len(), 1, "no StartReports");
    assert_eq!(
        calibration_message(&events[0]),
        "neru not found at \"/nonexistent/herdr-test/neru\". Install neru 1.58.0 or later, or set neru.path."
    );
}

#[cfg(unix)]
#[test]
fn an_old_neru_is_refused_before_calibration() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("herdr-neru-session-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("neru");
    std::fs::write(&path, "#!/bin/sh\necho 'Neru version v1.57.9'\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();

    let (send, events) = recorder();
    run_neru_calibration(path.display().to_string(), (80, 24), (640, 384), send);

    let events = events.borrow();
    assert_eq!(events.len(), 1, "no StartReports");
    assert_eq!(
        calibration_message(&events[0]),
        "neru 1.57.9 is too old. neru hints needs 1.58.0 or later."
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_worker_that_panics_still_ends_calibration() {
    let (send, events) = recorder();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _done = DoneGuard {
            send,
            done: Some(NeruCalibrationDone::for_test(
                Err("neru hints stopped unexpectedly.".into()),
                (80, 24),
                (640, 384),
            )),
        };
        panic!("calibration bug");
    }));
    assert!(result.is_err());
    let events = events.borrow();
    assert_eq!(events.len(), 1);
    assert_eq!(
        calibration_message(&events[0]),
        "neru hints stopped unexpectedly."
    );
}
