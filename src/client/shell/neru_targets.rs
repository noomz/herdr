//! The target list for neru hints (`SPEC-neru-hints.md` §T.3, `docs/design/neru-hints/design.md`).
//!
//! [`collect`] walks the hit map in the order a left press is dispatched and keeps each control
//! that a click at its target's center reaches now (§V.1, §V.2). [`to_targets`] turns those cell
//! rectangles into neru's `--targets` JSON in global top-left screen points.

// The client action (§T.5) is the first caller outside tests.
#![cfg_attr(not(test), allow(dead_code))]

use super::*;
use crate::client::terminal_geometry::ioctl_cell_size;

/// The open overlay, with the state its click guards read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NeruOverlay {
    /// Onboarding, product announcement or release notes: only the close button.
    Notice {
        close: Rect,
    },
    GlobalMenu,
    ContextMenu,
    /// Only rows below `eligible_rows` pick a pane.
    PaneTargetSelector {
        eligible_rows: usize,
    },
    /// Cancel does nothing while the worktree operation runs.
    Worktree {
        busy: bool,
    },
    /// Cancel does nothing while integrations install.
    Settings {
        installing: bool,
    },
    Help,
    Navigator,
    Rename,
    ConfirmClose,
}

/// What a left press does in the mobile layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NeruMobile {
    Off,
    /// Terminal or Resize mode: only the switch button.
    Switch,
    /// Navigate mode: the switcher rows and its close button.
    Switcher,
    /// Any other mode: nothing to click.
    Inert,
}

/// The client state that decides which hit-map layer takes a left press.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct NeruLayerState {
    pub(super) overlay: Option<NeruOverlay>,
    pub(super) popup_open: bool,
    pub(super) toast_clickable: bool,
    pub(super) mobile: NeruMobile,
    pub(super) retryable_machines: Vec<ClientEndpointId>,
    pub(super) collapsed_machines: Vec<ClientEndpointId>,
}

/// A control a left click at the center of `rect` reaches, in grid cells.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct NeruControl {
    pub(super) rect: Rect,
    pub(super) title: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NeruCollectError {
    /// A popup pane takes every click.
    PopupOpen,
}

/// Controls in dispatch order. Each control's rect is the widest run of its columns that no
/// earlier-dispatched rectangle claims, so its center never lands on a child or on a layer
/// above it (§V.13, design section 4).
pub(super) fn collect(
    hits: &ShellHitMap,
    state: &NeruLayerState,
) -> Result<Vec<NeruControl>, NeruCollectError> {
    let mut claims = Claims::new(hits);
    if let Some(NeruOverlay::Notice { close }) = state.overlay {
        claims.target(close, || "close".into());
        return Ok(claims.controls);
    }
    if state.popup_open {
        return Err(NeruCollectError::PopupOpen);
    }
    if state.toast_clickable {
        claims.target(hits.notification_toast, || "notification".into());
    } else {
        claims.block(hits.notification_toast);
    }
    if let Some(overlay) = state.overlay {
        collect_overlay(&mut claims, hits, overlay);
        return Ok(claims.controls);
    }
    for machine in &hits.machines {
        if state.retryable_machines.contains(&machine.endpoint_id) {
            let name = claims.title_of(machine.rect, "machine");
            claims.target(machine.status_badge, || format!("retry {name}"));
        }
    }
    for float in hits.floats.iter().rev() {
        for (id, rect) in &float.buttons {
            claims.target(*rect, || id.clone());
        }
        claims.block(float.outer);
    }
    match state.mobile {
        NeruMobile::Off => collect_base(&mut claims, hits, state),
        NeruMobile::Switch => claims.target(hits.mobile_switch, || "switch workspace".into()),
        NeruMobile::Switcher => {
            claims.target(hits.mobile_close, || "close".into());
            for (rect, _) in &hits.mobile_targets {
                claims.target(*rect, || "item".into());
            }
        }
        NeruMobile::Inert => {}
    }
    Ok(claims.controls)
}

