use super::*;

fn shell_client(
    server: &mut HeadlessServer,
    show_machine: bool,
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
    client.shell_show_machine = show_machine;
    server.clients.insert(1, client);
    server.foreground_client_id = Some(1);
    control
}

fn machine_show(server: &mut HeadlessServer, params: serde_json::Value) -> serde_json::Value {
    let (respond_to, response_rx) = std::sync::mpsc::channel();
    server.handle_api_request_with_shutdown_check(api::ApiRequestMessage {
        request: api::schema::Request {
            id: "show".into(),
            method: api::schema::Method::MachineShow(serde_json::from_value(params).unwrap()),
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

#[test]
fn machine_show_forwards_the_request_to_the_capable_foreground_client() {
    let mut server = test_headless_server();
    let control = shell_client(&mut server, true);

    let response = machine_show(
        &mut server,
        serde_json::json!({"machine": "drm", "pane_id": "w1:p2"}),
    );

    assert_eq!(response["result"]["type"], "ok");
    let ServerMessage::EndpointControl { kind, data } = read_server_message(
        control
            .recv_timeout(Duration::from_millis(100))
            .expect("show-machine control"),
    ) else {
        panic!("expected endpoint control");
    };
    assert_eq!(kind, crate::protocol::endpoint::SHOW_MACHINE_KIND);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&data).unwrap(),
        serde_json::json!({"machine": "drm", "pane_id": "w1:p2"})
    );
}

#[test]
fn machine_show_rejects_two_targets_without_sending() {
    let mut server = test_headless_server();
    let control = shell_client(&mut server, true);

    let response = machine_show(
        &mut server,
        serde_json::json!({"machine": "drm", "workspace_id": "w1", "tab_id": "w1:t1"}),
    );

    assert_eq!(response["error"]["code"], "invalid_params");
    assert!(control.recv_timeout(Duration::from_millis(50)).is_err());
}

#[test]
fn machine_show_fails_without_a_capable_foreground_client() {
    let mut server = test_headless_server();
    let response = machine_show(&mut server, serde_json::json!({"machine": "local"}));
    assert_eq!(response["error"]["code"], "no_capable_client");

    let control = shell_client(&mut server, false);
    let response = machine_show(&mut server, serde_json::json!({"machine": "local"}));
    assert_eq!(response["error"]["code"], "no_capable_client");
    assert!(control.recv_timeout(Duration::from_millis(50)).is_err());
}
