//! Floating terminals in the client shell: composition, focus, input routing, and dragging to
//! move or resize.
//!
//! The server owns float existence, geometry, visibility, and z-order. Focus and an in-flight
//! drag are client-local presentation state.

use super::*;
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

use crate::client::host_pointer::HostPointerShape;
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

    fn pointer_shape(self) -> HostPointerShape {
        match self {
            Self::Move { .. } => HostPointerShape::Move,
            Self::Resize {
                width: true,
                height: true,
            } => HostPointerShape::NwseResize,
            Self::Resize { width: true, .. } => HostPointerShape::EwResize,
            Self::Resize { .. } => HostPointerShape::NsResize,
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
    /// Rendered buttons as `(button id, hit rectangle)` in declaration order. The same
    /// layout drove the drawing, so hit testing cannot drift from what is on screen.
    pub(super) buttons: Vec<(String, Rect)>,
}

/// One rendered title-bar button. Declaration order is display order, left to right.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RenderedFloatButton {
    pub(super) id: String,
    pub(super) icon: String,
    pub(super) rect: Rect,
}

/// Lays out `buttons` inside the top border of `outer`, right-aligned as one group with
/// no gaps. Each button takes its icon width plus one blank cell on each side. Only the
/// largest declaration-order prefix that fits is rendered; a button is never drawn or
/// hit-tested partially. Returns the rendered buttons and the cell budget left for the
/// title. Pure: no terminal state, no registry, no I/O.
pub(super) fn layout_float_buttons(
    outer: Rect,
    buttons: &[crate::api::schema::PluginManifestButton],
) -> (Vec<RenderedFloatButton>, u16) {
    let interior = outer.width.saturating_sub(2);
    let mut widths = Vec::with_capacity(buttons.len());
    let mut used = 0u16;
    for button in buttons {
        let width = u16::try_from(unicode_width::UnicodeWidthStr::width(button.icon.as_str()))
            .unwrap_or(u16::MAX);
        let total = width.saturating_add(2);
        if total > interior.saturating_sub(used) {
            break;
        }
        used = used.saturating_add(total);
        widths.push(total);
    }
    if widths.is_empty() {
        return (Vec::new(), interior);
    }
    // The group's half-open right edge sits on the right border column, so the last
    // interior cell belongs to the last button's right padding.
    let group_end = outer.x.saturating_add(outer.width.saturating_sub(1));
    let group_start = group_end.saturating_sub(used);
    let mut rendered = Vec::with_capacity(widths.len());
    let mut cell = group_start;
    for (button, width) in buttons.iter().zip(widths) {
        rendered.push(RenderedFloatButton {
            id: button.id.clone(),
            icon: button.icon.clone(),
            rect: Rect::new(cell, outer.y, width, 1),
        });
        cell = cell.saturating_add(width);
    }
    (rendered, interior - used)
}

/// Truncates `text` to at most `max_cells` terminal cells without splitting a grapheme
/// cluster. A cluster that does not fit ends the title.
fn truncate_to_cells(text: &str, max_cells: u16) -> String {
    let mut truncated = String::with_capacity(text.len());
    let mut used = 0u16;
    for cluster in unicode_segmentation::UnicodeSegmentation::graphemes(text, true) {
        let Ok(width) = u16::try_from(unicode_width::UnicodeWidthStr::width(cluster)) else {
            break;
        };
        if used.saturating_add(width) > max_cells {
            break;
        }
        truncated.push_str(cluster);
        used = used.saturating_add(width);
    }
    truncated
}

/// A left press on a float title-bar button. Separate from the move/resize drag state:
/// activation happens on release inside the same rectangle, and a cancelled press keeps
/// consuming pointer events until its release so nothing reaches the pane beneath.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ClientFloatButtonPress {
    pub(super) float_id: FloatId,
    pub(super) button_id: String,
    /// Button rectangle at press time, relative to the pane surface.
    pub(super) rect: Rect,
    pub(super) state: ClientFloatButtonPressState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ClientFloatButtonPressState {
    Armed,
    /// Cancelled for this press; its release is still consumed.
    Cancelled,
}