/// Overlay controls in `handle_mouse` order (`mouse.rs`). Clicking outside an overlay closes
/// it, which Escape also does, so the outside is never a target.
fn collect_overlay(claims: &mut Claims, hits: &ShellHitMap, overlay: NeruOverlay) {
    match overlay {
        NeruOverlay::Notice { .. } => {}
        NeruOverlay::GlobalMenu => {
            claims.target(hits.global_launcher, || "menu".into());
            claims.rows(&hits.global_menu_rows, "menu item");
        }
        NeruOverlay::ContextMenu => claims.rows(&hits.context_menu_rows, "menu item"),
        NeruOverlay::PaneTargetSelector { eligible_rows } => {
            for (rect, index) in &hits.pane_target_rows {
                if *index < eligible_rows {
                    claims.target(*rect, || "pane".into());
                } else {
                    claims.block(*rect);
                }
            }
        }
        NeruOverlay::Worktree { busy } => {
            if busy {
                claims.block(hits.overlay_cancel);
            } else {
                claims.target(hits.overlay_cancel, || "cancel".into());
            }
            claims.target(hits.worktree_search, || "search".into());
            claims.rows(&hits.worktree_rows, "worktree");
            claims.target(hits.overlay_primary, || "confirm".into());
        }
        NeruOverlay::Settings { installing } => {
            claims.rows(&hits.settings_tabs, "settings section");
            claims.rows(&hits.settings_choices, "setting");
            claims.target(hits.overlay_primary, || "apply".into());
            if !installing {
                claims.target(hits.overlay_cancel, || "cancel".into());
            }
        }
        NeruOverlay::Help => {
            claims.block(hits.help_scrollbar);
            claims.target(hits.overlay_cancel, || "close".into());
        }
        NeruOverlay::Navigator => {
            claims.block(hits.navigator_scrollbar);
            claims.target(hits.navigator_search, || "search".into());
            claims.rows(&hits.navigator_rows, "item");
        }
        NeruOverlay::Rename => {
            claims.target(hits.overlay_primary, || "save".into());
            claims.target(hits.overlay_clear, || "clear".into());
        }
        NeruOverlay::ConfirmClose => claims.target(hits.overlay_primary, || "close".into()),
    }
}

/// Base chrome in the order of the left-press branch of `handle_mouse` (`mouse.rs`).
fn collect_base(claims: &mut Claims, hits: &ShellHitMap, state: &NeruLayerState) {
    claims.target(hits.sidebar_toggle, || "toggle sidebar".into());
    claims.block(hits.sidebar_divider);
    claims.block(hits.sidebar_section_divider);
    claims.block(hits.workspace_scrollbar);
    claims.block(hits.agent_scrollbar);
    claims.target(hits.agent_sort_toggle, || "sort agents".into());
    for machine in &hits.machines {
        let name = claims.title_of(machine.rect, "machine");
        let verb = if state.collapsed_machines.contains(&machine.endpoint_id) {
            "expand"
        } else {
            "collapse"
        };
        claims.target(machine.collapse_toggle, || format!("{verb} {name}"));
        claims.target(machine.rect, || name.clone());
    }
    claims.target(hits.global_launcher, || "menu".into());
    claims.target(hits.new_workspace, || "new workspace".into());
    claims.target(hits.new_tab, || "new tab".into());
    claims.target(hits.tab_scroll_left, || "scroll tabs left".into());
    claims.target(hits.tab_scroll_right, || "scroll tabs right".into());
    for workspace in &hits.workspaces {
        if let Some((toggle, _)) = &workspace.group_toggle {
            let name = claims.title_of(workspace.rect, "workspace");
            claims.target(*toggle, || format!("toggle group {name}"));
        }
    }
    for workspace in &hits.workspaces {
        claims.target(workspace.rect, || "workspace".into());
    }
    claims.rows(&hits.tabs, "tab");
    for (rect, _, _) in &hits.endpoint_agents {
        claims.target(*rect, || "agent".into());
    }
    claims.rows(&hits.agents, "agent");
}

/// The visible text of every control that draws some, read from the composed frame (§V.3).
/// Reading the frame keeps titles from drifting from the screen. Icon-only controls are left
/// out: they get the fixed descriptive titles in [`collect`], since nobody types a glyph.
pub(super) fn drawn_labels(hits: &ShellHitMap, frame: &FrameData) -> HashMap<Rect, String> {
    let rows = |rows: &[(Rect, usize)]| rows.iter().map(|(rect, _)| *rect).collect::<Vec<_>>();
    let mut rects = vec![
        hits.new_workspace,
        hits.global_launcher,
        hits.agent_sort_toggle,
        hits.notification_toast,
        hits.overlay_primary,
        hits.overlay_clear,
        hits.overlay_cancel,
        hits.navigator_search,
        hits.worktree_search,
    ];
    rects.extend(hits.machines.iter().map(|hit| hit.rect));
    rects.extend(hits.workspaces.iter().map(|hit| hit.rect));
    rects.extend(hits.tabs.iter().map(|(rect, _)| *rect));
    rects.extend(hits.agents.iter().map(|(rect, _)| *rect));
    rects.extend(hits.endpoint_agents.iter().map(|(rect, _, _)| *rect));
    rects.extend(hits.mobile_targets.iter().map(|(rect, _)| *rect));
    rects.extend(hits.navigator_rows.iter().map(|(rect, _)| *rect));
    rects.extend(hits.settings_tabs.iter().map(|(rect, _)| *rect));
    rects.extend(rows(&hits.global_menu_rows));
    rects.extend(rows(&hits.context_menu_rows));
    rects.extend(rows(&hits.pane_target_rows));
    rects.extend(rows(&hits.worktree_rows));
    rects.extend(rows(&hits.settings_choices));
    rects
        .into_iter()
        .filter(|rect| !rect.is_empty())
        .filter_map(|rect| {
            let text = clean_title(&frame_text(frame, rect));
            (!text.is_empty()).then_some((rect, text))
        })
        .collect()
}

