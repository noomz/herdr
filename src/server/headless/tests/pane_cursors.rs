use super::*;
use crate::protocol::endpoint::{EndpointPaneCursor, EndpointPaneCursors, PANE_CURSORS_KIND};

const CLIENT: u64 = 7;

/// Every message in one render write, in order.
fn render_write(render: &std::sync::mpsc::Receiver<Vec<u8>>) -> Vec<ServerMessage> {
    let bytes = render
        .recv_timeout(Duration::from_secs(1))
        .expect("render write");
    let len = bytes.len() as u64;
    let mut cursor = std::io::Cursor::new(bytes);
    let mut messages = Vec::new();
    while cursor.position() < len {
        messages.push(protocol::read_message(&mut cursor, MAX_FRAME_SIZE).expect("decode"));
    }
    messages
}

fn pane_cursors(messages: &[ServerMessage]) -> Option<EndpointPaneCursors> {
    let mut sets = messages.iter().filter_map(|message| match message {
        ServerMessage::EndpointControl { kind, data } if kind == PANE_CURSORS_KIND => {
            Some(serde_json::from_str(data).expect("pane cursor json"))
        }
        _ => None,
    });
    let set = sets.next();
    assert!(sets.next().is_none(), "one cursor set per write");
    set
}

/// Two panes, `left` unfocused and `right` focused, each with a prompt.
fn split_server(
    negotiated: bool,
) -> (
    HeadlessServer,
    std::sync::mpsc::Receiver<Vec<u8>>,
    crate::layout::PaneId,
    crate::layout::PaneId,
) {
    let mut server = test_headless_server();
    let mut workspace = crate::workspace::Workspace::test_new("cursors");
    let left = workspace.tabs[0].root_pane;
    let right = workspace.test_split(ratatui::layout::Direction::Horizontal);
    workspace.insert_test_runtime(
        left,
        crate::terminal::TerminalRuntime::test_with_screen_bytes(40, 23, b"$ "),
    );
    workspace.insert_test_runtime(
        right,
        crate::terminal::TerminalRuntime::test_with_screen_bytes(40, 23, b"> abc"),
    );
    server.app.state.workspaces = vec![workspace];
    server.app.state.active = Some(0);
    server.app.state.selected = 0;
    server.app.state.mode = crate::app::Mode::Terminal;
    let (_control, render) = connect_matching_test_shell(&mut server, CLIENT);
    server
        .clients
        .get_mut(&CLIENT)
        .unwrap()
        .shell_pane_cursors_enabled = negotiated;
    (server, render, left, right)
}

fn cursor_of(
    set: &EndpointPaneCursors,
    server: &HeadlessServer,
    pane: crate::layout::PaneId,
) -> EndpointPaneCursor {
    let pane_id = server.app.public_pane_id(0, pane).expect("public pane id");
    set.cursors
        .iter()
        .find(|cursor| cursor.pane_id == pane_id)
        .unwrap_or_else(|| panic!("no cursor for {pane_id} in {set:?}"))
        .clone()
}

