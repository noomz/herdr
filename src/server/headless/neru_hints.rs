use super::*;

/// The CLI prints these after `herdr neru-hints: ` (design section 10.5).
const NO_FOCUSED_CLIENT: &str = "no focused herdr window.";

impl HeadlessServer {
    /// `client.neru_hints.run`: hands `herdr neru-hints` to the one client whose host window
    /// is focused (§V.20). It never waits for that client (§V.22) and never guesses: no
    /// client, or more than one, is an error and nothing is sent.
    pub(super) fn handle_neru_hints_api(&mut self, id: String) -> String {
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
        let focused: Vec<u64> = self
            .clients
            .iter()
            .filter(|(_, client)| {
                client.is_shell_client()
                    && client.shell_neru_hints
                    && client.writer.is_some()
                    && client.host_window_focused == Some(true)
            })
            .map(|(client_id, _)| *client_id)
            .collect();
        let client_id = match focused.as_slice() {
            [] => return error(id, "no_focused_client", NO_FOCUSED_CLIENT),
            [client_id] => *client_id,
            several => {
                return error(
                    id,
                    "ambiguous_focused_client",
                    &format!(
                        "{} herdr windows report focus. Click one and try again.",
                        several.len()
                    ),
                )
            }
        };
        if !self.send_to_client(client_id, crate::protocol::endpoint::neru_hints_message()) {
            return error(id, "no_focused_client", NO_FOCUSED_CLIENT);
        }
        serde_json::to_string(&api::schema::SuccessResponse {
            id,
            result: api::schema::ResponseResult::Ok {},
        })
        .unwrap_or_else(|_| "{}".to_string())
    }
}