#[derive(Default)]
pub(super) struct ClientFloats {
    pub(super) layer: FloatsSurface,
    pub(super) buttons: Option<crate::protocol::endpoint::EndpointFloatButtons>,
    pub(super) buttons_revision: Option<(String, u64)>,
    pub(super) pending_buttons: std::collections::HashMap<String, String>,
    pub(super) focused: Option<FloatId>,
    pub(super) pending_float_focus: Option<FloatId>,
    pub(super) drag: Option<ClientFloatDrag>,
    /// A button went down on a float; its Drag and Up belong to the float.
    pub(super) pressed: bool,
    pub(super) button_press: Option<ClientFloatButtonPress>,
}

impl ClientFloats {
    /// Cancels an armed button press while keeping it as the gesture owner until its
    /// release. Every geometry, stacking, revision, or availability change calls this.
    pub(super) fn cancel_button_press(&mut self) {
        if let Some(press) = self.button_press.as_mut() {
            press.state = ClientFloatButtonPressState::Cancelled;
        }
    }
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

/// The rendered buttons of one float inside its clipped outer rect, or an empty list
/// when the float has no negotiated controls.
fn float_button_layout(
    floats: &ClientFloats,
    float_id: FloatId,
    outer: Rect,
) -> Vec<RenderedFloatButton> {
    floats
        .buttons
        .as_ref()
        .and_then(|buttons| {
            buttons
                .floats
                .iter()
                .find(|set| set.float_id == float_id)
                .map(|set| layout_float_buttons(outer, &set.buttons).0)
        })
        .unwrap_or_default()
}

fn float_shadow_areas(outer: Rect, width: u16, height: u16) -> [Rect; 2] {
    [
        Rect::new(
            outer.right(),
            outer.y.saturating_add(height),
            width,
            outer.height,
        ),
        Rect::new(
            outer.x.saturating_add(width),
            outer.bottom(),
            outer.width,
            height,
        ),
    ]
}

/// Draws floats bottom to top over the tiled surface and returns their hit targets.
pub(super) fn compose_floats(
    floats: &ClientFloats,
    focused: Option<FloatId>,
    palette: &Palette,
    shadow: FloatShadowStyle,
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
        let buttons = float_button_layout(floats, float.float_id, outer);
        let title = if buttons.is_empty() {
            float.title.clone()
        } else {
            // The title starts inside the left border and must stop before the group.
            let budget = buttons.first().map_or(0, |first| {
                first.rect.x.saturating_sub(outer.x).saturating_sub(1)
            });
            truncate_to_cells(&float.title, budget)
        };
        let block = ratatui::widgets::Block::default()
            .borders(ratatui::widgets::Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .title(title)
            .style(Style::default().bg(palette.panel_bg));
        if shadow.enabled {
            let style = Style::default()
                .bg(shadow.color)
                .add_modifier(Modifier::DIM);
            for area in float_shadow_areas(outer, shadow.width, shadow.height) {
                composed.set_style(area.intersection(pane_surface), style);
            }
        }
        ratatui::widgets::Widget::render(ratatui::widgets::Clear, outer, &mut composed);
        ratatui::widgets::Widget::render(block, outer, &mut composed);
        for button in &buttons {
            let style = Style::default().fg(border_color);
            for cell in button.rect.x..button.rect.right() {
                composed[(cell, button.rect.y)]
                    .set_char(' ')
                    .set_style(style);
            }
            composed.set_string(
                button.rect.x.saturating_add(1),
                button.rect.y,
                &button.icon,
                style,
            );
        }
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
            buttons: buttons
                .iter()
                .map(|button| (button.id.clone(), button.rect))
                .collect(),
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
    pub(crate) fn set_float_layer(&mut self, layer: FloatsSurface) -> ClientShellInput {
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
        if let Some(metadata) = self.floats.buttons.as_mut() {
            metadata.floats.retain(|entry| present(entry.float_id));
        }
        if let Some(press) = self.floats.button_press.as_ref() {
            let geometry = |floats: &[FloatSurface]| {
                floats
                    .iter()
                    .position(|float| float.float_id == press.float_id)
                    .map(|index| (index, outer_geometry(&floats[index])))
            };
            // A geometry or stacking change invalidates the armed press.
            if geometry(&self.floats.layer.floats) != geometry(&layer.floats) {
                self.floats.cancel_button_press();
            }
        }
        self.floats.layer = layer;
        let mut outcome = ClientShellInput::default();
        if let Some(float_id) = self.floats.pending_float_focus.take() {
            if self.floats.get(float_id).is_some() {
                self.focus_float(float_id, &mut outcome);
            }
        }
        self.reconcile_input_source();
        outcome
    }

    pub(crate) fn set_float_buttons(
        &mut self,
        boot_id: &str,
        metadata: Option<crate::protocol::endpoint::EndpointFloatButtons>,
    ) {
        let Some(mut metadata) = metadata else {
            self.floats.buttons = None;
            return;
        };
        if metadata.boot_id != boot_id {
            return;
        }
        if self
            .floats
            .buttons_revision
            .as_ref()
            .is_some_and(|(boot, revision)| {
                boot == &metadata.boot_id && *revision >= metadata.revision
            })
        {
            return;
        }
        metadata.floats.retain(|entry| {
            self.floats
                .layer
                .floats
                .iter()
                .any(|float| float.float_id == entry.float_id)
        });
        self.floats.buttons_revision = Some((metadata.boot_id.clone(), metadata.revision));
        // New metadata invalidates an armed press: the gesture's binding may be gone.
        self.floats.cancel_button_press();
        self.floats.buttons = Some(metadata);
    }

    pub(crate) fn receive_float_button_result(
        &mut self,
        result: crate::protocol::endpoint::EndpointFloatButtonResult,
    ) -> bool {
        if self
            .snapshot
            .as_ref()
            .is_none_or(|snapshot| snapshot.boot_id != result.boot_id)
            || self.floats.pending_buttons.get(&result.log_id) != Some(&result.request_id)
            || result.status == crate::api::schema::PluginCommandStatus::Running
            || result.button_id.is_empty()
            || result.button_id.len() > 120
            || !result
                .button_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'_' | b'-'))
        {
            return false;
        }
        self.floats.pending_buttons.remove(&result.log_id);
        if result.status == crate::api::schema::PluginCommandStatus::Failed {
            let detail = result
                .error
                .map(|error| {
                    error
                        .chars()
                        .filter(|ch| !ch.is_control())
                        .take(512)
                        .collect()
                })
                .unwrap_or_else(|| format!("exit code {:?}", result.exit_code));
            return self.push_endpoint_notice(
                ClientEndpointNoticeKind::Rejected,
                format!("float-button:{}:{}", result.float_id, result.button_id),
                "Float action failed",
                detail,
            );
        }
        false
    }

