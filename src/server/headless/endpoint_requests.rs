use super::*;

impl HeadlessServer {
    pub(super) fn finish_float_button_response(
        &mut self,
        client_id: u64,
        pending: crate::server::clients::PendingFloatButton,
        response: &[u8],
    ) {
        let Ok(response) = serde_json::from_slice::<api::schema::SuccessResponse>(response) else {
            return;
        };
        let api::schema::ResponseResult::FloatButtonInvoked { log } = response.result else {
            return;
        };
        let current = self
            .app
            .state
            .plugin_command_logs
            .iter()
            .find(|entry| entry.log_id == log.log_id)
            .cloned()
            .unwrap_or_else(|| api::schema::PluginCommandLogInfo {
                status: api::schema::PluginCommandStatus::Failed,
                error: Some("command result no longer available".into()),
                ..log
            });
        if current.status == api::schema::PluginCommandStatus::Running {
            if let Some(client) = self.clients.get_mut(&client_id) {
                client
                    .shell_pending_float_buttons
                    .insert(current.log_id, pending);
            }
        } else {
            self.send_float_button_result(client_id, pending, &current);
        }
    }

    pub(super) fn finish_float_button_command(
        &mut self,
        log_id: &str,
        exit_code: Option<i32>,
        error: Option<String>,
        stderr: String,
    ) {
        let log = self
            .app
            .state
            .plugin_command_logs
            .iter()
            .find(|entry| entry.log_id == log_id)
            .cloned()
            .unwrap_or_else(|| api::schema::PluginCommandLogInfo {
                log_id: log_id.into(),
                plugin_id: String::new(),
                action_id: None,
                event: None,
                command: vec![],
                status: if error.is_none() && exit_code == Some(0) {
                    api::schema::PluginCommandStatus::Succeeded
                } else {
                    api::schema::PluginCommandStatus::Failed
                },
                started_unix_ms: 0,
                finished_unix_ms: None,
                exit_code,
                stdout: None,
                stderr: Some(stderr),
                error,
            });
        let targets: Vec<_> = self
            .clients
            .iter_mut()
            .filter_map(|(&client_id, client)| {
                client
                    .shell_pending_float_buttons
                    .remove(log_id)
                    .map(|pending| (client_id, pending))
            })
            .collect();
        for (client_id, pending) in targets {
            self.send_float_button_result(client_id, pending, &log);
        }
    }

    fn send_float_button_result(
        &mut self,
        client_id: u64,
        pending: crate::server::clients::PendingFloatButton,
        log: &api::schema::PluginCommandLogInfo,
    ) {
        let Some(float_id) = crate::protocol::floats::FloatId::parse(&pending.target.float_id)
        else {
            return;
        };
        let error = log
            .error
            .as_deref()
            .or(log.stderr.as_deref())
            .filter(|text| !text.is_empty())
            .map(|text| {
                text.chars()
                    .filter(|ch| !ch.is_control())
                    .take(512)
                    .collect::<String>()
            });
        let result = crate::protocol::endpoint::EndpointFloatButtonResult {
            boot_id: self.client_shell_boot_id.clone(),
            request_id: pending.request_id,
            log_id: log.log_id.clone(),
            float_id,
            button_id: pending.target.button_id,
            status: log.status,
            exit_code: log.exit_code,
            error,
        };
        if let Ok(data) = serde_json::to_string(&result) {
            self.send_to_client(
                client_id,
                ServerMessage::EndpointControl {
                    kind: crate::protocol::endpoint::FLOAT_BUTTON_RESULT_KIND.into(),
                    data,
                },
            );
        }
    }

