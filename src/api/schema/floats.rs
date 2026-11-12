use serde::{Deserialize, Serialize};

use crate::popup_size::PopupSize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct FloatTarget {
    pub float_id: String,
}

#[cfg(test)]
mod button_tests {
    #[test]
    fn float_button_workspace_is_required_and_nullable() {
        let mut request = serde_json::json!({
            "boot_id": "boot", "float_id": "f1", "workspace_id": null,
            "plugin_id": "plugin", "entrypoint": "shell", "controls_revision": "rev", "button_id": "close"
        });
        assert!(serde_json::from_value::<super::FloatButtonInvokeParams>(request.clone()).is_ok());
        request.as_object_mut().unwrap().remove("workspace_id");
        assert!(serde_json::from_value::<super::FloatButtonInvokeParams>(request).is_err());
        let schema =
            serde_json::to_value(schemars::schema_for!(super::FloatButtonInvokeParams)).unwrap();
        assert!(schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|field| field == "workspace_id"));
        let field = &schema["properties"]["workspace_id"];
        assert!(field.to_string().contains("null"), "{field}");
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FloatButtonInvokeParams {
    pub boot_id: String,
    pub float_id: String,
    #[serde(deserialize_with = "required_nullable")]
    pub workspace_id: RequiredNullableString,
    pub plugin_id: String,
    pub entrypoint: String,
    pub controls_revision: String,
    pub button_id: String,
}

/// Hides one exact float from one server boot. Never shows, creates, or retargets a float.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FloatHideTargetParams {
    pub boot_id: String,
    pub float_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RequiredNullableString(pub Option<String>);

impl schemars::JsonSchema for RequiredNullableString {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "RequiredNullableString".into()
    }
    fn inline_schema() -> bool {
        true
    }
    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        generator.subschema_for::<Option<String>>()
    }
}

pub(crate) fn required_nullable<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<RequiredNullableString, D::Error> {
    Option::<String>::deserialize(deserializer).map(RequiredNullableString)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct FloatShowParams {
    pub float_id: String,
    /// Request keyboard focus in connected clients that can see the float.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct FloatMoveParams {
    pub float_id: String,
    /// Outer left edge in cells from the pane surface's left edge. Clamped to stay on screen.
    pub x: u16,
    /// Outer top edge in cells from the pane surface's top edge. Clamped to stay on screen.
    pub y: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct FloatResizeParams {
    pub float_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<PopupSize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<PopupSize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct FloatSetHideRuleParams {
    pub float_id: String,
    /// Hide the float while this pane is focused. `null` removes the rule.
    #[serde(default)]
    pub pane_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FloatVisibilityInfo {
    Shown,
    Hidden,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct FloatInfo {
    pub float_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<PopupSize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<PopupSize>,
    /// Requested outer position; absent while the float is centered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<u16>,
    pub visibility: FloatVisibilityInfo,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hide_while_focused: Option<String>,
    #[serde(default)]
    pub hide_on_blur: bool,
    /// Public workspace ID; absent when visible across all workspaces.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    /// Position in the stacking order; 0 is the bottom float.
    pub z: usize,
}
