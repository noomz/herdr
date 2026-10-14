use super::*;
use crate::api::schema::{Method, PaneTarget};
use crate::config::{FocusFollowsMouseArea, FocusFollowsMouseConfig};
use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

fn pane(id: &str, x: u16, width: u16) -> PaneHit {
    PaneHit {
        rect: Rect::new(x, 1, width, 8),
        inner_rect: Rect::new(x + 1, 2, width.saturating_sub(2), 6),
        scrollbar_rect: None,
        scroll: None,
        pane_id: id.to_owned(),
        popup: false,
        mouse_reporting: false,
        sgr_pixel_mouse: false,
        pixel_width: 0,
        pixel_height: 0,
    }
}

fn two_pane_state() -> ClientShellState {
    let mut projected = snapshot();
    let mut second = projected.panes[0].clone();
    second.pane_id = "pane_2".into();
    second.focused = false;
    projected.panes.push(second);
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(projected));
    state.hits.panes = vec![pane("pane_1", 0, 20), pane("pane_2", 21, 20)];
    state
}

fn three_pane_state() -> ClientShellState {
    let mut state = two_pane_state();
    let mut projected = state.snapshot.as_deref().expect("snapshot").clone();
    let mut third = projected.panes[0].clone();
    third.pane_id = "pane_3".into();
    third.focused = false;
    projected.panes.push(third);
    state.set_snapshot(Box::new(projected));
    state.hits.panes = vec![
        pane("pane_1", 0, 20),
        pane("pane_2", 21, 20),
        pane("pane_3", 42, 20),
    ];
    state
}

fn enable(state: &mut ClientShellState, delay_ms: u64) {
    state.config.focus_follows_mouse = FocusFollowsMouseConfig {
        enabled: true,
        delay_ms,
        ..FocusFollowsMouseConfig::default()
    };
}

fn moved(column: u16, row: u16) -> RawInputEvent {
    RawInputEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Moved,
        column,
        row,
        modifiers: KeyModifiers::empty(),
    })
}

