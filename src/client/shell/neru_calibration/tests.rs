use std::collections::VecDeque;

use super::*;

const MAIN: NeruScreenRect = rect(0, 0, 1920, 1280);
/// The built-in display below and left of the main one, as in the T1 setup.
const BUILT_IN: NeruScreenRect = rect(-1728, 1280, 1728, 1117);

const fn rect(x: i32, y: i32, width: i32, height: i32) -> NeruScreenRect {
    NeruScreenRect {
        x,
        y,
        width,
        height,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Step {
    Query(&'static str),
    StartReports,
    Drain,
    Move(ScreenPoint),
    Wait,
}

/// A host terminal and neru, as T1 measured them. neru warps only onto display 0 and moves a
/// point elsewhere to that display's nearest edge (finding 12). Only `terminal`, the part of
/// the window this client draws in, reports motion (finding 9).
struct FakeHost {
    window: NeruScreenRect,
    terminal: NeruScreenRect,
    /// Grid origin in points and device pixels per point.
    origin: (f64, f64),
    scale: f64,
    displays: Vec<NeruScreenRect>,
    cursor: ScreenPoint,
    reporting: bool,
    reports: VecDeque<(u32, u32)>,
    /// Someone moves the pointer by this much after the next warp.
    nudge: Option<ScreenPoint>,
    /// Every report has this pixel position, whatever the pointer does.
    frozen_report: Option<(u32, u32)>,
    window_query: Result<NeruScreenRect, NeruCommandError>,
    log: Vec<Step>,
}

impl FakeHost {
    /// A kitty window at scale 2 whose grid starts 10 × 9.5 pt inside its frame.
    fn kitty() -> Self {
        let window = rect(647, 73, 1263, 1150);
        Self {
            window,
            terminal: window,
            origin: (657.0, 82.5),
            scale: 2.0,
            displays: vec![MAIN, BUILT_IN],
            cursor: (1280, 996),
            reporting: false,
            reports: VecDeque::new(),
            nudge: None,
            frozen_report: None,
            window_query: Ok(window),
            log: Vec::new(),
        }
    }

    fn moves(&self) -> Vec<ScreenPoint> {
        self.log
            .iter()
            .filter_map(|step| match step {
                Step::Move(point) => Some(*point),
                _ => None,
            })
            .collect()
    }

    fn land(&self, (x, y): ScreenPoint) -> ScreenPoint {
        let display = self.displays[0];
        (
            x.clamp(display.x, display.x + display.width - 1),
            y.clamp(display.y, display.y + display.height - 1),
        )
    }
}

impl NeruCalibrationHost for FakeHost {
    fn query_window(&mut self) -> Result<NeruScreenRect, NeruCommandError> {
        self.log.push(Step::Query("window"));
        self.window_query.clone()
    }

    fn query_cursor(&mut self) -> Result<ScreenPoint, NeruCommandError> {
        self.log.push(Step::Query("cursor"));
        Ok(self.cursor)
    }

    fn query_displays(&mut self) -> Result<Vec<NeruScreenRect>, NeruCommandError> {
        self.log.push(Step::Query("displays"));
        Ok(self.displays.clone())
    }

    fn move_mouse(&mut self, point: ScreenPoint) -> Result<(), NeruCommandError> {
        self.log.push(Step::Move(point));
        let landed = self.land(point);
        if self.reporting && self.terminal.contains(landed) {
            let pixel = |at: i32, origin: f64| ((f64::from(at) - origin) * self.scale) as u32;
            let report = self.frozen_report.unwrap_or((
                pixel(landed.0, self.origin.0),
                pixel(landed.1, self.origin.1),
            ));
            self.reports.push_back(report);
        }
        self.cursor = match self.nudge.take() {
            Some((dx, dy)) => (landed.0 + dx, landed.1 + dy),
            None => landed,
        };
        Ok(())
    }

    fn start_reports(&mut self) {
        self.log.push(Step::StartReports);
        self.reporting = true;
    }

    fn drain_reports(&mut self) {
        self.log.push(Step::Drain);
        self.reports.clear();
    }

    fn next_motion(&mut self, _timeout: Duration) -> Option<(u32, u32)> {
        self.log.push(Step::Wait);
        self.reports.pop_front()
    }
}

/// The grid the fake host draws, for `columns` × `rows` cells of 18 × 43 device pixels.
fn expected(host: &FakeHost, grid: (u16, u16)) -> NeruCalibration {
    NeruCalibration::new(grid, extent(grid), host.origin, (host.scale, host.scale)).unwrap()
}

fn extent((columns, rows): (u16, u16)) -> (u32, u32) {
    (u32::from(columns) * 18, u32::from(rows) * 43)
}

const GRID: (u16, u16) = (138, 52);

fn run(host: &mut FakeHost) -> NeruCalibrationOutcome {
    calibrate(host, GRID, extent(GRID))
}

#[test]
fn measures_origin_and_scale_from_two_pixel_reports() {
    for (scale, origin) in [(2.0, (657.0, 82.5)), (1.0, (649.0, 107.0))] {
        let mut host = FakeHost::kitty();
        host.scale = scale;
        host.origin = origin;
        let start = host.cursor;
        let outcome = run(&mut host);
        assert_eq!(
            outcome.result,
            Ok(NeruCalibrated {
                calibration: expected(&host, GRID),
                displays: vec![MAIN, BUILT_IN],
            }),
            "scale {scale}"
        );
        assert!(outcome.pointer_restored);
        assert_eq!(host.cursor, start);
        // The center reported, so one warp, the second point and the warp back.
        assert_eq!(host.moves().len(), 3);
    }
}

#[test]
fn queries_neru_before_reports_start_and_drains_before_every_warp() {
    let mut host = FakeHost::kitty();
    let start = host.cursor;
    run(&mut host);
    let start_reports = host.log.iter().position(|step| *step == Step::StartReports);
    assert_eq!(start_reports, Some(3));
    assert!(host.log[..3]
        .iter()
        .all(|step| matches!(step, Step::Query(_))));
    for (index, step) in host.log.iter().enumerate() {
        if matches!(step, Step::Move(_)) {
            assert_eq!(host.log[index - 1], Step::Drain, "{:?}", host.log);
        }
    }
    // The warp back to a start inside the window waits for its report, then checks it.
    assert!(host.log.ends_with(&[
        Step::Drain,
        Step::Move(start),
        Step::Wait,
        Step::Query("cursor"),
    ]));
}

#[test]
fn a_neru_failure_before_calibration_touches_nothing() {
    let mut host = FakeHost::kitty();
    let error = failed("query window", "not running".into());
    host.window_query = Err(error.clone());
    let outcome = run(&mut host);
    assert_eq!(outcome.result, Err(NeruCalibrationError::Command(error)));
    assert!(outcome.pointer_restored);
    assert_eq!(host.log, [Step::Query("window")]);
}

#[test]
fn searches_the_window_for_the_split_that_reports() {
    let mut host = FakeHost::kitty();
    // A kitty horizontal split: this client is the bottom 30% of the window.
    let window = host.window;
    let top = window.y + window.height * 7 / 10;
    host.terminal = rect(window.x, top, window.width, window.y + window.height - top);
    let outcome = run(&mut host);
    assert_eq!(
        outcome.result.map(|calibrated| calibrated.calibration),
        Ok(expected(&host, GRID))
    );
    assert!(outcome.pointer_restored);
    let moves = host.moves();
    // Center and the top and middle rows of the 3 × 3 grid miss; the bottom row's first hits.
    assert_eq!(moves.len(), 9, "{moves:?}");
    assert!(moves[..6]
        .iter()
        .all(|point| !host.terminal.contains(*point)));
}

#[test]
fn fails_without_a_report_and_skips_points_off_every_display() {
    let mut host = FakeHost::kitty();
    // The window's bottom 220 pt are below the screen, like the scale-1 Ghostty runs in T1.
    host.window = rect(647, 700, 1263, 800);
    host.window_query = Ok(host.window);
    host.terminal = rect(0, 0, 0, 0);
    let outcome = run(&mut host);
    assert_eq!(outcome.result, Err(NeruCalibrationError::NoReport));
    assert!(outcome.pointer_restored);
    let moves = host.moves();
    // The center and the top two grid rows (the bottom row is off-screen), then the start.
    assert_eq!(moves.len(), 7, "{moves:?}");
    assert!(moves.iter().all(|point| MAIN.contains(*point)));
}

#[test]
fn fails_when_the_pointer_moves_during_calibration() {
    let mut host = FakeHost::kitty();
    host.nudge = Some((30, 0));
    let start = host.cursor;
    let outcome = run(&mut host);
    assert_eq!(outcome.result, Err(NeruCalibrationError::PointerMoved));
    assert!(outcome.pointer_restored);
    assert_eq!(host.cursor, start);
}

#[test]
fn fails_for_a_window_on_a_display_neru_cannot_reach() {
    let mut host = FakeHost::kitty();
    host.window = rect(-1500, 1400, 1200, 900);
    host.window_query = Ok(host.window);
    host.terminal = host.window;
    host.origin = (-1490.0, 1410.0);
    let start = host.cursor;
    let outcome = run(&mut host);
    assert_eq!(
        outcome.result,
        Err(NeruCalibrationError::UnreachableDisplay)
    );
    assert!(outcome.pointer_restored);
    assert_eq!(host.cursor, start);
    assert_eq!(host.moves().len(), 2, "one warp, then the warp back");
}

#[test]
fn reports_a_pointer_that_started_on_a_display_neru_cannot_reach() {
    let mut host = FakeHost::kitty();
    host.cursor = (-1373, 2440);
    let outcome = run(&mut host);
    assert!(outcome.result.is_ok());
    assert!(!outcome.pointer_restored);
    assert_eq!(
        POINTER_NOT_RESTORED,
        "Could not move the pointer back: it started on another display."
    );
}

#[test]
fn rejects_reports_that_do_not_follow_the_pointer() {
    let mut host = FakeHost::kitty();
    host.frozen_report = Some((400, 800));
    let outcome = run(&mut host);
    assert_eq!(
        outcome.result,
        Err(NeruCalibrationError::InconsistentReports)
    );
    assert!(outcome.pointer_restored);
}

#[test]
fn rejects_a_scale_that_differs_between_axes() {
    let mut host = FakeHost::kitty();
    // A late report from an earlier warp would skew one axis like this.
    let mut first = true;
    let outcome = {
        struct Skewed<'a>(&'a mut FakeHost, &'a mut bool);
        impl NeruCalibrationHost for Skewed<'_> {
            fn query_window(&mut self) -> Result<NeruScreenRect, NeruCommandError> {
                self.0.query_window()
            }
            fn query_cursor(&mut self) -> Result<ScreenPoint, NeruCommandError> {
                self.0.query_cursor()
            }
            fn query_displays(&mut self) -> Result<Vec<NeruScreenRect>, NeruCommandError> {
                self.0.query_displays()
            }
            fn move_mouse(&mut self, point: ScreenPoint) -> Result<(), NeruCommandError> {
                self.0.move_mouse(point)
            }
            fn start_reports(&mut self) {
                self.0.start_reports();
            }
            fn drain_reports(&mut self) {
                self.0.drain_reports();
            }
            fn next_motion(&mut self, timeout: Duration) -> Option<(u32, u32)> {
                let report = self.0.next_motion(timeout)?;
                let skew = if std::mem::take(self.1) { 0 } else { 40 };
                Some((report.0, report.1 + skew))
            }
        }
        calibrate(&mut Skewed(&mut host, &mut first), GRID, extent(GRID))
    };
    assert_eq!(
        outcome.result,
        Err(NeruCalibrationError::InconsistentReports)
    );
}

#[test]
fn fails_when_the_host_reports_no_pixel_size() {
    let mut host = FakeHost::kitty();
    let outcome = calibrate(&mut host, GRID, (0, 0));
    assert_eq!(outcome.result, Err(NeruCalibrationError::UnknownCellSize));
    assert!(outcome.pointer_restored);
}

#[test]
fn messages_match_the_design_note() {
    let cases = [
        (
            NeruCalibrationError::NoReport,
            "Calibration failed: the terminal sent no mouse report.",
        ),
        (
            NeruCalibrationError::PointerMoved,
            "Calibration failed: the pointer moved. Keep the mouse still and try again.",
        ),
        (
            NeruCalibrationError::UnreachableDisplay,
            "Calibration failed: neru cannot move the pointer to this window's display.",
        ),
        (
            NeruCalibrationError::InconsistentReports,
            "Calibration failed: inconsistent mouse reports.",
        ),
        (
            failed("query window", "accessibility permission missing".into()).into(),
            "neru query window failed: accessibility permission missing",
        ),
        (
            NeruCommandError::NotFound {
                path: "neru".into(),
            }
            .into(),
            "neru not found at \"neru\". Install neru 1.58.0 or later, or set neru.path.",
        ),
    ];
    for (error, message) in cases {
        assert_eq!(error.to_string(), message);
    }
}

#[test]
fn takes_only_plain_motion_reports() {
    assert_eq!(motion_pixels(b"\x1b[<35;440;465M"), Some((440, 465)));
    assert_eq!(motion_pixels(b"\x1b[<0;440;465M"), None, "left press");
    assert_eq!(motion_pixels(b"\x1b[<32;440;465M"), None, "left drag");
    assert_eq!(motion_pixels(b"\x1b[M#!!"), None, "legacy X10 report");
}

#[test]
fn parses_neru_query_output() {
    // Recorded from neru v1.62.0 in T1 (`poc/results`).
    assert_eq!(
        parse_window(br#"{"x": 647, "y": 73, "width": 1263, "height": 1249}"#),
        Ok(rect(647, 73, 1263, 1249))
    );
    assert_eq!(
        parse_cursor(br#"{"x": 1280, "y": 996, "display": "MateView", "display_index": 0}"#),
        Ok((1280, 996))
    );
    assert_eq!(
        parse_displays(
            br#"{"displays": [{"name": "MateView", "x": 0, "y": 0, "width": 1920, "height": 1280, "scale": 1}, {"name": "Built-in Retina Display", "x": -1728, "y": 1280, "width": 1728, "height": 1117, "scale": 1}]}"#
        ),
        Ok(vec![MAIN, BUILT_IN])
    );
    assert!(parse_window(b"neru is not running").is_err());
}

#[cfg(unix)]
mod cli {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn script(dir: &std::path::Path, body: &str) -> PathBuf {
        let path = dir.join("neru");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "herdr-neru-cli-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn runs_neru_and_reads_its_answer() {
        let dir = scratch();
        let neru = NeruCli::new(script(
            &dir,
            r#"[ "$1 $3" = "query --json" ] || exit 2
case "$2" in
  cursor) echo '{"x": 12.4, "y": 7.6}' ;;
  displays) echo '{"displays": [{"x": 0, "y": 0, "width": 1920, "height": 1280, "scale": 1}]}' ;;
  *) exit 3 ;;
esac"#,
        ));
        assert_eq!(neru.query_cursor(), Ok((12, 8)));
        assert_eq!(neru.query_displays(), Ok(vec![MAIN]));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn reports_the_first_stderr_line_of_a_failed_command() {
        let dir = scratch();
        let neru = NeruCli::new(script(&dir, "echo 'daemon not running' >&2\nexit 1"));
        assert_eq!(
            neru.move_mouse((1, 2)),
            Err(failed("action move_mouse", "daemon not running".into()))
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn reports_a_missing_binary() {
        let neru = NeruCli::new("/nonexistent/herdr-test/neru");
        assert_eq!(
            neru.query_window(),
            Err(NeruCommandError::NotFound {
                path: "/nonexistent/herdr-test/neru".into()
            })
        );
    }
}
