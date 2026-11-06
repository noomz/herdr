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
    connect_shell_with_controls(server, client_id, floats).1
}

fn connect_shell_with_controls(
    server: &mut HeadlessServer,
    client_id: u64,
    floats: bool,
) -> (
    std::sync::mpsc::Receiver<Vec<u8>>,
    std::sync::mpsc::Receiver<Vec<u8>>,
) {
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
    (control_rx, render_rx)
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
            hide_on_blur: false,
            workspace: None,
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
async fn workspace_float_layers_graphics_and_input_follow_each_clients_workspace() {
    let mut server = test_headless_server();
    let _pane_input = install_focused_test_runtime(&mut server, b"first-pane");
    let mut second = crate::workspace::Workspace::test_new("second");
    let second_pane = second.tabs[0].root_pane;
    let runtime =
        crate::terminal::TerminalRuntime::test_with_scrollback_bytes(80, 24, 0, b"second-pane");
    second.insert_test_runtime(second_pane, runtime);
    server.app.state.workspaces.push(second);
    server.app.state.ensure_test_terminals();
    let workspace = server.app.state.workspaces[0].id.clone();
    let first_tab = server.app.public_tab_id(0, 0).unwrap();
    let second_tab = server.app.public_tab_id(1, 0).unwrap();
    let (scoped, scoped_terminal, mut scoped_input) = install_float(&mut server, FLOAT_IMAGE, None);
    server.app.state.floats[0].workspace = Some(workspace);
    let (global, _, mut global_input) = install_float(&mut server, b"GLOBAL", None);
    server
        .app
        .edit_float(global, FloatEdit::Move(FloatPosition::At { x: 2, y: 2 }));
    let first_client = connect_shell(&mut server, 7, true);
    let second_client = connect_shell(&mut server, 8, true);
    assert!(server.focus_shell_client_on_tab(7, &first_tab));
    assert!(server.focus_shell_client_on_tab(8, &second_tab));
    server.render_and_stream();
    let first = read_server_messages(first_client.recv().unwrap());
    let second = read_server_messages(second_client.recv().unwrap());
    let ids = |messages: &[ServerMessage]| {
        float_layers(messages)[0]
            .floats
            .iter()
            .map(|float| float.float_id)
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&first), vec![scoped, global]);
    assert_eq!(ids(&second), vec![global]);
    let images = |messages: &[ServerMessage]| {
        messages
            .iter()
            .find_map(|message| match message {
                ServerMessage::PaneSurface(surface) => Some(float_image_targets(surface)),
                _ => None,
            })
            .unwrap()
    };
    assert_eq!(images(&first), vec![scoped_terminal.to_string()]);
    assert!(images(&second).is_empty());
    type_into_float(&mut server, 8, scoped, "wrong-workspace");
    assert!(scoped_input.try_recv().is_err());
    type_into_float(&mut server, 7, scoped, "matching-workspace");
    assert_eq!(
        scoped_input.try_recv().unwrap(),
        Bytes::from_static(b"matching-workspace")
    );
    for client in [7, 8] {
        type_into_float(&mut server, client, global, "global");
        assert_eq!(
            global_input.try_recv().unwrap(),
            Bytes::from_static(b"global")
        );
    }
    assert!(server.focus_shell_client_on_tab(7, &second_tab));
    server.render_and_stream();
    let after = read_server_messages(first_client.recv().unwrap());
    assert_eq!(ids(&after), vec![global]);
    assert!(images(&after).is_empty());
    type_into_float(&mut server, 7, scoped, "left-workspace");
    assert!(scoped_input.try_recv().is_err());
    assert!(
        !server.pty_sources_visible_to_any_render_target(&HashSet::from([server.app.state.floats
            [0]
        .pane_id]))
    );
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn moving_the_last_pane_out_closes_its_workspaces_float() {
    let mut server = test_headless_server();
    let _pane_input = install_focused_test_runtime(&mut server, b"base-pane");
    let (float_id, terminal_id, _) = install_float(&mut server, b"FLOAT", None);
    server.app.state.floats[0].workspace = Some(server.app.state.workspaces[0].id.clone());
    let pane = server.app.state.workspaces[0].tabs[0].root_pane;
    let pane_id = server.app.public_pane_id(0, pane).unwrap();
    server
        .app
        .state
        .workspaces
        .push(crate::workspace::Workspace::test_new("other"));
    server.app.state.active = Some(1);
    server.app.state.selected = 1;
    server.app.state.ensure_test_terminals();
    let response = server.app.handle_api_request(
        serde_json::from_value(serde_json::json!({
            "id": "move", "method": "pane.move", "params": {
                "pane_id": pane_id, "destination": {"type": "new_workspace"}, "focus": false,
            }
        }))
        .unwrap(),
    );
    let response: api::schema::SuccessResponse =
        serde_json::from_str(&response).unwrap_or_else(|err| panic!("{err}: {response}"));
    assert!(matches!(
        response.result,
        api::schema::ResponseResult::PaneMove { .. }
    ));
    assert!(server.app.state.float(float_id).is_none());
    assert!(server.app.terminal_runtimes.get(&terminal_id).is_none());
    server.app.state.assert_invariants_for_test();
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

#[tokio::test]
async fn a_client_shell_float_resize_resizes_the_program_terminal() {
    let mut server = test_headless_server();
    let _pane_input = install_focused_test_runtime(&mut server, b"base-pane");
    let (float_id, _terminal_id, _input) = install_float(&mut server, b"FLOAT_LIVE", None);
    let render_rx = connect_shell(&mut server, 7, true);
    server.render_and_stream();
    let _ = render_rx.recv().expect("first render");

    let boot_id = server.client_shell_boot_id.clone();
    server.handle_server_event(ServerEvent::ClientShellEndpointRequest {
        client_id: 7,
        boot_id,
        request: Box::new(api::schema::Request {
            id: "resize".into(),
            method: api::schema::Method::FloatResize(api::schema::FloatResizeParams {
                float_id: float_id.to_string(),
                width: Some(PopupSize::Cells(30)),
                height: Some(PopupSize::Cells(10)),
            }),
        }),
    });
    server.render_and_stream();
    let messages = read_server_messages(render_rx.recv().expect("resized render"));

    let layers = float_layers(&messages);
    let float = &layers.last().expect("float layer").floats[0];
    assert_eq!((float.outer.width, float.outer.height), (30, 10));
    assert_eq!(
        (float.frame.width, float.frame.height),
        (27, 8),
        "the program's terminal follows the committed size"
    );
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn float_blur_hides_only_flagged_floats_visible_to_the_reporting_client() {
    for (flagged, negotiated, hidden_by_rule) in [
        (true, true, false),
        (false, true, false),
        (true, false, false),
        (true, true, true),
    ] {
        let mut server = test_headless_server();
        let _pane_input = install_focused_test_runtime(&mut server, b"base-pane");
        let source = server.app.state.workspaces[0].tabs[0].root_pane;
        let (float_id, terminal_id, _input) =
            install_float(&mut server, b"FLOAT_LIVE", hidden_by_rule.then_some(source));
        server.app.state.floats[0].hide_on_blur = flagged;
        let render_rx = connect_shell(&mut server, 7, negotiated);
        server.render_and_stream();
        let _ = render_rx.recv().expect("initial layer");
        let before = server.app.state.float(float_id).unwrap().clone();
        let should_hide = flagged && negotiated && !hidden_by_rule;

        assert_eq!(
            server.handle_server_event(ServerEvent::ClientShellFloatBlur {
                client_id: 7,
                float_id
            }),
            should_hide
        );
        let after = server.app.state.float(float_id).unwrap().clone();
        if should_hide {
            let hidden = server.app.handle_api_request(crate::api::schema::Request {
                id: "hide-again".into(),
                method: crate::api::schema::Method::FloatHide(crate::api::schema::FloatTarget {
                    float_id: float_id.to_string(),
                }),
            });
            assert!(serde_json::from_str::<serde_json::Value>(&hidden)
                .unwrap()
                .get("result")
                .is_some());
            assert_eq!(server.app.state.float(float_id).unwrap(), &after);
            assert_eq!(after.visibility, FloatVisibility::Hidden);
            assert!(server.app.terminal_runtimes.get(&terminal_id).is_some());
            server.render_and_stream();
            assert_eq!(
                float_layers(&read_server_messages(
                    render_rx.recv().expect("hidden layer")
                )),
                vec![protocol::floats::FloatsSurface::default()]
            );
            assert!(
                !server.handle_server_event(ServerEvent::ClientShellFloatBlur {
                    client_id: 7,
                    float_id
                })
            );
        } else {
            assert_eq!(after, before);
        }
        assert!(
            !server.handle_server_event(ServerEvent::ClientShellFloatBlur {
                client_id: 99,
                float_id
            })
        );
        assert!(
            !server.handle_server_event(ServerEvent::ClientShellFloatBlur {
                client_id: 7,
                float_id: float_id.next()
            })
        );
        shutdown_test_runtimes(&mut server);
    }
}

fn float_api(
    server: &mut HeadlessServer,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    let (respond_to, response_rx) = std::sync::mpsc::channel();
    server.handle_api_request_with_shutdown_check(api::ApiRequestMessage {
        request: serde_json::from_value(serde_json::json!({
            "id": "float-focus", "method": method, "params": params
        }))
        .unwrap(),
        respond_to,
        response_write_complete: None,
    });
    serde_json::from_str(&response_rx.recv().unwrap()).unwrap()
}

fn float_focus_controls(
    server: &mut HeadlessServer,
    client_id: u64,
    rx: &std::sync::mpsc::Receiver<Vec<u8>>,
) -> Vec<serde_json::Value> {
    const BARRIER: &str = "test.float-focus-barrier";
    assert!(server.send_to_client(
        client_id,
        ServerMessage::EndpointControl {
            kind: BARRIER.into(),
            data: String::new(),
        }
    ));
    let mut controls = Vec::new();
    loop {
        for message in read_server_messages(
            rx.recv_timeout(Duration::from_secs(2))
                .expect("float focus barrier"),
        ) {
            if let ServerMessage::EndpointControl { kind, data } = message {
                if kind == BARRIER {
                    return controls;
                }
                if kind == protocol::floats::FOCUS_KIND {
                    controls.push(serde_json::from_str(&data).unwrap());
                }
            }
        }
    }
}

#[tokio::test]
async fn float_show_and_raise_focus_only_clients_that_see_the_float() {
    for method in ["float.show", "float.raise"] {
        let mut server = test_headless_server();
        let _pane_input = install_focused_test_runtime(&mut server, b"base-pane");
        let source = server.app.state.workspaces[0].tabs[0].root_pane;
        let (float_id, _, _input) = install_float(&mut server, b"FLOAT", None);
        let (visible, _render) = connect_shell_with_controls(&mut server, 7, true);
        let (unsupported, _render) = connect_shell_with_controls(&mut server, 8, false);
        let (hidden, _render) = connect_shell_with_controls(&mut server, 9, true);
        server
            .app
            .state
            .workspaces
            .push(crate::workspace::Workspace::test_new("other"));
        server.app.state.ensure_test_terminals();
        let workspace_id = server.app.public_workspace_id(1);
        let tab_id = server.app.public_tab_id(1, 0).unwrap();
        server.clients.get_mut(&7).unwrap().shell_location =
            Some(crate::server::clients::ClientShellLocation {
                focused_workspace_id: Some(workspace_id.clone()),
                active_tab_ids: HashMap::from([(workspace_id, tab_id)]),
            });
        server
            .app
            .edit_float(float_id, FloatEdit::HideWhileFocused(Some(source)));
        assert!(
            server.shell_client_sees_float(7, server.app.state.float(float_id).unwrap().pane_id)
        );
        assert!(
            !server.shell_client_sees_float(9, server.app.state.float(float_id).unwrap().pane_id)
        );
        server
            .app
            .edit_float(float_id, FloatEdit::Visibility(FloatVisibility::Hidden));
        for focus in [None, Some(false), Some(true)] {
            let mut params = serde_json::json!({"float_id": float_id.to_string()});
            if let Some(focus) = focus {
                params["focus"] = focus.into();
            }
            let response = float_api(&mut server, method, params);
            assert!(response.get("result").is_some(), "{response}");
            let controls = float_focus_controls(&mut server, 7, &visible);
            // Raising does not show a hidden float.
            let expected = focus == Some(true) && method == "float.show";
            assert_eq!(
                controls,
                if expected {
                    vec![serde_json::json!({"float_id": float_id.to_string()})]
                } else {
                    vec![]
                }
            );
            assert!(float_focus_controls(&mut server, 8, &unsupported).is_empty());
            assert!(float_focus_controls(&mut server, 9, &hidden).is_empty());
        }
        server
            .app
            .edit_float(float_id, FloatEdit::Visibility(FloatVisibility::Shown));
        let response = float_api(
            &mut server,
            method,
            serde_json::json!({"float_id": float_id.to_string(), "focus": true}),
        );
        assert!(response.get("result").is_some(), "{response}");
        assert_eq!(
            float_focus_controls(&mut server, 7, &visible),
            vec![serde_json::json!({"float_id": float_id.to_string()})]
        );
        let response = float_api(
            &mut server,
            method,
            serde_json::json!({"float_id": "f99999", "focus": true}),
        );
        assert_eq!(response["error"]["code"], "float_not_found");
        assert!(float_focus_controls(&mut server, 7, &visible).is_empty());
        shutdown_test_runtimes(&mut server);
    }
}

#[cfg(unix)]
#[tokio::test]
async fn plugin_float_open_with_focus_delivers_the_new_float_id() {
    let mut server = test_headless_server();
    let _pane_input = install_focused_test_runtime(&mut server, b"base-pane");
    let (controls, _render) = connect_shell_with_controls(&mut server, 7, true);
    let root = server
        .client_socket_path
        .parent()
        .unwrap()
        .join("focus-plugin");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("herdr-plugin.toml"),
        r#"
id = "example.float-focus"
name = "Float focus"
version = "0.1.0"
min_herdr_version = "0.6.10"
platforms = ["linux", "macos"]
[[panes]]
id = "peek"
title = "Peek"
placement = "float"
command = ["sh", "-c", "sleep 5"]
"#,
    )
    .unwrap();
    let linked = float_api(
        &mut server,
        "plugin.link",
        serde_json::json!({"path": root.display().to_string()}),
    );
    assert!(linked.get("result").is_some(), "{linked}");
    for focus in [None, Some(false), Some(true)] {
        let mut params = serde_json::json!({
            "plugin_id": "example.float-focus", "entrypoint": "peek", "placement": "float"
        });
        if let Some(focus) = focus {
            params["focus"] = focus.into();
        }
        let opened = float_api(&mut server, "plugin.pane.open", params);
        assert_eq!(opened["result"]["type"], "float_opened", "{opened}");
        let focus_controls = float_focus_controls(&mut server, 7, &controls);
        assert_eq!(
            focus_controls,
            if focus == Some(true) {
                vec![serde_json::json!({"float_id": opened["result"]["float"]["float_id"]})]
            } else {
                vec![]
            }
        );
    }
    shutdown_test_runtimes(&mut server);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "manual workspace float render scaling profile"]
async fn workspace_float_render_scale_profile() {
    use crate::server::client_shell_graphics::FloatGraphics;
    use std::hint::black_box;
    use std::time::Instant;

    for background in [true, false] {
        for count in [1, 15] {
            let mut server = test_headless_server();
            let history = "populated pane\r\n".repeat(100);
            let _input = install_focused_test_runtime(&mut server, history.as_bytes());
            let mut pane_ids = vec![server.app.state.workspaces[0].tabs[0].root_pane];
            for index in 1..count {
                let (ws_idx, pane_id) = if background {
                    let ws = crate::workspace::Workspace::test_new("background");
                    let pane = ws.tabs[0].root_pane;
                    server.app.state.workspaces.push(ws);
                    (server.app.state.workspaces.len() - 1, pane)
                } else {
                    let workspace = &mut server.app.state.workspaces[0];
                    workspace.tabs[0]
                        .layout
                        .focus_pane(pane_ids[(index - 1) / 2]);
                    let direction = if index % 2 == 0 {
                        ratatui::layout::Direction::Vertical
                    } else {
                        ratatui::layout::Direction::Horizontal
                    };
                    let pane = workspace.test_split(direction);
                    pane_ids.push(pane);
                    (0, pane)
                };
                server.app.state.workspaces[ws_idx].insert_test_runtime(
                    pane_id,
                    crate::terminal::TerminalRuntime::test_with_scrollback_bytes(
                        80,
                        24,
                        0,
                        history.as_bytes(),
                    ),
                );
            }
            server.app.state.ensure_test_terminals();
            let (_, _, _input) = install_float(&mut server, b"FLOAT", None);
            let workspace = server.app.state.workspaces[0].id.clone();
            let focused = server.app.state.workspaces[0].tabs[0].layout.focused();
            let area = ratatui::layout::Rect::new(0, 0, 80, 24);
            let cell = crate::kitty_graphics::HostCellSize {
                width_px: 10,
                height_px: 20,
            };
            let delivery = crate::kitty_graphics::surface::DeliveryCache::default();
            let mut samples = [Vec::new(), Vec::new()];
            for round in 0..6 {
                for scoped in [false, true] {
                    server.app.state.floats[0].workspace = scoped.then(|| workspace.clone());
                    let started = Instant::now();
                    for _ in 0..40 {
                        let surface = crate::server::client_shell::render_pane_surface(
                            &mut server.app,
                            Some(crate::ui::TabSurfaceTarget {
                                workspace_index: 0,
                                tab_index: 0,
                            }),
                            area,
                            true,
                            true,
                            FloatGraphics::VisibleTo(Some(focused), Some(0)),
                            cell,
                            &delivery,
                            1,
                        )
                        .expect("populated pane surface");
                        black_box(surface);
                        let layer = crate::server::client_shell::render_float_layer(
                            &server.app,
                            Some(focused),
                            Some(&workspace),
                            area,
                            cell,
                        )
                        .expect("float layer");
                        assert_eq!(layer.floats.len(), 1);
                        black_box(layer);
                    }
                    if round > 0 {
                        samples[usize::from(scoped)].push(started.elapsed().as_nanos() / 40);
                    }
                }
            }
            for sample in &mut samples {
                sample.sort_unstable();
            }
            println!(
                "workspace float {} count={count} fixed=80x24 ns/frame global={} [{}..{}] scoped={} [{}..{}] runs=5 frames/run=40",
                if background { "background" } else { "active" },
                samples[0][2], samples[0][0], samples[0][4],
                samples[1][2], samples[1][0], samples[1][4],
            );
            shutdown_test_runtimes(&mut server);
        }
    }
}
