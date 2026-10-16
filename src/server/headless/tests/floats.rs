use super::*;

use crate::app::floats::{FloatEdit, FloatSpec, FloatVisibility};
use crate::popup_size::{FloatPosition, PopupSize};

fn read_server_messages(bytes: Vec<u8>) -> Vec<ServerMessage> {
    let len = bytes.len() as u64;
    let mut cursor = std::io::Cursor::new(bytes);
    let mut messages = Vec::new();
    while cursor.position() < len {
        messages.push(protocol::read_message(&mut cursor, MAX_FRAME_SIZE).expect("decode message"));
    }
    messages
}

fn float_layers(messages: &[ServerMessage]) -> Vec<protocol::floats::FloatsSurface> {
    messages
        .iter()
        .filter_map(|message| match message {
            ServerMessage::EndpointControl { kind, data }
                if kind == protocol::floats::SURFACE_KIND =>
            {
                Some(protocol::floats::decode_surface(data).expect("float layer"))
            }
            _ => None,
        })
        .collect()
}

fn connect_shell(
    server: &mut HeadlessServer,
    client_id: u64,
    floats: bool,
) -> std::sync::mpsc::Receiver<Vec<u8>> {
    let (writer, control_rx, render_rx) = test_client_writer();
    assert!(
        server.handle_server_event(ServerEvent::ClientShellConnected {
            surface_reuse: false,
            surface_delta: false,
            surface_scroll: false,
            floats,
            client_id,
            surface_cols: 80,
            surface_rows: 24,
            cell_width_px: 10,
            cell_height_px: 20,
            pixel_mouse: false,
            direct_graphics: false,
            endpoint_keybindings: false,
            mouse_capture: false,
            surface_active: true,
            writer,
        })
    );
    let _snapshot = client_shell_snapshot(&control_rx);
    render_rx
}

fn install_float(
    server: &mut HeadlessServer,
    bytes: &[u8],
    hide_while_focused: Option<crate::layout::PaneId>,
) -> (
    crate::terminal::TerminalId,
    tokio::sync::mpsc::Receiver<Bytes>,
) {
    let (runtime, input) = crate::terminal::TerminalRuntime::test_with_channel_and_scrollback_bytes(
        17, 6, 0, bytes, 4,
    );
    let (_, terminal_id) = server.app.install_test_float_runtime(
        runtime,
        FloatSpec {
            width: Some(PopupSize::Cells(20)),
            height: Some(PopupSize::Cells(8)),
            position: FloatPosition::At { x: 50, y: 2 },
            hide_while_focused,
        },
    );
    (terminal_id, input)
}

fn type_into_float(server: &mut HeadlessServer, client_id: u64, terminal_id: &str, text: &str) {
    server.handle_server_event(ServerEvent::ClientShellFloatInput {
        client_id,
        terminal_id: terminal_id.into(),
        events: vec![protocol::ClientPaneInputEvent::TextCommit(text.into())],
    });
}

