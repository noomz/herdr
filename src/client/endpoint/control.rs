use super::ClientEndpointId;

pub(crate) struct DecodedAgentViewProjection {
    pub(crate) boot_id: String,
    pub(crate) revision: u64,
    pub(crate) view: Result<Option<crate::api::schema::AgentViewSetParams>, ()>,
}

pub(crate) enum EndpointControlMessage {
    HealthPong,
    AgentViewProjection(DecodedAgentViewProjection),
    AgentCompletions(crate::protocol::endpoint::EndpointAgentCompletions),
    Snapshot(Box<crate::protocol::ClientShellSnapshot>),
    Floats(crate::protocol::floats::FloatsSurface),
    FloatFocus(crate::protocol::floats::FloatFocus),
    FloatButtons(Option<crate::protocol::endpoint::EndpointFloatButtons>),
    FloatButtonResult(crate::protocol::endpoint::EndpointFloatButtonResult),
    Ignored,
}

pub(crate) fn decode_endpoint_control(
    kind: &str,
    data: &str,
) -> Result<EndpointControlMessage, String> {
    if kind == crate::protocol::endpoint::HEALTH_PONG_KIND {
        return Ok(EndpointControlMessage::HealthPong);
    }
    if kind == crate::protocol::endpoint::AGENT_COMPLETIONS_KIND {
        return Ok(serde_json::from_str(data)
            .map(EndpointControlMessage::AgentCompletions)
            .unwrap_or(EndpointControlMessage::Ignored));
    }
    if kind == crate::protocol::endpoint::AGENT_VIEW_PROJECTION_KIND {
        let Ok(projection): Result<crate::protocol::endpoint::EndpointAgentViewProjection, _> =
            serde_json::from_str(data)
        else {
            return Ok(EndpointControlMessage::Ignored);
        };
        let view = match projection.view {
            Some(value) => serde_json::from_value(value)
                .map(Some)
                .map_err(|_| ())
                .and_then(|mut view| {
                    view.as_mut()
                        .map(crate::app::agent_view::validate_agent_view)
                        .transpose()
                        .map(|_| view)
                        .map_err(|_| ())
                }),
            None => Ok(None),
        };
        return Ok(EndpointControlMessage::AgentViewProjection(
            DecodedAgentViewProjection {
                boot_id: projection.boot_id,
                revision: projection.revision,
                view,
            },
        ));
    }
    if kind == crate::protocol::endpoint::FLOAT_BUTTONS_KIND {
        let metadata =
            serde_json::from_str::<crate::protocol::endpoint::EndpointFloatButtons>(data)
                .ok()
                .filter(valid_float_button_metadata);
        return Ok(EndpointControlMessage::FloatButtons(metadata));
    }
    if kind == crate::protocol::endpoint::FLOAT_BUTTON_RESULT_KIND {
        return Ok(serde_json::from_str(data)
            .map(EndpointControlMessage::FloatButtonResult)
            .unwrap_or(EndpointControlMessage::Ignored));
    }
    if kind == crate::protocol::floats::FOCUS_KIND {
        return Ok(crate::protocol::floats::decode_float_focus(data)
            .map(EndpointControlMessage::FloatFocus)
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "ignoring invalid float focus");
                EndpointControlMessage::Ignored
            }));
    }
    if kind == crate::protocol::floats::SURFACE_KIND {
        return Ok(crate::protocol::floats::decode_surface(data)
            .map(EndpointControlMessage::Floats)
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "ignoring invalid float layer");
                EndpointControlMessage::Ignored
            }));
    }
    if kind == crate::protocol::endpoint::ENDPOINT_SNAPSHOT_KIND {
        let snapshot = serde_json::from_str(data)
            .map_err(|error| format!("invalid endpoint snapshot: {error}"))?;
        return Ok(EndpointControlMessage::Snapshot(Box::new(snapshot)));
    }
    if kind.starts_with("shell.snapshot.") {
        return Err(format!(
            "unsupported mandatory endpoint snapshot codec {kind:?}"
        ));
    }
    Ok(EndpointControlMessage::Ignored)
}

fn valid_float_button_metadata(metadata: &crate::protocol::endpoint::EndpointFloatButtons) -> bool {
    let mut floats = std::collections::HashSet::new();
    metadata.floats.iter().all(|float| {
        let mut buttons = std::collections::HashSet::new();
        floats.insert(float.float_id)
            && !float.plugin_id.is_empty()
            && !float.entrypoint.is_empty()
            && !float.controls_revision.is_empty()
            && float.controls_revision.len() <= 256
            && float.buttons.iter().all(|button| {
                let valid_id = |id: &str| {
                    !id.is_empty()
                        && id.len() <= 120
                        && id.bytes().all(|byte| {
                            byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'_' | b'-')
                        })
                };
                valid_id(&button.id)
                    && valid_id(&button.action)
                    && buttons.insert(&button.id)
                    && crate::protocol::endpoint::valid_button_icon(&button.icon)
            })
    })
}

