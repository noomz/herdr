use super::*;
use crate::api::schema::{ErrorResponse, Request, SuccessResponse};
use crate::app::{AppPolicy, AppState};
use crate::config::Config;

fn fixture() -> (App, usize, PaneRepositionParams) {
    let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let mut app = App::new(
        &Config::default(),
        AppPolicy::TEST,
        None,
        rx,
        crate::api::EventHub::default(),
    );
    app.state = AppState::test_with_adversarial_identity_state();
    // Enable scheduling only after construction; these tests never run the writer.
    app.policy.persist_session = true;
    let tab_idx = app.state.workspaces[0]
        .tabs
        .iter()
        .position(|tab| tab.layout.pane_count() >= 3)
        .unwrap();
    let ids = app.state.workspaces[0].tabs[tab_idx].layout.pane_ids();
    let params = PaneRepositionParams {
        source_pane_id: app.public_pane_id(0, ids[0]).unwrap(),
        target_pane_id: app.public_pane_id(0, ids[2]).unwrap(),
        tab_id: app.public_tab_id(0, tab_idx).unwrap(),
        side: PaneRepositionSide::Left,
    };
    app.state.session_dirty = false;
    app.state.assert_invariants_for_test();
    (app, tab_idx, params)
}

fn result(response: &str) -> PaneRepositionResult {
    let success: SuccessResponse = serde_json::from_str(response).unwrap();
    assert_eq!(success.id, "reposition");
    let ResponseResult::PaneReposition { reposition } = success.result else {
        panic!("expected reposition result");
    };
    reposition
}

#[test]
fn api_reposition_all_sides_preserve_adversarial_identity_and_shared_focus() {
    for side in [
        PaneRepositionSide::Left,
        PaneRepositionSide::Right,
        PaneRepositionSide::Above,
        PaneRepositionSide::Below,
    ] {
        let (mut app, tab_idx, mut params) = fixture();
        params.side = side;
        let (_, source) = app.parse_pane_id(&params.source_pane_id).unwrap();
        let (_, target) = app.parse_pane_id(&params.target_pane_id).unwrap();
        app.state.workspaces[0].tabs[tab_idx]
            .layout
            .focus_pane(target);
        let root_pane = app.state.workspaces[0].tabs[tab_idx].root_pane;
        let active_tab = app.state.workspaces[0].active_tab;
        let active = app.state.active;
        let history = app.state.previous_pane_focus.clone();
        let numbers = app.state.workspaces[0].public_pane_numbers.clone();
        let terminals: std::collections::HashMap<_, _> = app.state.workspaces[0].tabs[tab_idx]
            .panes
            .iter()
            .map(|(id, pane)| (*id, pane.attached_terminal_id.clone()))
            .collect();
        let terminal_id = terminals[&source].clone();
        let terminal = app.state.terminals.get_mut(&terminal_id).unwrap();
        terminal.manual_label = Some("retain source name".into());
        terminal.cwd = std::path::PathBuf::from("/retain-source-cwd");
        let terminal_count = app.state.terminals.len();

        let reposition = result(&app.handle_pane_reposition("reposition".into(), params.clone()));

        assert!(reposition.changed);
        assert_eq!(reposition.reason, None);
        assert_eq!(reposition.source_pane_id, params.source_pane_id);
        assert_eq!(reposition.target_pane_id, params.target_pane_id);
        assert_eq!(reposition.layout.tab_id, params.tab_id);
        assert_eq!(reposition.layout.focused_pane_id, params.target_pane_id);
        assert_eq!(app.state.active, active);
        assert_eq!(app.state.workspaces[0].active_tab, active_tab);
        assert_eq!(app.state.previous_pane_focus, history);
        assert_eq!(app.state.workspaces[0].public_pane_numbers, numbers);
        let tab = &app.state.workspaces[0].tabs[tab_idx];
        assert_eq!(tab.root_pane, root_pane);
        assert_eq!(tab.layout.focused(), target);
        assert_eq!(tab.panes.len(), terminals.len());
        for (id, terminal) in terminals {
            assert_eq!(tab.terminal_id(id), Some(&terminal));
        }
        let rects = tab.layout.panes(ratatui::layout::Rect::new(0, 0, 120, 60));
        let source_rect = rects.iter().find(|pane| pane.id == source).unwrap().rect;
        let target_rect = rects.iter().find(|pane| pane.id == target).unwrap().rect;
        match side {
            PaneRepositionSide::Left => assert_eq!(source_rect.right(), target_rect.x),
            PaneRepositionSide::Right => assert_eq!(target_rect.right(), source_rect.x),
            PaneRepositionSide::Above => assert_eq!(source_rect.bottom(), target_rect.y),
            PaneRepositionSide::Below => assert_eq!(target_rect.bottom(), source_rect.y),
        }
        assert_eq!(app.state.terminals.len(), terminal_count);
        let terminal = &app.state.terminals[&terminal_id];
        assert_eq!(terminal.manual_label.as_deref(), Some("retain source name"));
        assert_eq!(terminal.cwd, std::path::PathBuf::from("/retain-source-cwd"));
        assert!(app.state.session_dirty);
        assert!(app.session_save_deadline.is_some());
        let events = app.event_hub.events_after(0);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].1.event, EventKind::LayoutUpdated);
        assert!(
            matches!(&events[0].1.data, EventData::LayoutUpdated { layout } if *layout == reposition.layout)
        );
        app.state.assert_invariants_for_test();
    }
}

