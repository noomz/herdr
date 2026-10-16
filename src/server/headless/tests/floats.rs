use super::*;

use crate::app::floats::{FloatEdit, FloatId, FloatSpec, FloatVisibility};
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
    FloatId,
    crate::terminal::TerminalId,
    tokio::sync::mpsc::Receiver<Bytes>,
) {
    let (runtime, input) = crate::terminal::TerminalRuntime::test_with_channel_and_scrollback_bytes(
        17, 6, 0, bytes, 4,
    );
    let (_, terminal_id, float_id) = server.app.install_test_float_runtime(
        runtime,
        FloatSpec {
            width: Some(PopupSize::Cells(20)),
            height: Some(PopupSize::Cells(8)),
            position: FloatPosition::At { x: 50, y: 2 },
            hide_while_focused,
        },
    );
    (float_id, terminal_id, input)
}

fn type_into_float(server: &mut HeadlessServer, client_id: u64, float_id: FloatId, text: &str) {
    server.handle_server_event(ServerEvent::ClientShellFloatInput {
        client_id,
        float_id,
        events: vec![protocol::ClientPaneInputEvent::TextCommit(text.into())],
    });
}

#[tokio::test]
async fn negotiated_client_receives_positioned_float_layer_beside_the_pane_surface() {
    let mut server = test_headless_server();
    let _pane_input = install_focused_test_runtime(&mut server, b"base-pane");
    let (float_id, terminal_id, _input) = install_float(&mut server, b"FLOAT_LIVE", None);
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
    assert_eq!(float.float_id, float_id);
    assert_eq!(float.terminal_id, terminal_id.as_str());
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
    let (_float_id, _terminal_id, _input) = install_float(&mut server, b"FLOAT_LIVE", None);
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
    let (float_id, terminal_id, mut float_input) = install_float(&mut server, b"PIP", None);
    let render_rx = connect_shell(&mut server, 7, true);
    server.render_and_stream();
    let _ = render_rx.recv().expect("first render");

    type_into_float(&mut server, 7, float_id, "shown");
    assert_eq!(
        float_input.try_recv().expect("shown input"),
        Bytes::from_static(b"shown")
    );

    server
        .app
        .edit_float(float_id, FloatEdit::HideWhileFocused(Some(source)));
    server.render_and_stream();
    let layers = float_layers(&read_server_messages(
        render_rx.recv().expect("rule render"),
    ));
    assert_eq!(layers, vec![protocol::floats::FloatsSurface::default()]);
    type_into_float(&mut server, 7, float_id, "hidden-by-rule");
    assert!(float_input.try_recv().is_err());

    server
        .app
        .edit_float(float_id, FloatEdit::HideWhileFocused(None));
    server.render_and_stream();
    let layers = float_layers(&read_server_messages(
        render_rx.recv().expect("rule cleared"),
    ));
    assert_eq!(layers[0].floats.len(), 1);

    server
        .app
        .edit_float(float_id, FloatEdit::Visibility(FloatVisibility::Hidden));
    server.render_and_stream();
    let layers = float_layers(&read_server_messages(
        render_rx.recv().expect("hide render"),
    ));
    assert_eq!(layers, vec![protocol::floats::FloatsSurface::default()]);
    type_into_float(&mut server, 7, float_id, "hidden");
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
    let (float_id, _terminal_id, _input) = install_float(&mut server, b"PIP", None);
    let _render_rx = connect_shell(&mut server, 7, true);
    let float_pane = server.app.state.floats[0].pane_id;
    let sources = std::collections::HashSet::from([float_pane]);

    assert!(server.pty_sources_visible_to_any_render_target(&sources));

    server
        .app
        .edit_float(float_id, FloatEdit::Visibility(FloatVisibility::Hidden));

    assert!(!server.pty_sources_visible_to_any_render_target(&sources));
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn modal_popup_blocks_float_input() {
    let mut server = test_headless_server();
    let _pane_input = install_focused_test_runtime(&mut server, b"base-pane");
    let (float_id, _terminal_id, mut float_input) = install_float(&mut server, b"PIP", None);
    let (popup_runtime, _popup_input) =
        crate::terminal::TerminalRuntime::test_with_channel_and_scrollback_bytes(
            40, 12, 0, b"POPUP", 4,
        );
    server.app.install_test_popup_runtime(popup_runtime);
    let render_rx = connect_shell(&mut server, 7, true);
    server.render_and_stream();
    let _ = render_rx.recv().expect("first render");

    type_into_float(&mut server, 7, float_id, "under-popup");

    assert!(float_input.try_recv().is_err());
    shutdown_test_runtimes(&mut server);
}

const FLOAT_IMAGE: &[u8] = b"PIP\x1b_Ga=T,f=32,t=d,i=9,p=4,s=1,v=1,c=1,r=1,q=2;/wAA/w==\x1b\\";

fn float_image_targets(surface: &protocol::PaneSurfaceFrame) -> Vec<String> {
    surface
        .graphics
        .placements
        .iter()
        .filter_map(|placement| match &placement.asset.source {
            protocol::SurfaceGraphicsSource::Terminal {
                target: protocol::SurfaceGraphicsTarget::Popup { terminal_id },
                image_id: 9,
            } => Some(terminal_id.clone()),
            _ => None,
        })
        .collect()
}

fn next_pane_surface(render_rx: &std::sync::mpsc::Receiver<Vec<u8>>) -> protocol::PaneSurfaceFrame {
    read_server_messages(render_rx.recv().expect("render"))
        .into_iter()
        .find_map(|message| match message {
            ServerMessage::PaneSurface(surface) => Some(surface),
            _ => None,
        })
        .expect("pane surface")
}

#[tokio::test]
async fn float_images_reach_only_clients_that_negotiated_floats() {
    let mut server = test_headless_server();
    let _pane_input = install_focused_test_runtime(&mut server, b"base-pane");
    let (_float_id, terminal_id, _input) = install_float(&mut server, FLOAT_IMAGE, None);
    let with_floats = connect_shell(&mut server, 7, true);
    let without_floats = connect_shell(&mut server, 8, false);

    server.render_and_stream();

    let surface = next_pane_surface(&with_floats);
    assert_eq!(float_image_targets(&surface), vec![terminal_id.to_string()]);
    assert_eq!(surface.graphics.assets.len(), 1);
    let surface = next_pane_surface(&without_floats);
    assert!(surface.graphics.placements.is_empty());
    assert!(surface.graphics.assets.is_empty());
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn hidden_or_closed_floats_deliver_no_images() {
    let mut server = test_headless_server();
    let _pane_input = install_focused_test_runtime(&mut server, b"base-pane");
    let source = server.app.state.workspaces[0].tabs[0].root_pane;
    let (float_id, _terminal_id, _input) = install_float(&mut server, FLOAT_IMAGE, None);
    let render_rx = connect_shell(&mut server, 7, true);
    server.render_and_stream();
    assert_eq!(float_image_targets(&next_pane_surface(&render_rx)).len(), 1);

    server
        .app
        .edit_float(float_id, FloatEdit::Visibility(FloatVisibility::Hidden));
    server.render_and_stream();
    assert!(float_image_targets(&next_pane_surface(&render_rx)).is_empty());

    server
        .app
        .edit_float(float_id, FloatEdit::Visibility(FloatVisibility::Shown));
    server.render_and_stream();
    let shown = next_pane_surface(&render_rx);
    assert_eq!(float_image_targets(&shown).len(), 1);
    assert_eq!(
        shown.graphics.assets.len(),
        1,
        "pixels are resent after show"
    );

    server
        .app
        .edit_float(float_id, FloatEdit::HideWhileFocused(Some(source)));
    server.render_and_stream();
    assert!(float_image_targets(&next_pane_surface(&render_rx)).is_empty());
    server
        .app
        .edit_float(float_id, FloatEdit::HideWhileFocused(None));
    server.render_and_stream();
    assert_eq!(float_image_targets(&next_pane_surface(&render_rx)).len(), 1);

    assert!(server.app.close_float(float_id));
    server.render_and_stream();
    assert!(float_image_targets(&next_pane_surface(&render_rx)).is_empty());
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn retained_tiled_patches_keep_float_images() {
    let (mut server, _control_rx, render_rx, pane_id) =
        retained_test_server_with_control(b"tiled text");
    let client = server.clients.get_mut(&1).unwrap();
    client.mode = ClientConnectionMode::ClientShell;
    client.shell_floats = Some(Default::default());
    client.cell_size = crate::kitty_graphics::HostCellSize {
        width_px: 10,
        height_px: 20,
    };
    let (_float_id, terminal_id, _input) = install_float(&mut server, FLOAT_IMAGE, None);
    server.render_and_stream();
    assert_eq!(
        float_image_targets(&next_pane_surface(&render_rx)),
        vec![terminal_id.to_string()]
    );

    write_shared_test_pane(&mut server, pane_id, b"\r\nchanged tiled text");
    assert!(server.render_retained_pane_surface_and_stream(&HashSet::from([pane_id])));
    let surface = next_pane_surface(&render_rx);
    assert!(frame_text(&surface.frame).contains("changed tiled text"));
    assert_eq!(float_image_targets(&surface), vec![terminal_id.to_string()]);
    assert!(surface.graphics.assets.is_empty(), "pixels are not resent");
    shutdown_test_runtimes(&mut server);
}
