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
            "logs"
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