#[test]
fn api_reposition_noops_leave_tree_focus_and_persistence_untouched() {
    let (mut app, tab_idx, params) = fixture();
    assert!(result(&app.handle_pane_reposition("reposition".into(), params.clone())).changed);
    let before = app.state.workspaces[0].tabs[tab_idx].layout.root().clone();
    let focus = app.state.workspaces[0].tabs[tab_idx].layout.focused();
    let events = app.event_hub.events_after(0).len();
    app.state.session_dirty = false;
    app.session_save_deadline = None;
    for same in [false, true] {
        let mut request = params.clone();
        if same {
            request.target_pane_id = request.source_pane_id.clone();
        }
        let reposition = result(&app.handle_pane_reposition("reposition".into(), request));
        assert!(!reposition.changed);
        assert_eq!(
            reposition.reason,
            Some(if same {
                PaneRepositionReason::SamePane
            } else {
                PaneRepositionReason::Unchanged
            })
        );
        assert_eq!(app.state.workspaces[0].tabs[tab_idx].layout.root(), &before);
        assert_eq!(
            app.state.workspaces[0].tabs[tab_idx].layout.focused(),
            focus
        );
        assert_eq!(app.event_hub.events_after(0).len(), events);
        assert!(!app.state.session_dirty);
        assert!(app.session_save_deadline.is_none());
        app.state.assert_invariants_for_test();
    }
}

#[test]
fn api_reposition_rejections_preserve_exact_layout_and_identity() {
    for case in ["source", "target", "tab", "membership", "zoom"] {
        let (mut app, tab_idx, mut params) = fixture();
        let expected = match case {
            "source" => {
                params.source_pane_id = "missing".into();
                "pane_not_found"
            }
            "target" => {
                params.target_pane_id = "missing".into();
                "pane_not_found"
            }
            "tab" => {
                params.tab_id = "missing".into();
                "tab_not_found"
            }
            "membership" => {
                params.tab_id = app.public_tab_id(0, 0).unwrap();
                "pane_tab_mismatch"
            }
            "zoom" => {
                app.state.workspaces[0].tabs[tab_idx].zoomed = true;
                "pane_tab_zoomed"
            }
            _ => unreachable!(),
        };
        let before = app.state.workspaces[0].tabs[tab_idx].layout.root().clone();
        let focus = app.state.workspaces[0].tabs[tab_idx].layout.focused();
        let numbers = app.state.workspaces[0].public_pane_numbers.clone();
        let response = app.handle_pane_reposition("reposition".into(), params);
        let error: ErrorResponse = serde_json::from_str(&response).unwrap();
        assert_eq!(error.error.code, expected);
        assert_eq!(app.state.workspaces[0].tabs[tab_idx].layout.root(), &before);
        assert_eq!(
            app.state.workspaces[0].tabs[tab_idx].layout.focused(),
            focus
        );
        assert_eq!(app.state.workspaces[0].public_pane_numbers, numbers);
        assert!(!app.state.session_dirty);
        assert!(app.session_save_deadline.is_none());
        assert!(app.event_hub.events_after(0).is_empty());
        app.state.assert_invariants_for_test();
    }
}

