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
