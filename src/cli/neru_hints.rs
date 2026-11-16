//! `herdr neru-hints`: neru's hint hotkey asks the local server to start neru hints in the
//! herdr client whose window is focused (design note section 10). The command exits 0 once a
//! client has the request and 1 otherwise, so neru's `exec` step can fall back to its own hints.
//! It never waits for the client (§V.22) and never starts a server.

use crate::api::schema::{EmptyParams, Method, Request};

const PREFIX: &str = "herdr neru-hints";

pub(super) fn run(args: &[String]) -> std::io::Result<i32> {
    if !args.is_empty() {
        eprintln!("usage: herdr neru-hints");
        return Ok(2);
    }
    if super::target::is_remote() {
        eprintln!("{PREFIX} runs only against the local server.");
        return Ok(1);
    }
    let request = Request {
        id: "cli:neru-hints".into(),
        method: Method::ClientNeruHintsRun(EmptyParams::default()),
    };
    let (exit_code, line) = outcome(super::send_request(&request));
    if let Some(line) = line {
        eprintln!("{line}");
    }
    Ok(exit_code)
}

/// The exit code and the one stderr line for a request's result (design section 10.5).
fn outcome(result: std::io::Result<serde_json::Value>) -> (i32, Option<String>) {
    let response = match result {
        Ok(response) => response,
        // `ensure_server_protocol_compatible` has printed its own message already.
        Err(error) if super::protocol_mismatch_was_reported(&error) => return (1, None),
        Err(error) if super::server_not_running_was_reported(&error) => {
            return (1, Some(format!("{PREFIX}: no herdr server is running.")))
        }
        Err(error) => return (1, Some(format!("{PREFIX}: {error}"))),
    };
    let Some(error) = response.get("error") else {
        return (0, None);
    };
    let message = match error["code"].as_str().unwrap_or_default() {
        // A server older than the method cannot parse the request at all.
        "unknown_method" | "invalid_request" => {
            "the herdr server does not support neru-hints. Restart it after updating."
        }
        _ => error["message"].as_str().unwrap_or("request failed."),
    };
    (1, Some(format!("{PREFIX}: {message}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn error(code: &str, message: &str) -> std::io::Result<serde_json::Value> {
        Ok(serde_json::json!({"id": "cli:neru-hints", "error": {"code": code, "message": message}}))
    }

    #[test]
    fn a_client_that_took_the_request_exits_zero_silently() {
        assert_eq!(
            outcome(Ok(
                serde_json::json!({"id": "cli:neru-hints", "result": {"type": "ok"}})
            )),
            (0, None)
        );
    }

    #[test]
    fn server_refusals_print_one_plain_line_and_exit_one() {
        assert_eq!(
            outcome(error("no_focused_client", "no focused herdr window.")),
            (
                1,
                Some("herdr neru-hints: no focused herdr window.".to_owned())
            )
        );
        assert_eq!(
            outcome(error(
                "ambiguous_focused_client",
                "2 herdr windows report focus. Click one and try again."
            )),
            (
                1,
                Some(
                    "herdr neru-hints: 2 herdr windows report focus. Click one and try again."
                        .to_owned()
                )
            )
        );
        assert_eq!(
            outcome(error("internal_error", "boom")),
            (1, Some("herdr neru-hints: boom".to_owned()))
        );
    }

    #[test]
    fn an_older_server_is_told_apart_from_a_refusal() {
        let expected = Some(
            "herdr neru-hints: the herdr server does not support neru-hints. Restart it after updating."
                .to_owned(),
        );
        assert_eq!(
            outcome(error("unknown_method", "unknown method")),
            (1, expected.clone())
        );
        assert_eq!(
            outcome(error(
                "invalid_request",
                "invalid request: unknown variant `client.neru_hints.run`"
            )),
            (1, expected)
        );
    }

    #[test]
    fn a_missing_server_or_protocol_mismatch_exits_one() {
        let not_running = super::super::server_not_running::reported_error(
            super::super::server_not_running::response(
                "cli:neru-hints",
                std::path::Path::new("/nonexistent/herdr.sock"),
            ),
        );
        assert_eq!(
            outcome(Err(not_running)),
            (
                1,
                Some("herdr neru-hints: no herdr server is running.".to_owned())
            )
        );
        assert_eq!(
            outcome(Err(super::super::protocol_guard::reported_error())),
            (1, None)
        );
        assert_eq!(
            outcome(Err(std::io::Error::other("socket closed"))),
            (1, Some("herdr neru-hints: socket closed".to_owned()))
        );
    }
}
