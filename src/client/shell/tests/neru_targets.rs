use super::*;
use crate::client::shell::neru_targets::{collect, NeruControl};
use crossterm::event::{MouseButton, MouseEventKind};

fn shell() -> ClientShellState {
    let mut projected = snapshot();
    let mut tab = projected.tabs[0].clone();
    tab.tab_id = "tab_2".into();
    tab.number = 2;
    tab.label = "logs".into();
    tab.custom_label = true;
    tab.focused = false;
    projected.tabs.push(tab.clone());
    tab.tab_id = "tab_3".into();
    tab.number = 3;
    tab.label = "日本".into();
    projected.tabs.push(tab);
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(projected));
    state.set_pane_surface(surface());
    state.compose(106, 24).unwrap();
    state
}

fn controls(state: &ClientShellState) -> Vec<NeruControl> {
    collect(&state.hits, &state.neru_layer_state()).expect("no popup")
}

fn press_center(state: &mut ClientShellState, control: &NeruControl) -> ClientShellInput {
    let column = control.rect.x + control.rect.width / 2;
    let row = control.rect.y + control.rect.height / 2;
    state.handle_raw_events(vec![crate::raw_input::RawInputEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: crossterm::event::KeyModifiers::empty(),
    })])
}

fn find<'a>(controls: &'a [NeruControl], title: &str) -> &'a NeruControl {
    controls
        .iter()
        .find(|control| control.title == title)
        .unwrap_or_else(|| panic!("no {title:?} in {controls:?}"))
}

#[test]
fn base_chrome_targets_carry_drawn_titles_in_dispatch_order() {
    let state = shell();

    let titles: Vec<_> = controls(&state).into_iter().map(|c| c.title).collect();

    assert_eq!(
        titles,
        [
            "toggle sidebar",
            "grouped",
            "menu",
            "new",
            "new tab",
            "○ client-shell main",
            "1",
            "logs",
            "日本"
        ]
    );
}

#[test]
fn a_press_at_each_base_chrome_target_center_runs_that_control() {
    let targets = controls(&shell());

    let mut state = shell();
    press_center(&mut state, find(&targets, "○ client-shell main"));
    assert_eq!(state.workspace_press.as_ref().unwrap().workspace_id, "ws_1");

    let mut state = shell();
    press_center(&mut state, find(&targets, "logs"));
    assert_eq!(state.tab_press.as_ref().unwrap().tab_id, "tab_2");

    let mut state = shell();
    press_center(&mut state, find(&targets, "toggle sidebar"));
    assert!(state.sidebar_collapsed);

    let mut state = shell();
    let sort = state.config.agent_panel_sort;
    press_center(&mut state, find(&targets, "grouped"));
    assert_ne!(state.config.agent_panel_sort, sort);

    let mut state = shell();
    press_center(&mut state, find(&targets, "menu"));
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::GlobalMenu(_))
    ));

    for title in ["new", "new tab"] {
        let mut state = shell();
        let outcome = press_center(&mut state, find(&targets, title));
        assert!(
            state.overlay.is_some() || !outcome.requests.is_empty() || !outcome.actions.is_empty(),
            "{title} did nothing"
        );
    }
}

#[test]
fn an_open_menu_gives_only_its_launcher_and_rows_and_a_row_press_activates_it() {
    let mut state = shell();
    state.toggle_global_menu();
    state.compose(106, 24).unwrap();
    let targets = controls(&state);
    let rows = state.hits.global_menu_rows.len();

    assert!(rows > 0);
    assert_eq!(targets.len(), rows + 1, "{targets:?}");
    assert_eq!(targets[0].title, "menu");
    assert!(targets.iter().all(|target| target.title != "new tab"));

    let row = &targets[1];
    let index = state
        .hits
        .global_menu_rows
        .iter()
        .find(|(rect, _)| rect.contains((row.rect.x + row.rect.width / 2, row.rect.y).into()))
        .map(|(_, index)| *index);
    assert_eq!(index, Some(0), "{row:?}");
    press_center(&mut state, row);
    assert!(!matches!(
        state.overlay,
        Some(ClientShellOverlay::GlobalMenu(_))
    ));
}