#[tokio::test]
async fn negotiated_client_gets_every_pane_cursor_after_its_surface_in_one_write() {
    let (mut server, render, left, right) = split_server(true);
    server.render_and_stream();
    let messages = render_write(&render);
    assert!(
        matches!(messages.first(), Some(ServerMessage::PaneSurface(_))),
        "surface leads the write"
    );
    let set = pane_cursors(&messages).expect("pane cursors in the surface write");
    assert_eq!(set.boot_id, server.app.boot_id);
    assert_eq!(set.cursors.len(), 2);
    let left_cursor = cursor_of(&set, &server, left);
    assert_eq!(
        (left_cursor.x, left_cursor.y, left_cursor.visible),
        (2, 0, true)
    );
    let right_cursor = cursor_of(&set, &server, right);
    assert_eq!(
        (right_cursor.x, right_cursor.y, right_cursor.visible),
        (5, 0, true)
    );

    // Hidden cursors are sent as not visible (§V.18 gives them no target).
    write_shared_test_pane(&mut server, left, b"\x1b[?25l");
    server.render_and_stream();
    let set = pane_cursors(&render_write(&render)).expect("hidden cursor update");
    assert!(!cursor_of(&set, &server, left).visible);
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn unchanged_cursors_are_not_resent_and_moved_ones_ride_the_retained_patch() {
    let (mut server, render, left, right) = split_server(true);
    server.render_and_stream();
    assert!(pane_cursors(&render_write(&render)).is_some());

    // Output elsewhere in the pane, cursor saved and restored: cells change, cursors do not.
    write_shared_test_pane(&mut server, right, b"\x1b7\x1b[5;1Hlog line\x1b8");
    assert!(server.render_retained_pane_surface_and_stream(&HashSet::from([right])));
    let messages = render_write(&render);
    assert!(matches!(
        messages.first(),
        Some(ServerMessage::PaneSurfacePatch(_))
    ));
    assert!(pane_cursors(&messages).is_none(), "unchanged set resent");

    write_shared_test_pane(&mut server, right, b"de");
    assert!(server.render_retained_pane_surface_and_stream(&HashSet::from([right])));
    let messages = render_write(&render);
    assert!(matches!(
        messages.first(),
        Some(ServerMessage::PaneSurfacePatch(_))
    ));
    let set = pane_cursors(&messages).expect("moved cursor rides the patch");
    assert_eq!(cursor_of(&set, &server, right).x, 7);
    assert_eq!(cursor_of(&set, &server, left).x, 2);
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn an_unfocused_cursor_move_without_cell_changes_is_sent() {
    let (mut server, render, left, _right) = split_server(true);
    server.render_and_stream();
    assert!(pane_cursors(&render_write(&render)).is_some());

    // Retained path: no row, metadata or focused-cursor change.
    write_shared_test_pane(&mut server, left, b"\x1b[H");
    assert!(server.render_retained_pane_surface_and_stream(&HashSet::from([left])));
    let set = pane_cursors(&render_write(&render)).expect("retained cursor-only update");
    assert_eq!(cursor_of(&set, &server, left).x, 0);

    // Full path: the control is the last message of the write.
    write_shared_test_pane(&mut server, left, b"\x1b[1;3H");
    server.render_and_stream();
    let messages = render_write(&render);
    assert!(matches!(
        messages.last(),
        Some(ServerMessage::EndpointControl { kind, .. }) if kind == PANE_CURSORS_KIND
    ));
    let set = pane_cursors(&messages).expect("identical-frame cursor update");
    assert_eq!(cursor_of(&set, &server, left).x, 2);
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn a_full_render_slot_does_not_commit_the_cursor_set() {
    let (mut server, render, _left, _right) = split_server(true);
    let writer = server.clients[&CLIENT].writer.clone().expect("writer");
    writer
        .render
        .try_send(Vec::new())
        .expect("fill the render slot");
    server.render_and_stream();
    assert!(server.clients[&CLIENT].shell_pane_cursors.is_none());

    assert!(render
        .recv_timeout(Duration::from_secs(1))
        .unwrap()
        .is_empty());
    server.render_and_stream();
    assert!(
        pane_cursors(&render_write(&render)).is_some(),
        "resent after Full"
    );
    assert!(server.clients[&CLIENT].shell_pane_cursors.is_some());
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn a_client_without_pane_cursors_never_gets_the_control() {
    let (mut server, render, left, right) = split_server(false);
    server.render_and_stream();
    assert!(pane_cursors(&render_write(&render)).is_none());

    write_shared_test_pane(&mut server, right, b"de");
    assert!(server.render_retained_pane_surface_and_stream(&HashSet::from([right])));
    assert!(pane_cursors(&render_write(&render)).is_none());

    write_shared_test_pane(&mut server, left, b"\x1b[H");
    server.render_and_stream();
    assert!(pane_cursors(&render_write(&render)).is_none());
    shutdown_test_runtimes(&mut server);
}
