//! Calibration for neru hints (`SPEC-neru-hints.md` §T.4, research note "Calibration
//! measurements", Decision).
//!
//! [`calibrate`] finds where the client's grid sits on screen. It warps the pointer with
//! `neru action move_mouse` and reads the host's SGR pixel motion reports. Two points give the
//! scale and the grid origin, so no per-terminal config is needed (§I, §V.9). The pointer goes
//! back to where it started on every path (§V.8).
//!
//! The main loop owns the host mouse modes (`client::neru_modes`). Calibration runs on a worker
//! thread and asks for pixel reports through [`NeruCalibrationHost::start_reports`].

// The client action (§T.5) is the first caller outside tests.
#![cfg_attr(not(test), allow(dead_code))]

use std::fmt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use super::neru_targets::{NeruCalibration, NeruScreenRect};

/// How long a warp may take to produce its pixel report. A warp reports within a few
/// milliseconds (research note, finding 11). Each candidate point that does not report costs
/// one timeout.
pub(super) const REPORT_TIMEOUT: Duration = Duration::from_millis(150);
/// Offset of the second point on each axis (research note, Decision step 4).
const SECOND_POINT_PT: i32 = 40;
/// `neru query cursor` can differ from the warp target by rounding.
const CURSOR_TOLERANCE_PT: i32 = 1;
/// Largest relative difference between the horizontal and the vertical scale. Both come from
/// the same display, so a larger gap means a report did not come from its warp (§V.4).
const SCALE_TOLERANCE: f64 = 0.05;
/// How long one neru command may run before herdr kills it.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(2);

/// A point in global top-left screen points.
pub(super) type ScreenPoint = (i32, i32);

/// A neru command that could not run or failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum NeruCommandError {
    NotFound {
        path: String,
    },
    Failed {
        command: &'static str,
        detail: String,
    },
}

impl fmt::Display for NeruCommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound { path } => write!(
                f,
                "neru not found at \"{path}\". Install neru 1.58.0 or later, or set neru.path."
            ),
            Self::Failed { command, detail } => write!(f, "neru {command} failed: {detail}"),
        }
    }
}

/// Why calibration produced no grid position (design section 8, messages 7, 9 to 13).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum NeruCalibrationError {
    Command(NeruCommandError),
    /// No candidate point produced a pixel motion report (§V.10).
    NoReport,
    /// `neru query cursor` was not the warp target on the target's display (§V.10).
    PointerMoved,
    /// The warp landed on another display: neru cannot move the pointer to the target
    /// (§V.10, research note finding 12).
    UnreachableDisplay,
    /// The two reports give a scale that is not positive or differs between axes (§V.4).
    InconsistentReports,
    /// The host reported no usable pixel size, so the cell pitch is unknown (§V.9).
    UnknownCellSize,
}

impl From<NeruCommandError> for NeruCalibrationError {
    fn from(error: NeruCommandError) -> Self {
        Self::Command(error)
    }
}

impl fmt::Display for NeruCalibrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reason = match self {
            Self::Command(error) => return error.fmt(f),
            Self::NoReport => "the terminal sent no mouse report.",
            Self::PointerMoved => "the pointer moved. Keep the mouse still and try again.",
            Self::UnreachableDisplay => "neru cannot move the pointer to this window's display.",
            Self::InconsistentReports => "inconsistent mouse reports.",
            Self::UnknownCellSize => "the terminal did not report its size in pixels.",
        };
        write!(f, "Calibration failed: {reason}")
    }
}

/// Message 14 of design section 8 (§V.8).
pub(super) const POINTER_NOT_RESTORED: &str =
    "Could not move the pointer back: it started on another display.";

/// What calibration needs from neru and from the host's mouse reports.
pub(super) trait NeruCalibrationHost {
    fn query_window(&mut self) -> Result<NeruScreenRect, NeruCommandError>;
    fn query_cursor(&mut self) -> Result<ScreenPoint, NeruCommandError>;
    fn query_displays(&mut self) -> Result<Vec<NeruScreenRect>, NeruCommandError>;
    fn move_mouse(&mut self, point: ScreenPoint) -> Result<(), NeruCommandError>;
    /// Has the main loop turn on pixel reporting and returns once it has. From then on, host
    /// mouse reports reach only the calibrator, until [`calibrate`] returns (§V.14).
    fn start_reports(&mut self);
    /// Discards every report received so far.
    fn drain_reports(&mut self);
    /// The pixel position of the next plain-motion report, or `None` after `timeout`.
    fn next_motion(&mut self, timeout: Duration) -> Option<(u32, u32)>;
}

