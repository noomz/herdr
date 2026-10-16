use super::render::put_text;
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ClientPaneTargetOperation {
    Swap,
    Place(crate::api::schema::PaneRepositionSide),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ClientPaneDropTarget {
    Pane {
        pane_id: String,
        operation: ClientPaneTargetOperation,
    },
    NewTab,
}

#[derive(Debug)]
pub(super) struct ClientPaneRearrangementDrag {
    pub(super) boot_id: String,
    pub(super) source_pane_id: String,
    pub(super) workspace_id: String,
    pub(super) tab_id: String,
    pub(super) start_column: u16,
    pub(super) start_row: u16,
    pub(super) surface_signature: u64,
    pub(super) dragging: bool,
    pub(super) target: Option<ClientPaneDropTarget>,
}

impl ClientPaneTargetOperation {
    pub(super) fn title(self) -> &'static str {
        match self {
            Self::Swap => "swap pane with…",
            Self::Place(crate::api::schema::PaneRepositionSide::Left) => "place pane left of…",
            Self::Place(crate::api::schema::PaneRepositionSide::Right) => "place pane right of…",
            Self::Place(crate::api::schema::PaneRepositionSide::Above) => "place pane above…",
            Self::Place(crate::api::schema::PaneRepositionSide::Below) => "place pane below…",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct ClientPaneRearrangementAvailability {
    pub(super) swap_reason: Option<&'static str>,
    pub(super) reposition_reason: Option<&'static str>,
    pub(super) move_reason: Option<&'static str>,
}

#[derive(Debug)]
pub(super) struct ClientPaneTargetSelector {
    pub(super) boot_id: String,
    pub(super) source_pane_id: String,
    pub(super) workspace_id: String,
    pub(super) tab_id: String,
    pub(super) operation: ClientPaneTargetOperation,
    pub(super) eligible_pane_ids: Vec<String>,
    pub(super) surface_signature: u64,
    /// No target is selected until the user explicitly moves selection.
    pub(super) selected: Option<usize>,
}

impl ClientPaneTargetSelector {
    pub(super) fn selected_pane_id(&self) -> Option<&str> {
        self.selected
            .and_then(|index| self.eligible_pane_ids.get(index))
            .map(String::as_str)
    }

    pub(super) fn cycle(&mut self, reverse: bool) {
        if self.eligible_pane_ids.is_empty() {
            self.selected = None;
            return;
        }
        self.selected = Some(match self.selected {
            None if reverse => self.eligible_pane_ids.len() - 1,
            None => 0,
            Some(0) if reverse => self.eligible_pane_ids.len() - 1,
            Some(index) if reverse => index - 1,
            Some(index) => (index + 1) % self.eligible_pane_ids.len(),
        });
    }

    pub(super) fn select_pane(&mut self, pane_id: &str) -> bool {
        let Some(index) = self
            .eligible_pane_ids
            .iter()
            .position(|candidate| candidate == pane_id)
        else {
            return false;
        };
        let changed = self.selected != Some(index);
        self.selected = Some(index);
        changed
    }

    pub(super) fn select_direction(&mut self, hits: &[PaneHit], direction: PaneNavigation) {
        let origin_id = self.selected_pane_id().unwrap_or(&self.source_pane_id);
        let Some(origin) = hits.iter().find(|hit| hit.pane_id == origin_id) else {
            return;
        };
        let (origin_x, origin_y) = rect_center(origin.rect);
        let mut best: Option<(i32, i32, usize)> = None;
        for (index, pane_id) in self.eligible_pane_ids.iter().enumerate() {
            if pane_id == origin_id {
                continue;
            }
            let Some(candidate) = hits.iter().find(|hit| &hit.pane_id == pane_id) else {
                continue;
            };
            let (x, y) = rect_center(candidate.rect);
            let (primary, secondary) = match direction {
                PaneNavigation::Left if x < origin_x => (origin_x - x, (origin_y - y).abs()),
                PaneNavigation::Right if x > origin_x => (x - origin_x, (origin_y - y).abs()),
                PaneNavigation::Up if y < origin_y => (origin_y - y, (origin_x - x).abs()),
                PaneNavigation::Down if y > origin_y => (y - origin_y, (origin_x - x).abs()),
                _ => continue,
            };
            let score = (primary, secondary, index);
            if best.is_none_or(|current| score < current) {
                best = Some(score);
            }
        }
        if let Some((_, _, index)) = best {
            self.selected = Some(index);
        }
    }

    pub(super) fn method(&self) -> Option<crate::api::schema::Method> {
        let target_pane_id = self.selected_pane_id()?.to_owned();
        Some(match self.operation {
            ClientPaneTargetOperation::Swap => {
                crate::api::schema::Method::PaneSwap(crate::api::schema::PaneSwapParams {
                    pane_id: None,
                    direction: None,
                    source_pane_id: Some(self.source_pane_id.clone()),
                    target_pane_id: Some(target_pane_id),
                })
            }
            ClientPaneTargetOperation::Place(side) => crate::api::schema::Method::PaneReposition(
                crate::api::schema::PaneRepositionParams {
                    source_pane_id: self.source_pane_id.clone(),
                    target_pane_id,
                    tab_id: self.tab_id.clone(),
                    side,
                },
            ),
        })
    }
}

#[derive(Clone, Copy)]
pub(super) enum PaneNavigation {
    Left,
    Right,
    Up,
    Down,
}

fn rect_center(rect: Rect) -> (i32, i32) {
    (
        i32::from(rect.x) * 2 + i32::from(rect.width),
        i32::from(rect.y) * 2 + i32::from(rect.height),
    )
}

pub(super) fn availability(
    state: &ClientShellState,
    pane_count: usize,
    zoomed: bool,
) -> ClientPaneRearrangementAvailability {
    let rearrangement_pending = state.timed_out_endpoint.is_some()
        || state.pending_requests.values().any(|pending| {
            matches!(
                pending.method_name.as_str(),
                "pane.swap" | "pane.reposition" | "pane.move"
            )
        });
    let supported = |method_name: &str| state.advertises_endpoint_method(method_name);
    let reason = |method_name: &'static str, zoom_blocks: bool, sole_blocks: bool| {
        if rearrangement_pending {
            Some("operation pending")
        } else if !supported(method_name) {
            Some("server capability unavailable")
        } else if zoom_blocks && zoomed {
            Some("unavailable while zoomed")
        } else if sole_blocks && pane_count <= 1 {
            Some("requires another pane")
        } else {
            None
        }
    };
    ClientPaneRearrangementAvailability {
        swap_reason: reason("pane.swap", false, true),
        reposition_reason: reason("pane.reposition", true, true),
        move_reason: reason("pane.move", true, true),
    }
}

pub(super) fn selector_still_valid(
    selector: &ClientPaneTargetSelector,
    snapshot: &ClientShellSnapshot,
    surface: &PaneSurfaceFrame,
    endpoint_methods: Option<&HashSet<String>>,
    rearrangement_pending: bool,
) -> bool {
    let method_name = match selector.operation {
        ClientPaneTargetOperation::Swap => "pane.swap",
        ClientPaneTargetOperation::Place(_) => "pane.reposition",
    };
    let method_available = endpoint_methods.is_some_and(|methods| methods.contains(method_name));
    let tab_zoomed = snapshot
        .tabs
        .iter()
        .find(|tab| tab.tab_id == selector.tab_id && tab.workspace_id == selector.workspace_id)
        .is_some_and(|tab| tab.zoomed);
    !rearrangement_pending
        && method_available
        && snapshot.boot_id == selector.boot_id
        && snapshot.focused_workspace_id.as_deref() == Some(&selector.workspace_id)
        && snapshot.focused_tab_id.as_deref() == Some(&selector.tab_id)
        && surface_signature(surface) == selector.surface_signature
        && !(matches!(selector.operation, ClientPaneTargetOperation::Place(_)) && tab_zoomed)
        && snapshot.panes.iter().any(|pane| {
            pane.pane_id == selector.source_pane_id
                && pane.tab_id == selector.tab_id
                && pane.workspace_id == selector.workspace_id
        })
        && selector.eligible_pane_ids.iter().all(|pane_id| {
            snapshot.panes.iter().any(|pane| {
                pane.pane_id == *pane_id
                    && pane.tab_id == selector.tab_id
                    && pane.workspace_id == selector.workspace_id
            })
        })
}

pub(super) fn surface_signature(surface: &PaneSurfaceFrame) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    fn write(hash: &mut u64, bytes: &[u8]) {
        for byte in bytes {
            *hash ^= u64::from(*byte);
            *hash = hash.wrapping_mul(PRIME);
        }
    }
    let mut hash = OFFSET;
    for pane in &surface.panes {
        write(&mut hash, pane.pane_id.as_bytes());
        for value in [
            pane.rect.x,
            pane.rect.y,
            pane.rect.width,
            pane.rect.height,
            pane.inner_rect.x,
            pane.inner_rect.y,
            pane.inner_rect.width,
            pane.inner_rect.height,
        ] {
            write(&mut hash, &value.to_le_bytes());
        }
    }
    for split in &surface.splits {
        write(
            &mut hash,
            &[match split.direction {
                crate::protocol::PaneSurfaceSplitDirection::Horizontal => 0,
                crate::protocol::PaneSurfaceSplitDirection::Vertical => 1,
            }],
        );
        for value in [
            split.pos,
            split.area.x,
            split.area.y,
            split.area.width,
            split.area.height,
            split.hit_rect.x,
            split.hit_rect.y,
            split.hit_rect.width,
            split.hit_rect.height,
        ] {
            write(&mut hash, &value.to_le_bytes());
        }
        for branch in &split.path {
            write(&mut hash, &[u8::from(*branch)]);
        }
    }
    hash
}

impl ClientShellState {
    pub(super) fn cancel_pane_rearrangement_drag(&mut self, swallow_release: bool) -> bool {
        let canceled = self.pane_rearrangement_drag.take().is_some();
        if canceled && swallow_release {
            self.swallow_pane_rearrangement_left_up = true;
        }
        canceled
    }

    fn rearrangement_pending(&self) -> bool {
        self.timed_out_endpoint.is_some()
            || self.pending_requests.values().any(|pending| {
                matches!(
                    pending.method_name.as_str(),
                    "pane.swap" | "pane.reposition" | "pane.move"
                )
            })
    }

    fn pane_drag_still_valid(&self, drag: &ClientPaneRearrangementDrag) -> bool {
        let Some((snapshot, surface)) = self.snapshot.as_deref().zip(self.pane_surface.as_ref())
        else {
            return false;
        };
        let pane_count = snapshot
            .panes
            .iter()
            .filter(|pane| pane.tab_id == drag.tab_id)
            .count();
        let zoomed = snapshot
            .tabs
            .iter()
            .find(|tab| tab.tab_id == drag.tab_id && tab.workspace_id == drag.workspace_id)
            .is_none_or(|tab| tab.zoomed);
        let available = availability(self, pane_count, zoomed);
        !self.rearrangement_pending()
            && (available.swap_reason.is_none()
                || available.reposition_reason.is_none()
                || available.move_reason.is_none())
            && snapshot.boot_id == drag.boot_id
            && snapshot.focused_workspace_id.as_deref() == Some(&drag.workspace_id)
            && snapshot.focused_tab_id.as_deref() == Some(&drag.tab_id)
            && !zoomed
            && surface_signature(surface) == drag.surface_signature
            && snapshot.panes.iter().any(|pane| {
                pane.pane_id == drag.source_pane_id
                    && pane.workspace_id == drag.workspace_id
                    && pane.tab_id == drag.tab_id
            })
            && surface.panes.iter().all(|surface_pane| {
                snapshot.panes.iter().any(|pane| {
                    pane.pane_id == surface_pane.pane_id
                        && pane.workspace_id == drag.workspace_id
                        && pane.tab_id == drag.tab_id
                })
            })
    }

    pub(super) fn reconcile_pane_rearrangement_drag(&mut self) -> bool {
        let valid = self
            .pane_rearrangement_drag
            .as_ref()
            .is_none_or(|drag| self.pane_drag_still_valid(drag));
        if valid || !self.cancel_pane_rearrangement_drag(true) {
            return false;
        }
        self.push_pane_rearrangement_invalidation_notice();
        true
    }

    pub(super) fn try_arm_pane_rearrangement_drag(
        &mut self,
        mouse: crossterm::event::MouseEvent,
    ) -> bool {
        if !self.config.mouse_capture
            || self.overlay.is_some()
            || self.popup_pending
            || self.popup_terminal_id.is_some()
            || self.mode != ClientShellMode::Terminal
            || self.pane_mouse_gesture.is_some()
            || !mouse
                .modifiers
                .contains(crossterm::event::KeyModifiers::ALT)
        {
            return false;
        }
        let point = (mouse.column, mouse.row);
        let owners = self
            .hits
            .panes
            .iter()
            .filter(|hit| pane_owned_chrome_contains(hit, point))
            .collect::<Vec<_>>();
        let [source_hit] = owners.as_slice() else {
            return false;
        };
        let Some(snapshot) = self.snapshot.as_deref() else {
            return false;
        };
        let Some(surface) = self.pane_surface.as_ref() else {
            return false;
        };
        let Some(source) = snapshot
            .panes
            .iter()
            .find(|pane| pane.pane_id == source_hit.pane_id)
        else {
            return false;
        };
        let pane_count = snapshot
            .panes
            .iter()
            .filter(|pane| pane.tab_id == source.tab_id)
            .count();
        let zoomed = snapshot
            .tabs
            .iter()
            .find(|tab| tab.tab_id == source.tab_id)
            .is_none_or(|tab| tab.zoomed);
        let available = availability(self, pane_count, zoomed);
        if zoomed
            || (available.swap_reason.is_some()
                && available.reposition_reason.is_some()
                && available.move_reason.is_some())
        {
            return false;
        }
        self.pane_rearrangement_drag = Some(ClientPaneRearrangementDrag {
            boot_id: snapshot.boot_id.clone(),
            source_pane_id: source.pane_id.clone(),
            workspace_id: source.workspace_id.clone(),
            tab_id: source.tab_id.clone(),
            start_column: mouse.column,
            start_row: mouse.row,
            surface_signature: surface_signature(surface),
            dragging: false,
            target: None,
        });
        true
    }

    pub(super) fn handle_pane_rearrangement_mouse(
        &mut self,
        mouse: crossterm::event::MouseEvent,
        outcome: &mut ClientShellInput,
    ) -> bool {
        use crossterm::event::{MouseButton, MouseEventKind};
        if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
            if self.cancel_pane_rearrangement_drag(false) {
                outcome.repaint = true;
            }
            self.swallow_pane_rearrangement_left_up = false;
            return false;
        }
        if mouse.kind == MouseEventKind::Up(MouseButton::Left)
            && self.swallow_pane_rearrangement_left_up
        {
            self.swallow_pane_rearrangement_left_up = false;
            return true;
        }
        if !matches!(
            mouse.kind,
            MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left)
        ) || self.pane_rearrangement_drag.is_none()
        {
            return false;
        }
        let Some(mut drag) = self.pane_rearrangement_drag.take() else {
            return false;
        };
        if !self.pane_drag_still_valid(&drag) {
            self.swallow_pane_rearrangement_left_up =
                mouse.kind != MouseEventKind::Up(MouseButton::Left);
            outcome.repaint = true;
            return true;
        }
        let delta = mouse
            .column
            .abs_diff(drag.start_column)
            .max(mouse.row.abs_diff(drag.start_row));
        if delta >= 2 {
            drag.dragging = true;
        }
        drag.target = drag
            .dragging
            .then(|| self.pane_drop_target_at(&drag, (mouse.column, mouse.row)))
            .flatten();
        if mouse.kind == MouseEventKind::Drag(MouseButton::Left) {
            self.pane_rearrangement_drag = Some(drag);
            outcome.repaint = true;
            return true;
        }
        if drag.dragging {
            if let Some(target) = drag.target.take() {
                self.dispatch_pane_drop(drag, target, outcome);
            }
        }
        outcome.repaint = true;
        true
    }

    fn pane_drop_target_at(
        &self,
        drag: &ClientPaneRearrangementDrag,
        point: (u16, u16),
    ) -> Option<ClientPaneDropTarget> {
        let snapshot = self.snapshot.as_deref()?;
        let pane_count = snapshot
            .panes
            .iter()
            .filter(|pane| pane.tab_id == drag.tab_id)
            .count();
        let available = availability(self, pane_count, false);
        if available.move_reason.is_none()
            && (super::contains(self.hits.new_tab, point)
                || (self.hits.tab_scroll_left.is_empty()
                    && self.hits.tab_scroll_right.is_empty()
                    && super::contains(self.hits.tab_bar, point)
                    && !self
                        .hits
                        .tabs
                        .iter()
                        .any(|(rect, _)| super::contains(*rect, point))))
        {
            return Some(ClientPaneDropTarget::NewTab);
        }
        let hit =
            self.hits.panes.iter().find(|hit| {
                hit.pane_id != drag.source_pane_id && super::contains(hit.rect, point)
            })?;
        let operation = pane_drop_operation(hit.rect, point);
        match operation {
            ClientPaneTargetOperation::Swap if available.swap_reason.is_none() => {}
            ClientPaneTargetOperation::Place(_) if available.reposition_reason.is_none() => {}
            _ => return None,
        }
        Some(ClientPaneDropTarget::Pane {
            pane_id: hit.pane_id.clone(),
            operation,
        })
    }

    fn dispatch_pane_drop(
        &mut self,
        drag: ClientPaneRearrangementDrag,
        target: ClientPaneDropTarget,
        outcome: &mut ClientShellInput,
    ) {
        use crate::api::schema::{
            Method, PaneMoveDestination, PaneMoveParams, PaneRepositionParams, PaneSwapParams,
        };
        match target {
            ClientPaneDropTarget::Pane {
                pane_id,
                operation: ClientPaneTargetOperation::Swap,
            } => self.push_endpoint_method(
                Method::PaneSwap(PaneSwapParams {
                    pane_id: None,
                    direction: None,
                    source_pane_id: Some(drag.source_pane_id),
                    target_pane_id: Some(pane_id),
                }),
                outcome,
            ),
            ClientPaneDropTarget::Pane {
                pane_id,
                operation: ClientPaneTargetOperation::Place(side),
            } => self.push_pane_rearrangement_method(
                Method::PaneReposition(PaneRepositionParams {
                    source_pane_id: drag.source_pane_id.clone(),
                    target_pane_id: pane_id,
                    tab_id: drag.tab_id.clone(),
                    side,
                }),
                drag.source_pane_id,
                Some(drag.tab_id),
                PaneRearrangementFollow::Reposition,
                outcome,
            ),
            ClientPaneDropTarget::NewTab => self.push_pane_rearrangement_method(
                Method::PaneMove(PaneMoveParams {
                    pane_id: drag.source_pane_id.clone(),
                    destination: PaneMoveDestination::NewTab {
                        workspace_id: Some(drag.workspace_id),
                        label: None,
                    },
                    focus: false,
                }),
                drag.source_pane_id,
                None,
                PaneRearrangementFollow::MoveNewTab,
                outcome,
            ),
        }
    }

    pub(crate) fn revoke_rearrangement_follow(&mut self) {
        self.navigation_intent_generation = self.navigation_intent_generation.saturating_add(1);
    }

    pub(super) fn push_pane_rearrangement_invalidation_notice(&mut self) -> bool {
        self.endpoint_notice_seen.remove(&ClientEndpointNoticeKey {
            boot_id: self
                .snapshot
                .as_deref()
                .map(|snapshot| snapshot.boot_id.clone())
                .unwrap_or_default(),
            kind: ClientEndpointNoticeKind::Unavailable,
            code: "pane_rearrangement_invalidated".into(),
        });
        self.push_endpoint_notice(
            ClientEndpointNoticeKind::Unavailable,
            "pane_rearrangement_invalidated",
            "Pane rearrangement canceled",
            "The pane layout or active server changed. Start the rearrangement again.",
        )
    }

    pub(super) fn push_pane_rearrangement_method(
        &mut self,
        method: crate::api::schema::Method,
        source_pane_id: String,
        expected_tab_id: Option<String>,
        follow: PaneRearrangementFollow,
        outcome: &mut ClientShellInput,
    ) {
        let method_name = crate::api::api_method_name(&method);
        if !self.advertises_endpoint_method(method_name) {
            self.push_endpoint_notice(
                ClientEndpointNoticeKind::Unsupported,
                method_name,
                "Action unavailable",
                format!(
                    "This server does not support {method_name} yet. Update and restart it to enable this action."
                ),
            );
            outcome.repaint = true;
            return;
        }
        let navigation_generation = self.navigation_intent_generation;
        self.push_endpoint_method_with_kind(
            method,
            PendingEndpointKind::PaneRearrangement {
                source_pane_id,
                expected_tab_id,
                navigation_generation,
                follow,
            },
            outcome,
        );
    }

    pub(super) fn open_pane_target_selector(
        &mut self,
        source_pane_id: String,
        workspace_id: String,
        tab_id: String,
        operation: ClientPaneTargetOperation,
    ) {
        let Some(snapshot) = self.snapshot.as_deref() else {
            return;
        };
        let Some(surface) = self.pane_surface.as_ref() else {
            return;
        };
        let eligible_pane_ids = snapshot
            .panes
            .iter()
            .filter(|pane| pane.tab_id == tab_id && pane.pane_id != source_pane_id)
            .map(|pane| pane.pane_id.clone())
            .collect::<Vec<_>>();
        if eligible_pane_ids.is_empty() {
            return;
        }
        self.overlay = Some(ClientShellOverlay::PaneTargetSelector(
            ClientPaneTargetSelector {
                boot_id: snapshot.boot_id.clone(),
                source_pane_id,
                workspace_id,
                tab_id,
                operation,
                eligible_pane_ids,
                surface_signature: surface_signature(surface),
                selected: None,
            },
        ));
    }

    pub(super) fn pane_target_selector_valid(&self) -> bool {
        let Some(ClientShellOverlay::PaneTargetSelector(selector)) = self.overlay.as_ref() else {
            return false;
        };
        let Some((snapshot, surface)) = self.snapshot.as_deref().zip(self.pane_surface.as_ref())
        else {
            return false;
        };
        let rearrangement_pending = self.timed_out_endpoint.is_some()
            || self.pending_requests.values().any(|pending| {
                matches!(
                    pending.method_name.as_str(),
                    "pane.swap" | "pane.reposition" | "pane.move"
                )
            });
        selector_still_valid(
            selector,
            snapshot,
            surface,
            self.active_endpoint_methods(),
            rearrangement_pending,
        )
    }

    pub(super) fn confirm_pane_target(&mut self, outcome: &mut ClientShellInput) {
        if !self.pane_target_selector_valid() {
            self.overlay = None;
            outcome.repaint = true;
            return;
        }
        let request = match self.overlay.as_ref() {
            Some(ClientShellOverlay::PaneTargetSelector(selector)) => {
                selector.method().map(|method| {
                    (
                        method,
                        selector.source_pane_id.clone(),
                        Some(selector.tab_id.clone()),
                        match selector.operation {
                            ClientPaneTargetOperation::Swap => None,
                            ClientPaneTargetOperation::Place(_) => {
                                Some(PaneRearrangementFollow::Reposition)
                            }
                        },
                    )
                })
            }
            _ => None,
        };
        let Some((method, source_pane_id, expected_tab_id, follow)) = request else {
            return;
        };
        self.overlay = None;
        if let Some(follow) = follow {
            self.push_pane_rearrangement_method(
                method,
                source_pane_id,
                expected_tab_id,
                follow,
                outcome,
            );
        } else {
            // pane.swap already focuses its explicit source in shared runtime state.
            self.push_endpoint_method(method, outcome);
        }
        outcome.repaint = true;
    }

    pub(super) fn complete_pane_rearrangement(
        &mut self,
        boot_id: &str,
        source_pane_id: String,
        expected_tab_id: Option<String>,
        navigation_generation: u64,
        request_snapshot_revision: u64,
        follow: PaneRearrangementFollow,
        result: Result<crate::api::schema::ResponseResult, ClientShellEndpointError>,
    ) -> (bool, Vec<ClientShellAction>) {
        if self.navigation_intent_generation != navigation_generation
            || self
                .snapshot
                .as_deref()
                .is_none_or(|snapshot| snapshot.boot_id != boot_id)
        {
            return (true, Vec::new());
        }
        let current_snapshot = self.snapshot.as_deref().expect("validated snapshot");
        let current_membership_required = current_snapshot.revision > request_snapshot_revision;
        let follow_method = match (follow, result) {
            (
                PaneRearrangementFollow::Reposition,
                Ok(crate::api::schema::ResponseResult::PaneReposition { reposition }),
            ) if reposition.changed
                && reposition.source_pane_id == source_pane_id
                && expected_tab_id.as_deref() == Some(reposition.layout.tab_id.as_str())
                && reposition
                    .layout
                    .panes
                    .iter()
                    .any(|pane| pane.pane_id == source_pane_id)
                && (!current_membership_required
                    || current_snapshot.panes.iter().any(|pane| {
                        pane.pane_id == source_pane_id
                            && Some(pane.tab_id.as_str()) == expected_tab_id.as_deref()
                    })) =>
            {
                Some(crate::api::schema::Method::PaneFocus(
                    crate::api::schema::PaneTarget {
                        pane_id: source_pane_id,
                    },
                ))
            }
            (
                PaneRearrangementFollow::MoveNewTab,
                Ok(crate::api::schema::ResponseResult::PaneMove { move_result }),
            ) if move_result.changed
                && move_result.previous_pane_id == source_pane_id
                && move_result.pane.tab_id == move_result.target_layout.tab_id
                && move_result
                    .target_layout
                    .panes
                    .iter()
                    .any(|pane| pane.pane_id == move_result.pane.pane_id)
                && (!current_membership_required
                    || current_snapshot.panes.iter().any(|pane| {
                        pane.pane_id == move_result.pane.pane_id
                            && pane.tab_id == move_result.target_layout.tab_id
                    })) =>
            {
                Some(crate::api::schema::Method::TabFocus(
                    crate::api::schema::TabTarget {
                        tab_id: move_result.target_layout.tab_id,
                    },
                ))
            }
            _ => None,
        };
        let Some(method) = follow_method else {
            return (true, Vec::new());
        };
        let mut outcome = ClientShellInput::default();
        self.push_endpoint_method(method, &mut outcome);
        (true, outcome.actions)
    }
}

