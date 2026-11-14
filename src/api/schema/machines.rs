use serde::{Deserialize, Serialize};

/// Wire shape of `machine.show`. At most one target ID may be set; [`Self::parse`] turns it
/// into a [`MachineShow`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct MachineShowParams {
    /// Saved machine label or profile ID as the attached client knows it, or `local`.
    pub machine: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineShow {
    pub machine: String,
    pub target: Option<MachineShowTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MachineShowTarget {
    Workspace(String),
    Tab(String),
    Pane(String),
}

impl MachineShowParams {
    pub fn parse(self) -> Result<MachineShow, &'static str> {
        if self.machine.trim().is_empty() {
            return Err("machine is empty");
        }
        let targets = [
            self.workspace_id.map(MachineShowTarget::Workspace),
            self.tab_id.map(MachineShowTarget::Tab),
            self.pane_id.map(MachineShowTarget::Pane),
        ];
        let mut targets = targets.into_iter().flatten();
        let target = targets.next();
        if targets.next().is_some() {
            return Err("set at most one of workspace_id, tab_id, or pane_id");
        }
        if target.as_ref().is_some_and(|target| {
            let (MachineShowTarget::Workspace(id)
            | MachineShowTarget::Tab(id)
            | MachineShowTarget::Pane(id)) = target;
            id.is_empty()
        }) {
            return Err("target ID is empty");
        }
        Ok(MachineShow {
            machine: self.machine,
            target,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(value: serde_json::Value) -> MachineShowParams {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn parse_accepts_a_machine_with_no_target_or_one_target() {
        assert_eq!(
            params(serde_json::json!({"machine": "drm"})).parse(),
            Ok(MachineShow {
                machine: "drm".into(),
                target: None,
            })
        );
        assert_eq!(
            params(serde_json::json!({"machine": "local", "tab_id": "w1:t2"})).parse(),
            Ok(MachineShow {
                machine: "local".into(),
                target: Some(MachineShowTarget::Tab("w1:t2".into())),
            })
        );
    }

    #[test]
    fn parse_rejects_two_targets_an_empty_machine_and_an_empty_target() {
        for value in [
            serde_json::json!({"machine": "drm", "workspace_id": "w1", "pane_id": "w1:p1"}),
            serde_json::json!({"machine": " "}),
            serde_json::json!({"machine": "drm", "pane_id": ""}),
        ] {
            assert!(params(value).parse().is_err());
        }
    }
}
