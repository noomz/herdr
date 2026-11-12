use super::responses::{encode_error, encode_success};
use crate::api::schema::{
    FloatHideTargetParams, FloatInfo, FloatMoveParams, FloatResizeParams, FloatSetHideRuleParams,
    FloatShowParams, FloatTarget, FloatVisibilityInfo, ResponseResult,
};
use crate::app::floats::{FloatEdit, FloatId, FloatState, FloatVisibility};
use crate::app::App;
use crate::popup_size::FloatPosition;

impl App {
    pub(crate) fn float_info(&self, z: usize, float: &FloatState) -> FloatInfo {
        let (x, y) = match float.position {
            FloatPosition::Centered => (None, None),
            FloatPosition::At { x, y } => (Some(x), Some(y)),
        };
        FloatInfo {
            float_id: float.id.to_string(),
            title: self
                .state
                .terminals
                .get(&float.terminal_id)
                .and_then(|terminal| terminal.manual_label.clone()),
            width: float.width,
            height: float.height,
            x,
            y,
            visibility: match float.visibility {
                FloatVisibility::Shown => FloatVisibilityInfo::Shown,
                FloatVisibility::Hidden => FloatVisibilityInfo::Hidden,
            },
            hide_while_focused: float.hide_while_focused.and_then(|pane_id| {
                let (ws_idx, _) = self.find_pane(pane_id)?;
                self.public_pane_id(ws_idx, pane_id)
            }),
            hide_on_blur: float.hide_on_blur,
            workspace_id: float.workspace.clone(),
            z,
        }
    }

    pub(super) fn float_info_by_id(&self, float_id: FloatId) -> Option<FloatInfo> {
        self.state
            .floats
            .iter()
            .enumerate()
            .find(|(_, float)| float.id == float_id)
            .map(|(z, float)| self.float_info(z, float))
    }

    pub(super) fn handle_float_list(&self, id: String) -> String {
        let floats = self
            .state
            .floats
            .iter()
            .enumerate()
            .map(|(z, float)| self.float_info(z, float))
            .collect();
        encode_success(id, ResponseResult::FloatList { floats })
    }

    pub(super) fn handle_float_move(&mut self, id: String, params: FloatMoveParams) -> String {
        let position = FloatPosition::At {
            x: params.x,
            y: params.y,
        };
        self.handle_float_edit(id, &params.float_id, FloatEdit::Move(position), false)
    }

    pub(super) fn handle_float_resize(&mut self, id: String, params: FloatResizeParams) -> String {
        let edit = FloatEdit::Resize {
            width: params.width,
            height: params.height,
        };
        self.handle_float_edit(id, &params.float_id, edit, false)
    }

    pub(super) fn handle_float_visibility(
        &mut self,
        id: String,
        target: FloatTarget,
        visibility: FloatVisibility,
    ) -> String {
        self.handle_float_edit(
            id,
            &target.float_id,
            FloatEdit::Visibility(visibility),
            false,
        )
    }

    /// Hides one exact float from this boot. An already-hidden float succeeds unchanged.
    pub(super) fn handle_float_hide_target(
        &mut self,
        id: String,
        params: FloatHideTargetParams,
    ) -> String {
        if params.boot_id != self.state.server_boot_id {
            return encode_error(
                id,
                "stale_boot",
                format!("float {} targeted an earlier server boot", params.float_id),
            );
        }
        self.handle_float_visibility(
            id,
            FloatTarget {
                float_id: params.float_id,
            },
            FloatVisibility::Hidden,
        )
    }

    pub(super) fn handle_float_show(&mut self, id: String, params: FloatShowParams) -> String {
        self.handle_float_edit(
            id,
            &params.float_id,
            FloatEdit::Visibility(FloatVisibility::Shown),
            params.focus == Some(true),
        )
    }

    pub(super) fn handle_float_raise(&mut self, id: String, params: FloatShowParams) -> String {
        self.handle_float_edit(
            id,
            &params.float_id,
            FloatEdit::Raise,
            params.focus == Some(true),
        )
    }

    pub(super) fn handle_float_set_hide_rule(
        &mut self,
        id: String,
        params: FloatSetHideRuleParams,
    ) -> String {
        let pane_id = match params.pane_id.as_deref() {
            None => None,
            Some(public_id) => match self.parse_pane_id(public_id) {
                Some((_, pane_id)) => Some(pane_id),
                None => return encode_error(id, "pane_not_found", "pane not found"),
            },
        };
        self.handle_float_edit(
            id,
            &params.float_id,
            FloatEdit::HideWhileFocused(pane_id),
            false,
        )
    }

    pub(super) fn handle_float_close(&mut self, id: String, target: FloatTarget) -> String {
        let closed = FloatId::parse(&target.float_id).is_some_and(|float| self.close_float(float));
        if !closed {
            return float_not_found(id);
        }
        encode_success(
            id,
            ResponseResult::FloatClosed {
                float_id: target.float_id,
            },
        )
    }