fn pane_owned_chrome_contains(hit: &PaneHit, point: (u16, u16)) -> bool {
    if !super::contains(hit.rect, point)
        || super::contains(hit.inner_rect, point)
        || hit
            .scrollbar_rect
            .is_some_and(|rect| super::contains(rect, point))
    {
        return false;
    }
    let top = hit.inner_rect.y > hit.rect.y && point.1 == hit.rect.y;
    let left = hit.inner_rect.x > hit.rect.x && point.0 == hit.rect.x;
    let bottom = hit.inner_rect.bottom() < hit.rect.bottom()
        && point.1 == hit.rect.bottom().saturating_sub(1);
    let right =
        hit.inner_rect.right() < hit.rect.right() && point.0 == hit.rect.right().saturating_sub(1);
    top || left || bottom || right
}

pub(super) fn pane_drop_operation(rect: Rect, point: (u16, u16)) -> ClientPaneTargetOperation {
    use crate::api::schema::PaneRepositionSide as Side;
    if rect.width < 5 || rect.height < 5 {
        return ClientPaneTargetOperation::Swap;
    }
    let x = point.0.saturating_sub(rect.x).min(rect.width - 1);
    let y = point.1.saturating_sub(rect.y).min(rect.height - 1);
    let candidates = [
        (Side::Left, x, rect.width, 0u8),
        (Side::Right, rect.width - 1 - x, rect.width, 1),
        (Side::Above, y, rect.height, 2),
        (Side::Below, rect.height - 1 - y, rect.height, 3),
    ];
    let mut best: Option<(Side, u16, u16, u8)> = None;
    for candidate in candidates
        .into_iter()
        .filter(|(_, distance, length, _)| u32::from(*distance) * 4 + 2 <= u32::from(*length))
    {
        if best.is_none_or(|(_, best_distance, best_length, best_order)| {
            (u32::from(candidate.1) * 2 + 1) * u32::from(best_length)
                < (u32::from(best_distance) * 2 + 1) * u32::from(candidate.2)
                || ((u32::from(candidate.1) * 2 + 1) * u32::from(best_length)
                    == (u32::from(best_distance) * 2 + 1) * u32::from(candidate.2)
                    && candidate.3 < best_order)
        }) {
            best = Some(candidate);
        }
    }
    best.map_or(ClientPaneTargetOperation::Swap, |(side, _, _, _)| {
        ClientPaneTargetOperation::Place(side)
    })
}