#[test]
fn api_reposition_json_requires_explicit_membership_and_valid_side() {
    let request = serde_json::json!({
        "id": "reposition", "method": "pane.reposition", "params": {
            "source_pane_id": "p_1_1", "target_pane_id": "p_1_2",
            "tab_id": "t_1_1", "side": "left"
        }
    });
    assert!(serde_json::from_value::<Request>(request.clone()).is_ok());
    for field in ["source_pane_id", "target_pane_id", "tab_id", "side"] {
        let mut incomplete = request.clone();
        incomplete["params"].as_object_mut().unwrap().remove(field);
        assert!(
            serde_json::from_value::<Request>(incomplete).is_err(),
            "missing {field}"
        );
    }
    for side in ["up", "down", "diagonal", ""] {
        let mut invalid = request.clone();
        invalid["params"]["side"] = side.into();
        assert!(serde_json::from_value::<Request>(invalid).is_err());
    }
}

#[test]
fn api_legacy_swap_explicit_unfocused_source_becomes_shared_focus() {
    let (mut app, tab_idx, params) = fixture();
    let (_, source) = app.parse_pane_id(&params.source_pane_id).unwrap();
    let (_, target) = app.parse_pane_id(&params.target_pane_id).unwrap();
    app.state.workspaces[0].tabs[tab_idx]
        .layout
        .focus_pane(target);
    let response = app.handle_pane_swap(
        "swap".into(),
        PaneSwapParams {
            source_pane_id: Some(params.source_pane_id.clone()),
            target_pane_id: Some(params.target_pane_id),
            ..PaneSwapParams::default()
        },
    );
    let success: SuccessResponse = serde_json::from_str(&response).unwrap();
    let ResponseResult::PaneSwap { swap } = success.result else {
        panic!("expected swap");
    };
    assert!(swap.changed);
    assert_eq!(swap.focused_pane_id, params.source_pane_id);
    assert_eq!(
        app.state.workspaces[0].tabs[tab_idx].layout.focused(),
        source
    );
    assert_eq!(app.event_hub.events_after(0).len(), 1);
    app.state.assert_invariants_for_test();
}

#[test]
fn api_move_new_tab_without_label_or_focus_preserves_manual_pane_name() {
    for label in [None, Some("named source")] {
        let (mut app, tab_idx, params) = fixture();
        let (_, source) = app.parse_pane_id(&params.source_pane_id).unwrap();
        let terminal_id = app.state.workspaces[0].tabs[tab_idx]
            .terminal_id(source)
            .unwrap()
            .clone();
        app.state
            .terminals
            .get_mut(&terminal_id)
            .unwrap()
            .manual_label = label.map(str::to_owned);
        let active_tab_id = app
            .public_tab_id(0, app.state.workspaces[0].active_tab)
            .unwrap();
        let terminal_count = app.state.terminals.len();
        let response = app.handle_pane_move(
            "move".into(),
            PaneMoveParams {
                pane_id: params.source_pane_id.clone(),
                destination: PaneMoveDestination::NewTab {
                    workspace_id: Some(app.public_workspace_id(0)),
                    label: None,
                },
                focus: false,
            },
        );
        let success: SuccessResponse = serde_json::from_str(&response).unwrap();
        let ResponseResult::PaneMove { move_result } = success.result else {
            panic!("expected move");
        };
        assert!(move_result.changed);
        assert_eq!(move_result.pane.pane_id, params.source_pane_id);
        assert_eq!(move_result.pane.terminal_id, terminal_id.to_string());
        assert_eq!(app.state.terminals.len(), terminal_count);
        assert_eq!(
            app.state.terminals[&terminal_id].manual_label.as_deref(),
            label
        );
        assert_eq!(
            app.public_tab_id(0, app.state.workspaces[0].active_tab)
                .unwrap(),
            active_tab_id
        );
        let created = move_result.created_tab.unwrap();
        assert!(!created.focused);
        assert!(created.label.parse::<usize>().is_ok());
        assert_eq!(move_result.closed_tab_id, None);
        app.state.assert_invariants_for_test();
    }
}