/// The symbols in `rect`, row by row, with a space between rows.
fn frame_text(frame: &FrameData, rect: Rect) -> String {
    let width = usize::from(frame.width);
    let mut text = String::new();
    for y in rect.y..rect.bottom().min(frame.height) {
        // A wide grapheme fills the cells after it with blanks; skip them so `日本` stays whole.
        let mut x = rect.x;
        while x < rect.right().min(frame.width) {
            let Some(cell) = frame.cells.get(usize::from(y) * width + usize::from(x)) else {
                break;
            };
            text.push_str(&cell.symbol);
            let cells = u16::try_from(cell.symbol.width()).unwrap_or(1).max(1);
            x = x.saturating_add(cells);
        }
        text.push(' ');
    }
    text
}

/// Builds the control list while remembering every rectangle that takes a click first.
struct Claims<'a> {
    labels: &'a HashMap<Rect, String>,
    taken: Vec<Rect>,
    controls: Vec<NeruControl>,
}

impl<'a> Claims<'a> {
    fn new(hits: &'a ShellHitMap) -> Self {
        Self {
            labels: &hits.labels,
            taken: Vec::new(),
            controls: Vec::new(),
        }
    }

    /// A clickable rectangle that is not a target: later candidates must avoid it.
    fn block(&mut self, rect: Rect) {
        if !rect.is_empty() {
            self.taken.push(rect);
        }
    }

    /// A target titled with its recorded drawn label, or `fallback` when render recorded none.
    fn target(&mut self, rect: Rect, fallback: impl FnOnce() -> String) {
        if rect.is_empty() {
            return;
        }
        if let Some(span) = free_span(rect, &self.taken) {
            let title = clean_title(self.labels.get(&rect).map_or("", String::as_str));
            let title = if title.is_empty() {
                clean_title(&fallback())
            } else {
                title
            };
            self.controls.push(NeruControl { rect: span, title });
        }
        self.taken.push(rect);
    }

    fn rows<T>(&mut self, rows: &[(Rect, T)], fallback: &str) {
        for (rect, _) in rows {
            self.target(*rect, || fallback.into());
        }
    }

    fn title_of(&self, rect: Rect, fallback: &str) -> String {
        let title = clean_title(self.labels.get(&rect).map_or("", String::as_str));
        if title.is_empty() {
            fallback.into()
        } else {
            title
        }
    }
}

/// The widest run of `rect`'s columns that no `taken` rectangle overlaps on any of its rows.
/// Ties keep the leftmost run. `None` when every column is taken.
fn free_span(rect: Rect, taken: &[Rect]) -> Option<Rect> {
    let column_free = |x: u16| {
        !taken.iter().any(|other| {
            other.x <= x && x < other.right() && other.y < rect.bottom() && rect.y < other.bottom()
        })
    };
    let mut best: Option<(u16, u16)> = None;
    let mut run_start = None;
    for x in rect.x..=rect.right() {
        let free = x < rect.right() && column_free(x);
        match (free, run_start) {
            (true, None) => run_start = Some(x),
            (false, Some(start)) => {
                let width = x - start;
                if best.is_none_or(|(_, best_width)| width > best_width) {
                    best = Some((start, width));
                }
                run_start = None;
            }
            _ => {}
        }
    }
    best.map(|(x, width)| Rect { x, width, ..rect })
}

const MAX_TITLE_CHARS: usize = 64;

/// Trims, collapses whitespace runs to one space, drops control characters and keeps at most
/// 64 characters (design section 5). Line and paragraph separators count as whitespace, so the
/// result never holds a character Go's JSON encoder writes as a six-byte escape other than
/// `<`, `>` and `&`.
fn clean_title(raw: &str) -> String {
    raw.split_whitespace()
        .map(|word| word.chars().filter(|c| !c.is_control()).collect::<String>())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_TITLE_CHARS)
        .collect::<String>()
        .trim_end()
        .to_string()
}

/// Where herdr's grid sits on screen, measured by calibration (T4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct NeruCalibration {
    /// Global top-left screen point of the grid's top-left corner.
    origin: (f64, f64),
    /// Device pixels per screen point, from the two calibration reports (§V.9).
    scale: (f64, f64),
    /// Cell pitch in device pixels (§V.9).
    cell_px: (u32, u32),
}

impl NeruCalibration {
    /// `grid` is the client's columns and rows and `extent_px` the ioctl pixel size. The cell
    /// pitch is their integer quotient, as `ioctl_cell_size` computes it (§V.9). `None` when the
    /// extent is unknown or the scale is not a positive number (§V.4).
    pub(super) fn new(
        grid: (u16, u16),
        extent_px: (u32, u32),
        origin: (f64, f64),
        scale: (f64, f64),
    ) -> Option<Self> {
        let valid = |value: f64| value.is_finite() && value > 0.0;
        if !valid(scale.0) || !valid(scale.1) || !origin.0.is_finite() || !origin.1.is_finite() {
            return None;
        }
        let cell_px = ioctl_cell_size(grid.0, grid.1, extent_px.0, extent_px.1)?;
        Some(Self {
            origin,
            scale,
            cell_px,
        })
    }