pub(super) fn render_target_selector(
    buffer: &mut Buffer,
    selector: &ClientPaneTargetSelector,
    hits: &[PaneHit],
    snapshot: &ClientShellSnapshot,
    palette: &Palette,
) -> Vec<(Rect, usize)> {
    let selected = selector.selected_pane_id();
    let mut rows = Vec::new();
    for (index, pane_id) in selector.eligible_pane_ids.iter().enumerate() {
        let Some(hit) = hits.iter().find(|hit| &hit.pane_id == pane_id) else {
            continue;
        };
        let active = selected == Some(pane_id.as_str());
        let style = Style::default()
            .fg(if active {
                palette.panel_bg
            } else {
                palette.accent
            })
            .bg(if active {
                palette.accent
            } else {
                palette.panel_bg
            })
            .add_modifier(if active {
                Modifier::BOLD
            } else {
                Modifier::empty()
            });
        draw_outline(buffer, hit.rect, style);
        let label = snapshot
            .panes
            .iter()
            .find(|pane| pane.pane_id == *pane_id)
            .and_then(|pane| pane.label.as_deref())
            .unwrap_or(pane_id);
        if hit.rect.width > 2 && hit.rect.height > 1 {
            put_text(
                buffer,
                hit.rect.x.saturating_add(1),
                hit.rect.y,
                hit.rect.width.saturating_sub(2),
                &format!(" {label} "),
                style,
            );
        }
        rows.push((hit.rect, index));
    }
    if buffer.area.height > 0 {
        let y = buffer.area.bottom().saturating_sub(1);
        let style = Style::default().fg(palette.text).bg(palette.panel_bg);
        let message = match selected {
            Some(pane_id) => format!(
                " {} → {}  enter confirm · esc cancel ",
                selector.operation.title(),
                pane_id
            ),
            None => format!(
                " {}  tab/arrows select · esc cancel ",
                selector.operation.title()
            ),
        };
        put_text(buffer, buffer.area.x, y, buffer.area.width, &message, style);
    }
    rows
}

