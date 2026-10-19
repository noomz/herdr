//! Floating terminals in the client shell: composition, focus, input routing, and dragging to
//! move or resize.
//!
//! The server owns float existence, geometry, visibility, and z-order. Focus and an in-flight
//! drag are client-local presentation state.

use super::*;
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

use crate::protocol::floats::{FloatId, FloatSurface, FloatsSurface};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ClientFloatDrag {
    pub(super) float_id: FloatId,
    pub(super) gesture: ClientFloatGesture,
    /// Previewed outer rect, relative to the pane surface.
    pub(super) rect: Rect,
    pub(super) phase: ClientFloatDragPhase,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ClientFloatGesture {
    /// Pointer offset from the float's outer top-left cell.
    Move { grab: (u16, u16) },
    /// The outer top-left stays put while the flagged edges follow the pointer.
    Resize { width: bool, height: bool },
}

impl ClientFloatGesture {
    /// The title row and left border move the float; the right and bottom borders resize it.
    fn for_border(outer: Rect, point: (u16, u16)) -> Self {
        let right = point.0 == outer.right().saturating_sub(1);
        let bottom = point.1 == outer.bottom().saturating_sub(1);
        if point.1 == outer.y || !(right || bottom) {
            return Self::Move {
                grab: (
                    point.0.saturating_sub(outer.x),
                    point.1.saturating_sub(outer.y),
                ),
            };
        }
        Self::Resize {
            width: right,
            height: bottom,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ClientFloatDragPhase {
    Dragging,
    /// The button is up and `float.move` or `float.resize` was sent. The preview holds until the
    /// server's float geometry moves off `origin`, or until the first layer after the request
    /// is answered.
    Committing {
        origin: (u16, u16, u16, u16),
        answered: bool,
    },
}

fn outer_geometry(float: &FloatSurface) -> (u16, u16, u16, u16) {
    (
        float.outer.x,
        float.outer.y,
        float.outer.width,
        float.outer.height,
    )
}

#[derive(Clone)]
pub(super) struct FloatHit {
    pub(super) float_id: FloatId,
    pub(super) outer: Rect,
    pub(super) pane: PaneHit,
}

#[derive(Default)]
pub(super) struct ClientFloats {
    pub(super) layer: FloatsSurface,
    pub(super) focused: Option<FloatId>,
    pub(super) drag: Option<ClientFloatDrag>,
    /// A button went down on a float; its Drag and Up belong to the float.
    pub(super) pressed: bool,
}

impl ClientFloats {
    fn get(&self, float_id: FloatId) -> Option<&FloatSurface> {
        self.layer
            .floats
            .iter()
            .find(|float| float.float_id == float_id)
    }

    fn is_top(&self, float_id: FloatId) -> bool {
        self.layer
            .floats
            .last()
            .is_some_and(|float| float.float_id == float_id)
    }

    /// Outer rect relative to the pane surface, with an in-flight drag applied.
    fn outer(&self, float: &FloatSurface) -> Rect {
        self.drag
            .as_ref()
            .filter(|drag| drag.float_id == float.float_id)
            .map_or_else(|| float_rect(float), |drag| drag.rect)
    }
}

fn float_rect(float: &FloatSurface) -> Rect {
    let (x, y, width, height) = outer_geometry(float);
    Rect::new(x, y, width, height)
}

/// Clamps a dragged outer top-left so the float stays inside the pane surface.
fn clamp_drag_position(surface: Rect, size: (u16, u16), wanted: (i32, i32)) -> (u16, u16) {
    let max_x = i32::from(surface.width.saturating_sub(size.0));
    let max_y = i32::from(surface.height.saturating_sub(size.1));
    (
        wanted.0.clamp(0, max_x) as u16,
        wanted.1.clamp(0, max_y) as u16,
    )
}

/// Clamps a dragged outer size between the float minimum and the pane surface edge.
fn clamp_resize(surface: Rect, position: (u16, u16), wanted: (i32, i32)) -> (u16, u16) {
    let (min_width, min_height) = crate::popup_size::MIN_FLOAT_OUTER;
    let max_width = i32::from(surface.width.saturating_sub(position.0));
    let max_height = i32::from(surface.height.saturating_sub(position.1));
    (
        wanted.0.max(i32::from(min_width)).min(max_width) as u16,
        wanted.1.max(i32::from(min_height)).min(max_height) as u16,
    )
}

/// Draws floats bottom to top over the tiled surface and returns their hit targets.
pub(super) fn compose_floats(
    floats: &ClientFloats,
    focused: Option<FloatId>,
    palette: &Palette,
    frame: &mut FrameData,
    pane_surface: Rect,
    occlusion: &mut crate::kitty_graphics::surface::Occlusion,
) -> Option<Vec<FloatHit>> {
    let mut hits = Vec::with_capacity(floats.layer.floats.len());
    let mut cursor = frame.cursor.clone();
    for float in &floats.layer.floats {
        let relative = floats.outer(float);
        let outer = Rect::new(
            pane_surface.x.saturating_add(relative.x),
            pane_surface.y.saturating_add(relative.y),
            relative.width,
            relative.height,
        )
        .intersection(pane_surface);
        if outer.is_empty() {
            continue;
        }
        // A resize preview can shrink the border inside the last frame the server sent.
        let interior = Rect::new(
            outer.x.saturating_add(1),
            outer.y.saturating_add(1),
            outer.width.saturating_sub(2),
            outer.height.saturating_sub(2),
        );
        let inner = Rect::new(
            interior.x,
            interior.y,
            float.frame.width,
            float.frame.height,
        )
        .intersection(interior);
        let is_focused = focused == Some(float.float_id);
        occlusion.start_terminal(&float.terminal_id, outer, (inner.x, inner.y));
        let mut composed = frame.to_ratatui_buffer()?;
        let border_color = if is_focused {
            palette.accent
        } else {
            palette.overlay0
        };
        let block = ratatui::widgets::Block::default()
            .borders(ratatui::widgets::Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .title(float.title.clone())
            .style(Style::default().bg(palette.panel_bg));
        ratatui::widgets::Widget::render(ratatui::widgets::Clear, outer, &mut composed);
        ratatui::widgets::Widget::render(block, outer, &mut composed);
        frame.replace_from_ratatui_buffer_preserving_effects(&composed, None);
        blit_pane_surface(frame, &float.frame, inner);
        if is_focused {
            cursor = frame.cursor.clone();
        } else if cursor
            .as_ref()
            .is_some_and(|cursor| super::contains(outer, (cursor.x, cursor.y)))
        {
            cursor = None;
        }
        hits.push(FloatHit {
            float_id: float.float_id,
            outer,
            pane: PaneHit {
                rect: outer,
                inner_rect: inner,
                scrollbar_rect: None,
                scroll: None,
                pane_id: float.terminal_id.clone(),
                layer: PaneHitLayer::Float(float.float_id),
                mouse_reporting: float.mouse_reporting,
                sgr_pixel_mouse: float.sgr_pixel_mouse,
                pixel_width: float.pixel_width,
                pixel_height: float.pixel_height,
            },
        });
    }
    frame.cursor = cursor;
    Some(hits)
}

impl ClientShellState {
    pub(crate) fn set_float_layer(&mut self, layer: FloatsSurface) {
        let present = |id: FloatId| layer.floats.iter().any(|float| float.float_id == id);
        if let Some(focused) = self.floats.focused {
            if !present(focused) {
                self.floats.focused = None;
                self.input_leases
                    .remove_target(&ClientInputTarget::Float(focused));
            }
        }
        if let Some(drag) = self.floats.drag.as_ref() {
            let server = layer
                .floats
                .iter()
                .find(|float| float.float_id == drag.float_id)
                .map(outer_geometry);
            let settled = match (server, drag.phase) {
                (None, _) => true,
                (Some(_), ClientFloatDragPhase::Dragging) => false,
                (Some(_), ClientFloatDragPhase::Committing { answered: true, .. }) => true,
                (Some(server), ClientFloatDragPhase::Committing { origin, .. }) => server != origin,
            };
            if settled {
                self.floats.drag = None;
            }
        }
        if self.pane_mouse_gesture.as_ref().is_some_and(|gesture| {
            matches!(gesture.hit.layer, PaneHitLayer::Float(float_id) if !present(float_id))
        }) {
            self.pane_mouse_gesture = None;
        }
        self.floats.layer = layer;
        self.reconcile_input_source();
    }

    pub(super) fn clear_float_projection(&mut self) {
        self.floats = ClientFloats::default();
        self.hits.floats.clear();
    }

    /// The float that receives typed input, unless the modal popup owns input.
    pub(super) fn focused_float_id(&self) -> Option<FloatId> {
        if self.popup_terminal_id.is_some() || self.popup_pending {
            return None;
        }
        self.floats.focused
    }

    /// The cursor the frame shows while a float has focus. `None` when no float has focus or
    /// the focused float has not been composed yet.
    pub(super) fn focused_float_cursor(&self) -> Option<Option<crate::protocol::CursorState>> {
        let focused = self.focused_float_id()?;
        let float = self.floats.get(focused)?;
        let hit = self
            .hits
            .floats
            .iter()
            .find(|hit| hit.pane.layer == PaneHitLayer::Float(focused))?;
        Some(super::blitted_cursor(&float.frame, hit.pane.inner_rect))
    }

    /// Whether a pane patch can be presented without recomposing floats over it.
    pub(super) fn floats_block_patch(&self, patch: &crate::protocol::PaneSurfacePatch) -> bool {
        if self.focused_float_id().is_some() && self.focused_float_cursor().is_none() {
            return true;
        }
        self.floats.layer.floats.iter().any(|float| {
            let outer = self.floats.outer(float);
            patch.rows.iter().any(|row| {
                let width = row.cells.len().min(u16::MAX as usize) as u16;
                row.y >= outer.y
                    && row.y < outer.bottom()
                    && row.x < outer.right()
                    && row.x.saturating_add(width) > outer.x
            })
        })
    }

    /// Handles mouse input aimed at floats. Returns true when the event was consumed.
    pub(super) fn handle_float_mouse(
        &mut self,
        mouse: MouseEvent,
        outcome: &mut ClientShellInput,
    ) -> bool {
        let point = (mouse.column, mouse.row);
        if self
            .floats
            .drag
            .as_ref()
            .is_some_and(|drag| drag.phase == ClientFloatDragPhase::Dragging)
        {
            match mouse.kind {
                MouseEventKind::Drag(MouseButton::Left) => {
                    self.drag_float_to(point);
                    outcome.repaint = true;
                    return true;
                }
                MouseEventKind::Up(MouseButton::Left) => {
                    self.floats.pressed = false;
                    self.drag_float_to(point);
                    self.commit_float_drag(outcome);
                    outcome.repaint = true;
                    return true;
                }
                _ => {}
            }
        }
        if matches!(mouse.kind, MouseEventKind::Down(_)) {
            self.floats.pressed = false;
        }
        if self.overlay.is_some() {
            return false;
        }
        if matches!(mouse.kind, MouseEventKind::Drag(_) | MouseEventKind::Up(_)) {
            let pressed = self.floats.pressed;
            if matches!(mouse.kind, MouseEventKind::Up(_)) {
                self.floats.pressed = false;
            }
            return pressed;
        }
        let Some(hit) = self
            .hits
            .floats
            .iter()
            .rev()
            .find(|hit| super::contains(hit.outer, point))
            .cloned()
        else {
            if matches!(mouse.kind, MouseEventKind::Down(_)) && self.floats.focused.is_some() {
                self.blur_float(outcome);
                outcome.repaint = true;
            }
            return false;
        };
        let in_content = super::contains(hit.pane.inner_rect, point);
        match mouse.kind {
            MouseEventKind::Down(button) => {
                self.floats.pressed = true;
                self.focus_float(hit.float_id, outcome);
                if !in_content {
                    if button == MouseButton::Left {
                        let surface = self.float_pane_surface();
                        self.floats.drag = Some(ClientFloatDrag {
                            float_id: hit.float_id,
                            gesture: ClientFloatGesture::for_border(hit.outer, point),
                            rect: Rect::new(
                                hit.outer.x.saturating_sub(surface.x),
                                hit.outer.y.saturating_sub(surface.y),
                                hit.outer.width,
                                hit.outer.height,
                            ),
                            phase: ClientFloatDragPhase::Dragging,
                        });
                    }
                    return true;
                }
                self.push_pane_mouse_event(&hit.pane, mouse, mouse.modifiers, outcome);
                if hit.pane.mouse_reporting {
                    self.pane_mouse_gesture = Some(ClientPaneMouseGesture {
                        last_position: self.pane_mouse_position(&hit.pane, mouse),
                        hit: hit.pane,
                        button,
                        stripped_modifiers: crossterm::event::KeyModifiers::empty(),
                        last_event: mouse,
                    });
                }
            }
            MouseEventKind::Moved if in_content && hit.pane.mouse_reporting => {
                self.push_pane_mouse_event(&hit.pane, mouse, mouse.modifiers, outcome);
            }
            MouseEventKind::ScrollUp
            | MouseEventKind::ScrollDown
            | MouseEventKind::ScrollLeft
            | MouseEventKind::ScrollRight
                if in_content =>
            {
                self.push_pane_mouse_event(&hit.pane, mouse, mouse.modifiers, outcome);
            }
            _ => {}
        }
        true
    }

    fn float_pane_surface(&self) -> Rect {
        let (cols, rows) = self.last_composed_size.unwrap_or_default();
        self.layout(cols, rows).pane_surface
    }

    fn drag_float_to(&mut self, point: (u16, u16)) {
        let surface = self.float_pane_surface();
        let Some(drag) = self.floats.drag.as_ref() else {
            return;
        };
        if self.floats.get(drag.float_id).is_none() {
            self.floats.drag = None;
            return;
        }
        let mut rect = drag.rect;
        let pointer = (
            i32::from(point.0) - i32::from(surface.x),
            i32::from(point.1) - i32::from(surface.y),
        );
        match drag.gesture {
            ClientFloatGesture::Move { grab } => {
                let wanted = (pointer.0 - i32::from(grab.0), pointer.1 - i32::from(grab.1));
                (rect.x, rect.y) = clamp_drag_position(surface, (rect.width, rect.height), wanted);
            }
            ClientFloatGesture::Resize { width, height } => {
                let wanted = (
                    pointer.0 - i32::from(rect.x) + 1,
                    pointer.1 - i32::from(rect.y) + 1,
                );
                let size = clamp_resize(surface, (rect.x, rect.y), wanted);
                if width {
                    rect.width = size.0;
                }
                if height {
                    rect.height = size.1;
                }
            }
        }
        if let Some(drag) = self.floats.drag.as_mut() {
            drag.rect = rect;
        }
    }

    /// Sends the previewed geometry; the drag preview stays until the server confirms it.
    fn commit_float_drag(&mut self, outcome: &mut ClientShellInput) {
        use crate::api::schema::{FloatMoveParams, FloatResizeParams, Method};

        let Some(drag) = self.floats.drag.clone() else {
            return;
        };
        let Some(origin) = self.floats.get(drag.float_id).map(outer_geometry) else {
            self.floats.drag = None;
            return;
        };
        let rect = drag.rect;
        if origin == (rect.x, rect.y, rect.width, rect.height) {
            self.floats.drag = None;
            return;
        }
        if let Some(drag) = self.floats.drag.as_mut() {
            drag.phase = ClientFloatDragPhase::Committing {
                origin,
                answered: false,
            };
        }
        let float_id = drag.float_id.to_string();
        let pin = Method::FloatMove(FloatMoveParams {
            float_id: float_id.clone(),
            x: rect.x,
            y: rect.y,
        });
        let method = match drag.gesture {
            ClientFloatGesture::Move { .. } => pin,
            ClientFloatGesture::Resize { .. } => {
                let resize = Method::FloatResize(FloatResizeParams {
                    float_id,
                    width: Some(crate::popup_size::PopupSize::Cells(rect.width)),
                    height: Some(crate::popup_size::PopupSize::Cells(rect.height)),
                });
                // A centered float recenters when resized, so pin its top-left first.
                if self.supports_endpoint_method(&resize) {
                    self.push_endpoint_method(pin, outcome);
                }
                resize
            }
        };
        let sent = self.push_endpoint_method_with_kind(
            method,
            PendingEndpointKind::FloatGeometry {
                float_id: drag.float_id,
            },
            outcome,
        );
        if !sent {
            self.floats.drag = None;
        }
    }

    /// A failed move or resize reverts the preview to the server geometry; a successful one
    /// lets the next layer settle it.
    pub(super) fn complete_float_geometry(&mut self, float_id: FloatId, succeeded: bool) -> bool {
        let Some(drag) = self
            .floats
            .drag
            .as_mut()
            .filter(|drag| drag.float_id == float_id)
        else {
            return false;
        };
        let ClientFloatDragPhase::Committing { answered, .. } = &mut drag.phase else {
            return false;
        };
        if succeeded {
            *answered = true;
            false
        } else {
            self.floats.drag = None;
            true
        }
    }

    fn focus_float(&mut self, float_id: FloatId, outcome: &mut ClientShellInput) {
        if self.floats.focused != Some(float_id) {
            self.release_input_leases(outcome);
            self.floats.focused = Some(float_id);
            self.selection = None;
            self.stop_selection_autoscroll();
            outcome.repaint = true;
        }
        if !self.floats.is_top(float_id) {
            if let Some(index) = self
                .floats
                .layer
                .floats
                .iter()
                .position(|float| float.float_id == float_id)
            {
                let float = self.floats.layer.floats.remove(index);
                self.floats.layer.floats.push(float);
            }
            self.push_endpoint_method(
                crate::api::schema::Method::FloatRaise(crate::api::schema::FloatTarget {
                    float_id: float_id.to_string(),
                }),
                outcome,
            );
            outcome.repaint = true;
        }
        self.reconcile_input_source();
    }

    /// Returns typed input to the tiled layout and releases keys the float saw pressed.
    pub(super) fn blur_float(&mut self, outcome: &mut ClientShellInput) {
        if let Some(focused) = self.floats.focused.take() {
            let leases = self
                .input_leases
                .remove_target(&ClientInputTarget::Float(focused));
            self.push_lease_releases(leases, outcome);
            self.reconcile_input_source();
        }
    }

    /// Returns typed input to the tiled layout when the server moves pane focus. Held keys keep
    /// their lease, so the host's own release still reaches the float that saw the press.
    pub(super) fn yield_float_focus(&mut self) {
        if self.floats.focused.take().is_some() {
            self.reconcile_input_source();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{clamp_drag_position, clamp_resize};
    use ratatui::layout::Rect;

    #[test]
    fn drag_position_stays_inside_the_pane_surface() {
        let surface = Rect::new(20, 1, 60, 20);

        assert_eq!(clamp_drag_position(surface, (10, 5), (-4, -9)), (0, 0));
        assert_eq!(clamp_drag_position(surface, (10, 5), (12, 3)), (12, 3));
        assert_eq!(clamp_drag_position(surface, (10, 5), (99, 99)), (50, 15));
    }

    #[test]
    fn resized_size_stays_between_the_minimum_and_the_pane_surface_edge() {
        let surface = Rect::new(20, 1, 60, 20);

        assert_eq!(clamp_resize(surface, (10, 5), (-3, 1)), (6, 4));
        assert_eq!(clamp_resize(surface, (10, 5), (20, 8)), (20, 8));
        assert_eq!(clamp_resize(surface, (10, 5), (99, 99)), (50, 15));
    }
}
