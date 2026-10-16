use crate::kitty_graphics::surface::{DeliveryCache, OverlayTerminal, SourceFiles};
use crate::protocol::{ClientShellPopupSurface, SurfaceGraphicsScene};

pub(crate) fn collect_retained(
    app: &crate::app::App,
    panes: &[crate::protocol::PaneSurfacePane],
    target: crate::ui::TabSurfaceTarget,
    floats: &[OverlayTerminal<'_>],
    cell_size: crate::kitty_graphics::HostCellSize,
    delivered: &DeliveryCache,
    client_id: u64,
) -> Option<(SurfaceGraphicsScene, DeliveryCache, SourceFiles)> {
    let rect = |rect: crate::protocol::SurfaceRect| {
        ratatui::layout::Rect::new(rect.x, rect.y, rect.width, rect.height)
    };
    let pane_infos = panes
        .iter()
        .map(|pane| {
            let (workspace_index, id) = app.parse_pane_id(&pane.pane_id)?;
            if workspace_index != target.workspace_index {
                return None;
            }
            Some(crate::layout::PaneInfo {
                id,
                rect: rect(pane.rect),
                inner_rect: rect(pane.inner_rect),
                scrollbar_rect: pane.scrollbar_rect.map(rect),
                borders: ratatui::widgets::Borders::NONE,
                is_focused: pane.focused,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    // Image clipping uses the retained content rectangles, not borders or split handles.
    Some(collect(
        app,
        &pane_infos,
        &[],
        None,
        floats,
        Some(target),
        cell_size,
        delivered,
        client_id,
    ))
}

pub(crate) fn collect(
    app: &crate::app::App,
    pane_infos: &[crate::layout::PaneInfo],
    split_borders: &[crate::layout::SplitBorder],
    popup: Option<&ClientShellPopupSurface>,
    floats: &[OverlayTerminal<'_>],
    target: Option<crate::ui::TabSurfaceTarget>,
    cell_size: crate::kitty_graphics::HostCellSize,
    delivered: &DeliveryCache,
    client_id: u64,
) -> (SurfaceGraphicsScene, DeliveryCache, SourceFiles) {
    let popup = popup
        .zip(app.state.popup_pane.as_ref())
        .map(|(surface, popup)| OverlayTerminal {
            pane_id: popup.pane_id,
            terminal_id: &popup.terminal_id,
            width: surface.frame.width,
            height: surface.frame.height,
        });
    let overlays = popup
        .into_iter()
        .chain(floats.iter().cloned())
        .collect::<Vec<_>>();
    crate::kitty_graphics::surface::collect_scene(
        app,
        crate::ui::TabSurfaceView {
            target,
            pane_infos,
            split_borders,
        },
        &overlays,
        cell_size,
        delivered,
        client_id,
    )
}

/// Which floats' images a viewer receives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FloatGraphics {
    /// The viewer did not negotiate floats.
    None,
    /// The floats visible to a viewer whose tab focuses this pane.
    VisibleTo(Option<crate::layout::PaneId>),
}

/// The floats a viewer sees, sized to each float's content inside `area`.
pub(crate) fn visible_floats(
    app: &crate::app::App,
    viewer: FloatGraphics,
    area: ratatui::layout::Rect,
) -> Vec<OverlayTerminal<'_>> {
    let FloatGraphics::VisibleTo(focused_pane) = viewer else {
        return Vec::new();
    };
    app.state
        .floats
        .iter()
        .filter(|float| float.visible_to(focused_pane))
        .filter_map(|float| {
            let inner = float.geometry(area)?.inner;
            Some(OverlayTerminal {
                pane_id: float.pane_id,
                terminal_id: &float.terminal_id,
                width: inner.width,
                height: inner.height,
            })
        })
        .collect()
}