fn pane_focus_ids(outcome: &ClientShellInput) -> Vec<String> {
    outcome
        .actions
        .iter()
        .filter_map(|action| match action {
            ClientShellAction::Endpoint { request, .. } => match &request.method {
                Method::PaneFocus(PaneTarget { pane_id }) => Some(pane_id.clone()),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

fn body_point(hit: &PaneHit, inset: u16) -> (u16, u16) {
    (hit.inner_rect.x + inset, hit.inner_rect.y + inset)
}

#[test]
fn hover_into_unfocused_pane_sends_one_focus_when_delay_is_zero() {
    let mut state = two_pane_state();
    enable(&mut state, 0);
    let hit = state.hits.panes[1].clone();
    let (column, row) = body_point(&hit, 1);
    let outcome = state.handle_raw_events(vec![moved(column, row)]);
    assert_eq!(pane_focus_ids(&outcome), vec!["pane_2".to_owned()]);
    assert!(state.hover_focus_dwell.is_none());
    assert_eq!(state.hover_focus_last_origin, Some((column, row)));
}

#[test]
fn dwell_waits_for_deadline_and_rearms_after_leave() {
    let mut state = two_pane_state();
    enable(&mut state, 200);
    let hit = state.hits.panes[1].clone();
    let (column, row) = body_point(&hit, 1);
    let armed = state.handle_raw_events(vec![moved(column, row)]);
    assert!(pane_focus_ids(&armed).is_empty());
    let deadline = state.hover_focus_dwell.as_ref().expect("dwell").deadline;
    assert!(
        state.timer_delay(deadline - std::time::Duration::from_millis(1))
            <= std::time::Duration::from_millis(100)
    );
    assert!(pane_focus_ids(
        &state.tick_hover_focus(deadline - std::time::Duration::from_millis(1))
    )
    .is_empty());
    assert!(state.hover_focus_dwell.is_some());

    let left = state.handle_raw_events(vec![moved(20, row)]);
    assert!(pane_focus_ids(&left).is_empty());
    assert!(state.hover_focus_dwell.is_none());
    assert!(pane_focus_ids(&state.tick_hover_focus(deadline)).is_empty());

    let rearmed = state.handle_raw_events(vec![moved(column, row)]);
    assert!(pane_focus_ids(&rearmed).is_empty());
    let next_deadline = state.hover_focus_dwell.as_ref().expect("rearmed").deadline;
    assert_eq!(
        pane_focus_ids(&state.tick_hover_focus(next_deadline)),
        vec!["pane_2".to_owned()]
    );
    assert!(state.hover_focus_dwell.is_none());
    assert!(pane_focus_ids(&state.tick_hover_focus(next_deadline)).is_empty());
}

#[test]
fn sweep_before_deadline_focuses_only_the_dwell_winner() {
    let mut state = three_pane_state();
    enable(&mut state, 200);
    let second = state.hits.panes[1].clone();
    let third = state.hits.panes[2].clone();
    let (column, row) = body_point(&second, 1);
    state.handle_raw_events(vec![moved(column, row)]);
    let (column, row) = body_point(&third, 1);
    let swept = state.handle_raw_events(vec![moved(column, row)]);
    assert!(pane_focus_ids(&swept).is_empty());
    assert_eq!(
        state
            .hover_focus_dwell
            .as_ref()
            .map(|dwell| dwell.pane_id.as_str()),
        Some("pane_3")
    );
    let deadline = state.hover_focus_dwell.as_ref().expect("dwell").deadline;
    assert_eq!(
        pane_focus_ids(&state.tick_hover_focus(deadline)),
        vec!["pane_3".to_owned()]
    );
}

#[test]
fn already_focused_pane_does_not_hover_focus() {
    let mut state = two_pane_state();
    enable(&mut state, 0);
    let hit = state.hits.panes[0].clone();
    let (column, row) = body_point(&hit, 1);
    let outcome = state.handle_raw_events(vec![moved(column, row)]);
    assert!(pane_focus_ids(&outcome).is_empty());
    assert!(state.hover_focus_dwell.is_none());
}

#[test]
fn disabled_hover_does_nothing_and_click_still_focuses() {
    let mut state = two_pane_state();
    let hit = state.hits.panes[1].clone();
    let (column, row) = body_point(&hit, 1);
    let hover = state.handle_raw_events(vec![moved(column, row)]);
    assert!(pane_focus_ids(&hover).is_empty());
    let click = state.handle_raw_events(vec![RawInputEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::empty(),
    })]);
    assert_eq!(pane_focus_ids(&click), vec!["pane_2".to_owned()]);
}

#[test]
fn hover_focus_gates_block_selection_modes_gestures_and_overlays() {
    let mut state = two_pane_state();
    enable(&mut state, 0);
    let hit = state.hits.panes[1].clone();
    let (column, row) = body_point(&hit, 1);

    state.selection = Some(crate::selection::Selection::anchor(
        "pane_1".into(),
        0,
        0,
        None,
    ));
    assert!(pane_focus_ids(&state.handle_raw_events(vec![moved(column, row)])).is_empty());
    state.selection = None;

    state.mode = ClientShellMode::Copy;
    assert!(pane_focus_ids(&state.handle_raw_events(vec![moved(column, row)])).is_empty());
    state.mode = ClientShellMode::Navigate;
    assert!(pane_focus_ids(&state.handle_raw_events(vec![moved(column, row)])).is_empty());
    state.mode = ClientShellMode::Terminal;

    state.overlay = Some(ClientShellOverlay::Onboarding);
    assert!(pane_focus_ids(&state.handle_raw_events(vec![moved(column, row)])).is_empty());
    state.overlay = None;

    let mut outcome = ClientShellInput::default();
    state.request_word_selection(&state.hits.panes[0].clone(), 0, 0, &mut outcome);
    assert!(state.word_selection_gesture.is_some());
    assert!(pane_focus_ids(&state.handle_raw_events(vec![moved(column, row)])).is_empty());
    state.word_selection_gesture = None;

    state.pane_mouse_gesture = Some(ClientPaneMouseGesture {
        hit: state.hits.panes[0].clone(),
        button: MouseButton::Left,
        stripped_modifiers: KeyModifiers::empty(),
        last_event: MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 2,
            row: 3,
            modifiers: KeyModifiers::empty(),
        },
        last_position: ClientMousePosition::Cell { column: 0, row: 0 },
    });
    assert!(pane_focus_ids(&state.handle_raw_events(vec![moved(column, row)])).is_empty());
}

#[test]
fn body_mode_ignores_frame_and_borders_mode_attributes_nearest_without_ties() {
    let mut state = two_pane_state();
    enable(&mut state, 0);
    let frame = state.handle_raw_events(vec![moved(0, 2)]);
    assert!(
        pane_focus_ids(&frame).is_empty(),
        "body mode ignores the frame"
    );

    state.config.focus_follows_mouse.area = FocusFollowsMouseArea::Borders;
    let frame = state.handle_raw_events(vec![moved(21, 3)]);
    assert_eq!(
        pane_focus_ids(&frame),
        vec!["pane_2".to_owned()],
        "borders mode attributes the frame to the nearest pane"
    );

    state.hover_focus_last_origin = None;
    state.hits.panes[1].inner_rect.x = 21;
    state.hits.panes[1].inner_rect.width = 19;
    let gap = state.handle_raw_events(vec![moved(20, 3)]);
    assert_eq!(pane_focus_ids(&gap), vec!["pane_2".to_owned()]);

    state.hover_focus_last_origin = None;
    state.hits.panes = vec![
        PaneHit {
            rect: Rect::new(0, 0, 10, 4),
            inner_rect: Rect::new(0, 0, 10, 4),
            ..pane("pane_1", 0, 10)
        },
        PaneHit {
            rect: Rect::new(11, 0, 10, 4),
            inner_rect: Rect::new(11, 0, 10, 4),
            ..pane("pane_2", 11, 10)
        },
    ];
    let tie = state.handle_raw_events(vec![moved(10, 1)]);
    assert!(
        pane_focus_ids(&tie).is_empty(),
        "exact distance tie does nothing"
    );
}

#[test]
fn enter_cells_requires_inset_and_rejects_empty_inset() {
    let mut state = two_pane_state();
    enable(&mut state, 0);
    state.config.focus_follows_mouse.enter_cells = 2;
    let hit = state.hits.panes[1].clone();
    let shallow = state.handle_raw_events(vec![moved(hit.inner_rect.x + 1, hit.inner_rect.y + 1)]);
    assert!(pane_focus_ids(&shallow).is_empty());
    let deep = state.handle_raw_events(vec![moved(hit.inner_rect.x + 2, hit.inner_rect.y + 2)]);
    assert_eq!(pane_focus_ids(&deep), vec!["pane_2".to_owned()]);

    state.hover_focus_last_origin = None;
    state.hits.panes[1].inner_rect = Rect::new(22, 3, 3, 3);
    let empty = state.handle_raw_events(vec![moved(23, 4)]);
    assert!(pane_focus_ids(&empty).is_empty());
}

#[test]
fn travel_cells_blocks_the_next_hover_until_the_pointer_moves_far_enough() {
    let mut state = three_pane_state();
    enable(&mut state, 0);
    state.config.focus_follows_mouse.travel_cells = 3;
    state.hits.panes[0].rect = Rect::new(0, 0, 6, 4);
    state.hits.panes[0].inner_rect = Rect::new(0, 0, 6, 4);
    state.hits.panes[1].rect = Rect::new(10, 0, 3, 3);
    state.hits.panes[1].inner_rect = Rect::new(10, 1, 3, 2);
    state.hits.panes[2].rect = Rect::new(13, 0, 6, 4);
    state.hits.panes[2].inner_rect = Rect::new(13, 1, 5, 2);
    let first = state.handle_raw_events(vec![moved(11, 1)]);
    assert_eq!(pane_focus_ids(&first), vec!["pane_2".to_owned()]);

    let blocked = state.handle_raw_events(vec![moved(13, 1)]);
    assert!(pane_focus_ids(&blocked).is_empty());

    let allowed = state.handle_raw_events(vec![moved(14, 1)]);
    assert_eq!(pane_focus_ids(&allowed), vec!["pane_3".to_owned()]);
}

#[test]
fn mouse_reporting_motion_still_forwards_when_hover_focus_fires() {
    let mut state = two_pane_state();
    enable(&mut state, 0);
    state.hits.panes[1].mouse_reporting = true;
    let hit = state.hits.panes[1].clone();
    let (column, row) = body_point(&hit, 1);
    let outcome = state.handle_raw_events(vec![moved(column, row)]);
    assert_eq!(pane_focus_ids(&outcome), vec!["pane_2".to_owned()]);
    assert!(matches!(
        &outcome.requests[..],
        [ClientMessage::ClientShellPaneInput { pane_id, events }]
            if pane_id == "pane_2"
                && matches!(
                    &events[..],
                    [ClientPaneInputEvent::Mouse {
                        kind: crate::protocol::ClientMouseKind::Moved,
                        position: ClientMousePosition::Cell { column: col, row: event_row },
                        ..
                    }] if *col == column - hit.inner_rect.x && *event_row == row - hit.inner_rect.y
                )
    ));
}

#[test]
fn double_click_word_selection_still_starts_and_blocks_hover_focus() {
    let mut state = two_pane_state();
    enable(&mut state, 0);
    let focused = state.hits.panes[0].clone();
    let (column, row) = body_point(&focused, 1);
    let down = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::empty(),
    };
    state.handle_raw_events(vec![RawInputEvent::Mouse(down)]);
    state.handle_raw_events(vec![RawInputEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Up(MouseButton::Left),
        ..down
    })]);
    state.handle_raw_events(vec![RawInputEvent::Mouse(down)]);
    assert!(state.word_selection_gesture.is_some());
    let other = state.hits.panes[1].clone();
    let (column, row) = body_point(&other, 1);
    assert!(pane_focus_ids(&state.handle_raw_events(vec![moved(column, row)])).is_empty());
}