    fn handle_float_edit(
        &mut self,
        id: String,
        float_id: &str,
        edit: FloatEdit,
        focus: bool,
    ) -> String {
        let Some(float_id) = FloatId::parse(float_id) else {
            return float_not_found(id);
        };
        if self.edit_float(float_id, edit).is_none() {
            return float_not_found(id);
        }
        match self.float_info_by_id(float_id) {
            Some(float) => {
                if focus {
                    self.pending_float_focus.push(float_id);
                }
                encode_success(id, ResponseResult::FloatInfo { float })
            }
            None => float_not_found(id),
        }
    }
}

fn float_not_found(id: String) -> String {
    encode_error(id, "float_not_found", "float not found")
}

#[cfg(test)]
mod tests {
    use crate::api::schema::{
        FloatHideTargetParams, FloatMoveParams, FloatSetHideRuleParams, FloatShowParams,
        FloatTarget, Method, Request, ResponseResult, SuccessResponse,
    };
    use crate::app::floats::FloatVisibility;
    use crate::app::App;
    use crate::popup_size::{FloatPosition, PopupSize};

    fn app_with_floats(count: usize) -> (App, Vec<String>) {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &crate::config::Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            crate::api::EventHub::default(),
        );
        app.state.workspaces = vec![crate::workspace::Workspace::test_new("floats")];
        app.state.ensure_test_terminals();
        app.state.active = Some(0);
        let ids = (0..count)
            .map(|_| {
                let terminal_id = crate::terminal::TerminalId::alloc();
                app.state.terminals.insert(
                    terminal_id.clone(),
                    crate::terminal::TerminalState::new(terminal_id.clone(), "/float".into()),
                );
                let id = app.state.mint_float_id();
                app.state.floats.push(crate::app::floats::FloatState {
                    id,
                    pane_id: crate::layout::PaneId::alloc(),
                    terminal_id,
                    plugin_binding: None,
                    width: Some(PopupSize::Cells(20)),
                    height: Some(PopupSize::Cells(8)),
                    position: FloatPosition::Centered,
                    visibility: FloatVisibility::Shown,
                    hide_while_focused: None,
                    hide_on_blur: false,
                    workspace: None,
                });
                id.to_string()
            })
            .collect();
        (app, ids)
    }

    fn call(app: &mut App, method: Method) -> serde_json::Value {
        let response = app.handle_api_request(Request {
            id: "float".into(),
            method,
        });
        serde_json::from_str(&response).unwrap()
    }

    fn target(float_id: &str) -> FloatTarget {
        FloatTarget {
            float_id: float_id.into(),
        }
    }

    fn show_params(float_id: &str) -> FloatShowParams {
        FloatShowParams {
            float_id: float_id.into(),
            focus: None,
        }
    }

    #[test]
    fn float_info_omits_global_workspace_and_set_hide_rule_preserves_ownership() {
        let (mut app, ids) = app_with_floats(1);
        let listed = call(&mut app, Method::FloatList(Default::default()));
        assert!(listed["result"]["floats"][0].get("workspace_id").is_none());
        let workspace = app.state.workspaces[0].id.clone();
        app.state.floats[0].workspace = Some(workspace.clone());
        let response = call(
            &mut app,
            Method::FloatSetHideRule(FloatSetHideRuleParams {
                float_id: ids[0].clone(),
                pane_id: None,
            }),
        );
        assert_eq!(response["result"]["float"]["workspace_id"], workspace);
        assert_eq!(
            app.state.floats[0].workspace.as_deref(),
            Some(workspace.as_str())
        );
    }

    #[test]
    fn move_hide_show_and_raise_round_trip_through_the_api() {
        let (mut app, ids) = app_with_floats(2);

        let moved = call(
            &mut app,
            Method::FloatMove(FloatMoveParams {
                float_id: ids[0].clone(),
                x: 4,
                y: 2,
            }),
        );
        assert_eq!(moved["result"]["float"]["x"], 4);
        assert_eq!(moved["result"]["float"]["y"], 2);

        let hidden = call(&mut app, Method::FloatHide(target(&ids[0])));
        assert_eq!(hidden["result"]["float"]["visibility"], "hidden");
        assert_eq!(
            app.state.floats[0].visibility,
            FloatVisibility::Hidden,
            "hide keeps the float and its terminal"
        );
        assert!(app
            .state
            .terminals
            .contains_key(&app.state.floats[0].terminal_id));

        let shown = call(&mut app, Method::FloatShow(show_params(&ids[0])));
        assert_eq!(shown["result"]["float"]["visibility"], "shown");

        let raised = call(&mut app, Method::FloatRaise(show_params(&ids[0])));
        assert_eq!(raised["result"]["float"]["z"], 1);

        let listed = call(&mut app, Method::FloatList(Default::default()));
        let listed: Vec<_> = listed["result"]["floats"]
            .as_array()
            .unwrap()
            .iter()
            .map(|float| float["float_id"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(listed, vec![ids[1].clone(), ids[0].clone()]);
    }

    #[test]
    fn hide_rule_uses_public_pane_ids() {
        let (mut app, ids) = app_with_floats(1);
        let pane_id = app.state.workspaces[0].tabs[0].root_pane;
        let public_id = app.public_pane_id(0, pane_id).unwrap();

        let response = call(
            &mut app,
            Method::FloatSetHideRule(FloatSetHideRuleParams {
                float_id: ids[0].clone(),
                pane_id: Some(public_id.clone()),
            }),
        );
        assert_eq!(response["result"]["float"]["hide_while_focused"], public_id);
        assert_eq!(app.state.floats[0].hide_while_focused, Some(pane_id));

        let missing = call(
            &mut app,
            Method::FloatSetHideRule(FloatSetHideRuleParams {
                float_id: ids[0].clone(),
                pane_id: Some("p_missing".into()),
            }),
        );
        assert_eq!(missing["error"]["code"], "pane_not_found");

        call(
            &mut app,
            Method::FloatSetHideRule(FloatSetHideRuleParams {
                float_id: ids[0].clone(),
                pane_id: None,
            }),
        );
        assert_eq!(app.state.floats[0].hide_while_focused, None);
    }

    #[test]
    fn close_removes_the_float_and_unknown_ids_fail() {
        let (mut app, ids) = app_with_floats(1);

        let closed: SuccessResponse =
            serde_json::from_value(call(&mut app, Method::FloatClose(target(&ids[0])))).unwrap();
        assert_eq!(
            closed.result,
            ResponseResult::FloatClosed {
                float_id: ids[0].clone()
            }
        );
        assert!(app.state.floats.is_empty());

        for method in [
            Method::FloatClose(target(&ids[0])),
            Method::FloatHide(target(&ids[0])),
            Method::FloatRaise(show_params(&ids[0])),
        ] {
            assert_eq!(call(&mut app, method)["error"]["code"], "float_not_found");
        }
    }

    fn hide_target(app: &mut App, float_id: &str) -> serde_json::Value {
        let boot_id = app.state.server_boot_id.clone();
        call(
            app,
            Method::FloatHideTarget(FloatHideTargetParams {
                boot_id,
                float_id: float_id.into(),
            }),
        )
    }

    #[test]
    fn hide_target_hides_exact_float_and_repeats_without_mutation() {
        let (mut app, ids) = app_with_floats(2);
        call(
            &mut app,
            Method::FloatMove(FloatMoveParams {
                float_id: ids[0].clone(),
                x: 4,
                y: 2,
            }),
        );

        let hidden = hide_target(&mut app, &ids[0]);
        assert_eq!(hidden["result"]["float"]["float_id"], ids[0]);
        assert_eq!(hidden["result"]["float"]["visibility"], "hidden");
        assert_eq!(app.state.floats[1].visibility, FloatVisibility::Shown);
        assert!(app
            .state
            .terminals
            .contains_key(&app.state.floats[0].terminal_id));

        let before = app.state.floats.clone();
        for _ in 0..2 {
            let again = hide_target(&mut app, &ids[0]);
            assert_eq!(again["result"]["float"]["visibility"], "hidden");
        }
        assert_eq!(
            app.state.floats, before,
            "repeat hide keeps id, geometry, stacking, and terminal"
        );
    }

    #[test]
    fn hide_target_rejects_stale_boot_and_missing_float_without_changes() {
        let (mut app, ids) = app_with_floats(1);
        app.state.floats[0].visibility = FloatVisibility::Hidden;
        let before = app.state.floats.clone();

        let stale = call(
            &mut app,
            Method::FloatHideTarget(FloatHideTargetParams {
                boot_id: "earlier-boot".into(),
                float_id: ids[0].clone(),
            }),
        );
        assert_eq!(stale["error"]["code"], "stale_boot");

        let missing = hide_target(&mut app, "f999");
        assert_eq!(missing["error"]["code"], "float_not_found");
        assert_eq!(app.state.floats, before, "no float shown or created");

        let closed = ids[0].clone();
        call(&mut app, Method::FloatClose(target(&closed)));
        let after_close = hide_target(&mut app, &closed);
        assert_eq!(after_close["error"]["code"], "float_not_found");
        assert!(app.state.floats.is_empty(), "hide never recreates a float");
    }

    #[test]
    fn hide_target_params_require_both_fields() {
        for params in [
            serde_json::json!({"float_id": "f1"}),
            serde_json::json!({"boot_id": "b"}),
            serde_json::json!({"boot_id": "b", "float_id": "f1", "focus": true}),
        ] {
            assert!(serde_json::from_value::<FloatHideTargetParams>(params).is_err());
        }
    }
}