/// The grid position and the displays its targets must fall on (§V.11).
#[derive(Clone, Debug, PartialEq)]
pub(super) struct NeruCalibrated {
    pub(super) calibration: NeruCalibration,
    pub(super) displays: Vec<NeruScreenRect>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct NeruCalibrationOutcome {
    pub(super) result: Result<NeruCalibrated, NeruCalibrationError>,
    /// False when the pointer could not go back to its start point (§V.8). Reported with
    /// [`POINTER_NOT_RESTORED`], also after a successful calibration.
    pub(super) pointer_restored: bool,
}

/// Measures the grid of `grid` columns and rows, `extent_px` device pixels in all.
///
/// Every neru query runs before pixel reporting starts, so a missing or broken neru leaves the
/// session untouched (§V.7). Once reporting has started, the pointer goes back to its start
/// on every path, and `calibrate` returns only after the report from that warp has arrived
/// or timed out (§V.8, §V.14).
pub(super) fn calibrate(
    host: &mut impl NeruCalibrationHost,
    grid: (u16, u16),
    extent_px: (u32, u32),
) -> NeruCalibrationOutcome {
    let scene = match Scene::query(host) {
        Ok(scene) => scene,
        Err(error) => {
            return NeruCalibrationOutcome {
                result: Err(error.into()),
                pointer_restored: true,
            }
        }
    };
    host.start_reports();
    let result = measure(host, &scene, grid, extent_px).map(|calibration| NeruCalibrated {
        calibration,
        displays: scene.displays.clone(),
    });
    let pointer_restored = restore_pointer(host, &scene);
    NeruCalibrationOutcome {
        result,
        pointer_restored,
    }
}

/// The screen as calibration starts.
struct Scene {
    /// The focused host window. It can hold several terminals (kitty splits).
    window: NeruScreenRect,
    start: ScreenPoint,
    displays: Vec<NeruScreenRect>,
}

impl Scene {
    fn query(host: &mut impl NeruCalibrationHost) -> Result<Self, NeruCommandError> {
        Ok(Self {
            window: host.query_window()?,
            start: host.query_cursor()?,
            displays: host.query_displays()?,
        })
    }

    fn display_of(&self, point: ScreenPoint) -> Option<usize> {
        self.displays
            .iter()
            .position(|display| display.contains(point))
    }