#[test]
fn wheel_scroll_still_focuses_without_waiting_for_dwell() {
    let mut state = two_pane_state();
    enable(&mut state, 200);
    let hit = state.hits.panes[1].clone();
    let (column, row) = body_point(&hit, 1);
    let outcome = state.handle_raw_events(vec![RawInputEvent::Mouse(MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column,
        row,
        modifiers: KeyModifiers::empty(),
    })]);
    assert_eq!(pane_focus_ids(&outcome), vec!["pane_2".to_owned()]);
    assert!(state.hover_focus_dwell.is_none());
}

#[test]
fn live_config_reloads_focus_follows_mouse() {
    let mut shell = ClientShellConfig::from_config(&Config::default());
    assert!(!shell.focus_follows_mouse.enabled);
    let loaded: Config = toml::from_str(
        r#"
[ui.focus_follows_mouse]
enabled = true
area = "borders"
delay_ms = 0
enter_cells = 2
travel_cells = 4
"#,
    )
    .unwrap();
    shell.apply_live_config(&loaded, &[], &[]);
    assert!(shell.focus_follows_mouse.enabled);
    assert_eq!(
        shell.focus_follows_mouse.area,
        FocusFollowsMouseArea::Borders
    );
    assert_eq!(shell.focus_follows_mouse.delay_ms, 0);
    assert_eq!(shell.focus_follows_mouse.enter_cells, 2);
    assert_eq!(shell.focus_follows_mouse.travel_cells, 4);
}
