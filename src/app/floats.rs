//! Floating terminals: session-owned, non-modal terminals drawn above the tiled layout.
//!
//! A float is not a pane. It has no layout membership, persistence, or agent detection.
//! `AppState::floats` order is the z-order: the last float is drawn on top.

use std::path::PathBuf;

use ratatui::layout::Rect;

use crate::app::state::AppState;
use crate::app::App;
use crate::layout::PaneId;
use crate::popup_size::{resolve_float_geometry, FloatPosition, PopupResolvedGeometry, PopupSize};
pub(crate) use crate::protocol::floats::FloatId;
#[cfg(unix)]
use crate::terminal::TerminalRuntime;
use crate::terminal::{TerminalId, TerminalRuntimeRegistry};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum FloatVisibility {
    #[default]
    Shown,
    Hidden,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FloatState {
    pub id: FloatId,
    /// Runtime-only id so process exit (`PaneDied`) maps back to the float.
    pub pane_id: PaneId,
    /// Runtime-only: live handoff gives the float a new terminal id.
    pub terminal_id: TerminalId,
    pub width: Option<PopupSize>,
    pub height: Option<PopupSize>,
    pub position: FloatPosition,
    pub visibility: FloatVisibility,
    /// Hide the float while this pane is the focused pane of the viewer's tab.
    pub hide_while_focused: Option<PaneId>,
}

impl FloatState {
    /// Whether a viewer whose tab focuses `focused_pane` should see this float.
    pub(crate) fn visible_to(&self, focused_pane: Option<PaneId>) -> bool {
        self.visibility == FloatVisibility::Shown
            && (self.hide_while_focused.is_none() || self.hide_while_focused != focused_pane)
    }

    pub(crate) fn geometry(&self, area: Rect) -> Option<PopupResolvedGeometry> {
        resolve_float_geometry(self.width, self.height, self.position, area)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FloatSpec {
    pub width: Option<PopupSize>,
    pub height: Option<PopupSize>,
    pub position: FloatPosition,
    pub hide_while_focused: Option<PaneId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FloatEdit {
    Move(FloatPosition),
    Resize {
        width: Option<PopupSize>,
        height: Option<PopupSize>,
    },
    Visibility(FloatVisibility),
    HideWhileFocused(Option<PaneId>),
    Raise,
}

impl AppState {
    pub(crate) fn float(&self, id: FloatId) -> Option<&FloatState> {
        self.floats.iter().find(|float| float.id == id)
    }

    /// Mints a float id no float in this session has had, including floats carried in by
    /// live handoff and floats already closed.
    pub(crate) fn mint_float_id(&mut self) -> FloatId {
        let id = self.next_float_id;
        self.next_float_id = id.next();
        id
    }

    /// Keeps the allocator past `next`, an id a previous server could have minted.
    pub(crate) fn reserve_float_ids_below(&mut self, next: FloatId) {
        self.next_float_id = self.next_float_id.max(next);
    }

    pub(crate) fn float_for_pane(&self, pane_id: PaneId) -> Option<&FloatState> {
        self.floats.iter().find(|float| float.pane_id == pane_id)
    }

    /// Applies one edit. Returns `None` for an unknown float, else whether anything changed.
    pub(crate) fn edit_float(&mut self, id: FloatId, edit: FloatEdit) -> Option<bool> {
        let index = self.floats.iter().position(|float| float.id == id)?;
        if edit == FloatEdit::Raise {
            let top = self.floats.len() - 1;
            let float = self.floats.remove(index);
            self.floats.push(float);
            return Some(index != top);
        }
        let float = &mut self.floats[index];
        let before = float.clone();
        match edit {
            FloatEdit::Move(position) => float.position = position,
            FloatEdit::Resize { width, height } => {
                float.width = width;
                float.height = height;
            }
            FloatEdit::Visibility(visibility) => float.visibility = visibility,
            FloatEdit::HideWhileFocused(pane_id) => float.hide_while_focused = pane_id,
            FloatEdit::Raise => {}
        }
        Some(*float != before)
    }

    fn remove_float(&mut self, id: FloatId) -> Option<FloatState> {
        let index = self.floats.iter().position(|float| float.id == id)?;
        let float = self.floats.remove(index);
        self.direct_attach_resize_locks.remove(&float.terminal_id);
        self.terminals.remove(&float.terminal_id);
        Some(float)
    }

    /// The pane surface floats are placed in when no client geometry is known yet.
    pub(crate) fn float_area(&self) -> Rect {
        if self.view.terminal_area.width >= 4 && self.view.terminal_area.height >= 4 {
            self.view.terminal_area
        } else {
            let (cols, rows) = self.headless_size;
            Rect::new(0, 0, cols, rows)
        }
    }
}

impl App {
    pub(crate) fn spawn_float_argv_command(
        &mut self,
        argv: &[String],
        cwd: Option<PathBuf>,
        mut extra_env: Vec<(String, String)>,
        spec: FloatSpec,
    ) -> std::io::Result<FloatId> {
        let area = self.state.float_area();
        let geometry = resolve_float_geometry(spec.width, spec.height, spec.position, area)
            .ok_or_else(|| std::io::Error::other("terminal area too small for float"))?;
        let id = self.state.mint_float_id();
        extra_env.push(("HERDR_FLOAT_ID".into(), id.to_string()));
        let (pane_id, terminal_id) = self.spawn_pane_less_terminal(
            cwd,
            extra_env,
            geometry.inner,
            |pane_id, rows, cols, cwd, env, app| {
                super::popup::spawn_argv_runtime(app, argv, pane_id, rows, cols, cwd, env)
            },
        )?;
        self.state.floats.push(FloatState {
            id,
            pane_id,
            terminal_id,
            width: spec.width,
            height: spec.height,
            position: spec.position,
            visibility: FloatVisibility::Shown,
            hide_while_focused: spec.hide_while_focused,
        });
        self.request_float_render();
        Ok(id)
    }

    /// Returns `None` for an unknown float, else whether anything changed.
    pub(crate) fn edit_float(&mut self, id: FloatId, edit: FloatEdit) -> Option<bool> {
        let changed = self.state.edit_float(id, edit)?;
        if changed {
            if matches!(edit, FloatEdit::Resize { .. } | FloatEdit::Visibility(_)) {
                self.resize_float_runtime(id, self.state.float_area());
            }
            self.request_float_render();
        }
        Some(changed)
    }

    pub(crate) fn close_float(&mut self, id: FloatId) -> bool {
        let Some(float) = self.state.remove_float(id) else {
            return false;
        };
        self.shutdown_terminal_runtime(float.terminal_id);
        self.request_float_render();
        true
    }

    pub(crate) fn resize_float_runtime(&self, id: FloatId, area: Rect) {
        if let Some(float) = self.state.float(id) {
            resize_float_runtime(
                &self.state,
                &self.terminal_runtimes,
                float,
                area,
                self.state.host_cell_size,
            );
        }
    }

    fn request_float_render(&self) {
        self.render_dirty.request_generic();
        self.render_notify.notify_one();
    }
}

#[cfg(unix)]
impl App {
    /// Float state for a live handoff, bottom to top, with the terminal each entry came from.
    pub(crate) fn handoff_floats(&self) -> Vec<(TerminalId, crate::handoff_runtime::HandoffFloat)> {
        self.state
            .floats
            .iter()
            .filter_map(|float| {
                let runtime = self.terminal_runtimes.get(&float.terminal_id)?;
                let terminal = self.state.terminals.get(&float.terminal_id)?;
                let mut handoff_runtime = runtime.handoff_runtime_state(float.pane_id.raw());
                handoff_runtime.initial_history_ansi = runtime.handoff_history_ansi();
                Some((
                    float.terminal_id.clone(),
                    crate::handoff_runtime::HandoffFloat {
                        id: Some(float.id),
                        runtime: handoff_runtime,
                        cwd: terminal.cwd.clone(),
                        title: terminal.manual_label.clone(),
                        launch_argv: terminal.launch_argv.clone(),
                        width: float.width,
                        height: float.height,
                        position: match float.position {
                            FloatPosition::Centered => None,
                            FloatPosition::At { x, y } => Some((x, y)),
                        },
                        hidden: float.visibility == FloatVisibility::Hidden,
                        hide_while_focused: float
                            .hide_while_focused
                            .filter(|pane_id| self.find_pane(*pane_id).is_some())
                            .map(PaneId::raw),
                    },
                ))
            })
            .collect()
    }

    /// Rebuilds floats from a handoff. A float whose runtime cannot be restored is dropped
    /// and its fd closed; the panes it floated over are unaffected.
    pub(crate) fn restore_handoff_floats(
        &mut self,
        snapshot: &crate::persist::SessionSnapshot,
        floats: crate::handoff_runtime::HandoffFloats,
        fds: Vec<std::os::fd::RawFd>,
    ) {
        let handoff_panes = crate::persist::handoff_pane_ids(snapshot, &self.state.workspaces);
        if let Some(next) = floats.next_float_id {
            self.state.reserve_float_ids_below(next);
        }
        for (float, fd) in floats.floats.into_iter().zip(fds) {
            let pane_id = PaneId::alloc();
            let runtime = match TerminalRuntime::from_handoff_fd(
                crate::handoff_runtime::ImportedHandoffRuntime {
                    master_fd: fd,
                    state: float.runtime.with_pane_id(pane_id),
                },
                self.state.pane_scrollback_limit_bytes,
                self.state.host_terminal_theme,
                self.state.host_terminal_appearance,
                self.event_tx.clone(),
                self.render_notify.clone(),
                self.render_dirty.clone(),
                crate::pane::AgentDetection::Disabled,
            ) {
                Ok(runtime) => runtime,
                Err(err) => {
                    tracing::warn!(err = %err, "dropping float that failed to restore after handoff");
                    continue;
                }
            };
            let terminal_id = TerminalId::alloc();
            let mut terminal = crate::terminal::TerminalState::new(terminal_id.clone(), float.cwd);
            if let Some(argv) = float.launch_argv {
                terminal = terminal.with_launch_argv(argv);
            }
            if let Some(title) = float.title {
                terminal.set_manual_label(title);
            }
            let hide_while_focused = float
                .hide_while_focused
                .and_then(|raw| handoff_panes.get(&raw).copied())
                .filter(|pane_id| self.find_pane(*pane_id).is_some());
            let id = float.id.unwrap_or_else(|| self.state.mint_float_id());
            self.terminal_runtimes.insert(terminal_id.clone(), runtime);
            self.state.terminals.insert(terminal_id.clone(), terminal);
            self.state.floats.push(FloatState {
                id,
                pane_id,
                terminal_id,
                width: float.width,
                height: float.height,
                position: float
                    .position
                    .map_or(FloatPosition::Centered, |(x, y)| FloatPosition::At { x, y }),
                visibility: if float.hidden {
                    FloatVisibility::Hidden
                } else {
                    FloatVisibility::Shown
                },
                hide_while_focused,
            });
        }
    }
}

/// Hidden floats keep their PTY size; only shown floats follow the surface geometry.
pub(crate) fn resize_float_runtime(
    state: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    float: &FloatState,
    area: Rect,
    cell_size: crate::kitty_graphics::HostCellSize,
) {
    if float.visibility == FloatVisibility::Hidden
        || state
            .direct_attach_resize_locks
            .contains(&float.terminal_id)
    {
        return;
    }
    let Some(geometry) = float.geometry(area) else {
        return;
    };
    if let Some(runtime) = terminal_runtimes.get(&float.terminal_id) {
        runtime.resize(
            geometry.inner.height,
            geometry.inner.width,
            cell_size.width_px,
            cell_size.height_px,
        );
    }
}

#[cfg(test)]
impl App {
    pub(crate) fn install_test_float_runtime(
        &mut self,
        runtime: crate::terminal::TerminalRuntime,
        spec: FloatSpec,
    ) -> (PaneId, TerminalId, FloatId) {
        let pane_id = PaneId::alloc();
        let terminal_id = TerminalId::alloc();
        let id = self.state.mint_float_id();
        self.terminal_runtimes.insert(terminal_id.clone(), runtime);
        self.state.terminals.insert(
            terminal_id.clone(),
            crate::terminal::TerminalState::new(terminal_id.clone(), PathBuf::from("/float")),
        );
        self.state.floats.push(FloatState {
            id,
            pane_id,
            terminal_id: terminal_id.clone(),
            width: spec.width,
            height: spec.height,
            position: spec.position,
            visibility: FloatVisibility::Shown,
            hide_while_focused: spec.hide_while_focused,
        });
        (pane_id, terminal_id, id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::TerminalState;

    fn spec() -> FloatSpec {
        FloatSpec {
            width: Some(PopupSize::Cells(20)),
            height: Some(PopupSize::Cells(8)),
            position: FloatPosition::At { x: 2, y: 3 },
            hide_while_focused: None,
        }
    }

    fn state_with_floats(count: usize) -> (AppState, Vec<FloatId>) {
        let mut state = AppState::test_new();
        state.workspaces = vec![crate::workspace::Workspace::test_new("floats")];
        state.ensure_test_terminals();
        state.active = Some(0);
        let ids = (0..count)
            .map(|_| {
                let terminal_id = TerminalId::alloc();
                state.terminals.insert(
                    terminal_id.clone(),
                    TerminalState::new(terminal_id.clone(), PathBuf::from("/float")),
                );
                let id = state.mint_float_id();
                state.floats.push(FloatState {
                    id,
                    pane_id: PaneId::alloc(),
                    terminal_id,
                    width: spec().width,
                    height: spec().height,
                    position: spec().position,
                    visibility: FloatVisibility::Shown,
                    hide_while_focused: None,
                });
                id
            })
            .collect();
        (state, ids)
    }

    fn z_order(state: &AppState) -> Vec<FloatId> {
        state.floats.iter().map(|float| float.id).collect()
    }

    #[test]
    fn hide_while_focused_hides_only_for_viewers_focused_on_that_pane() {
        let source = PaneId::alloc();
        let other = PaneId::alloc();
        let float = FloatState {
            id: FloatId::FIRST,
            pane_id: PaneId::alloc(),
            terminal_id: TerminalId::alloc(),
            width: None,
            height: None,
            position: FloatPosition::Centered,
            visibility: FloatVisibility::Shown,
            hide_while_focused: Some(source),
        };

        assert!(!float.visible_to(Some(source)));
        assert!(float.visible_to(Some(other)));
        assert!(float.visible_to(None));
    }

    #[test]
    fn hidden_float_is_invisible_to_every_viewer() {
        let float = FloatState {
            id: FloatId::FIRST,
            pane_id: PaneId::alloc(),
            terminal_id: TerminalId::alloc(),
            width: None,
            height: None,
            position: FloatPosition::Centered,
            visibility: FloatVisibility::Hidden,
            hide_while_focused: None,
        };

        assert!(!float.visible_to(None));
        assert!(!float.visible_to(Some(PaneId::alloc())));
    }

    #[test]
    fn raise_moves_float_to_top_of_z_order() {
        let (mut state, ids) = state_with_floats(3);

        assert_eq!(state.edit_float(ids[0], FloatEdit::Raise), Some(true));
        assert_eq!(z_order(&state), vec![ids[1], ids[2], ids[0]]);

        assert_eq!(state.edit_float(ids[0], FloatEdit::Raise), Some(false));
        state.assert_invariants_for_test();
    }

    #[test]
    fn edits_report_unknown_float_and_unchanged_values() {
        let (mut state, ids) = state_with_floats(1);
        let id = ids[0];

        assert_eq!(state.edit_float(id.next(), FloatEdit::Raise), None);
        assert_eq!(
            state.edit_float(id, FloatEdit::Move(FloatPosition::At { x: 2, y: 3 })),
            Some(false)
        );
        assert_eq!(
            state.edit_float(id, FloatEdit::Move(FloatPosition::At { x: 9, y: 1 })),
            Some(true)
        );
        assert_eq!(
            state.float(id).unwrap().position,
            FloatPosition::At { x: 9, y: 1 }
        );
        assert_eq!(
            state.edit_float(id, FloatEdit::Visibility(FloatVisibility::Hidden)),
            Some(true)
        );
        assert_eq!(state.float(id).unwrap().visibility, FloatVisibility::Hidden);
        assert_eq!(
            state.edit_float(
                id,
                FloatEdit::Resize {
                    width: Some(PopupSize::Percent(50)),
                    height: None
                }
            ),
            Some(true)
        );
        assert_eq!(state.float(id).unwrap().width, Some(PopupSize::Percent(50)));
        assert_eq!(state.float(id).unwrap().height, None);
        state.assert_invariants_for_test();
    }

    #[test]
    fn remove_float_detaches_terminal_and_resize_lock() {
        let (mut state, ids) = state_with_floats(2);
        let terminal_id = state.float(ids[0]).unwrap().terminal_id.clone();
        state.direct_attach_resize_locks.insert(terminal_id.clone());

        let removed = state.remove_float(ids[0]).unwrap();

        assert_eq!(removed.id, ids[0]);
        assert!(!state.terminals.contains_key(&terminal_id));
        assert!(!state.direct_attach_resize_locks.contains(&terminal_id));
        assert_eq!(z_order(&state), vec![ids[1]]);
        assert!(state.remove_float(ids[0]).is_none());
        state.assert_invariants_for_test();
    }

    #[test]
    fn floats_survive_workspace_removal() {
        let (mut state, _ids) = state_with_floats(2);
        state.workspaces.clear();
        state.active = None;

        state.assert_invariants_for_test();

        assert_eq!(state.floats.len(), 2);
    }

    fn app_with_float() -> (App, PaneId, TerminalId, FloatId) {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &crate::config::Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            crate::api::EventHub::default(),
        );
        app.state.workspaces = vec![crate::workspace::Workspace::test_new("floats")];
        app.state.ensure_test_terminals();
        app.state.active = Some(0);
        let (mut state, _ids) = state_with_floats(1);
        let float = state.floats.remove(0);
        let terminal = state.terminals.remove(&float.terminal_id).unwrap();
        app.state
            .terminals
            .insert(float.terminal_id.clone(), terminal);
        app.state.next_float_id = state.next_float_id;
        app.state.floats.push(float.clone());
        (app, float.pane_id, float.terminal_id, float.id)
    }

    #[test]
    fn process_exit_removes_the_float_and_keeps_the_layout() {
        let (mut app, pane_id, terminal_id, _float_id) = app_with_float();
        let tiled_pane = app.state.workspaces[0].tabs[0].root_pane;

        app.handle_internal_event(crate::events::AppEvent::PaneDied {
            pane_id,
            exit_reason: crate::platform::ChildExitReason::Exited,
        });

        assert!(app.state.floats.is_empty());
        assert!(!app.state.terminals.contains_key(&terminal_id));
        assert!(app.find_pane(tiled_pane).is_some());
        app.state.assert_invariants_for_test();
    }

    #[test]
    fn close_float_reports_whether_a_float_was_removed() {
        let (mut app, _pane_id, _terminal_id, float_id) = app_with_float();

        assert!(app.close_float(float_id));
        assert!(!app.close_float(float_id));
        assert!(app.state.floats.is_empty());
    }

    #[test]
    fn a_hide_rule_ends_when_its_pane_dies() {
        let (mut app, _pane_id, _terminal_id, _float_id) = app_with_float();
        let tiled_pane = app.state.workspaces[0].tabs[0].root_pane;
        app.state.floats[0].hide_while_focused = Some(tiled_pane);

        app.handle_internal_event(crate::events::AppEvent::PaneDied {
            pane_id: tiled_pane,
            exit_reason: crate::platform::ChildExitReason::Exited,
        });

        assert_eq!(app.state.floats[0].hide_while_focused, None);
        app.state.assert_invariants_for_test();
    }

    #[test]
    fn a_hide_rule_ends_when_its_workspace_closes() {
        let (mut state, _ids) = state_with_floats(1);
        state.floats[0].hide_while_focused = Some(state.workspaces[0].tabs[0].root_pane);

        state.close_selected_workspace();

        assert_eq!(state.floats[0].hide_while_focused, None);
        state.assert_invariants_for_test();
    }

    #[cfg(unix)]
    fn handoff_pty() -> (std::os::fd::RawFd, std::fs::File) {
        use std::os::fd::{FromRawFd, OwnedFd};
        let mut master = -1;
        let mut slave = -1;
        assert_eq!(
            unsafe {
                libc::openpty(
                    &mut master,
                    &mut slave,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            },
            0
        );
        (
            master,
            std::fs::File::from(unsafe { OwnedFd::from_raw_fd(slave) }),
        )
    }

    #[cfg(unix)]
    fn handoff_float(hide_while_focused: Option<u32>) -> crate::handoff_runtime::HandoffFloat {
        crate::handoff_runtime::HandoffFloat {
            id: None,
            runtime: crate::handoff_runtime::HandoffRuntimeState {
                pane_id: 0,
                child_pid: 0,
                rows: 6,
                cols: 18,
                cell_width_px: 0,
                cell_height_px: 0,
                keyboard_protocol_flags: 0,
                keyboard_protocol_ansi: None,
                input_state: None,
                terminal_title: None,
                initial_history_ansi: None,
                agent_state: None,
            },
            cwd: PathBuf::from("/float"),
            title: None,
            launch_argv: None,
            width: spec().width,
            height: spec().height,
            position: Some((2, 3)),
            hidden: false,
            hide_while_focused,
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_restored_hide_rule_binds_only_to_a_pane_the_handoff_carried() {
        let (mut app, _pane_id, _terminal_id, float_id) = app_with_float();
        app.close_float(float_id);
        let carried = app.state.workspaces[0].tabs[0].root_pane;
        let snapshot = crate::persist::capture(
            &app.state.workspaces,
            &app.state.terminals,
            &app.terminal_runtimes,
            app.state.active,
            app.state.selected,
        );
        let unrelated = app.state.workspaces[0].test_split(ratatui::layout::Direction::Horizontal);
        let (carried_fd, _carried_slave) = handoff_pty();
        let (dangling_fd, _dangling_slave) = handoff_pty();

        app.restore_handoff_floats(
            &snapshot,
            crate::handoff_runtime::HandoffFloats {
                floats: vec![
                    handoff_float(Some(carried.raw())),
                    handoff_float(Some(unrelated.raw())),
                ],
                next_float_id: None,
            },
            vec![carried_fd, dangling_fd],
        );

        let rules: Vec<_> = app
            .state
            .floats
            .iter()
            .map(|float| float.hide_while_focused)
            .collect();
        assert_eq!(rules, vec![Some(carried), None]);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_restored_float_runs_no_agent_detection() {
        let (mut app, _pane_id, _terminal_id, float_id) = app_with_float();
        app.close_float(float_id);
        let snapshot = crate::persist::capture(
            &app.state.workspaces,
            &app.state.terminals,
            &app.terminal_runtimes,
            app.state.active,
            app.state.selected,
        );
        let (fd, _slave) = handoff_pty();

        app.restore_handoff_floats(
            &snapshot,
            crate::handoff_runtime::HandoffFloats {
                floats: vec![handoff_float(None)],
                next_float_id: None,
            },
            vec![fd],
        );

        let restored = &app.state.floats[0].terminal_id;
        let runtime = app
            .terminal_runtimes
            .get(restored)
            .expect("restored runtime");
        assert!(!runtime.test_runs_agent_detection());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn restored_floats_keep_their_ids_and_new_floats_never_reuse_one() {
        let (mut app, _pane_id, _terminal_id, float_id) = app_with_float();
        app.close_float(float_id);
        let snapshot = crate::persist::capture(
            &app.state.workspaces,
            &app.state.terminals,
            &app.terminal_runtimes,
            app.state.active,
            app.state.selected,
        );
        let mut importer = app_with_float().0;
        importer.state.floats.clear();
        importer.state.next_float_id = FloatId::FIRST;
        let carried = FloatId::parse("f2").unwrap();
        let closed_at_source = FloatId::parse("f5").unwrap();
        let (fd, _slave) = handoff_pty();
        let (legacy_fd, _legacy_slave) = handoff_pty();

        importer.restore_handoff_floats(
            &snapshot,
            crate::handoff_runtime::HandoffFloats {
                floats: vec![
                    crate::handoff_runtime::HandoffFloat {
                        id: Some(carried),
                        ..handoff_float(None)
                    },
                    handoff_float(None),
                ],
                next_float_id: Some(closed_at_source.next()),
            },
            vec![fd, legacy_fd],
        );

        let restored: Vec<_> = importer.state.floats.iter().map(|float| float.id).collect();
        assert_eq!(restored[0], carried);
        assert!(restored[1] > closed_at_source, "{restored:?}");
        let minted = importer.state.mint_float_id();
        assert!(minted > restored[1], "{minted} after {restored:?}");
        importer.state.assert_invariants_for_test();
    }

    #[test]
    #[should_panic(expected = "float")]
    fn invariants_reject_float_without_terminal() {
        let (mut state, ids) = state_with_floats(1);
        let terminal_id = state.float(ids[0]).unwrap().terminal_id.clone();
        state.terminals.remove(&terminal_id);

        state.assert_invariants_for_test();
    }

    #[test]
    #[should_panic(expected = "float")]
    fn invariants_reject_duplicate_float_terminal() {
        let (mut state, _ids) = state_with_floats(1);
        let mut duplicate = state.floats[0].clone();
        duplicate.id = state.mint_float_id();
        duplicate.pane_id = PaneId::alloc();
        state.floats.push(duplicate);

        state.assert_invariants_for_test();
    }
}
