use super::*;

const NO_CAPABLE_CLIENT: &str = "no attached client supports machine.show";

impl HeadlessServer {
    pub(super) fn handle_machine_show_api(
        &mut self,
        id: String,
        params: api::schema::MachineShowParams,
    ) -> String {
        let error = |id, code: &str, message: &str| {
            serde_json::to_string(&api::schema::ErrorResponse {
                id,
                error: api::schema::ErrorBody {
                    code: code.into(),
                    message: message.into(),
                },
            })
            .unwrap_or_else(|_| "{}".to_string())
        };
        if let Err(message) = params.clone().parse() {
            return error(id, "invalid_params", message);
        }
        let capable = self.foreground_client_id.filter(|client_id| {
            self.clients.get(client_id).is_some_and(|client| {
                client.is_shell_client() && client.shell_show_machine && client.writer.is_some()
            })
        });
        let Some(client_id) = capable else {
            return error(id, "no_capable_client", NO_CAPABLE_CLIENT);
        };
        let message = match crate::protocol::endpoint::show_machine_message(&params) {
            Ok(message) => message,
            Err(err) => return error(id, "internal_error", &err.to_string()),
        };
        if !self.send_to_client(client_id, message) {
            return error(id, "no_capable_client", NO_CAPABLE_CLIENT);
        }
        serde_json::to_string(&api::schema::SuccessResponse {
            id,
            result: api::schema::ResponseResult::Ok {},
        })
        .unwrap_or_else(|_| "{}".to_string())
    }
}