/// Two side-by-side panes with top borders: `pane_1` (focused, border title `claude`) and
/// `pane_2` (unfocused, no border title, agent `codex`), each with a cursor on its prompt.
fn two_pane_shell(pane_1_cursor_visible: bool) -> ClientShellState {
    let mut projected = snapshot();
    let mut second = projected.panes[0].clone();
    second.pane_id = "pane_2".into();
    second.focused = false;
    projected.panes.push(second);
    projected.agents.push(ClientShellAgent {
        pane_id: "pane_2".into(),
        workspace_id: "ws_1".into(),
        tab_id: "tab_1".into(),
        name: Some("codex".into()),
        display_agent: None,
        agent: Some("codex".into()),
        title: Some("terminal title".into()),
        terminal_title: None,
        terminal_title_stripped: None,
        agent_status: AgentStatus::Idle,
        state_change_seq: 0,
        state_labels: Vec::new(),
        tokens: Vec::new(),
        focused: false,
    });
    let buffer = Buffer::with_lines([
        "─ claude ───────────",
        "$ ls      > hi      ",
        "                    ",
    ]);
    let mut frame = surface();
    frame.frame = FrameData::from_ratatui_buffer_with_hyperlinks(&buffer, None, &[]);
    let pane = |pane_id: &str, x: u16, focused: bool| PaneSurfacePane {
        pane_id: pane_id.into(),
        rect: SurfaceRect {
            x,
            y: 0,
            width: 10,
            height: 3,
        },
        inner_rect: SurfaceRect {
            x,
            y: 1,
            width: 10,
            height: 2,
        },
        focused,
        ..frame.panes[0].clone()
    };
    frame.panes = vec![pane("pane_1", 0, true), pane("pane_2", 10, false)];
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(projected));
    state.set_pane_surface(frame);
    let cursor = |pane_id: &str, visible: bool| crate::protocol::endpoint::EndpointPaneCursor {
        pane_id: pane_id.into(),
        x: 2,
        y: 0,
        visible,
    };
    state.set_pane_cursors(Some(crate::protocol::endpoint::EndpointPaneCursors {
        boot_id: "boot-1".into(),
        cursors: vec![
            cursor("pane_1", pane_1_cursor_visible),
            cursor("pane_2", true),
        ],
    }));
    state.compose(106, 24).unwrap();
    state
}