pub(super) fn render_pane_drag(
    buffer: &mut Buffer,
    drag: &ClientPaneRearrangementDrag,
    hits: &[PaneHit],
    snapshot: &ClientShellSnapshot,
    palette: &Palette,
) {
    let source_style = Style::default()
        .fg(palette.yellow)
        .add_modifier(Modifier::BOLD);
    let target_style = Style::default()
        .fg(palette.accent)
        .add_modifier(Modifier::BOLD);
    if let Some(source) = hits.iter().find(|hit| hit.pane_id == drag.source_pane_id) {
        draw_outline(buffer, source.rect, source_style);
    }
    if drag.dragging {
        for hit in hits.iter().filter(|hit| hit.pane_id != drag.source_pane_id) {
            draw_outline(buffer, hit.rect, target_style);
        }
    }
    let Some(target) = drag.target.as_ref() else {
        return;
    };
    let message = match target {
        ClientPaneDropTarget::NewTab => " move pane to new tab ".to_owned(),
        ClientPaneDropTarget::Pane { pane_id, operation } => {
            let label = snapshot
                .panes
                .iter()
                .find(|pane| pane.pane_id == *pane_id)
                .and_then(|pane| pane.label.as_deref())
                .unwrap_or(pane_id);
            if let Some(hit) = hits.iter().find(|hit| hit.pane_id == *pane_id) {
                draw_drop_marker(buffer, hit.rect, *operation, target_style);
            }
            format!(" {} {} ", operation.title().trim_end_matches('…'), label)
        }
    };
    if buffer.area.height > 0 {
        put_text(
            buffer,
            buffer.area.x,
            buffer.area.bottom() - 1,
            buffer.area.width,
            &message,
            Style::default().fg(palette.text).bg(palette.panel_bg),
        );
    }
}

