use super::*;

/// A shell client as `herdr neru-hints` sees it: the hello flag and the last host focus report.
fn shell_client(
    server: &mut HeadlessServer,
    client_id: u64,
    neru_hints: bool,
    host_window_focused: Option<bool>,
) -> std::sync::mpsc::Receiver<Vec<u8>> {
    let (writer, control, _frames) = test_client_writer();
    let mut client = ClientConnection::new_with_mode(
        ClientConnectionMode::ClientShell,
        (80, 24),
        crate::kitty_graphics::HostCellSize::default(),
        1,
        RenderEncoding::SemanticFrame,
        Some(writer),
    );
    client.shell_neru_hints = neru_hints;
    client.host_window_focused = host_window_focused;
    server.clients.insert(client_id, client);
    control
}

fn run(server: &mut HeadlessServer) -> serde_json::Value {
    let (respond_to, response_rx) = std::sync::mpsc::channel();
    server.handle_api_request_with_shutdown_check(api::ApiRequestMessage {
        request: api::schema::Request {
            id: "neru".into(),
            method: api::schema::Method::ClientNeruHintsRun(Default::default()),
        },
        respond_to,
        response_write_complete: None,
    });
    serde_json::from_str(
        &response_rx
            .recv_timeout(Duration::from_millis(100))
            .unwrap(),
    )
    .unwrap()
}

/// Whether a neru-hints request reaches this client. The test writer delivers on its own
/// thread, so wait briefly for each control instead of reading only what has arrived.
fn got_request(control: &std::sync::mpsc::Receiver<Vec<u8>>) -> bool {
    std::iter::from_fn(|| control.recv_timeout(Duration::from_millis(100)).ok()).any(|bytes| {
        matches!(
            read_server_message(bytes),
            ServerMessage::EndpointControl { kind, data }
                if kind == crate::protocol::endpoint::NERU_HINTS_KIND && data == "{}"
        )
    })
}

fn error_code(response: &serde_json::Value) -> &str {
    response["error"]["code"].as_str().unwrap_or_default()
}

#[test]
fn neru_hints_without_clients_reports_no_focused_window() {
    let mut server = test_headless_server();
    let response = run(&mut server);
    assert_eq!(error_code(&response), "no_focused_client");
    assert_eq!(response["error"]["message"], "no focused herdr window.");
}

#[test]
fn neru_hints_goes_to_the_one_focused_client() {
    let mut server = test_headless_server();
    let focused = shell_client(&mut server, 1, true, Some(true));
    let unfocused = shell_client(&mut server, 2, true, Some(false));
    let unreported = shell_client(&mut server, 3, true, None);

    let response = run(&mut server);

    assert_eq!(response["result"]["type"], "ok");
    assert!(got_request(&focused));
    assert!(!got_request(&unfocused));
    assert!(!got_request(&unreported));
}

#[test]
fn neru_hints_refuses_two_focused_clients_and_sends_nothing() {
    let mut server = test_headless_server();
    let first = shell_client(&mut server, 1, true, Some(true));
    let second = shell_client(&mut server, 2, true, Some(true));

    let response = run(&mut server);

    assert_eq!(error_code(&response), "ambiguous_focused_client");
    assert_eq!(
        response["error"]["message"],
        "2 herdr windows report focus. Click one and try again."
    );
    assert!(!got_request(&first));
    assert!(!got_request(&second));
}

#[test]
fn unfocused_or_unreported_clients_do_not_qualify() {
    let mut server = test_headless_server();
    let unfocused = shell_client(&mut server, 1, true, Some(false));
    let unreported = shell_client(&mut server, 2, true, None);

    assert_eq!(error_code(&run(&mut server)), "no_focused_client");
    assert!(!got_request(&unfocused));
    assert!(!got_request(&unreported));
}

#[test]
fn a_client_without_the_hello_flag_never_receives_the_request() {
    let mut server = test_headless_server();
    let older = shell_client(&mut server, 1, false, Some(true));

    assert_eq!(error_code(&run(&mut server)), "no_focused_client");
    assert!(!got_request(&older));

    // It does not count as focused either, so it cannot make a new client ambiguous.
    let newer = shell_client(&mut server, 2, true, Some(true));
    assert_eq!(run(&mut server)["result"]["type"], "ok");
    assert!(got_request(&newer));
    assert!(!got_request(&older));
}

#[test]
fn a_focused_client_showing_a_remote_machine_still_qualifies() {
    let mut server = test_headless_server();
    let control = shell_client(&mut server, 1, true, Some(true));
    let client = server.clients.get_mut(&1).unwrap();
    // Showing a saved machine turns the local surface off; the client is not foreground here.
    client.shell_surface_active = false;
    server.foreground_client_id = None;

    assert_eq!(run(&mut server)["result"]["type"], "ok");
    assert!(got_request(&control));
}

#[test]
fn a_direct_terminal_client_never_qualifies() {
    let mut server = test_headless_server();
    let control = shell_client(&mut server, 1, true, Some(true));
    server.clients.get_mut(&1).unwrap().mode = ClientConnectionMode::TerminalAttach {
        terminal_id: "t1".into(),
    };

    assert_eq!(error_code(&run(&mut server)), "no_focused_client");
    assert!(!got_request(&control));
}

/// After a server restart or live handoff the client reconnects: the new server builds the
/// fields from the new hello and waits for the host focus the client sends after the welcome.
#[tokio::test]
async fn a_reconnected_client_qualifies_from_its_hello_and_post_welcome_focus() {
    let mut server = test_headless_server();
    let (writer, control, _render) = test_client_writer();
    assert!(
        server.handle_server_event(ServerEvent::ClientShellConnected {
            surface_reuse: false,
            surface_delta: false,
            surface_scroll: false,
            floats: false,
            show_machine: false,
            pane_cursors: false,
            neru_hints: true,
            client_id: 9,
            surface_cols: 80,
            surface_rows: 23,
            cell_width_px: 0,
            cell_height_px: 0,
            pixel_mouse: false,
            direct_graphics: false,
            endpoint_keybindings: false,
            mouse_capture: false,
            surface_active: true,
            writer,
        })
    );
    assert!(server.clients[&9].shell_neru_hints);
    assert_eq!(server.clients[&9].host_window_focused, None);
    assert_eq!(error_code(&run(&mut server)), "no_focused_client");

    assert!(!server.handle_server_event(ServerEvent::ClientHostFocus {
        client_id: 9,
        focused: true,
    }));
    assert_eq!(run(&mut server)["result"]["type"], "ok");
    assert!(got_request(&control));

    server.handle_server_event(ServerEvent::ClientHostFocus {
        client_id: 9,
        focused: false,
    });
    assert_eq!(error_code(&run(&mut server)), "no_focused_client");
}

#[test]
fn host_focus_changes_no_pty_focus_or_foreground_client() {
    let mut server = test_headless_server();
    let _control = shell_client(&mut server, 1, true, None);
    server.foreground_client_id = None;
    let pty_focus = server.app.state.outer_terminal_focus;

    server.handle_server_event(ServerEvent::ClientHostFocus {
        client_id: 1,
        focused: true,
    });

    assert_eq!(server.clients[&1].host_window_focused, Some(true));
    assert_eq!(server.clients[&1].outer_terminal_focus, None);
    assert_eq!(server.app.state.outer_terminal_focus, pty_focus);
    assert_eq!(server.foreground_client_id, None);
}
