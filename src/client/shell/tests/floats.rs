use super::*;

use crate::protocol::floats::{FloatSurface, FloatsSurface, INPUT_KIND};

fn float(id: &str, x: u16, y: u16, text: &str) -> FloatSurface {
    let first = format!("{text:<9}");
    let buffer = Buffer::with_lines([first.as_str(), "", ""]);
    FloatSurface {
        float_id: id.into(),
        title: format!("{id} title"),
        outer: SurfaceRect {
            x,
            y,
            width: 12,
            height: 5,
        },
        frame: FrameData::from_ratatui_buffer_with_hyperlinks(
            &buffer,
            Some(crate::protocol::CursorState {
                x: 1,
                y: 0,
                visible: true,
                shape: 1,
            }),
            &[],
        ),
        mouse_reporting: false,
        sgr_pixel_mouse: false,
        pixel_width: 0,
        pixel_height: 0,
    }
}

fn shell_with_floats(floats: Vec<FloatSurface>) -> ClientShellState {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    state.set_float_layer(FloatsSurface { floats });
    state.compose(106, 20).expect("composed frame");
    state
}

fn click(
    state: &mut ClientShellState,
    kind: MouseEventKind,
    column: u16,
    row: u16,
) -> ClientShellInput {
    state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::empty(),
    })])
}

fn float_inputs(input: &ClientShellInput) -> Vec<crate::protocol::floats::FloatInput> {
    input
        .requests
        .iter()
        .filter_map(|message| match message {
            ClientMessage::EndpointControl { kind, data } if kind == INPUT_KIND => {
                Some(crate::protocol::floats::decode_input(data).expect("float input"))
            }
            _ => None,
        })
        .collect()
}

fn endpoint_methods(input: &ClientShellInput) -> Vec<crate::api::schema::Method> {
    input
        .actions
        .iter()
        .filter_map(|action| match action {
            ClientShellAction::Endpoint { request, .. } => Some(request.method.clone()),
            _ => None,
        })
        .collect()
}

fn hit(state: &ClientShellState, id: &str) -> super::super::floats::FloatHit {
    state
        .hits
        .floats
        .iter()
        .find(|hit| hit.pane.pane_id == id)
        .cloned()
        .expect("float hit")
}

#[test]
fn floats_draw_over_the_pane_surface_at_their_server_position() {
    let state = shell_with_floats(vec![float("term_a", 10, 3, "pip-live")]);
    let surface = state.layout(106, 20).pane_surface;
    let a = hit(&state, "term_a");

    assert_eq!(a.outer, Rect::new(surface.x + 10, surface.y + 3, 12, 5));
    assert_eq!(
        a.pane.inner_rect,
        Rect::new(a.outer.x + 1, a.outer.y + 1, 9, 3)
    );
    let mut state = state;
    let frame = state.compose(106, 20).expect("frame");
    assert_eq!(
        cell_symbol_position(&frame, a.outer, "pip-l"),
        (a.outer.x + 1, a.outer.y + 1)
    );
    assert_eq!(
        cell_symbol_position(&frame, a.outer, "term_a"),
        (a.outer.x + 1, a.outer.y)
    );
    assert_eq!(
        frame.cursor.as_ref().map(|cursor| (cursor.x, cursor.y)),
        Some((surface.x + 1, surface.y + 1)),
        "an unfocused float leaves the cursor with the focused pane"
    );
}

#[test]
fn keys_reach_a_float_only_after_it_is_clicked() {
    let mut state = shell_with_floats(vec![float("term_a", 10, 3, "pip")]);

    let before = state.handle_input_bytes(b"x");
    assert!(matches!(
        &before.requests[..],
        [ClientMessage::ClientShellPaneInput { pane_id, .. }] if pane_id == "pane_1"
    ));

    let a = hit(&state, "term_a");
    let down = click(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        a.pane.inner_rect.x + 2,
        a.pane.inner_rect.y,
    );
    assert_eq!(
        float_inputs(&down).len(),
        1,
        "the click itself reaches the float"
    );
    assert!(
        endpoint_methods(&down).is_empty(),
        "the top float needs no raise"
    );

    let typed = state.handle_input_bytes(b"y");
    let inputs = float_inputs(&typed);
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0].float_id, "term_a");

    let frame = state.compose(106, 20).expect("focused frame");
    let a = hit(&state, "term_a");
    assert_eq!(
        frame.cursor.as_ref().map(|cursor| (cursor.x, cursor.y)),
        Some((a.pane.inner_rect.x + 1, a.pane.inner_rect.y)),
        "the focused float owns the cursor"
    );

    let prefix = state.handle_input_bytes(b"\x02");
    assert!(
        prefix.requests.is_empty(),
        "the prefix key stays a shell binding"
    );
    assert_eq!(state.mode, ClientShellMode::Prefix);
}