    pub(crate) fn set_float_focus(
        &mut self,
        float_focus: crate::protocol::floats::FloatFocus,
    ) -> ClientShellInput {
        let mut outcome = ClientShellInput::default();
        self.floats.pending_float_focus = None;
        if self.floats.get(float_focus.float_id).is_some() {
            self.focus_float(float_focus.float_id, &mut outcome);
        } else {
            self.floats.pending_float_focus = Some(float_focus.float_id);
        }
        outcome
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
                let overlaps = |area: Rect| {
                    !area.is_empty()
                        && row.y >= area.y
                        && row.y < area.bottom()
                        && row.x < area.right()
                        && row.x.saturating_add(width) > area.x
                };
                overlaps(outer)
                    || (self.config.float_shadow.enabled
                        && float_shadow_areas(
                            outer,
                            self.config.float_shadow.width,
                            self.config.float_shadow.height,
                        )
                        .into_iter()
                        .any(overlaps))
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
            // A new press supersedes any armed button gesture.
            self.floats.button_press = None;
        }
        if let Some(press) = self.floats.button_press.clone() {
            return self.continue_float_button_press(press, mouse, outcome);
        }
        if !self.floats_own_point(point) {
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
                if let Some((button_id, rect)) = hit
                    .buttons
                    .iter()
                    .find(|(_, rect)| super::contains(*rect, point))
                {
                    // A press on a button is owned by the button: it neither focuses nor
                    // raises the float, starts no drag, and reaches no pane.
                    self.floats.pressed = true;
                    if button == MouseButton::Left {
                        self.floats.button_press = Some(ClientFloatButtonPress {
                            float_id: hit.float_id,
                            button_id: button_id.clone(),
                            rect: *rect,
                            state: ClientFloatButtonPressState::Armed,
                        });
                    }
                    return true;
                }
                self.floats.pressed = true;
                let previously_focused = self.floats.focused;
                self.focus_float(hit.float_id, outcome);
                if let Some(old) = previously_focused.filter(|old| *old != hit.float_id) {
                    outcome.requests.extend(super::float_blur_message(old));
                }
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

    /// Whether a new pointer gesture at `point` belongs to the float layer: no overlay or
    /// modal popup owns the pointer and the notification toast is not on top. Mouse routing
    /// and the host pointer shape share this, so a shape always promises the gesture a press
    /// would start.
    fn floats_own_point(&self, point: (u16, u16)) -> bool {
        self.overlay.is_none()
            && !self.popup_pending
            && self.hits.popup.is_none()
            && self.popup_terminal_id.is_none()
            && !super::contains(self.hits.notification_toast, point)
    }

    /// The host pointer shape for float interactions, or `None` to leave the host's own
    /// pointer. An active move or resize keeps its gesture shape until the button is up, even
    /// off the border; otherwise the shape follows the topmost float under the pointer with
    /// the same rules as a press there.
    pub(in crate::client) fn host_pointer_shape(&self) -> Option<HostPointerShape> {
        if let Some(drag) = self
            .floats
            .drag
            .as_ref()
            .filter(|drag| drag.phase == ClientFloatDragPhase::Dragging)
        {
            return Some(drag.gesture.pointer_shape());
        }
        if let Some(press) = self.floats.button_press.as_ref() {
            return (press.state == ClientFloatButtonPressState::Armed)
                .then_some(HostPointerShape::Pointer);
        }
        // Another gesture (selection, pane drag, rearrangement) owns a held button.
        if self.pointer_held {
            return None;
        }
        let point = self.pointer_position?;
        if !self.floats_own_point(point) {
            return None;
        }
        let hit = self
            .hits
            .floats
            .iter()
            .rev()
            .find(|hit| super::contains(hit.outer, point))?;
        if hit
            .buttons
            .iter()
            .any(|(_, rect)| super::contains(*rect, point))
        {
            return Some(HostPointerShape::Pointer);
        }
        if super::contains(hit.pane.inner_rect, point) {
            return None;
        }
        Some(ClientFloatGesture::for_border(hit.outer, point).pointer_shape())
    }

    /// Continues an armed or cancelled button gesture until its release. All pointer
    /// events belong to the press while it exists, so nothing reaches the pane beneath.
    fn continue_float_button_press(
        &mut self,
        press: ClientFloatButtonPress,
        mouse: MouseEvent,
        outcome: &mut ClientShellInput,
    ) -> bool {
        let point = (mouse.column, mouse.row);
        match mouse.kind {
            MouseEventKind::Drag(MouseButton::Left) => {
                if press.state == ClientFloatButtonPressState::Armed
                    && !super::contains(press.rect, point)
                {
                    // Leaving the rectangle cancels permanently for this press, even
                    // after re-entry.
                    self.floats.cancel_button_press();
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                self.floats.button_press = None;
                self.floats.pressed = false;
                if press.state == ClientFloatButtonPressState::Armed
                    && super::contains(press.rect, point)
                {
                    self.activate_float_button(&press, point, outcome);
                }
                outcome.repaint = true;
            }
            // Right, middle, wheel, and unmatched releases are consumed without
            // activation, focus, or drag.
            _ => {}
        }
        true
    }

    /// Sends `float.button.invoke` for the pressed button after re-checking, against the
    /// current compose, that the same topmost float still shows the same button under
    /// the pointer. A failed check invokes nothing.
    fn activate_float_button(
        &mut self,
        press: &ClientFloatButtonPress,
        point: (u16, u16),
        outcome: &mut ClientShellInput,
    ) {
        use crate::api::schema::{FloatButtonInvokeParams, Method, RequiredNullableString};

        let Some(hit) = self
            .hits
            .floats
            .iter()
            .rev()
            .find(|hit| super::contains(hit.outer, point))
        else {
            return;
        };
        if hit.float_id != press.float_id
            || !hit.buttons.iter().any(|(id, rect)| {
                *id == press.button_id && *rect == press.rect && super::contains(*rect, point)
            })
        {
            return;
        }
        let Some(snapshot) = self.snapshot.as_deref() else {
            return;
        };
        let Some(set) = self.floats.buttons.as_ref().and_then(|buttons| {
            buttons
                .floats
                .iter()
                .find(|set| set.float_id == press.float_id)
        }) else {
            return;
        };
        let Some(button) = set
            .buttons
            .iter()
            .find(|button| button.id == press.button_id)
        else {
            return;
        };
        self.push_endpoint_method_with_kind(
            Method::FloatButtonInvoke(FloatButtonInvokeParams {
                boot_id: snapshot.boot_id.clone(),
                float_id: press.float_id.to_string(),
                workspace_id: RequiredNullableString(set.workspace_id.0.clone()),
                plugin_id: set.plugin_id.clone(),
                entrypoint: set.entrypoint.clone(),
                controls_revision: set.controls_revision.clone(),
                button_id: button.id.clone(),
            }),
            PendingEndpointKind::Generic,
            outcome,
        );
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
                crate::api::schema::Method::FloatRaise(crate::api::schema::FloatShowParams {
                    float_id: float_id.to_string(),
                    focus: None,
                }),
                outcome,
            );
            outcome.repaint = true;
        }
        self.reconcile_input_source();
    }