fn draw_drop_marker(
    buffer: &mut Buffer,
    rect: Rect,
    operation: ClientPaneTargetOperation,
    style: Style,
) {
    use crate::api::schema::PaneRepositionSide as Side;
    match operation {
        ClientPaneTargetOperation::Swap => {
            if rect.width > 2 && rect.height > 2 {
                draw_outline(
                    buffer,
                    Rect::new(
                        rect.x + rect.width / 4,
                        rect.y + rect.height / 4,
                        (rect.width / 2).max(1),
                        (rect.height / 2).max(1),
                    ),
                    style,
                );
            }
        }
        ClientPaneTargetOperation::Place(Side::Left | Side::Right) => {
            let x = if operation == ClientPaneTargetOperation::Place(Side::Left) {
                rect.x
            } else {
                rect.right().saturating_sub(1)
            };
            for y in rect.y..rect.bottom() {
                buffer[(x, y)].set_symbol("│").set_style(style);
            }
        }
        ClientPaneTargetOperation::Place(Side::Above | Side::Below) => {
            let y = if operation == ClientPaneTargetOperation::Place(Side::Above) {
                rect.y
            } else {
                rect.bottom().saturating_sub(1)
            };
            for x in rect.x..rect.right() {
                buffer[(x, y)].set_symbol("─").set_style(style);
            }
        }
    }
}