#[test]
fn clicking_the_tiled_layout_returns_keys_to_the_focused_pane() {
    let mut state = shell_with_floats(vec![float("term_a", 10, 3, "pip")]);
    let a = hit(&state, "term_a");
    click(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        a.pane.inner_rect.x,
        a.pane.inner_rect.y,
    );
    click(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        a.pane.inner_rect.x,
        a.pane.inner_rect.y,
    );
    let pane = state.hits.panes[0].clone();

    click(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        pane.inner_rect.x,
        pane.inner_rect.y,
    );
    click(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        pane.inner_rect.x,
        pane.inner_rect.y,
    );

    let typed = state.handle_input_bytes(b"z");
    assert!(float_inputs(&typed).is_empty());
    assert!(matches!(
        &typed.requests[..],
        [ClientMessage::ClientShellPaneInput { pane_id, .. }] if pane_id == "pane_1"
    ));
}

#[test]
fn clicking_a_lower_float_raises_it() {
    let mut state = shell_with_floats(vec![
        float("term_a", 10, 3, "lower"),
        float("term_b", 40, 3, "upper"),
    ]);
    let a = hit(&state, "term_a");

    let down = click(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        a.pane.inner_rect.x,
        a.pane.inner_rect.y,
    );

    assert!(matches!(
        &endpoint_methods(&down)[..],
        [crate::api::schema::Method::FloatRaise(target)] if target.float_id == "term_a"
    ));
    state.compose(106, 20).expect("raised frame");
    let order: Vec<_> = state
        .hits
        .floats
        .iter()
        .map(|hit| hit.pane.pane_id.clone())
        .collect();
    assert_eq!(order, vec!["term_b".to_owned(), "term_a".to_owned()]);
}

#[test]
fn dragging_the_border_previews_then_commits_a_clamped_move() {
    let mut state = shell_with_floats(vec![float("term_a", 10, 3, "pip")]);
    let surface = state.layout(106, 20).pane_surface;
    let a = hit(&state, "term_a");
    let grab = (a.outer.x + 4, a.outer.y);

    let down = click(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        grab.0,
        grab.1,
    );
    assert!(
        float_inputs(&down).is_empty(),
        "border clicks never reach the program"
    );
    click(
        &mut state,
        MouseEventKind::Drag(MouseButton::Left),
        grab.0 + 5,
        grab.1 + 2,
    );
    state.compose(106, 20).expect("drag preview");
    assert_eq!(
        hit(&state, "term_a").outer,
        Rect::new(a.outer.x + 5, a.outer.y + 2, 12, 5)
    );

    let up = click(&mut state, MouseEventKind::Up(MouseButton::Left), 500, 500);
    let max = (surface.width - 12, surface.height - 5);
    assert!(matches!(
        &endpoint_methods(&up)[..],
        [crate::api::schema::Method::FloatMove(params)]
            if params.float_id == "term_a" && (params.x, params.y) == max
    ));
    state.compose(106, 20).expect("held preview");
    assert_eq!(
        hit(&state, "term_a").outer,
        Rect::new(surface.x + max.0, surface.y + max.1, 12, 5),
        "the preview holds until the server confirms"
    );

    state.set_float_layer(FloatsSurface {
        floats: vec![float("term_a", max.0, max.1, "pip")],
    });
    assert!(state.floats.drag.is_none());
}