    fn cell_corner(&self, column: u16, row: u16) -> (f64, f64) {
        (
            self.origin.0 + f64::from(column) * f64::from(self.cell_px.0) / self.scale.0,
            self.origin.1 + f64::from(row) * f64::from(self.cell_px.1) / self.scale.1,
        )
    }

    /// The integer screen rectangle inside `rect`'s cells: edges round inward, so neru's
    /// integer center lands inside the control (§V.2). `None` when no whole point fits.
    fn screen_rect(&self, rect: Rect) -> Option<NeruScreenRect> {
        let (left, top) = self.cell_corner(rect.x, rect.y);
        let (right, bottom) = self.cell_corner(rect.right(), rect.bottom());
        let (x, y) = (left.ceil(), top.ceil());
        let (width, height) = (right.floor() - x, bottom.floor() - y);
        (width >= 1.0 && height >= 1.0).then_some(NeruScreenRect {
            x: x as i32,
            y: y as i32,
            width: width as i32,
            height: height as i32,
        })
    }
}

/// A rectangle in global top-left screen points, as neru's `move_mouse` and `query` use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub(super) struct NeruScreenRect {
    pub(super) x: i32,
    pub(super) y: i32,
    pub(super) width: i32,
    pub(super) height: i32,
}

impl NeruScreenRect {
    fn center(&self) -> (i32, i32) {
        (self.x + self.width / 2, self.y + self.height / 2)
    }

    pub(super) fn contains(&self, (x, y): (i32, i32)) -> bool {
        self.x <= x && x < self.x + self.width && self.y <= y && y < self.y + self.height
    }
}

#[derive(serde::Serialize)]
struct NeruHint<'a> {
    #[serde(flatten)]
    rect: NeruScreenRect,
    title: &'a str,
}

/// The `--targets` JSON and how many controls it holds.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct NeruTargets {
    pub(super) json: String,
    /// Targets in `json`.
    pub(super) sent: usize,
    /// Targets on a display, before the size budget. `sent < total` is the over-budget case.
    pub(super) total: usize,
}

/// The largest targets JSON, measured as escaped inside neru's IPC command (design section 6).
pub(super) const TARGETS_BUDGET_BYTES: usize = 56 << 10;
const DOCUMENT_PREFIX: &str = "{\"hints\":[";
const DOCUMENT_SUFFIX: &str = "]}";

/// Converts controls to neru's targets JSON. A control whose center is off every display is
/// dropped (§V.11). When the JSON would pass the budget, controls drop from the end.
pub(super) fn to_targets(
    controls: &[NeruControl],
    calibration: &NeruCalibration,
    displays: &[NeruScreenRect],
) -> NeruTargets {
    let hints: Vec<String> = controls
        .iter()
        .filter_map(|control| {
            let rect = calibration.screen_rect(control.rect)?;
            let center = rect.center();
            displays
                .iter()
                .any(|display| display.contains(center))
                .then(|| {
                    serde_json::to_string(&NeruHint {
                        rect,
                        title: &control.title,
                    })
                    .expect("a hint serializes")
                })
        })
        .collect();
    let mut used = escaped_len(DOCUMENT_PREFIX) + escaped_len(DOCUMENT_SUFFIX);
    let mut sent = 0;
    for hint in &hints {
        let cost = escaped_len(hint) + usize::from(sent > 0);
        if used + cost > TARGETS_BUDGET_BYTES {
            break;
        }
        used += cost;
        sent += 1;
    }
    NeruTargets {
        json: format!(
            "{DOCUMENT_PREFIX}{}{DOCUMENT_SUFFIX}",
            hints[..sent].join(",")
        ),
        sent,
        total: hints.len(),
    }
}

/// Bytes `json` takes once Go's JSON encoder writes it as a string value: `"` and `\` gain a
/// backslash, and `<`, `>`, `&` become `<`-style escapes.
fn escaped_len(json: &str) -> usize {
    json.chars()
        .map(|c| match c {
            '"' | '\\' => 2,
            '<' | '>' | '&' | '\u{2028}' | '\u{2029}' => 6,
            c => c.len_utf8(),
        })
        .sum()
}

