use super::*;
use crate::api::schema::{Method, PaneRepositionSide};
use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

fn mouse(kind: MouseEventKind, column: u16, row: u16, modifiers: KeyModifiers) -> RawInputEvent {
    RawInputEvent::Mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers,
    })
}

fn drag_state() -> ClientShellState {
    let mut projected = snapshot();
    let mut second = projected.panes[0].clone();
    second.pane_id = "pane_2".into();
    second.focused = false;
    projected.panes.push(second);
    let mut pane_surface = surface();
    pane_surface.frame = FrameData::from_ratatui_buffer_with_hyperlinks(
        &Buffer::empty(Rect::new(0, 0, 40, 10)),
        None,
        &[],
    );
    pane_surface.panes[0].rect = SurfaceRect {
        x: 0,
        y: 0,
        width: 20,
        height: 10,
    };
    pane_surface.panes[0].inner_rect = SurfaceRect {
        x: 1,
        y: 1,
        width: 18,
        height: 8,
    };
    let mut second_surface = pane_surface.panes[0].clone();
    second_surface.pane_id = "pane_2".into();
    second_surface.rect.x = 20;
    second_surface.inner_rect.x = 21;
    second_surface.focused = false;
    pane_surface.panes.push(second_surface);
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(projected));
    state.set_pane_surface(pane_surface);
    state.set_endpoint_methods(Some(vec![
        "pane.swap".into(),
        "pane.reposition".into(),
        "pane.move".into(),
    ]));
    state.hits.panes = vec![
        PaneHit {
            rect: Rect::new(0, 0, 20, 10),
            inner_rect: Rect::new(1, 1, 18, 8),
            scrollbar_rect: None,
            scroll: None,
            pane_id: "pane_1".into(),
            popup: false,
            mouse_reporting: false,
            sgr_pixel_mouse: false,
            pixel_width: 0,
            pixel_height: 0,
        },
        PaneHit {
            rect: Rect::new(20, 0, 20, 10),
            inner_rect: Rect::new(21, 1, 18, 8),
            scrollbar_rect: None,
            scroll: None,
            pane_id: "pane_2".into(),
            popup: false,
            mouse_reporting: false,
            sgr_pixel_mouse: false,
            pixel_width: 0,
            pixel_height: 0,
        },
    ];
    state
}

#[test]
fn alt_chrome_drag_waits_for_threshold_and_dispatches_one_swap() {
    let mut state = drag_state();
    let down = state.handle_raw_events(vec![mouse(
        MouseEventKind::Down(MouseButton::Left),
        5,
        0,
        KeyModifiers::ALT,
    )]);
    assert!(down.actions.is_empty());
    assert!(state.pane_rearrangement_drag.is_some());
    let short = state.handle_raw_events(vec![mouse(
        MouseEventKind::Drag(MouseButton::Left),
        6,
        0,
        KeyModifiers::ALT,
    )]);
    assert!(short.actions.is_empty());
    let release = state.handle_raw_events(vec![mouse(
        MouseEventKind::Up(MouseButton::Left),
        30,
        5,
        KeyModifiers::empty(),
    )]);
    assert!(matches!(
        &release.actions[..],
        [ClientShellAction::Endpoint { request, .. }]
            if matches!(request.method, Method::PaneSwap(_))
    ));
    assert!(state.pane_rearrangement_drag.is_none());
}

#[test]
fn escape_cancels_drag_and_swallows_trailing_release() {
    let mut state = drag_state();
    state.handle_raw_events(vec![mouse(
        MouseEventKind::Down(MouseButton::Left),
        5,
        0,
        KeyModifiers::ALT,
    )]);
    state.handle_raw_events(vec![RawInputEvent::Key(crate::input::TerminalKey::new(
        KeyCode::Esc,
        KeyModifiers::empty(),
    ))]);
    let release = state.handle_raw_events(vec![mouse(
        MouseEventKind::Up(MouseButton::Left),
        30,
        5,
        KeyModifiers::empty(),
    )]);
    assert!(release.actions.is_empty());
    assert!(release.requests.is_empty());
    assert!(!state.swallow_pane_rearrangement_left_up);
}

#[test]
fn alt_terminal_content_does_not_arm_rearrangement() {
    let mut state = drag_state();
    state.handle_raw_events(vec![mouse(
        MouseEventKind::Down(MouseButton::Left),
        5,
        5,
        KeyModifiers::ALT,
    )]);
    assert!(state.pane_rearrangement_drag.is_none());
}

#[test]
fn alt_right_border_arms_when_the_cell_has_one_pane_owner() {
    let mut state = drag_state();
    state.handle_raw_events(vec![mouse(
        MouseEventKind::Down(MouseButton::Left),
        39,
        5,
        KeyModifiers::ALT,
    )]);
    assert_eq!(
        state
            .pane_rearrangement_drag
            .as_ref()
            .map(|drag| drag.source_pane_id.as_str()),
        Some("pane_2")
    );
}

#[test]
fn external_drag_invalidation_shows_notice() {
    let mut state = drag_state();
    state.handle_raw_events(vec![mouse(
        MouseEventKind::Down(MouseButton::Left),
        5,
        0,
        KeyModifiers::ALT,
    )]);
    let mut changed_surface = state.pane_surface.as_ref().unwrap().clone();
    changed_surface.surface_revision += 1;
    changed_surface.panes[0].rect.width -= 1;
    state.set_pane_surface(changed_surface);

    assert!(state.pane_rearrangement_drag.is_none());
    assert!(state
        .visible_endpoint_notice
        .as_ref()
        .is_some_and(|notice| notice.title == "Pane rearrangement canceled"));
}

#[test]
fn external_selector_invalidation_shows_notice() {
    let mut state = drag_state();
    state.open_pane_target_selector(
        "pane_1".into(),
        "ws_1".into(),
        "tab_1".into(),
        super::super::rearrangement::ClientPaneTargetOperation::Swap,
    );
    state.set_endpoint_methods(None);
    state.compose(40, 10).unwrap();

    assert!(!matches!(
        state.overlay,
        Some(ClientShellOverlay::PaneTargetSelector(_))
    ));
    assert!(state
        .visible_endpoint_notice
        .as_ref()
        .is_some_and(|notice| notice.title == "Pane rearrangement canceled"));
}

#[test]
fn drop_zones_use_quarters_tiny_swap_and_horizontal_corner_ties() {
    let rect = Rect::new(10, 10, 20, 20);
    assert_eq!(
        super::super::rearrangement::pane_drop_operation(rect, (20, 20)),
        super::super::rearrangement::ClientPaneTargetOperation::Swap
    );
    assert_eq!(
        super::super::rearrangement::pane_drop_operation(rect, (10, 10)),
        super::super::rearrangement::ClientPaneTargetOperation::Place(PaneRepositionSide::Left)
    );
    assert_eq!(
        super::super::rearrangement::pane_drop_operation(rect, (14, 20)),
        super::super::rearrangement::ClientPaneTargetOperation::Place(PaneRepositionSide::Left)
    );
    assert_eq!(
        super::super::rearrangement::pane_drop_operation(rect, (15, 20)),
        super::super::rearrangement::ClientPaneTargetOperation::Swap
    );
    assert_eq!(
        super::super::rearrangement::pane_drop_operation(Rect::new(0, 0, 4, 20), (0, 10)),
        super::super::rearrangement::ClientPaneTargetOperation::Swap
    );
}