    /// The warp to `target` left the pointer at `cursor`. neru does not cross displays: it
    /// moves the pointer to the nearest edge of a display it can reach (finding 12). So a
    /// cursor on another display means the target is unreachable, and a cursor on the same
    /// display means someone moved the pointer.
    fn miss(&self, target: ScreenPoint, cursor: ScreenPoint) -> NeruCalibrationError {
        match (self.display_of(target), self.display_of(cursor)) {
            (Some(target), Some(cursor)) if target == cursor => NeruCalibrationError::PointerMoved,
            _ => NeruCalibrationError::UnreachableDisplay,
        }
    }
}

/// Finds a point that reports, then a second one. Their two reports give the scale in device
/// pixels per point and the grid origin in screen points.
fn measure(
    host: &mut impl NeruCalibrationHost,
    scene: &Scene,
    grid: (u16, u16),
    extent_px: (u32, u32),
) -> Result<NeruCalibration, NeruCalibrationError> {
    let mut first = None;
    for point in candidate_points(scene.window) {
        // A window can extend past the screen edge. A warp there would land on the edge.
        if scene.display_of(point).is_none() {
            continue;
        }
        if let Some(report) = warp(host, scene, point)? {
            first = Some((point, report));
            break;
        }
    }
    let (p1, r1) = first.ok_or(NeruCalibrationError::NoReport)?;
    let p2 = second_point(scene, p1).ok_or(NeruCalibrationError::NoReport)?;
    let r2 = warp(host, scene, p2)?.ok_or(NeruCalibrationError::NoReport)?;
    let axis_scale = |r1: u32, r2: u32, p1: i32, p2: i32| {
        (f64::from(r2) - f64::from(r1)) / (f64::from(p2) - f64::from(p1))
    };
    let scale = (
        axis_scale(r1.0, r2.0, p1.0, p2.0),
        axis_scale(r1.1, r2.1, p1.1, p2.1),
    );
    let positive = |value: f64| value.is_finite() && value > 0.0;
    if !positive(scale.0)
        || !positive(scale.1)
        || (scale.0 - scale.1).abs() > SCALE_TOLERANCE * scale.0.max(scale.1)
    {
        return Err(NeruCalibrationError::InconsistentReports);
    }
    let origin = (
        f64::from(p1.0) - f64::from(r1.0) / scale.0,
        f64::from(p1.1) - f64::from(r1.1) / scale.1,
    );
    NeruCalibration::new(grid, extent_px, origin, scale)
        .ok_or(NeruCalibrationError::UnknownCellSize)
}

/// The window center, then a 3 × 3 grid, for a window that holds several terminals.
fn candidate_points(window: NeruScreenRect) -> impl Iterator<Item = ScreenPoint> {
    const FRACTIONS: [f64; 3] = [0.2, 0.5, 0.8];
    let at = move |fx: f64, fy: f64| {
        (
            window.x + (f64::from(window.width) * fx) as i32,
            window.y + (f64::from(window.height) * fy) as i32,
        )
    };
    let center = at(0.5, 0.5);
    std::iter::once(center).chain(
        FRACTIONS
            .into_iter()
            .flat_map(move |fy| FRACTIONS.into_iter().map(move |fx| at(fx, fy)))
            .filter(move |point| *point != center),
    )
}

/// A point 40 pt from `first` on both axes, toward the window center first, so it stays in
/// the terminal that reported `first` as often as possible.
fn second_point(scene: &Scene, first: ScreenPoint) -> Option<ScreenPoint> {
    let window = scene.window;
    let toward_center = |value: i32, start: i32, size: i32| {
        if value <= start + size / 2 {
            [SECOND_POINT_PT, -SECOND_POINT_PT]
        } else {
            [-SECOND_POINT_PT, SECOND_POINT_PT]
        }
    };
    let dxs = toward_center(first.0, window.x, window.width);
    let dys = toward_center(first.1, window.y, window.height);
    dxs.into_iter()
        .flat_map(|dx| dys.into_iter().map(move |dy| (first.0 + dx, first.1 + dy)))
        .find(|point| window.contains(*point) && scene.display_of(*point).is_some())
}

/// Warps to `point` and returns the pixel report it produced, if any. Fails when the pointer
/// is not at `point` afterwards (§V.10).
fn warp(
    host: &mut impl NeruCalibrationHost,
    scene: &Scene,
    point: ScreenPoint,
) -> Result<Option<(u32, u32)>, NeruCalibrationError> {
    host.drain_reports();
    host.move_mouse(point)?;
    let report = host.next_motion(REPORT_TIMEOUT);
    let cursor = host.query_cursor()?;
    if !near(cursor, point) {
        return Err(scene.miss(point, cursor));
    }
    Ok(report)
}

fn restore_pointer(host: &mut impl NeruCalibrationHost, scene: &Scene) -> bool {
    host.drain_reports();
    if host.move_mouse(scene.start).is_err() {
        return false;
    }
    // Only a start point over the window can produce a report. Waiting for it keeps the warp's
    // report away from herdr's mouse handling (§V.14).
    if scene.window.contains(scene.start) {
        let _ = host.next_motion(REPORT_TIMEOUT);
    }
    host.query_cursor()
        .is_ok_and(|cursor| near(cursor, scene.start))
}

fn near(a: ScreenPoint, b: ScreenPoint) -> bool {
    (a.0 - b.0).abs() <= CURSOR_TOLERANCE_PT && (a.1 - b.1).abs() <= CURSOR_TOLERANCE_PT
}

/// The pixel position of an SGR report that is plain pointer motion, as a warp produces.
pub(super) fn motion_pixels(report: &[u8]) -> Option<(u32, u32)> {
    let report = crate::raw_input::parse_sgr_mouse_report(report)?;
    report.is_plain_motion().then_some((report.x, report.y))
}

/// Runs the neru commands calibration needs, each with a deadline. A hung neru is killed and
/// reaped, so it cannot hold the mouse modes (§V.6).
pub(super) struct NeruCli {
    path: PathBuf,
}

impl NeruCli {
    pub(super) fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub(super) fn query_window(&self) -> Result<NeruScreenRect, NeruCommandError> {
        let output = self.run("query window", &["query", "window", "--json"])?;
        parse_window(&output).map_err(|detail| failed("query window", detail))
    }