#[tokio::test]
async fn negotiated_client_receives_positioned_float_layer_beside_the_pane_surface() {
    let mut server = test_headless_server();
    let _pane_input = install_focused_test_runtime(&mut server, b"base-pane");
    let (terminal_id, _input) = install_float(&mut server, b"FLOAT_LIVE", None);
    let render_rx = connect_shell(&mut server, 7, true);

    server.render_and_stream();
    let messages = read_server_messages(render_rx.recv().expect("first render"));

    let ServerMessage::PaneSurface(surface) = &messages[0] else {
        panic!("pane surface comes first");
    };
    assert!(
        surface.popup.is_none(),
        "floats never ride the frozen popup field"
    );
    let layers = float_layers(&messages);
    assert_eq!(layers.len(), 1);
    let float = &layers[0].floats[0];
    assert_eq!(float.float_id, terminal_id.as_str());
    assert_eq!(
        float.outer,
        protocol::SurfaceRect {
            x: 50,
            y: 2,
            width: 20,
            height: 8
        }
    );
    assert_eq!((float.frame.width, float.frame.height), (17, 6));
    assert!(frame_text(&float.frame).contains("FLOAT_LIVE"));

    server.render_and_stream();
    assert!(
        render_rx.try_recv().is_err(),
        "an unchanged layer is not resent"
    );
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn clients_without_the_capability_never_receive_floats() {
    let mut server = test_headless_server();
    let _pane_input = install_focused_test_runtime(&mut server, b"base-pane");
    let (_terminal_id, _input) = install_float(&mut server, b"FLOAT_LIVE", None);
    let render_rx = connect_shell(&mut server, 7, false);

    server.render_and_stream();
    let messages = read_server_messages(render_rx.recv().expect("first render"));

    assert!(float_layers(&messages).is_empty());
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn hiding_or_focusing_the_rule_pane_clears_the_layer_and_blocks_input() {
    let mut server = test_headless_server();
    let _pane_input = install_focused_test_runtime(&mut server, b"base-pane");
    let source = server.app.state.workspaces[0].tabs[0].root_pane;
    let (terminal_id, mut float_input) = install_float(&mut server, b"PIP", None);
    let render_rx = connect_shell(&mut server, 7, true);
    server.render_and_stream();
    let _ = render_rx.recv().expect("first render");

    type_into_float(&mut server, 7, terminal_id.as_str(), "shown");
    assert_eq!(
        float_input.try_recv().expect("shown input"),
        Bytes::from_static(b"shown")
    );

    server.app.edit_float(
        terminal_id.as_str(),
        FloatEdit::HideWhileFocused(Some(source)),
    );
    server.render_and_stream();
    let layers = float_layers(&read_server_messages(
        render_rx.recv().expect("rule render"),
    ));
    assert_eq!(layers, vec![protocol::floats::FloatsSurface::default()]);
    type_into_float(&mut server, 7, terminal_id.as_str(), "hidden-by-rule");
    assert!(float_input.try_recv().is_err());

    server
        .app
        .edit_float(terminal_id.as_str(), FloatEdit::HideWhileFocused(None));
    server.render_and_stream();
    let layers = float_layers(&read_server_messages(
        render_rx.recv().expect("rule cleared"),
    ));
    assert_eq!(layers[0].floats.len(), 1);

    server.app.edit_float(
        terminal_id.as_str(),
        FloatEdit::Visibility(FloatVisibility::Hidden),
    );
    server.render_and_stream();
    let layers = float_layers(&read_server_messages(
        render_rx.recv().expect("hide render"),
    ));
    assert_eq!(layers, vec![protocol::floats::FloatsSurface::default()]);
    type_into_float(&mut server, 7, terminal_id.as_str(), "hidden");
    assert!(float_input.try_recv().is_err());
    assert!(
        server.app.terminal_runtimes.get(&terminal_id).is_some(),
        "hidden floats keep their terminal"
    );
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn hidden_float_output_does_not_count_as_visible_presentation_work() {
    let mut server = test_headless_server();
    let _pane_input = install_focused_test_runtime(&mut server, b"base-pane");
    let (terminal_id, _input) = install_float(&mut server, b"PIP", None);
    let _render_rx = connect_shell(&mut server, 7, true);
    let float_pane = server.app.state.floats[0].pane_id;
    let sources = std::collections::HashSet::from([float_pane]);

    assert!(server.pty_sources_visible_to_any_render_target(&sources));

    server.app.edit_float(
        terminal_id.as_str(),
        FloatEdit::Visibility(FloatVisibility::Hidden),
    );

    assert!(!server.pty_sources_visible_to_any_render_target(&sources));
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn modal_popup_blocks_float_input() {
    let mut server = test_headless_server();
    let _pane_input = install_focused_test_runtime(&mut server, b"base-pane");
    let (terminal_id, mut float_input) = install_float(&mut server, b"PIP", None);
    let (popup_runtime, _popup_input) =
        crate::terminal::TerminalRuntime::test_with_channel_and_scrollback_bytes(
            40, 12, 0, b"POPUP", 4,
        );
    server.app.install_test_popup_runtime(popup_runtime);
    let render_rx = connect_shell(&mut server, 7, true);
    server.render_and_stream();
    let _ = render_rx.recv().expect("first render");

    type_into_float(&mut server, 7, terminal_id.as_str(), "under-popup");

    assert!(float_input.try_recv().is_err());
    shutdown_test_runtimes(&mut server);
}
