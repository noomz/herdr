use crate::api::schema::InstalledPluginInfo;
use crate::app::floats::{FloatPluginBinding, FloatPluginControls};
use crate::app::App;

use super::manifest::{effective_platforms, ensure_platform_supported};

fn controls_for(
    plugin: &InstalledPluginInfo,
    binding: &FloatPluginBinding,
) -> Option<FloatPluginControls> {
    if plugin.installation_id.as_deref() != Some(&binding.installation_id)
        || !plugin.enabled
        || !super::plugin_manifest_available(plugin)
    {
        return None;
    }
    let pane = plugin
        .panes
        .iter()
        .find(|pane| pane.id == binding.entrypoint)?;
    ensure_platform_supported(
        effective_platforms(&pane.platforms, &plugin.platforms),
        "pane",
    )
    .ok()?;
    let mut buttons = Vec::new();
    let mut actions = Vec::new();
    for button in &pane.buttons {
        let action = plugin
            .actions
            .iter()
            .find(|action| action.id == button.action)?;
        if ensure_platform_supported(
            effective_platforms(&action.platforms, &plugin.platforms),
            "action",
        )
        .is_ok()
        {
            buttons.push(button.clone());
            actions.push(action.clone());
        }
    }
    Some(FloatPluginControls {
        plugin_root: plugin.plugin_root.clone(),
        plugin_version: plugin.version.clone(),
        buttons,
        actions,
    })
}

impl App {
    pub(in crate::app::api) fn handle_float_button_invoke(
        &mut self,
        id: String,
        target: crate::api::schema::FloatButtonInvokeParams,
    ) -> String {
        use crate::api::schema::ResponseResult;
        use crate::app::api::responses::{encode_error, encode_success};

        if target.boot_id != self.state.server_boot_id {
            return encode_error(
                id,
                "stale_boot",
                "float button targeted an earlier server boot",
            );
        }
        if let Err(err) = self.refresh_installed_plugins() {
            return encode_error(id, "plugin_registry_load_failed", err.to_string());
        }
        let Some(float_id) = crate::protocol::floats::FloatId::parse(&target.float_id) else {
            return encode_error(
                id,
                "float_not_found",
                format!("float {} not found", target.float_id),
            );
        };
        let Some(float) = self.state.float(float_id) else {
            return encode_error(
                id,
                "float_not_found",
                format!("float {} not found", target.float_id),
            );
        };
        let Some(binding) = &float.plugin_binding else {
            return encode_error(
                id,
                "float_button_target_mismatch",
                format!("float {} has no plugin binding", target.float_id),
            );
        };
        if float.workspace != target.workspace_id.0
            || binding.plugin_id != target.plugin_id
            || binding.entrypoint != target.entrypoint
        {
            return encode_error(
                id,
                "float_button_target_mismatch",
                format!(
                    "float {} button {} target mismatch",
                    target.float_id, target.button_id
                ),
            );
        }
        if binding.controls_revision != target.controls_revision {
            return encode_error(
                id,
                "stale_float_controls",
                format!(
                    "float {} button {} controls changed",
                    target.float_id, target.button_id
                ),
            );
        }
        let Some(controls) = &binding.controls else {
            return encode_error(
                id,
                "plugin_unavailable",
                format!(
                    "float {} button {} plugin unavailable",
                    target.float_id, target.button_id
                ),
            );
        };
        let Some(index) = controls
            .buttons
            .iter()
            .position(|button| button.id == target.button_id)
        else {
            return encode_error(
                id,
                "unknown_float_button",
                format!(
                    "float {} button {} not found",
                    target.float_id, target.button_id
                ),
            );
        };
        let Some(action) = controls
            .actions
            .get(index)
            .filter(|action| action.id == controls.buttons[index].action)
            .cloned()
        else {
            return encode_error(id, "plugin_unavailable", "button action unavailable");
        };
        let Some(plugin) = self
            .state
            .installed_plugins
            .get(&binding.plugin_id)
            .cloned()
        else {
            return encode_error(id, "plugin_unavailable", "button plugin unavailable");
        };
        let mut context = match &float.workspace {
            Some(workspace_id) => {
                let Some(index) = self
                    .state
                    .workspaces
                    .iter()
                    .position(|workspace| &workspace.id == workspace_id)
                else {
                    return encode_error(id, "workspace_not_found", "float workspace unavailable");
                };
                self.plugin_context_for_workspace(index, &id)
            }
            None => super::context::empty_plugin_context(&id),
        };
        context.focused_pane_id = None;
        context.focused_pane_cwd = None;
        context.focused_pane_agent = None;
        context.focused_pane_status = None;
        context.invocation_source = Some("float_button".into());
        let log = match self.start_plugin_command_with_target(
            &plugin,
            Some(action.id),
            None,
            action.command,
            &context,
            None,
            Some(&target),
        ) {
            Ok(log) => log,
            Err((code, message)) => {
                return encode_error(
                    id,
                    code,
                    format!(
                        "float {} button {}: {message}",
                        target.float_id, target.button_id
                    ),
                )
            }
        };
        encode_success(id, ResponseResult::FloatButtonInvoked { log })
    }

    pub(super) fn revise_float_plugin_bindings(&mut self, plugin_id: &str) {
        let mut changed = false;
        for binding in self
            .state
            .floats
            .iter_mut()
            .filter_map(|float| float.plugin_binding.as_mut())
        {
            if binding.plugin_id == plugin_id {
                binding.controls_revision = uuid::Uuid::new_v4().to_string();
                changed = true;
            }
        }
        if changed {
            self.state.float_controls_generation =
                self.state.float_controls_generation.wrapping_add(1);
            self.render_dirty.request_generic();
            self.render_notify.notify_one();
        }
    }

    pub(crate) fn reconcile_float_plugin_bindings(&mut self, force_revision: bool) {
        let mut changed = false;
        for float in &mut self.state.floats {
            let Some(binding) = float.plugin_binding.as_mut() else {
                continue;
            };
            let controls = self
                .state
                .installed_plugins
                .get(&binding.plugin_id)
                .and_then(|plugin| controls_for(plugin, binding));
            if force_revision || controls != binding.controls {
                binding.controls = controls;
                binding.controls_revision = uuid::Uuid::new_v4().to_string();
                changed = true;
            }
        }
        if changed {
            self.state.float_controls_generation =
                self.state.float_controls_generation.wrapping_add(1);
            self.render_dirty.request_generic();
            self.render_notify.notify_one();
        }
    }
}

pub(super) fn new_binding(
    plugin: &InstalledPluginInfo,
    entrypoint: &str,
) -> Option<FloatPluginBinding> {
    let mut binding = FloatPluginBinding {
        plugin_id: plugin.plugin_id.clone(),
        installation_id: plugin.installation_id.clone()?,
        entrypoint: entrypoint.to_owned(),
        controls_revision: uuid::Uuid::new_v4().to_string(),
        controls: None,
    };
    binding.controls = controls_for(plugin, &binding);
    Some(binding)
}