#[test]
fn modal_popup_takes_input_from_a_focused_float() {
    let mut state = shell_with_floats(vec![float("term_a", 10, 3, "pip")]);
    let a = hit(&state, "term_a");
    click(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        a.pane.inner_rect.x,
        a.pane.inner_rect.y,
    );

    state.set_pane_surface(surface_with_popup());
    state.compose(106, 20).expect("popup frame");
    let typed = state.handle_input_bytes(b"x");

    assert!(float_inputs(&typed).is_empty());
    assert!(matches!(
        &typed.requests[..],
        [ClientMessage::ClientShellPopupInput { terminal_id, .. }] if terminal_id == "terminal-popup"
    ));
}

#[test]
fn a_float_that_leaves_the_layer_gives_focus_back_to_the_layout() {
    let mut state = shell_with_floats(vec![float("term_a", 10, 3, "pip")]);
    let a = hit(&state, "term_a");
    click(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        a.pane.inner_rect.x,
        a.pane.inner_rect.y,
    );

    state.set_float_layer(FloatsSurface::default());
    state.compose(106, 20).expect("empty layer");

    assert!(state.hits.floats.is_empty());
    let typed = state.handle_input_bytes(b"x");
    assert!(matches!(
        &typed.requests[..],
        [ClientMessage::ClientShellPaneInput { pane_id, .. }] if pane_id == "pane_1"
    ));
}

#[test]
fn pane_patches_under_a_float_recompose_instead_of_overwriting_it() {
    let mut state = shell_with_floats(vec![float("term_a", 0, 0, "pip")]);
    let mut updated_pane = surface().panes[0].clone();
    updated_pane.content_revision = 2;
    let patch = crate::protocol::PaneSurfacePatch {
        boot_id: "boot-1".into(),
        projection_revision: 1,
        base_surface_revision: 1,
        surface_revision: 2,
        rows: vec![crate::protocol::PaneSurfacePatchRow {
            x: 0,
            y: 0,
            cells: vec![
                crate::protocol::CellData {
                    symbol: "N".into(),
                    fg: 0,
                    bg: 0,
                    modifier: 0,
                    skip: false,
                    hyperlink: None,
                };
                4
            ],
        }],
        panes: vec![updated_pane],
        cursor: None,
    };

    assert!(matches!(
        state.apply_pane_surface_patch(patch),
        ClientPaneSurfacePatchOutcome::Applied(None)
    ));
}

#[test]
fn clipboard_images_are_not_misrouted_to_the_pane_under_a_focused_float() {
    let mut state = shell_with_floats(vec![float("term_a", 10, 3, "pip")]);
    assert!(state.clipboard_image_target().is_some());
    let a = hit(&state, "term_a");

    click(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        a.pane.inner_rect.x,
        a.pane.inner_rect.y,
    );

    assert_eq!(state.clipboard_image_target(), None);
}

#[test]
fn a_selection_dragged_across_a_float_still_finishes() {
    let mut state = shell_with_floats(vec![float("term_a", 10, 3, "pip")]);
    let pane = state.hits.panes[0].clone();
    let a = hit(&state, "term_a");

    click(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        pane.inner_rect.x,
        pane.inner_rect.y,
    );
    click(
        &mut state,
        MouseEventKind::Drag(MouseButton::Left),
        a.pane.inner_rect.x + 1,
        a.pane.inner_rect.y,
    );
    let up = click(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        a.pane.inner_rect.x + 1,
        a.pane.inner_rect.y,
    );

    assert!(float_inputs(&up).is_empty());
    assert!(matches!(
        &endpoint_methods(&up)[..],
        [crate::api::schema::Method::PaneSelectionRead(params)] if params.pane_id == "pane_1"
    ));
}

#[test]
fn a_sidebar_drag_released_over_a_float_ends_the_chrome_drag() {
    let mut state = shell_with_floats(vec![float("term_a", 10, 3, "pip")]);
    let a = hit(&state, "term_a");
    state.chrome_drag = Some(ClientChromeDrag::SidebarWidth);

    click(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        a.pane.inner_rect.x + 1,
        a.pane.inner_rect.y,
    );

    assert!(state.chrome_drag.is_none());
}