fn draw_outline(buffer: &mut Buffer, rect: Rect, style: Style) {
    if rect.is_empty() {
        return;
    }
    for x in rect.x..rect.right() {
        buffer[(x, rect.y)].set_symbol("─").set_style(style);
        if rect.height > 1 {
            buffer[(x, rect.bottom() - 1)]
                .set_symbol("─")
                .set_style(style);
        }
    }
    for y in rect.y..rect.bottom() {
        buffer[(rect.x, y)].set_symbol("│").set_style(style);
        if rect.width > 1 {
            buffer[(rect.right() - 1, y)]
                .set_symbol("│")
                .set_style(style);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn completion_state() -> ClientShellState {
        let mut snapshot = crate::client::shell::tests::snapshot();
        snapshot.panes.push(crate::protocol::ClientShellPane {
            pane_id: "target".into(),
            workspace_id: "ws_1".into(),
            tab_id: "tab_1".into(),
            label: None,
            cwd: None,
            foreground_cwd: None,
            focused: false,
            right_click_passthrough: false,
        });
        let mut state = ClientShellState::new(ClientShellConfig::from_config(
            &crate::config::Config::default(),
        ));
        state.set_snapshot(Box::new(snapshot));
        state.set_endpoint_methods(Some(vec![
            "pane.swap".into(),
            "pane.reposition".into(),
            "pane.move".into(),
            "pane.focus".into(),
            "tab.focus".into(),
            "tab.create".into(),
        ]));
        state
    }

    fn layout(tab_id: &str, pane_id: &str) -> crate::api::schema::PaneLayoutSnapshot {
        crate::api::schema::PaneLayoutSnapshot {
            workspace_id: "ws_1".into(),
            tab_id: tab_id.into(),
            zoomed: false,
            area: crate::api::schema::PaneLayoutRect {
                x: 0,
                y: 0,
                width: 80,
                height: 24,
            },
            focused_pane_id: pane_id.into(),
            panes: vec![crate::api::schema::PaneLayoutPane {
                pane_id: pane_id.into(),
                focused: true,
                rect: crate::api::schema::PaneLayoutRect {
                    x: 0,
                    y: 0,
                    width: 80,
                    height: 24,
                },
            }],
            splits: Vec::new(),
        }
    }

    fn moved_pane(tab_id: &str) -> crate::api::schema::PaneInfo {
        crate::api::schema::PaneInfo {
            pane_id: "pane_1".into(),
            terminal_id: "terminal_1".into(),
            workspace_id: "ws_1".into(),
            tab_id: tab_id.into(),
            focused: false,
            cwd: None,
            foreground_cwd: None,
            restore_error: None,
            label: None,
            agent: None,
            title: None,
            terminal_title: None,
            terminal_title_stripped: None,
            display_agent: None,
            agent_status: crate::api::schema::AgentStatus::Idle,
            state_labels: HashMap::new(),
            tokens: HashMap::new(),
            agent_session: None,
            scroll: None,
            revision: 1,
        }
    }

    fn enqueue_follow(
        state: &mut ClientShellState,
        follow: PaneRearrangementFollow,
        expected_tab_id: Option<&str>,
    ) -> String {
        let mut input = ClientShellInput::default();
        state.push_pane_rearrangement_method(
            crate::api::schema::Method::PaneReposition(crate::api::schema::PaneRepositionParams {
                source_pane_id: "pane_1".into(),
                target_pane_id: "target".into(),
                tab_id: "tab_1".into(),
                side: crate::api::schema::PaneRepositionSide::Left,
            }),
            "pane_1".into(),
            expected_tab_id.map(str::to_owned),
            follow,
            &mut input,
        );
        let [ClientShellAction::Endpoint { request, .. }] = &input.actions[..] else {
            panic!("rearrangement request");
        };
        request.id.clone()
    }

    fn changed_reposition(changed: bool) -> crate::api::schema::ResponseResult {
        crate::api::schema::ResponseResult::PaneReposition {
            reposition: crate::api::schema::PaneRepositionResult {
                changed,
                reason: (!changed).then_some(crate::api::schema::PaneRepositionReason::Unchanged),
                source_pane_id: "pane_1".into(),
                target_pane_id: "target".into(),
                layout: layout("tab_1", "pane_1"),
            },
        }
    }

    fn changed_move() -> crate::api::schema::ResponseResult {
        crate::api::schema::ResponseResult::PaneMove {
            move_result: crate::api::schema::PaneMoveResult {
                changed: true,
                reason: None,
                previous_pane_id: "pane_1".into(),
                previous_workspace_id: "ws_1".into(),
                previous_tab_id: "tab_1".into(),
                pane: Box::new(moved_pane("tab_2")),
                source_layout: None,
                target_layout: Box::new(layout("tab_2", "pane_1")),
                created_workspace: None,
                created_tab: None,
                closed_workspace_id: None,
                closed_tab_id: None,
                focused_pane_id: "target".into(),
            },
        }
    }

    fn hit(id: &str, x: u16, y: u16) -> PaneHit {
        PaneHit {
            rect: Rect::new(x, y, 10, 5),
            inner_rect: Rect::new(x, y, 10, 5),
            scrollbar_rect: None,
            scroll: None,
            pane_id: id.to_owned(),
            layer: crate::client::shell::state::PaneHitLayer::Tiled,
            mouse_reporting: false,
            sgr_pixel_mouse: false,
            pixel_width: 0,
            pixel_height: 0,
        }
    }

    fn selector() -> ClientPaneTargetSelector {
        ClientPaneTargetSelector {
            boot_id: "boot".into(),
            source_pane_id: "source".into(),
            workspace_id: "workspace".into(),
            tab_id: "tab".into(),
            operation: ClientPaneTargetOperation::Swap,
            eligible_pane_ids: vec!["left".into(), "right".into(), "down".into()],
            surface_signature: 0,
            selected: None,
        }
    }

    #[test]
    fn selector_starts_without_target_and_cycles_explicitly() {
        let mut selector = selector();
        assert_eq!(selector.selected_pane_id(), None);
        selector.cycle(false);
        assert_eq!(selector.selected_pane_id(), Some("left"));
        selector.cycle(true);
        assert_eq!(selector.selected_pane_id(), Some("down"));
    }

    #[test]
    fn directional_selection_uses_geometry_without_changing_focus() {
        let hits = [
            hit("source", 20, 10),
            hit("left", 0, 10),
            hit("right", 40, 10),
            hit("down", 20, 20),
        ];
        let mut selector = selector();
        selector.select_direction(&hits, PaneNavigation::Right);
        assert_eq!(selector.selected_pane_id(), Some("right"));
        selector.select_direction(&hits, PaneNavigation::Down);
        assert_eq!(selector.selected_pane_id(), Some("down"));
    }

    #[test]
    fn explicit_swap_request_uses_captured_ids() {
        let mut selector = selector();
        selector.select_pane("right");
        assert!(matches!(
            selector.method(),
            Some(crate::api::schema::Method::PaneSwap(crate::api::schema::PaneSwapParams {
                source_pane_id: Some(ref source),
                target_pane_id: Some(ref target),
                ..
            })) if source == "source" && target == "right"
        ));
    }

    #[test]
    fn confirmed_reposition_follows_captured_source() {
        let mut state = completion_state();
        let request_id = enqueue_follow(
            &mut state,
            PaneRearrangementFollow::Reposition,
            Some("tab_1"),
        );
        let (_, actions) =
            state.handle_endpoint_result("boot-1", &request_id, Ok(changed_reposition(true)));
        assert!(matches!(
            &actions[..],
            [ClientShellAction::Endpoint { request, .. }]
                if matches!(&request.method, crate::api::schema::Method::PaneFocus(target) if target.pane_id == "pane_1")
        ));
    }

    #[test]
    fn confirmed_move_follows_returned_tab() {
        let mut state = completion_state();
        let request_id = enqueue_follow(&mut state, PaneRearrangementFollow::MoveNewTab, None);
        let (_, actions) = state.handle_endpoint_result("boot-1", &request_id, Ok(changed_move()));
        assert!(matches!(
            &actions[..],
            [ClientShellAction::Endpoint { request, .. }]
                if matches!(&request.method, crate::api::schema::Method::TabFocus(target) if target.tab_id == "tab_2")
        ));
    }

    #[test]
    fn newer_snapshot_membership_prevents_stale_follow_up() {
        let mut reposition = completion_state();
        let reposition_id = enqueue_follow(
            &mut reposition,
            PaneRearrangementFollow::Reposition,
            Some("tab_1"),
        );
        let mut newer = reposition.snapshot.as_deref().unwrap().clone();
        newer.revision += 1;
        newer.panes.retain(|pane| pane.pane_id != "pane_1");
        reposition.set_snapshot(Box::new(newer));
        assert!(reposition
            .handle_endpoint_result("boot-1", &reposition_id, Ok(changed_reposition(true)),)
            .1
            .is_empty());

        let mut moved = completion_state();
        let move_id = enqueue_follow(&mut moved, PaneRearrangementFollow::MoveNewTab, None);
        let mut newer = moved.snapshot.as_deref().unwrap().clone();
        newer.revision += 1;
        newer.panes.retain(|pane| pane.pane_id != "pane_1");
        moved.set_snapshot(Box::new(newer));
        assert!(moved
            .handle_endpoint_result("boot-1", &move_id, Ok(changed_move()))
            .1
            .is_empty());
    }

    #[test]
    fn later_focus_bearing_endpoint_action_revokes_rearrangement_follow() {
        let mut state = completion_state();
        let request_id = enqueue_follow(
            &mut state,
            PaneRearrangementFollow::Reposition,
            Some("tab_1"),
        );
        let mut input = ClientShellInput::default();
        state.push_endpoint_method(
            crate::api::schema::Method::TabCreate(crate::api::schema::TabCreateParams {
                workspace_id: Some("ws_1".into()),
                cwd: None,
                focus: true,
                label: None,
                env: Default::default(),
            }),
            &mut input,
        );
        assert_eq!(input.actions.len(), 1);
        assert!(state
            .handle_endpoint_result("boot-1", &request_id, Ok(changed_reposition(true)))
            .1
            .is_empty());
    }

    #[test]
    fn noop_error_late_and_navigation_change_never_follow() {
        let mut state = completion_state();
        let noop = enqueue_follow(
            &mut state,
            PaneRearrangementFollow::Reposition,
            Some("tab_1"),
        );
        assert!(state
            .handle_endpoint_result("boot-1", &noop, Ok(changed_reposition(false)))
            .1
            .is_empty());

        let error = enqueue_follow(
            &mut state,
            PaneRearrangementFollow::Reposition,
            Some("tab_1"),
        );
        assert!(state
            .handle_endpoint_result(
                "boot-1",
                &error,
                Err(ClientShellEndpointError {
                    code: Some("rejected".into()),
                    message: "rejected".into(),
                }),
            )
            .1
            .is_empty());

        let navigated = enqueue_follow(
            &mut state,
            PaneRearrangementFollow::Reposition,
            Some("tab_1"),
        );
        state.revoke_rearrangement_follow();
        assert!(state
            .handle_endpoint_result("boot-1", &navigated, Ok(changed_reposition(true)))
            .1
            .is_empty());

        let late = enqueue_follow(
            &mut state,
            PaneRearrangementFollow::Reposition,
            Some("tab_1"),
        );
        assert!(state
            .handle_endpoint_result(
                "boot-1",
                &late,
                Err(ClientShellEndpointError {
                    code: Some("endpoint_timeout".into()),
                    message: "timeout".into(),
                }),
            )
            .1
            .is_empty());
        assert!(state.handle_late_endpoint_result("boot-1", &late, Ok(changed_reposition(true)),));
        assert!(state.pending_requests.is_empty());
    }
}
