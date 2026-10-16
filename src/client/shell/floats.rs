//! Floating terminals in the client shell: composition, focus, input routing, and drag-to-move.
//!
//! The server owns float existence, geometry, visibility, and z-order. Focus and an in-flight
//! drag are client-local presentation state.

use super::*;
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

use crate::protocol::floats::{FloatSurface, FloatsSurface};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ClientFloatDrag {
    pub(super) float_id: String,
    /// Pointer offset from the float's outer top-left cell.
    pub(super) grab: (u16, u16),
    /// Previewed outer top-left, relative to the pane surface.
    pub(super) position: (u16, u16),
    /// The button is up and `float.move` is in flight; the preview holds until a layer confirms it.
    pub(super) released: bool,
}

#[derive(Clone)]
pub(super) struct FloatHit {
    pub(super) outer: Rect,
    pub(super) pane: PaneHit,
}

#[derive(Default)]
pub(super) struct ClientFloats {
    pub(super) layer: FloatsSurface,
    pub(super) focused: Option<String>,
    pub(super) drag: Option<ClientFloatDrag>,
}

impl ClientFloats {
    fn get(&self, float_id: &str) -> Option<&FloatSurface> {
        self.layer
            .floats
            .iter()
            .find(|float| float.float_id == float_id)
    }

    fn is_top(&self, float_id: &str) -> bool {
        self.layer
            .floats
            .last()
            .is_some_and(|float| float.float_id == float_id)
    }

    /// Outer rect relative to the pane surface, with an in-flight drag applied.
    fn outer(&self, float: &FloatSurface) -> Rect {
        let (x, y) = self
            .drag
            .as_ref()
            .filter(|drag| drag.float_id == float.float_id)
            .map_or((float.outer.x, float.outer.y), |drag| drag.position);
        Rect::new(x, y, float.outer.width, float.outer.height)
    }
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

/// Draws floats bottom to top over the tiled surface and returns their hit targets.
pub(super) fn compose_floats(
    floats: &ClientFloats,
    focused: Option<&str>,
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
        let inner = Rect::new(
            outer.x.saturating_add(1),
            outer.y.saturating_add(1),
            float.frame.width,
            float.frame.height,
        )
        .intersection(outer);
        let is_focused = focused == Some(float.float_id.as_str());
        occlusion.cover(outer);
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
            outer,
            pane: PaneHit {
                rect: outer,
                inner_rect: inner,
                scrollbar_rect: None,
                scroll: None,
                pane_id: float.float_id.clone(),
                layer: PaneHitLayer::Float,
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
        let present = |id: &str| layer.floats.iter().any(|float| float.float_id == id);
        if let Some(focused) = self.floats.focused.clone() {
            if !present(&focused) {
                self.floats.focused = None;
                self.input_leases
                    .remove_target(&ClientInputTarget::Float(focused));
            }
        }
        if let Some(drag) = self.floats.drag.as_ref() {
            let committed = drag.released
                && layer.floats.iter().any(|float| {
                    float.float_id == drag.float_id
                        && (float.outer.x, float.outer.y) == drag.position
                });
            if !present(&drag.float_id) || committed {
                self.floats.drag = None;
            }
        }
        if self.pane_mouse_gesture.as_ref().is_some_and(|gesture| {
            gesture.hit.layer == PaneHitLayer::Float && !present(&gesture.hit.pane_id)
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
    pub(super) fn focused_float_id(&self) -> Option<&str> {
        if self.popup_terminal_id.is_some() || self.popup_pending {
            return None;
        }
        self.floats.focused.as_deref()
    }

    /// Whether a pane patch can be presented without recomposing floats over it.
    pub(super) fn floats_block_patch(&self, patch: &crate::protocol::PaneSurfacePatch) -> bool {
        if self.focused_float_id().is_some() {
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
        if self.floats.drag.as_ref().is_some_and(|drag| !drag.released) {
            match mouse.kind {
                MouseEventKind::Drag(MouseButton::Left) => {
                    self.drag_float_to(point);
                    outcome.repaint = true;
                    return true;
                }
                MouseEventKind::Up(MouseButton::Left) => {
                    self.drag_float_to(point);
                    self.commit_float_drag(outcome);
                    outcome.repaint = true;
                    return true;
                }
                _ => {}
            }
        }
        if self.overlay.is_some() {
            return false;
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
                self.blur_float();
                outcome.repaint = true;
            }
            return false;
        };
        let in_content = super::contains(hit.pane.inner_rect, point);
        match mouse.kind {
            MouseEventKind::Down(button) => {
                self.focus_float(&hit.pane.pane_id, outcome);
                if !in_content {
                    if button == MouseButton::Left {
                        let surface = self.float_pane_surface();
                        self.floats.drag = Some(ClientFloatDrag {
                            float_id: hit.pane.pane_id.clone(),
                            grab: (
                                point.0.saturating_sub(hit.outer.x),
                                point.1.saturating_sub(hit.outer.y),
                            ),
                            position: (
                                hit.outer.x.saturating_sub(surface.x),
                                hit.outer.y.saturating_sub(surface.y),
                            ),
                            released: false,
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
        let Some(size) = self
            .floats
            .get(&drag.float_id)
            .map(|float| (float.outer.width, float.outer.height))
        else {
            self.floats.drag = None;
            return;
        };
        let wanted = (
            i32::from(point.0) - i32::from(surface.x) - i32::from(drag.grab.0),
            i32::from(point.1) - i32::from(surface.y) - i32::from(drag.grab.1),
        );
        let position = clamp_drag_position(surface, size, wanted);
        if let Some(drag) = self.floats.drag.as_mut() {
            drag.position = position;
        }
    }

    /// Sends the previewed position; the drag preview stays until the server confirms it.
    fn commit_float_drag(&mut self, outcome: &mut ClientShellInput) {
        let Some(drag) = self.floats.drag.clone() else {
            return;
        };
        let unchanged = self
            .floats
            .get(&drag.float_id)
            .is_some_and(|float| (float.outer.x, float.outer.y) == drag.position);
        if unchanged {
            self.floats.drag = None;
            return;
        }
        if let Some(drag) = self.floats.drag.as_mut() {
            drag.released = true;
        }
        self.push_endpoint_method(
            crate::api::schema::Method::FloatMove(crate::api::schema::FloatMoveParams {
                float_id: drag.float_id,
                x: drag.position.0,
                y: drag.position.1,
            }),
            outcome,
        );
    }

    fn focus_float(&mut self, float_id: &str, outcome: &mut ClientShellInput) {
        if self.floats.focused.as_deref() != Some(float_id) {
            self.release_input_leases(outcome);
            self.floats.focused = Some(float_id.to_owned());
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
                    float_id: float_id.to_owned(),
                }),
                outcome,
            );
            outcome.repaint = true;
        }
        self.reconcile_input_source();
    }

    /// Returns typed input to the tiled layout.
    pub(super) fn blur_float(&mut self) {
        if let Some(focused) = self.floats.focused.take() {
            self.input_leases
                .remove_target(&ClientInputTarget::Float(focused));
            self.reconcile_input_source();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::clamp_drag_position;
    use ratatui::layout::Rect;

    #[test]
    fn drag_position_stays_inside_the_pane_surface() {
        let surface = Rect::new(20, 1, 60, 20);

        assert_eq!(clamp_drag_position(surface, (10, 5), (-4, -9)), (0, 0));
        assert_eq!(clamp_drag_position(surface, (10, 5), (12, 3)), (12, 3));
        assert_eq!(clamp_drag_position(surface, (10, 5), (99, 99)), (50, 15));
    }
}