    pub(super) fn query_cursor(&self) -> Result<ScreenPoint, NeruCommandError> {
        let output = self.run("query cursor", &["query", "cursor", "--json"])?;
        parse_cursor(&output).map_err(|detail| failed("query cursor", detail))
    }

    /// Display rectangles only. Their scale field reads 1 on HiDPI displays (§V.9).
    pub(super) fn query_displays(&self) -> Result<Vec<NeruScreenRect>, NeruCommandError> {
        let output = self.run("query displays", &["query", "displays", "--json"])?;
        parse_displays(&output).map_err(|detail| failed("query displays", detail))
    }

    pub(super) fn move_mouse(&self, (x, y): ScreenPoint) -> Result<(), NeruCommandError> {
        let (x, y) = (x.to_string(), y.to_string());
        self.run(
            "action move_mouse",
            &["action", "move_mouse", "--x", &x, "--y", &y],
        )
        .map(drop)
    }

    fn run(&self, command: &'static str, args: &[&str]) -> Result<Vec<u8>, NeruCommandError> {
        let child = Command::new(&self.path)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::NotFound => NeruCommandError::NotFound {
                    path: self.path.display().to_string(),
                },
                _ => failed(command, error.to_string()),
            })?;
        let output = crate::remote::wait_with_output_timeout(
            child,
            None,
            COMMAND_TIMEOUT,
            Arc::new(AtomicBool::new(false)),
        )
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::TimedOut => failed(
                command,
                format!("no answer within {} s", COMMAND_TIMEOUT.as_secs()),
            ),
            _ => failed(command, error.to_string()),
        })?;
        if !output.status.success() {
            return Err(failed(
                command,
                first_line(&output.stderr)
                    .or_else(|| first_line(&output.stdout))
                    .unwrap_or_else(|| output.status.to_string()),
            ));
        }
        Ok(output.stdout)
    }
}

fn failed(command: &'static str, detail: String) -> NeruCommandError {
    NeruCommandError::Failed { command, detail }
}

fn first_line(bytes: &[u8]) -> Option<String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

#[derive(serde::Deserialize)]
struct WireRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl From<WireRect> for NeruScreenRect {
    fn from(rect: WireRect) -> Self {
        Self {
            x: rect.x.round() as i32,
            y: rect.y.round() as i32,
            width: rect.width.round() as i32,
            height: rect.height.round() as i32,
        }
    }
}

#[derive(serde::Deserialize)]
struct WirePoint {
    x: f64,
    y: f64,
}

#[derive(serde::Deserialize)]
struct WireDisplays {
    displays: Vec<WireRect>,
}

fn parse_window(json: &[u8]) -> Result<NeruScreenRect, String> {
    serde_json::from_slice::<WireRect>(json)
        .map(NeruScreenRect::from)
        .map_err(|error| format!("unexpected output: {error}"))
}

fn parse_cursor(json: &[u8]) -> Result<ScreenPoint, String> {
    serde_json::from_slice::<WirePoint>(json)
        .map(|point| (point.x.round() as i32, point.y.round() as i32))
        .map_err(|error| format!("unexpected output: {error}"))
}

fn parse_displays(json: &[u8]) -> Result<Vec<NeruScreenRect>, String> {
    serde_json::from_slice::<WireDisplays>(json)
        .map(|wire| wire.displays.into_iter().map(Into::into).collect())
        .map_err(|error| format!("unexpected output: {error}"))
}

#[cfg(test)]
mod tests;