#[test]
fn each_visible_pane_cursor_is_a_target_and_a_press_there_focuses_its_pane() {
    let state = two_pane_shell(true);
    let targets = controls(&state);
    let inner = |pane_id: &str| {
        state
            .hits
            .panes
            .iter()
            .find(|hit| hit.pane_id == pane_id)
            .unwrap()
            .inner_rect
    };

    let claude = find(&targets, "claude");
    let codex = find(&targets, "codex");
    assert_eq!(
        claude.rect,
        Rect::new(inner("pane_1").x + 2, inner("pane_1").y, 1, 1)
    );
    assert_eq!(
        codex.rect,
        Rect::new(inner("pane_2").x + 2, inner("pane_2").y, 1, 1)
    );
    // Pane bodies give nothing else.
    assert!(targets.iter().all(|target| target.title != "pane"));

    let mut state = two_pane_shell(true);
    let outcome = press_center(&mut state, codex);
    let focused: Vec<_> = outcome
        .actions
        .iter()
        .filter_map(|action| match action {
            ClientShellAction::Endpoint { request, .. } => match &request.method {
                crate::api::schema::Method::PaneFocus(target) => Some(target.pane_id.as_str()),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert_eq!(focused, ["pane_2"]);
}

#[test]
fn a_hidden_pane_cursor_gives_no_target() {
    let state = two_pane_shell(false);

    let targets = controls(&state);

    assert!(
        targets.iter().all(|target| target.title != "claude"),
        "{targets:?}"
    );
    find(&targets, "codex");
}

mod session {
    use super::*;
    use crate::client::shell::neru_calibration::{
        NeruCalibrated, NeruCalibrationError, NeruCalibrationOutcome, POINTER_NOT_RESTORED,
    };
    use crate::client::shell::neru_session::NeruCalibrationDone;
    use crate::client::shell::neru_targets::{NeruCalibration, NeruScreenRect};

    const GRID: (u16, u16) = (106, 24);
    const EXTENT: (u32, u32) = (106 * 16, 24 * 32);

    fn calibrated(pointer_restored: bool) -> NeruCalibrationOutcome {
        NeruCalibrationOutcome {
            result: Ok(NeruCalibrated {
                calibration: NeruCalibration::new(GRID, EXTENT, (100.0, 50.0), (2.0, 2.0)).unwrap(),
                displays: vec![NeruScreenRect {
                    x: 0,
                    y: 0,
                    width: 3000,
                    height: 2000,
                }],
            }),
            pointer_restored,
        }
    }

    fn notice(state: &ClientShellState) -> Option<&str> {
        state
            .visible_endpoint_notice
            .as_ref()
            .map(|notice| notice.body.as_str())
    }

    #[test]
    fn targets_come_from_the_hit_map_when_calibration_ends() {
        let mut state = shell();
        // The menu opens while calibration runs: its rows are what the screen shows now.
        state.toggle_global_menu();
        state.compose(GRID.0, GRID.1).unwrap();
        let done = NeruCalibrationDone::for_test(Ok(calibrated(true)), GRID, EXTENT);

        let (json, _) = state.finish_neru_calibration(done, GRID, EXTENT);

        let json = json.expect("targets");
        let hints: serde_json::Value = serde_json::from_str(&json).unwrap();
        let hints = hints["hints"].as_array().unwrap();
        assert_eq!(hints.len(), state.hits.global_menu_rows.len() + 1);
        assert!(!json.contains("\"new tab\""), "{json}");
        assert_eq!(notice(&state), None);
    }

    #[test]
    fn a_resize_during_calibration_sends_nothing_and_says_why() {
        let mut state = shell();
        let done = NeruCalibrationDone::for_test(Ok(calibrated(true)), GRID, EXTENT);

        let (json, repaint) = state.finish_neru_calibration(done, (GRID.0 - 1, GRID.1), EXTENT);

        assert_eq!(json, None);
        assert!(repaint);
        assert_eq!(
            notice(&state),
            Some("Calibration failed: the terminal resized. Try again.")
        );
    }

    #[test]
    fn a_failed_calibration_sends_nothing_and_reports_its_reason() {
        let mut state = shell();
        let outcome = NeruCalibrationOutcome {
            result: Err(NeruCalibrationError::PointerMoved),
            pointer_restored: true,
        };
        let done = NeruCalibrationDone::for_test(Ok(outcome), GRID, EXTENT);

        let (json, _) = state.finish_neru_calibration(done, GRID, EXTENT);

        assert_eq!(json, None);
        assert_eq!(
            notice(&state),
            Some("Calibration failed: the pointer moved. Keep the mouse still and try again.")
        );
    }

    #[test]
    fn a_second_press_failing_for_the_same_reason_reports_it_again() {
        let mut state = shell();
        let failed = || {
            let outcome = NeruCalibrationOutcome {
                result: Err(NeruCalibrationError::PointerMoved),
                pointer_restored: true,
            };
            NeruCalibrationDone::for_test(Ok(outcome), GRID, EXTENT)
        };
        let moved = "Calibration failed: the pointer moved. Keep the mouse still and try again.";

        state.finish_neru_calibration(failed(), GRID, EXTENT);
        assert_eq!(notice(&state), Some(moved));
        state.visible_endpoint_notice = None;
        state.finish_neru_calibration(failed(), GRID, EXTENT);
        assert_eq!(notice(&state), Some(moved), "§V.16: calibration failure");

        let missing = "neru not found at /nowhere/neru.";
        assert!(state.show_neru_notice(missing));
        state.visible_endpoint_notice = None;
        assert!(state.show_neru_notice(missing));
        assert_eq!(notice(&state), Some(missing), "§V.16: missing neru");
    }

    #[test]
    fn an_unrestored_pointer_is_reported_with_the_calibration_failure() {
        let mut state = shell();
        let outcome = NeruCalibrationOutcome {
            result: Err(NeruCalibrationError::PointerMoved),
            pointer_restored: false,
        };
        let done = NeruCalibrationDone::for_test(Ok(outcome), GRID, EXTENT);

        let (json, _) = state.finish_neru_calibration(done, GRID, EXTENT);

        assert_eq!(json, None);
        let body = notice(&state).expect("notice");
        assert!(
            body.starts_with("Calibration failed: the pointer moved."),
            "{body}"
        );
        assert!(body.ends_with(POINTER_NOT_RESTORED), "§V.8: {body}");
    }

    #[test]
    fn an_unrestored_pointer_is_reported_after_a_success() {
        let mut state = shell();
        let done = NeruCalibrationDone::for_test(Ok(calibrated(false)), GRID, EXTENT);

        let (json, _) = state.finish_neru_calibration(done, GRID, EXTENT);

        assert_eq!(notice(&state), Some(POINTER_NOT_RESTORED));
        // The notice's toast is drawn before the targets are collected, so the only target on
        // it is the toast itself (layer 4), and nothing under it is sent.
        let toast = state.hits.notification_toast;
        assert!(!toast.is_empty(), "the notice draws a toast");
        let json: serde_json::Value = serde_json::from_str(&json.expect("targets")).unwrap();
        let hints = json["hints"].as_array().unwrap();
        let on_toast = hints
            .iter()
            .filter(|hint| {
                let center = |start: &str, size: &str| {
                    hint[start].as_i64().unwrap() + hint[size].as_i64().unwrap() / 2
                };
                // Origin (100, 50) pt; a 16 x 32 px cell at scale 2 is 8 x 16 pt.
                let column = u16::try_from((center("x", "width") - 100) / 8).unwrap();
                let row = u16::try_from((center("y", "height") - 50) / 16).unwrap();
                toast.contains((column, row).into())
            })
            .count();
        assert_eq!(on_toast, 1, "{hints:?}");
        assert!(hints.len() > 1);
    }

    #[test]
    fn the_keybinding_requests_neru_hints_once() {
        let mut state = shell();
        state.record_binding(
            crate::input::KeybindMatch::Action(crate::input::KeybindAction::NeruHints),
            &mut ClientShellInput::default(),
        );
        assert!(state.take_neru_hints_request());
        assert!(!state.take_neru_hints_request());
    }
}

mod menu_binding {
    //! §V.15: an open menu lets `keys.neru_hints` through and stays open.
    use super::*;
    use crate::raw_input::RawInputEvent;

    fn press(state: &mut ClientShellState, code: KeyCode, modifiers: KeyModifiers) {
        state.handle_raw_events(vec![RawInputEvent::Key(crate::input::TerminalKey::new(
            code, modifiers,
        ))]);
    }

    fn prefix_then(state: &mut ClientShellState, code: KeyCode) {
        press(state, KeyCode::Char('b'), KeyModifiers::CONTROL);
        press(state, code, KeyModifiers::empty());
    }

    fn open_menus() -> [(&'static str, ClientShellState); 2] {
        let mut global = shell();
        global.toggle_global_menu();
        let mut context = shell();
        context.open_tab_context_menu("tab_2".into(), 10, 3);
        assert!(matches!(
            context.overlay,
            Some(ClientShellOverlay::ContextMenu(_))
        ));
        [("global", global), ("context", context)]
    }

    fn menu_open(state: &ClientShellState) -> bool {
        matches!(
            state.overlay,
            Some(ClientShellOverlay::ContextMenu(_) | ClientShellOverlay::GlobalMenu(_))
        )
    }

    #[test]
    fn the_binding_runs_the_action_and_leaves_each_menu_open() {
        for (name, mut state) in open_menus() {
            press(&mut state, KeyCode::Char('b'), KeyModifiers::CONTROL);
            assert_eq!(state.mode, ClientShellMode::Prefix, "{name}");
            assert!(menu_open(&state), "{name}: prefix keeps the menu");

            press(&mut state, KeyCode::Char('f'), KeyModifiers::empty());

            assert!(state.take_neru_hints_request(), "{name}");
            assert!(menu_open(&state), "{name}: the action keeps the menu");
            assert_eq!(state.mode, ClientShellMode::Terminal, "{name}");
        }
    }

    #[test]
    fn the_binding_runs_from_a_menu_opened_in_resize_or_navigate_mode() {
        use crate::input::{KeybindAction, KeybindMatch};
        for (action, mode) in [
            (KeybindAction::EnterResizeMode, ClientShellMode::Resize),
            (KeybindAction::WorkspacePicker, ClientShellMode::Navigate),
        ] {
            for (name, mut state) in open_menus() {
                state.record_binding(
                    KeybindMatch::Action(action),
                    &mut ClientShellInput::default(),
                );
                assert_eq!(state.mode, mode, "{name}");
                assert!(menu_open(&state), "{name} {mode:?}");

                prefix_then(&mut state, KeyCode::Char('f'));

                assert!(state.take_neru_hints_request(), "{name} {mode:?}");
                assert!(menu_open(&state), "{name} {mode:?}");
                assert_eq!(state.mode, ClientShellMode::Terminal, "{name} {mode:?}");
                assert_eq!(state.navigate_workspace_id, None, "{name} {mode:?}");
            }
        }
    }

    #[test]
    fn another_key_after_the_prefix_reaches_the_menu() {
        for (name, mut state) in open_menus() {
            prefix_then(&mut state, KeyCode::Down);

            assert!(!state.take_neru_hints_request(), "{name}");
            assert_eq!(state.mode, ClientShellMode::Terminal, "{name}");
            let highlighted = match &state.overlay {
                Some(ClientShellOverlay::GlobalMenu(menu)) => menu.highlighted,
                Some(ClientShellOverlay::ContextMenu(menu)) => menu.highlighted,
                other => panic!("{name}: menu closed: {other:?}"),
            };
            assert_eq!(
                highlighted, 1,
                "{name}: Down after the prefix moves the menu"
            );

            prefix_then(&mut state, KeyCode::Esc);
            assert!(
                !menu_open(&state),
                "{name}: Esc after the prefix closes the menu"
            );
        }
    }

    #[test]
    fn a_text_input_overlay_takes_the_binding_as_text() {
        let mut state = shell();
        state.overlay = Some(ClientShellOverlay::Rename(ClientRenameOverlay {
            title: "rename pane",
            input: TextEditor::new("", true),
            target: ClientRenameTarget::Pane {
                pane_id: "pane_1".into(),
            },
        }));

        prefix_then(&mut state, KeyCode::Char('f'));

        assert!(!state.take_neru_hints_request());
        assert_eq!(state.mode, ClientShellMode::Terminal);
        assert!(matches!(
            state.overlay,
            Some(ClientShellOverlay::Rename(ClientRenameOverlay { ref input, .. }))
                if input.as_str() == "f"
        ));
    }
}

mod cli_request {
    //! §V.21, §V.23: `herdr neru-hints` runs the key path, and only for the local server.
    use super::*;

    fn request(state: &mut ClientShellState, origin: &ClientEndpointId) -> ClientShellInput {
        let mut outcome = ClientShellInput::default();
        state.request_neru_hints(origin, &mut outcome);
        outcome
    }

    fn remote_id() -> ClientEndpointId {
        ClientEndpointId::Ssh(
            crate::client::endpoint::ProfileId::parse("0123456789abcdef0123456789abcdef").unwrap(),
        )
    }

    fn notice(state: &ClientShellState) -> Option<&str> {
        state
            .visible_endpoint_notice
            .as_ref()
            .map(|notice| notice.body.as_str())
    }

    #[test]
    fn a_request_and_the_key_set_the_same_request_once() {
        let mut state = shell();
        request(&mut state, &ClientEndpointId::Local);
        assert!(state.take_neru_hints_request());
        assert!(!state.take_neru_hints_request());
        assert_eq!(notice(&state), None);
    }

    #[test]
    fn a_request_with_a_menu_open_leaves_it_open() {
        let mut global = shell();
        global.toggle_global_menu();
        let mut context = shell();
        context.open_tab_context_menu("tab_2".into(), 10, 3);
        for (name, mut state) in [("global", global), ("context", context)] {
            request(&mut state, &ClientEndpointId::Local);
            assert!(state.take_neru_hints_request(), "{name}");
            assert!(
                matches!(
                    state.overlay,
                    Some(ClientShellOverlay::ContextMenu(_) | ClientShellOverlay::GlobalMenu(_))
                ),
                "{name}"
            );
        }
    }

    #[test]
    fn a_request_ends_prefix_mode_and_leaves_resize_mode() {
        let mut state = shell();
        state.mode = ClientShellMode::Prefix;
        let outcome = request(&mut state, &ClientEndpointId::Local);
        assert!(outcome.repaint);
        assert_eq!(state.mode, ClientShellMode::Terminal);
        assert!(state.take_neru_hints_request());

        state.mode = ClientShellMode::Resize;
        request(&mut state, &ClientEndpointId::Local);
        assert_eq!(state.mode, ClientShellMode::Resize);
        assert!(state.take_neru_hints_request());
    }

    #[test]
    fn a_request_with_a_dialog_open_says_close_the_dialog_first() {
        let mut state = shell();
        state.overlay = Some(ClientShellOverlay::Rename(ClientRenameOverlay {
            title: "rename pane",
            input: TextEditor::new("draft", true),
            target: ClientRenameTarget::Pane {
                pane_id: "pane_1".into(),
            },
        }));
        let outcome = request(&mut state, &ClientEndpointId::Local);
        assert!(outcome.repaint);
        assert!(!state.take_neru_hints_request());
        assert_eq!(notice(&state), Some("Close the dialog first."));
        assert!(matches!(
            state.overlay,
            Some(ClientShellOverlay::Rename(ClientRenameOverlay { ref input, .. }))
                if input.as_str() == "draft"
        ));

        // Every request shows its reason (§V.16).
        state.visible_endpoint_notice = None;
        request(&mut state, &ClientEndpointId::Local);
        assert_eq!(notice(&state), Some("Close the dialog first."));
    }

    #[test]
    fn a_request_from_a_remote_endpoint_is_dropped() {
        let mut state = shell();
        state.mode = ClientShellMode::Prefix;
        let outcome = request(&mut state, &remote_id());
        assert!(!outcome.repaint);
        assert!(!state.take_neru_hints_request());
        assert_eq!(state.mode, ClientShellMode::Prefix);
        assert_eq!(notice(&state), None);
    }

    #[test]
    fn a_local_request_runs_while_a_remote_machine_is_shown() {
        let mut state = shell();
        state.active_endpoint_id = remote_id();
        request(&mut state, &ClientEndpointId::Local);
        assert!(state.take_neru_hints_request());
    }
}

#[test]
fn pane_cursors_keep_only_the_active_server_boot() {
    use crate::protocol::endpoint::{EndpointPaneCursor, EndpointPaneCursors};
    let cursor = EndpointPaneCursor {
        pane_id: "w1:p1".into(),
        x: 4,
        y: 2,
        visible: true,
    };
    let set = |boot_id: &str| {
        Some(EndpointPaneCursors {
            boot_id: boot_id.into(),
            cursors: vec![cursor.clone()],
        })
    };
    let mut state = shell();
    assert!(state.pane_cursor("w1:p1").is_none());

    state.set_pane_cursors(set("other-boot"));
    assert!(state.pane_cursor("w1:p1").is_none());

    state.set_pane_cursors(set("boot-1"));
    assert_eq!(state.pane_cursor("w1:p1"), Some(&cursor));
    assert!(state.pane_cursor("w1:p2").is_none());

    // An endpoint without `pane_cursors_v1` clears the set.
    state.set_pane_cursors(None);
    assert!(state.pane_cursor("w1:p1").is_none());
}