impl ClientShellState {
    /// The state [`collect`] needs, read the way `handle_input` and `handle_mouse` read it.
    pub(super) fn neru_layer_state(&self) -> NeruLayerState {
        let overlay = self.overlay.as_ref().map(|overlay| match overlay {
            ClientShellOverlay::Onboarding | ClientShellOverlay::ProductAnnouncement(_) => {
                NeruOverlay::Notice {
                    close: self.hits.overlay_primary,
                }
            }
            ClientShellOverlay::ReleaseNotes(_) => NeruOverlay::Notice {
                close: self
                    .current_release_notes_input_geometry()
                    .map_or(self.hits.overlay_primary, |(close, _, _)| close),
            },
            ClientShellOverlay::GlobalMenu(_) => NeruOverlay::GlobalMenu,
            ClientShellOverlay::ContextMenu(_) => NeruOverlay::ContextMenu,
            ClientShellOverlay::PaneTargetSelector(selector) => NeruOverlay::PaneTargetSelector {
                eligible_rows: selector.eligible_pane_ids.len(),
            },
            ClientShellOverlay::WorktreeCreate(create) => NeruOverlay::Worktree {
                busy: create.creating,
            },
            ClientShellOverlay::WorktreeOpen(open) => NeruOverlay::Worktree { busy: open.opening },
            ClientShellOverlay::WorktreeRemove(remove) => NeruOverlay::Worktree {
                busy: remove.removing,
            },
            ClientShellOverlay::Settings(settings) => NeruOverlay::Settings {
                installing: settings.installing_integrations,
            },
            ClientShellOverlay::Help(_) => NeruOverlay::Help,
            ClientShellOverlay::Navigator(_) => NeruOverlay::Navigator,
            ClientShellOverlay::Rename(_) => NeruOverlay::Rename,
            ClientShellOverlay::ConfirmClose(_) => NeruOverlay::ConfirmClose,
        });
        let mobile = if !self.mobile_layout_active() {
            NeruMobile::Off
        } else {
            match self.mode {
                ClientShellMode::Navigate => NeruMobile::Switcher,
                ClientShellMode::Terminal | ClientShellMode::Resize => NeruMobile::Switch,
                _ => NeruMobile::Inert,
            }
        };
        NeruLayerState {
            overlay,
            popup_open: self.popup_pending
                || self.popup_terminal_id.is_some()
                || self.hits.popup.is_some(),
            toast_clickable: self.visible_endpoint_notice.is_some()
                || (self.overlay.is_none()
                    && self.mode == ClientShellMode::Terminal
                    && self
                        .visible_notification
                        .as_ref()
                        .is_some_and(|notification| notification.event.pane_id.is_some())),
            mobile,
            retryable_machines: self
                .endpoints
                .iter()
                .filter(|endpoint| machine_diagnostics::machine_retryable(endpoint))
                .map(|endpoint| endpoint.endpoint_id.clone())
                .collect(),
            collapsed_machines: self.collapsed_endpoints.iter().cloned().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::floats::FloatId;

    fn base() -> NeruLayerState {
        NeruLayerState {
            overlay: None,
            popup_open: false,
            toast_clickable: false,
            mobile: NeruMobile::Off,
            retryable_machines: Vec::new(),
            collapsed_machines: Vec::new(),
        }
    }

    fn rect(x: u16, y: u16, width: u16, height: u16) -> Rect {
        Rect::new(x, y, width, height)
    }

    fn titles(controls: &[NeruControl]) -> Vec<&str> {
        controls
            .iter()
            .map(|control| control.title.as_str())
            .collect()
    }

    fn pane(at: Rect) -> PaneHit {
        PaneHit {
            rect: at,
            inner_rect: at,
            scrollbar_rect: Some(at),
            scroll: None,
            pane_id: "pane_1".into(),
            layer: PaneHitLayer::Tiled,
            mouse_reporting: false,
            sgr_pixel_mouse: false,
            pixel_width: 0,
            pixel_height: 0,
        }
    }

    fn float(id: &str, outer: Rect, buttons: &[(&str, Rect)]) -> floats::FloatHit {
        floats::FloatHit {
            float_id: FloatId::parse(id).expect("float id"),
            outer,
            pane: pane(outer),
            buttons: buttons
                .iter()
                .map(|(id, at)| ((*id).to_string(), *at))
                .collect(),
        }
    }

    fn center(at: Rect) -> (u16, u16) {
        (at.x + at.width / 2, at.y + at.height / 2)
    }

    #[test]
    fn base_chrome_follows_dispatch_order_with_drawn_or_fixed_titles() {
        let mut hits = ShellHitMap {
            sidebar_toggle: rect(0, 30, 1, 1),
            new_workspace: rect(2, 29, 5, 1),
            new_tab: rect(40, 0, 3, 1),
            ..ShellHitMap::default()
        };
        hits.labels.insert(hits.new_workspace, "new".into());
        hits.workspaces.push(WorkspaceHit {
            rect: rect(0, 2, 20, 1),
            endpoint_id: ClientEndpointId::Local,
            workspace_id: "ws_1".into(),
            indented: false,
            group_toggle: None,
        });
        hits.labels
            .insert(rect(0, 2, 20, 1), "1 client-shell".into());
        hits.tabs.push((rect(22, 0, 8, 1), "tab_1".into()));
        hits.labels.insert(rect(22, 0, 8, 1), " 1  build ".into());
        hits.agents.push((rect(0, 20, 20, 1), "pane_1".into()));

        let controls = collect(&hits, &base()).unwrap();

        assert_eq!(
            titles(&controls),
            [
                "toggle sidebar",
                "new",
                "new tab",
                "1 client-shell",
                "1 build",
                "agent"
            ]
        );
        assert_eq!(controls[3].rect, rect(0, 2, 20, 1));
    }

    #[test]
    fn panes_splits_scrollbars_dividers_and_backgrounds_are_never_targets() {
        let mut hits = ShellHitMap {
            workspace_body: rect(0, 2, 20, 10),
            workspace_scrollbar: rect(19, 2, 1, 10),
            agent_body: rect(0, 14, 20, 10),
            agent_scrollbar: rect(19, 14, 1, 10),
            sidebar_divider: rect(20, 0, 1, 30),
            sidebar_section_divider: rect(0, 13, 20, 1),
            tab_bar: rect(21, 0, 60, 1),
            help_popup: rect(10, 5, 40, 10),
            settings_popup: rect(10, 5, 40, 10),
            navigator_popup: rect(10, 5, 40, 10),
            ..ShellHitMap::default()
        };
        hits.panes.push(pane(rect(20, 1, 60, 20)));
        hits.pane_splits.push(PaneSplitHit {
            direction: crate::protocol::PaneSurfaceSplitDirection::Vertical,
            pos: 50,
            area: rect(20, 1, 60, 20),
            hit_rect: rect(50, 1, 1, 20),
            path: Vec::new(),
            topology_signature: 0,
        });

        assert_eq!(collect(&hits, &base()), Ok(Vec::new()));
    }

    #[test]
    fn a_parent_is_sent_as_the_widest_span_no_child_covers() {
        let mut hits = ShellHitMap::default();
        let row = rect(0, 2, 20, 1);
        hits.workspaces.push(WorkspaceHit {
            rect: row,
            endpoint_id: ClientEndpointId::Local,
            workspace_id: "ws_1".into(),
            indented: false,
            group_toggle: Some((rect(19, 2, 1, 1), "group".into())),
        });
        hits.labels.insert(row, "herdr".into());

        let controls = collect(&hits, &base()).unwrap();

        assert_eq!(titles(&controls), ["toggle group herdr", "herdr"]);
        assert_eq!(controls[0].rect, rect(19, 2, 1, 1));
        assert_eq!(controls[1].rect, rect(0, 2, 19, 1));
        assert!(!controls[0].rect.contains(center(controls[1].rect).into()));
    }

    #[test]
    fn a_machine_row_avoids_its_toggle_and_only_a_retryable_badge() {
        let remote = ClientEndpointId::Ssh(crate::client::endpoint::ProfileId::generate());
        let mut hits = ShellHitMap::default();
        for (endpoint_id, y) in [(ClientEndpointId::Local, 0), (remote.clone(), 1)] {
            let row = rect(0, y, 20, 1);
            hits.machines.push(MachineHit {
                rect: row,
                status_badge: rect(17, y, 3, 1),
                collapse_toggle: rect(0, y, 1, 1),
                endpoint_id,
            });
        }
        hits.labels.insert(rect(0, 0, 20, 1), "local".into());
        hits.labels.insert(rect(0, 1, 20, 1), "devbox".into());
        let state = NeruLayerState {
            retryable_machines: vec![remote.clone()],
            collapsed_machines: vec![remote],
            ..base()
        };

        let controls = collect(&hits, &state).unwrap();

        assert_eq!(
            titles(&controls),
            [
                "retry devbox",
                "collapse local",
                "local",
                "expand devbox",
                "devbox"
            ]
        );
        // The local badge is not retryable, so a click there falls through to the row.
        assert_eq!(controls[2].rect, rect(1, 0, 19, 1));
        assert_eq!(controls[4].rect, rect(1, 1, 16, 1));
    }

    #[test]
    fn a_parent_with_no_free_cell_is_dropped() {
        let mut hits = ShellHitMap::default();
        hits.workspaces.push(WorkspaceHit {
            rect: rect(0, 2, 1, 1),
            endpoint_id: ClientEndpointId::Local,
            workspace_id: "ws_1".into(),
            indented: false,
            group_toggle: Some((rect(0, 2, 1, 1), "group".into())),
        });

        let controls = collect(&hits, &base()).unwrap();

        assert_eq!(titles(&controls), ["toggle group workspace"]);
    }

    #[test]
    fn floats_cover_lower_float_buttons_and_base_chrome() {
        let mut hits = ShellHitMap {
            new_tab: rect(40, 0, 3, 1),
            floats: vec![
                float(
                    "f1",
                    rect(30, 0, 20, 8),
                    &[
                        ("lower-hidden", rect(45, 0, 2, 1)),
                        ("lower", rect(31, 0, 2, 1)),
                    ],
                ),
                float("f2", rect(44, 0, 20, 8), &[("upper", rect(60, 0, 2, 1))]),
            ],
            ..ShellHitMap::default()
        };
        hits.tabs.push((rect(30, 0, 10, 1), "tab_1".into()));

        let controls = collect(&hits, &base()).unwrap();

        assert_eq!(titles(&controls), ["upper", "lower"]);
    }

    #[test]
    fn a_drawn_toast_covers_floats_and_chrome_and_is_a_target_only_when_clickable() {
        let mut hits = ShellHitMap {
            notification_toast: rect(50, 0, 30, 10),
            floats: vec![float(
                "f1",
                rect(40, 0, 40, 8),
                &[("close", rect(70, 0, 2, 1))],
            )],
            new_tab: rect(60, 0, 3, 1),
            ..ShellHitMap::default()
        };
        hits.labels
            .insert(hits.notification_toast, "build finished".into());
        hits.tabs.push((rect(45, 0, 10, 1), "tab_1".into()));
        // Under the toast only, below the float.
        hits.agents.push((rect(52, 9, 6, 1), "pane_1".into()));

        let hidden = collect(&hits, &base()).unwrap();
        let clickable = collect(
            &hits,
            &NeruLayerState {
                toast_clickable: true,
                ..base()
            },
        )
        .unwrap();

        assert!(hidden.is_empty(), "{hidden:?}");
        assert_eq!(titles(&clickable), ["build finished"]);
    }

    #[test]
    fn an_open_overlay_is_exclusive() {
        let mut hits = ShellHitMap {
            new_tab: rect(40, 0, 3, 1),
            global_launcher: rect(2, 30, 6, 1),
            global_menu_rows: vec![(rect(2, 20, 20, 1), 0), (rect(2, 21, 20, 1), 1)],
            ..ShellHitMap::default()
        };
        hits.labels.insert(hits.global_launcher, "menu".into());
        hits.labels.insert(rect(2, 20, 20, 1), "Settings".into());
        hits.labels.insert(rect(2, 21, 20, 1), "Help".into());
        let state = NeruLayerState {
            overlay: Some(NeruOverlay::GlobalMenu),
            ..base()
        };

        let controls = collect(&hits, &state).unwrap();

        assert_eq!(titles(&controls), ["menu", "Settings", "Help"]);
    }

    #[test]
    fn overlay_guards_hide_controls_that_do_nothing() {
        let hits = ShellHitMap {
            pane_target_rows: vec![(rect(5, 5, 20, 1), 0), (rect(5, 6, 20, 1), 1)],
            overlay_primary: rect(5, 10, 6, 1),
            overlay_cancel: rect(12, 10, 8, 1),
            ..ShellHitMap::default()
        };
        let with = |overlay| NeruLayerState {
            overlay: Some(overlay),
            ..base()
        };

        let selector = collect(
            &hits,
            &with(NeruOverlay::PaneTargetSelector { eligible_rows: 1 }),
        );
        let busy = collect(&hits, &with(NeruOverlay::Worktree { busy: true }));
        let idle = collect(&hits, &with(NeruOverlay::Worktree { busy: false }));
        let installing = collect(&hits, &with(NeruOverlay::Settings { installing: true }));
        let confirm = collect(&hits, &with(NeruOverlay::ConfirmClose));

        assert_eq!(titles(&selector.unwrap()), ["pane"]);
        assert_eq!(titles(&busy.unwrap()), ["confirm"]);
        assert_eq!(titles(&idle.unwrap()), ["cancel", "confirm"]);
        assert_eq!(titles(&installing.unwrap()), ["apply"]);
        assert_eq!(titles(&confirm.unwrap()), ["close"]);
    }

    #[test]
    fn a_full_screen_notice_wins_over_a_popup_and_a_popup_gives_no_targets() {
        let mut hits = ShellHitMap {
            new_tab: rect(40, 0, 3, 1),
            overlay_primary: rect(30, 20, 8, 1),
            ..ShellHitMap::default()
        };
        hits.labels.insert(hits.overlay_primary, "Got it".into());
        let popup = NeruLayerState {
            popup_open: true,
            ..base()
        };
        let notice = NeruLayerState {
            overlay: Some(NeruOverlay::Notice {
                close: hits.overlay_primary,
            }),
            ..popup.clone()
        };

        assert_eq!(collect(&hits, &popup), Err(NeruCollectError::PopupOpen));
        assert_eq!(titles(&collect(&hits, &notice).unwrap()), ["Got it"]);
    }

    #[test]
    fn the_mobile_layout_is_exclusive_and_depends_on_mode() {
        let mut hits = ShellHitMap {
            new_tab: rect(40, 0, 3, 1),
            mobile_switch: rect(0, 0, 10, 1),
            mobile_close: rect(50, 0, 3, 1),
            mobile_targets: vec![(rect(0, 2, 40, 1), ClientMobileTarget::NewWorkspace)],
            ..ShellHitMap::default()
        };
        hits.labels
            .insert(rect(0, 2, 40, 1), "+ new workspace".into());
        let mode = |mobile| NeruLayerState { mobile, ..base() };

        assert_eq!(
            titles(&collect(&hits, &mode(NeruMobile::Switch)).unwrap()),
            ["switch workspace"]
        );
        assert_eq!(
            titles(&collect(&hits, &mode(NeruMobile::Switcher)).unwrap()),
            ["close", "+ new workspace"]
        );
        assert!(collect(&hits, &mode(NeruMobile::Inert)).unwrap().is_empty());
    }

    #[test]
    fn titles_are_cleaned_and_cut_to_64_characters() {
        assert_eq!(clean_title("  a\t b\u{2028}c \u{1b}d  "), "a b c d");
        assert_eq!(clean_title("\u{7}"), "");
        let long = "é".repeat(70);
        assert_eq!(clean_title(&long).chars().count(), 64);
        assert_eq!(
            clean_title(&format!("{} x", "a".repeat(63))),
            "a".repeat(63)
        );

        let mut hits = ShellHitMap {
            new_tab: rect(40, 0, 3, 1),
            ..ShellHitMap::default()
        };
        hits.labels.insert(hits.new_tab, " \u{1b} ".into());
        assert_eq!(titles(&collect(&hits, &base()).unwrap()), ["new tab"]);
    }

    #[test]
    fn calibration_uses_the_integer_ioctl_pitch_and_a_positive_scale() {
        // Ghostty: the ioctl extent includes padding (research note, B2).
        let ghostty = NeruCalibration::new((152, 40), (2446, 1290), (0.0, 0.0), (2.0, 2.0));
        assert_eq!(ghostty.unwrap().cell_px, (16, 32));
        assert!(NeruCalibration::new((80, 24), (800, 480), (0.0, 0.0), (0.0, 2.0)).is_none());
        assert!(NeruCalibration::new((80, 24), (800, 480), (0.0, 0.0), (f64::NAN, 2.0)).is_none());
        assert!(NeruCalibration::new((80, 24), (0, 480), (0.0, 0.0), (2.0, 2.0)).is_none());
    }

    /// The pixel a host terminal reports for screen point `pt`, then the cell it falls in.
    fn cell_at(calibration: &NeruCalibration, (x, y): (i32, i32)) -> (u16, u16) {
        let column = ((f64::from(x) - calibration.origin.0) * calibration.scale.0).floor()
            / f64::from(calibration.cell_px.0);
        let row = ((f64::from(y) - calibration.origin.1) * calibration.scale.1).floor()
            / f64::from(calibration.cell_px.1);
        (column.floor() as u16, row.floor() as u16)
    }

    #[test]
    fn every_cell_center_lands_inside_its_cell_at_scale_1_and_2() {
        let cases = [
            // Ghostty at scale 2, 20 pt padding, odd extent.
            NeruCalibration::new((152, 40), (2446, 1290), (1013.5, 117.0), (2.0, 2.0)),
            // kitty at scale 2 with a 7 px-wide cell (a half-point pitch).
            NeruCalibration::new((200, 50), (1400, 1500), (10.0, 73.0), (2.0, 2.0)),
            // Scale 1.
            NeruCalibration::new((120, 40), (960, 680), (-1910.0, 25.0), (1.0, 1.0)),
        ];
        for calibration in cases.map(Option::unwrap) {
            for column in 0..120 {
                let cell = rect(column, column % 40, 1, 1);
                let screen = calibration.screen_rect(cell).expect("a cell spans a point");
                assert_eq!(
                    cell_at(&calibration, screen.center()),
                    (cell.x, cell.y),
                    "{calibration:?} {screen:?}"
                );
            }
        }
    }

    fn display(x: i32, y: i32, width: i32, height: i32) -> NeruScreenRect {
        NeruScreenRect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn targets_json_has_neru_shape_and_drops_targets_off_every_display() {
        let calibration =
            NeruCalibration::new((100, 40), (1000, 800), (100.0, 50.0), (1.0, 1.0)).unwrap();
        let controls = [
            NeruControl {
                rect: rect(2, 1, 4, 1),
                title: "a <b> & \"c\"".into(),
            },
            NeruControl {
                rect: rect(95, 1, 4, 1),
                title: "off".into(),
            },
        ];
        let displays = [display(0, 0, 1000, 800), display(-1000, 0, 1000, 800)];

        let targets = to_targets(&controls, &calibration, &displays);

        assert_eq!((targets.sent, targets.total), (1, 1));
        let parsed: serde_json::Value = serde_json::from_str(&targets.json).unwrap();
        assert_eq!(
            parsed,
            serde_json::json!({"hints": [
                {"x": 120, "y": 70, "width": 40, "height": 20, "title": "a <b> & \"c\""}
            ]})
        );
    }

    #[test]
    fn targets_over_the_size_budget_drop_from_the_end() {
        let calibration =
            NeruCalibration::new((200, 60), (2000, 1200), (0.0, 0.0), (1.0, 1.0)).unwrap();
        let title = "<&>".repeat(21);
        let controls: Vec<_> = (0..2000u16)
            .map(|index| NeruControl {
                rect: rect(index % 200, index / 200, 1, 1),
                title: title.clone(),
            })
            .collect();

        let targets = to_targets(&controls, &calibration, &[display(0, 0, 4000, 4000)]);

        assert_eq!(targets.total, 2000);
        assert!(targets.sent > 0 && targets.sent < targets.total);
        assert!(escaped_len(&targets.json) <= TARGETS_BUDGET_BYTES);
        let parsed: serde_json::Value = serde_json::from_str(&targets.json).unwrap();
        let hints = parsed["hints"].as_array().unwrap();
        assert_eq!(hints.len(), targets.sent);
        assert_eq!(
            hints.last().unwrap()["x"],
            (targets.sent as i64 - 1) % 200 * 10
        );
        let one_more = escaped_len(&targets.json)
            + 1
            + escaped_len(&{
                let hint = &hints[0];
                serde_json::to_string(hint).unwrap()
            });
        assert!(one_more > TARGETS_BUDGET_BYTES);
    }
}