    pub(super) fn handle_client_shell_endpoint_request(
        &mut self,
        client_id: u64,
        boot_id: String,
        mut request: Box<api::schema::Request>,
    ) -> bool {
        let Some(client) = self.clients.get(&client_id) else {
            return false;
        };
        if !matches!(client.mode, ClientConnectionMode::ClientShell) {
            self.remove_client_and_resize_if_needed(client_id);
            return true;
        }
        let request_id = request.id.clone();
        if !crate::server::client_commands::supports_client_shell_method(&request.method) {
            let message = crate::server::client_commands::error_message(
                boot_id,
                request_id,
                "unsupported_endpoint_command",
                "this method is not available through the client shell command lane",
            );
            self.send_to_client(client_id, message);
            return false;
        }
        if boot_id != self.client_shell_boot_id {
            let message = crate::server::client_commands::error_message(
                boot_id,
                request_id,
                "stale_boot",
                "endpoint command targeted an earlier server boot",
            );
            self.send_to_client(client_id, message);
            return false;
        }
        if matches!(&request.method, api::schema::Method::FloatButtonInvoke(_))
            && !client.shell_float_buttons_enabled
        {
            let message = crate::server::client_commands::error_message(
                boot_id,
                request_id,
                "unsupported_endpoint_command",
                "float buttons were not negotiated",
            );
            self.send_to_client(client_id, message);
            return false;
        }
        let surface_active = client.shell_surface_active;
        if let api::schema::Method::ClientShellSurfaceSet(params) = &request.method {
            let Some((changed, projection_revision)) =
                self.set_client_shell_surface_active(client_id, params.active)
            else {
                return false;
            };
            self.send_to_client(
                client_id,
                crate::server::client_commands::success_message_with_result(
                    boot_id,
                    request_id,
                    api::schema::ResponseResult::ClientShellSurfaceSet {
                        active: params.active,
                        projection_revision,
                    },
                ),
            );
            return changed;
        }
        if client.shell_endpoint_command_in_flight {
            let message = crate::server::client_commands::error_message(
                boot_id,
                request_id,
                "endpoint_busy",
                "this endpoint is still processing another command",
            );
            self.send_to_client(client_id, message);
            return false;
        }
        if !surface_active {
            let message = crate::server::client_commands::error_message(
                boot_id,
                request_id,
                "surface_inactive",
                "this method requires an active client shell surface",
            );
            self.send_to_client(client_id, message);
            return false;
        }

        let api_request_id = format!(
            "endpoint:{}:{client_id}:{request_id}",
            self.client_shell_boot_id
        );
        request.id = api_request_id.clone();
        let (respond_to, response_rx) = std::sync::mpsc::channel();
        if let Err(err) = crate::server::client_commands::spawn_response_waiter(
            client_id,
            boot_id.clone(),
            request_id.clone(),
            response_rx,
            self.server_event_tx.clone(),
        ) {
            let message = crate::server::client_commands::error_message(
                boot_id,
                request_id,
                "server_unavailable",
                format!("failed to start endpoint response bridge: {err}"),
            );
            self.send_to_client(client_id, message);
            return false;
        }
        if let Some(client) = self.clients.get_mut(&client_id) {
            if let api::schema::Method::FloatButtonInvoke(target) = &request.method {
                client.shell_float_button_response = Some((
                    crate::server::clients::PendingFloatButton {
                        request_id: request_id.clone(),
                        target: target.clone(),
                    },
                    Vec::new(),
                ));
            }
            client.shell_endpoint_command_in_flight = true;
            // A later source restore has a new projection revision. Keep this request's lease
            // so a delayed worktree response cannot focus a pane after endpoint switching.
            client.shell_endpoint_command_surface_revision = Some(client.shell_projection_revision);
            let deferred_worktree = matches!(
                &request.method,
                api::schema::Method::WorktreeCreate(_)
                    | api::schema::Method::WorktreeRemove(_)
                    | api::schema::Method::WorktreeList(_)
                    | api::schema::Method::WorktreeOpen(_)
            );
            let deferred_navigation = match &request.method {
                api::schema::Method::WorktreeCreate(params) => params.focus,
                api::schema::Method::WorktreeOpen(params) => params.focus,
                _ => false,
            };
            client.shell_deferred_navigation_request_id =
                deferred_worktree.then(|| api_request_id.clone());
            client.shell_deferred_navigation_response = deferred_navigation.then(Vec::new);
        }
        let foreground_changed = self.promote_client_to_foreground(client_id);
        foreground_changed
            | self.handle_client_shell_api_request(
                client_id,
                api::ApiRequestMessage {
                    request: *request,
                    respond_to,
                    response_write_complete: None,
                },
            )
    }
}