    /// Returns typed input to the tiled layout and releases keys the float saw pressed.
    pub(super) fn blur_float(&mut self, outcome: &mut ClientShellInput) {
        self.floats.cancel_button_press();
        if let Some(focused) = self.floats.focused.take() {
            let leases = self
                .input_leases
                .remove_target(&ClientInputTarget::Float(focused));
            self.push_lease_releases(leases, outcome);
            outcome.requests.extend(super::float_blur_message(focused));
            self.reconcile_input_source();
        }
    }

    /// Returns typed input to the tiled layout when the server moves pane focus. Held keys keep
    /// their lease, so the host's own release still reaches the float that saw the press.
    pub(super) fn yield_float_focus(&mut self) {
        self.floats.cancel_button_press();
        if self.floats.focused.take().is_some() {
            self.reconcile_input_source();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{clamp_drag_position, clamp_resize, layout_float_buttons, truncate_to_cells};
    use crate::api::schema::PluginManifestButton;
    use ratatui::layout::Rect;

    fn button(id: &str, icon: &str) -> PluginManifestButton {
        PluginManifestButton {
            id: id.to_owned(),
            icon: icon.to_owned(),
            action: format!("{id}-action"),
        }
    }

    #[test]
    fn buttons_form_a_right_aligned_gapless_group_in_declaration_order() {
        let outer = Rect::new(10, 4, 20, 6);
        let (rendered, title_budget) =
            layout_float_buttons(outer, &[button("a", "?"), button("b", "x")]);

        assert_eq!(rendered.len(), 2);
        // Interior spans columns 11..=28; two one-cell icons take 6 cells, so the group
        // starts at column 23.
        assert_eq!(rendered[0].rect, Rect::new(23, 4, 3, 1));
        assert_eq!(rendered[1].rect, Rect::new(26, 4, 3, 1));
        assert_eq!(rendered[0].icon, "?");
        assert_eq!(title_budget, 18 - 6);
    }

    #[test]
    fn buttons_show_the_largest_prefix_that_fits_and_never_a_partial_button() {
        let outer = Rect::new(0, 0, 12, 5);
        let buttons: Vec<_> = (0..4)
            .map(|index| button(&format!("b{index}"), "?"))
            .collect();
        let (rendered, title_budget) = layout_float_buttons(outer, &buttons);

        // Interior is 10 cells; each button takes 3, so three fit and the fourth is
        // omitted whole.
        assert_eq!(rendered.len(), 3);
        assert_eq!(rendered[2].id, "b2");
        assert_eq!(title_budget, 1);
    }

    #[test]
    fn a_button_that_cannot_fit_is_omitted_and_the_border_is_unchanged() {
        let outer = Rect::new(0, 0, 4, 3);
        let (rendered, title_budget) = layout_float_buttons(outer, &[button("a", "?")]);

        assert!(rendered.is_empty());
        assert_eq!(title_budget, 2);
    }

    #[test]
    fn a_two_cell_icon_occupies_two_cells_plus_padding() {
        let outer = Rect::new(0, 0, 12, 5);
        let (rendered, _) = layout_float_buttons(outer, &[button("wide", "中")]);

        assert_eq!(rendered[0].rect, Rect::new(7, 0, 4, 1));
    }

    #[test]
    fn truncation_never_splits_a_grapheme_cluster() {
        assert_eq!(truncate_to_cells("abc", 2), "ab");
        assert_eq!(truncate_to_cells("a中", 2), "a");
        assert_eq!(truncate_to_cells("a中x", 3), "a中");
        assert_eq!(truncate_to_cells("é", 1), "é");
        assert_eq!(truncate_to_cells("", 3), "");
    }

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