pub(crate) fn protocol_failure_is_fatal(endpoint_id: &ClientEndpointId) -> bool {
    endpoint_id.is_local()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::endpoint::ProfileId;

    #[test]
    fn float_button_metadata_rejects_invalid_icons_duplicates_and_missing_workspace() {
        let mut value = serde_json::json!({
            "boot_id": "boot", "revision": 1, "floats": [{
                "float_id": "f1", "workspace_id": null, "plugin_id": "plugin", "entrypoint": "shell",
                "controls_revision": uuid::Uuid::new_v4().to_string(),
                "buttons": [{"id": "close", "icon": "x", "action": "hide"}]
            }]
        });
        let decode = |value: &serde_json::Value| {
            decode_endpoint_control(
                crate::protocol::endpoint::FLOAT_BUTTONS_KIND,
                &value.to_string(),
            )
            .unwrap()
        };
        assert!(matches!(
            decode(&value),
            EndpointControlMessage::FloatButtons(Some(_))
        ));
        value["floats"][0]["buttons"][0]["icon"] = "\u{1b}".into();
        assert!(matches!(
            decode(&value),
            EndpointControlMessage::FloatButtons(None)
        ));
        value["floats"][0]["buttons"][0]["icon"] = "x".into();
        let duplicate = value["floats"][0]["buttons"][0].clone();
        value["floats"][0]["buttons"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
        assert!(matches!(
            decode(&value),
            EndpointControlMessage::FloatButtons(None)
        ));
        value["floats"][0]["buttons"].as_array_mut().unwrap().pop();
        value["floats"][0]
            .as_object_mut()
            .unwrap()
            .remove("workspace_id");
        assert!(matches!(
            decode(&value),
            EndpointControlMessage::FloatButtons(None)
        ));
    }

    #[test]
    fn unknown_optional_controls_are_ignored() {
        assert!(matches!(
            decode_endpoint_control("future.optional", "not json").unwrap(),
            EndpointControlMessage::Ignored
        ));
    }

    #[test]
    fn completion_guard_optional_control_round_trips_without_changing_snapshot_codec() {
        let projection = crate::protocol::endpoint::EndpointAgentCompletions {
            boot_id: "boot".into(),
            revision: 3,
            completions: [("pane".into(), 7)].into_iter().collect(),
        };
        let crate::protocol::ServerMessage::EndpointControl { kind, data } =
            crate::protocol::endpoint::agent_completions_message(&projection).unwrap()
        else {
            panic!("expected optional control");
        };
        let EndpointControlMessage::AgentCompletions(decoded) =
            decode_endpoint_control(&kind, &data).unwrap()
        else {
            panic!("expected completion projection");
        };
        assert_eq!(decoded, projection);
        assert!(matches!(
            decode_endpoint_control(&kind, "invalid").unwrap(),
            EndpointControlMessage::Ignored
        ));
    }

    #[test]
    fn agent_view_projection_decodes_and_validates_the_view() {
        let view = crate::api::schema::AgentViewSetParams {
            source: "example.views".into(),
            label: Some("focus".into()),
            filter: None,
            sort: Vec::new(),
        };
        let crate::protocol::ServerMessage::EndpointControl { kind, data } =
            crate::protocol::endpoint::agent_view_projection_message("boot", 4, Some(&view))
                .unwrap()
        else {
            panic!("projection control");
        };
        let EndpointControlMessage::AgentViewProjection(decoded) =
            decode_endpoint_control(&kind, &data).unwrap()
        else {
            panic!("decoded projection");
        };
        assert_eq!(decoded.boot_id, "boot");
        assert_eq!(decoded.revision, 4);
        assert_eq!(decoded.view, Ok(Some(view)));
    }

    #[test]
    fn unsupported_agent_view_payload_falls_back_without_rejecting_endpoint() {
        let projection = crate::protocol::endpoint::EndpointAgentViewProjection {
            boot_id: "boot".into(),
            revision: 5,
            view: Some(serde_json::json!({
                "source": "example.views",
                "filter": {"op": "future_filter"}
            })),
        };
        let decoded = decode_endpoint_control(
            crate::protocol::endpoint::AGENT_VIEW_PROJECTION_KIND,
            &serde_json::to_string(&projection).unwrap(),
        )
        .unwrap();
        let EndpointControlMessage::AgentViewProjection(decoded) = decoded else {
            panic!("decoded projection");
        };
        assert_eq!(decoded.view, Err(()));
    }

    #[test]
    fn malformed_agent_view_envelope_is_ignored() {
        assert!(matches!(
            decode_endpoint_control(
                crate::protocol::endpoint::AGENT_VIEW_PROJECTION_KIND,
                "not json"
            )
            .unwrap(),
            EndpointControlMessage::Ignored
        ));
    }

    #[test]
    fn unknown_snapshot_codecs_are_rejected() {
        assert_eq!(
            decode_endpoint_control("shell.snapshot.v2", "{}")
                .err()
                .as_deref(),
            Some("unsupported mandatory endpoint snapshot codec \"shell.snapshot.v2\"")
        );
    }

    #[test]
    fn only_local_protocol_failures_end_the_client() {
        let remote =
            ClientEndpointId::Ssh(ProfileId::parse("0123456789abcdef0123456789abcdef").unwrap());
        assert!(protocol_failure_is_fatal(&ClientEndpointId::Local));
        assert!(!protocol_failure_is_fatal(&remote));
    }
}
