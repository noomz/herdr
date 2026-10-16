use serde::{Deserialize, Serialize};

use crate::popup_size::PopupSize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct FloatTarget {
    pub float_id: String,
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
    /// Position in the stacking order; 0 is the bottom float.
    pub z: usize,
}
