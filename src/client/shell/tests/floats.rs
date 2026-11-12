use super::*;

use crate::protocol::floats::{FloatId, FloatSurface, FloatsSurface, INPUT_KIND};

/// Fixture floats are named by terminal id; `term_a` is float `f1`, `term_b` is `f2`.
fn fid(terminal_id: &str) -> FloatId {
    let letter = terminal_id.bytes().last().expect("terminal id");
    FloatId::parse(&format!("f{}", letter - b'a' + 1)).expect("fixture float id")
}

fn float(id: &str, x: u16, y: u16, text: &str) -> FloatSurface {
    let first = format!("{text:<9}");
    let buffer = Buffer::with_lines([first.as_str(), "", ""]);
    FloatSurface {
        float_id: fid(id),
        terminal_id: id.into(),
        title: format!("{id} title"),
        outer: SurfaceRect {
            x,
            y,
            width: 12,
            height: 5,
        },
        frame: FrameData::from_ratatui_buffer_with_hyperlinks(
            &buffer,
            Some(crate::protocol::CursorState {
                x: 1,
                y: 0,
                visible: true,
                shape: 1,
            }),
            &[],
        ),
        mouse_reporting: false,
        sgr_pixel_mouse: false,
        pixel_width: 0,
        pixel_height: 0,
    }
}

fn shell_with_floats(floats: Vec<FloatSurface>) -> ClientShellState {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    state.set_float_layer(FloatsSurface { floats });
    state.compose(106, 20).expect("composed frame");
    state
}

#[test]
fn float_button_failure_notice_requires_the_original_pending_log() {
    use crate::api::schema::PluginCommandStatus;
    use crate::protocol::endpoint::EndpointFloatButtonResult;
    let mut state = shell_with_floats(vec![float("term_a", 1, 1, "float")]);
    let boot = state.snapshot.as_ref().unwrap().boot_id.clone();
    state
        .floats
        .pending_buttons
        .insert("log".into(), "request".into());
    let mut result = EndpointFloatButtonResult {
        boot_id: "old".into(),
        request_id: "request".into(),
        log_id: "log".into(),
        float_id: fid("term_a"),
        button_id: "close".into(),
        status: PluginCommandStatus::Failed,
        exit_code: Some(1),
        error: Some("bad\u{1b}output".into()),
    };
    assert!(!state.receive_float_button_result(result.clone()));
    assert!(state.floats.pending_buttons.contains_key("log"));
    result.boot_id = boot;
    assert!(state.receive_float_button_result(result));
    let notice = state.visible_endpoint_notice.as_ref().unwrap();
    assert_eq!(notice.body, "badoutput");
    assert!(!state.floats.pending_buttons.contains_key("log"));
}

#[test]
fn float_button_metadata_never_rebinds_old_or_removed_float_ids() {
    use crate::protocol::endpoint::{EndpointFloatButtonSet, EndpointFloatButtons};
    let mut state = shell_with_floats(vec![float("term_a", 1, 1, "float")]);
    let boot = state.snapshot.as_ref().unwrap().boot_id.clone();
    let entry = EndpointFloatButtonSet {
        float_id: fid("term_a"),
        workspace_id: crate::api::schema::RequiredNullableString(None),
        plugin_id: "plugin".into(),
        entrypoint: "shell".into(),
        controls_revision: uuid::Uuid::new_v4().to_string(),
        buttons: vec![],
    };
    let metadata = EndpointFloatButtons {
        boot_id: boot.clone(),
        revision: 2,
        floats: vec![entry],
    };
    state.set_float_buttons(&boot, Some(metadata.clone()));
    assert_eq!(state.floats.buttons.as_ref().unwrap().floats.len(), 1);
    state.set_float_layer(FloatsSurface::default());
    assert!(state.floats.buttons.as_ref().unwrap().floats.is_empty());
    state.set_float_buttons(&boot, None);
    state.set_float_layer(FloatsSurface {
        floats: vec![float("term_a", 1, 1, "new")],
    });
    state.set_float_buttons(&boot, Some(metadata));
    assert!(state.floats.buttons.is_none());
    state.clear_float_projection();
    assert!(state.floats.buttons_revision.is_none());
}

fn click(
    state: &mut ClientShellState,
    kind: MouseEventKind,
    column: u16,
    row: u16,
) -> ClientShellInput {
    state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::empty(),
    })])
}

fn float_inputs(input: &ClientShellInput) -> Vec<crate::protocol::floats::FloatInput> {
    input
        .requests
        .iter()
        .filter_map(|message| match message {
            ClientMessage::EndpointControl { kind, data } if kind == INPUT_KIND => {
                Some(crate::protocol::floats::decode_input(data).expect("float input"))
            }
            _ => None,
        })
        .collect()
}

fn float_blurs(input: &ClientShellInput) -> Vec<FloatId> {
    input
        .requests
        .iter()
        .filter_map(|message| match message {
            ClientMessage::EndpointControl { kind, data }
                if kind == crate::protocol::floats::BLUR_KIND =>
            {
                Some(
                    crate::protocol::floats::decode_blur(data)
                        .expect("float blur")
                        .float_id,
                )
            }
            _ => None,
        })
        .collect()
}

fn endpoint_methods(input: &ClientShellInput) -> Vec<crate::api::schema::Method> {
    input
        .actions
        .iter()
        .filter_map(|action| match action {
            ClientShellAction::Endpoint { request, .. } => Some(request.method.clone()),
            _ => None,
        })
        .collect()
}

fn hit(state: &ClientShellState, id: &str) -> super::super::floats::FloatHit {
    state
        .hits
        .floats
        .iter()
        .find(|hit| hit.pane.pane_id == id)
        .cloned()
        .expect("float hit")
}

fn shadow_shell(enabled: bool, floats: Vec<FloatSurface>) -> ClientShellState {
    let config: Config = toml::from_str(&format!("[ui]\nfloat_shadow = {enabled}\n")).unwrap();
    styled_shadow_shell(config, floats)
}

fn styled_shadow_shell(config: Config, floats: Vec<FloatSurface>) -> ClientShellState {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
    state.set_snapshot(Box::new(snapshot()));
    let area = state.layout(106, 20).pane_surface;
    let mut cell = ratatui::buffer::Cell::new("x");
    cell.set_style(
        Style::default()
            .fg(ratatui::style::Color::Yellow)
            .bg(ratatui::style::Color::Blue)
            .add_modifier(Modifier::ITALIC),
    );
    let mut buffer = Buffer::filled(Rect::new(0, 0, area.width, area.height), cell);
    buffer[(0, 0)].set_symbol("s");
    let mut pane_surface = surface();
    pane_surface.frame = FrameData::from_ratatui_buffer_with_hyperlinks(&buffer, None, &[]);
    pane_surface.panes[0].rect.width = area.width;
    pane_surface.panes[0].rect.height = area.height;
    pane_surface.panes[0].inner_rect = pane_surface.panes[0].rect;
    state.set_pane_surface(pane_surface);
    state.set_float_layer(FloatsSurface { floats });
    state.compose(106, 20).expect("shadow fixture");
    state
}

#[test]
fn float_shadow_is_off_by_default_and_leaves_surrounding_cells_unchanged() {
    let mut state = shell_with_floats(vec![float("term_a", 10, 3, "pip")]);
    let outer = hit(&state, "term_a").outer;
    let actual = state.compose(106, 20).unwrap().to_ratatui_buffer().unwrap();
    state.set_float_layer(FloatsSurface::default());
    let under = state.compose(106, 20).unwrap().to_ratatui_buffer().unwrap();
    for y in outer.y + 1..=outer.bottom() {
        assert_eq!(actual[(outer.right(), y)], under[(outer.right(), y)]);
    }
    for x in outer.x + 1..=outer.right() {
        assert_eq!(actual[(x, outer.bottom())], under[(x, outer.bottom())]);
    }
}

#[test]
fn float_shadow_dims_exactly_the_offset_edges_without_changing_symbols_or_foreground() {
    let floats = vec![float("term_a", 10, 3, "pip")];
    let mut off = shadow_shell(false, floats.clone());
    let mut on = shadow_shell(true, floats);
    let outer = hit(&on, "term_a").outer;
    let area = on.layout(106, 20).pane_surface;
    let under = off.compose(106, 20).unwrap().to_ratatui_buffer().unwrap();
    let actual = on.compose(106, 20).unwrap().to_ratatui_buffer().unwrap();
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            let mut expected = under[(x, y)].clone();
            if (x == outer.right() && y > outer.y && y <= outer.bottom())
                || (y == outer.bottom() && x > outer.x && x <= outer.right())
            {
                expected.set_style(
                    Style::default()
                        .bg(ratatui::style::Color::Rgb(0, 0, 0))
                        .add_modifier(Modifier::DIM),
                );
            }
            assert_eq!(actual[(x, y)], expected, "cell {x},{y}");
        }
    }
}

#[test]
fn float_shadow_custom_bands_color_and_zero_sizes_cover_exact_cells() {
    for (width, height) in [(2, 1), (0, 1), (2, 0), (0, 0), (4, 4)] {
        let config: Config = toml::from_str(&format!(
            "[ui.float_shadow]\nenabled = true\ncolor = \"#1d2021\"\nwidth = {width}\nheight = {height}"
        )).unwrap();
        let floats = vec![float("term_a", 10, 3, "pip")];
        let mut on = styled_shadow_shell(config, floats.clone());
        let mut off = shadow_shell(false, floats);
        let outer = hit(&on, "term_a").outer;
        let actual = on.compose(106, 20).unwrap().to_ratatui_buffer().unwrap();
        let under = off.compose(106, 20).unwrap().to_ratatui_buffer().unwrap();
        for y in 0..20 {
            for x in 0..106 {
                let mut expected = under[(x, y)].clone();
                if (x >= outer.right()
                    && x < outer.right() + width
                    && y >= outer.y + height
                    && y < outer.bottom() + height)
                    || (y >= outer.bottom()
                        && y < outer.bottom() + height
                        && x >= outer.x + width
                        && x < outer.right() + width)
                {
                    expected.set_style(
                        Style::default()
                            .bg(ratatui::style::Color::Rgb(29, 32, 33))
                            .add_modifier(Modifier::DIM),
                    );
                }
                assert_eq!(actual[(x, y)], expected, "{width}x{height} cell {x},{y}");
            }
        }
    }
}

#[test]
fn float_shadow_live_reload_changes_color_size_and_accepts_reset_colors() {
    let mut state = shadow_shell(true, vec![float("term_a", 10, 3, "pip")]);
    let outer = hit(&state, "term_a").outer;
    for (color, expected) in [
        ("#1d2021", ratatui::style::Color::Rgb(29, 32, 33)),
        ("none", ratatui::style::Color::Reset),
        ("reset", ratatui::style::Color::Reset),
    ] {
        let config: Config = toml::from_str(&format!(
            "[ui.float_shadow]\nenabled = true\ncolor = \"{color}\"\nwidth = 2\nheight = 1"
        ))
        .unwrap();
        assert!(state.config.apply_live_config(&config, &[], &[]).is_empty());
        let actual = state.compose(106, 20).unwrap().to_ratatui_buffer().unwrap();
        assert_eq!(actual[(outer.right() + 1, outer.y + 1)].bg, expected);
        assert!(actual[(outer.right() + 1, outer.y + 1)]
            .modifier
            .contains(Modifier::DIM));
        assert!(!actual[(outer.x + 1, outer.bottom())]
            .modifier
            .contains(Modifier::DIM));
    }
}

#[test]
fn float_shadow_clips_at_each_pane_surface_edge() {
    let fixture = shadow_shell(false, vec![]);
    let area = fixture.layout(106, 20).pane_surface;
    for (x, y) in [
        (area.width - 14, 3),
        (10, area.height - 7),
        (area.width - 14, area.height - 7),
        (area.width - 12, 3),
        (10, area.height - 5),
        (area.width - 12, area.height - 5),
    ] {
        let floats = vec![float("term_a", x, y, "pip")];
        let config: Config =
            toml::from_str("[ui.float_shadow]\nenabled = true\nwidth = 4\nheight = 4").unwrap();
        let mut on = styled_shadow_shell(config, floats.clone());
        let mut off = shadow_shell(false, floats);
        let outer = hit(&on, "term_a").outer;
        let actual = on.compose(106, 20).unwrap().to_ratatui_buffer().unwrap();
        let under = off.compose(106, 20).unwrap().to_ratatui_buffer().unwrap();
        for row in 0..20 {
            for col in 0..106 {
                if !super::super::contains(area, (col, row))
                    || (outer.right() == area.right() && outer.bottom() == area.bottom())
                {
                    assert_eq!(actual[(col, row)], under[(col, row)]);
                }
            }
        }
    }
}

#[test]
fn higher_floats_cover_lower_shadows_and_cast_shadows_on_lower_floats() {
    let floats = vec![
        float("term_a", 10, 3, "low"),
        float("term_b", 20, 4, "high"),
    ];
    let mut on = shadow_shell(true, floats.clone());
    let mut off = shadow_shell(false, floats);
    let low = hit(&on, "term_a").outer;
    let high = hit(&on, "term_b").outer;
    let actual = on.compose(106, 20).unwrap().to_ratatui_buffer().unwrap();
    let under = off.compose(106, 20).unwrap().to_ratatui_buffer().unwrap();
    assert_eq!(actual[(low.right(), high.y)], under[(low.right(), high.y)]);
    assert!(!actual[(low.right(), high.y)]
        .modifier
        .contains(Modifier::DIM));

    let mut state = shadow_shell(
        true,
        vec![float("term_a", 10, 3, "low"), float("term_b", 8, 2, "high")],
    );
    let high = hit(&state, "term_b").outer;
    let actual = state.compose(106, 20).unwrap().to_ratatui_buffer().unwrap();
    assert_eq!(
        actual[(high.right(), high.bottom())].bg,
        ratatui::style::Color::Rgb(0, 0, 0)
    );
    assert!(actual[(high.right(), high.bottom())]
        .modifier
        .contains(Modifier::DIM));
}

#[test]
fn clicking_a_float_shadow_blurs_without_hitting_the_float() {
    let config: Config =
        toml::from_str("[ui.float_shadow]\nenabled = true\nwidth = 2\nheight = 1").unwrap();
    let mut state = styled_shadow_shell(config, vec![float("term_a", 10, 3, "pip")]);
    let a = hit(&state, "term_a");
    assert_eq!(a.pane.rect, a.outer);
    let points = (a.outer.y + 1..a.outer.bottom() + 1)
        .flat_map(|y| (a.outer.right()..a.outer.right() + 2).map(move |x| (x, y)))
        .chain((a.outer.x + 2..a.outer.right()).map(|x| (x, a.outer.bottom())));
    for (column, row) in points {
        state.floats.focused = Some(fid("term_a"));
        let mut outcome = ClientShellInput::default();
        assert!(!state.handle_float_mouse(
            crossterm::event::MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column,
                row,
                modifiers: KeyModifiers::empty(),
            },
            &mut outcome
        ));
        assert_eq!(float_blurs(&outcome), vec![fid("term_a")]);
        assert!(float_inputs(&outcome).is_empty());
        assert!(state.floats.focused.is_none());
    }
}

#[test]
fn float_shadow_live_config_reload_turns_the_shadow_on_and_off() {
    assert!(!Config::default().ui.float_shadow.enabled);
    assert!(
        !toml::from_str::<Config>("[ui]\n")
            .unwrap()
            .ui
            .float_shadow
            .enabled
    );
    let mut state = shadow_shell(false, vec![float("term_a", 10, 3, "pip")]);
    let outer = hit(&state, "term_a").outer;
    let original = state.compose(106, 20).unwrap().to_ratatui_buffer().unwrap();
    for enabled in [true, false] {
        let config: Config = toml::from_str(&format!("[ui]\nfloat_shadow = {enabled}\n")).unwrap();
        assert_eq!(config.ui.float_shadow.enabled, enabled);
        assert!(state.config.apply_live_config(&config, &[], &[]).is_empty());
        let actual = state.compose(106, 20).unwrap().to_ratatui_buffer().unwrap();
        assert_eq!(
            actual[(outer.right(), outer.bottom())]
                .modifier
                .contains(Modifier::DIM),
            enabled
        );
        if !enabled {
            assert_eq!(actual, original);
        }
    }
}

#[test]
fn float_shadow_invalid_live_reload_keeps_previous_style() {
    let _guard = crate::config::test_config_env_lock().lock().unwrap();
    let previous_path = std::env::var_os(crate::config::CONFIG_PATH_ENV_VAR);
    let path = std::env::temp_dir().join(format!(
        "herdr-float-shadow-reload-{}.toml",
        std::process::id()
    ));
    std::env::set_var(crate::config::CONFIG_PATH_ENV_VAR, &path);
    let mut state = shadow_shell(true, vec![float("term_a", 10, 3, "pip")]);
    std::fs::write(
        &path,
        "[ui.float_shadow]\nenabled = true\ncolor = \"#1d2021\"\nwidth = 2\nheight = 1",
    )
    .unwrap();
    state.reload_client_config();
    let previous = state.config.float_shadow;
    let original = state.compose(106, 20).unwrap().to_ratatui_buffer().unwrap();
    for key in ["width", "height"] {
        std::fs::write(&path, format!("[ui.float_shadow]\n{key} = 5")).unwrap();
        let loaded = crate::config::load_live_config().unwrap();
        assert_eq!(loaded.invalid_sections, vec!["ui"]);
        assert!(loaded
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains(key)));
        state.reload_client_config();
        assert_eq!(state.config.float_shadow, previous);
        let actual = state.compose(106, 20).unwrap().to_ratatui_buffer().unwrap();
        let area = state.layout(106, 20).pane_surface;
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                assert_eq!(actual[(x, y)], original[(x, y)], "cell {x},{y}");
            }
        }
    }
    std::fs::remove_file(path).unwrap();
    match previous_path {
        Some(value) => std::env::set_var(crate::config::CONFIG_PATH_ENV_VAR, value),
        None => std::env::remove_var(crate::config::CONFIG_PATH_ENV_VAR),
    }
}

#[test]
fn float_shadow_follows_move_and_resize_previews_and_repaints_the_old_cells() {
    let mut state = shadow_shell(true, vec![float("term_a", 10, 3, "pip")]);
    let original = hit(&state, "term_a").outer;
    let grab = (original.x + 4, original.y);
    preview_border_drag(&mut state, grab, (grab.0 + 15, grab.1 + 2));
    let moved = hit(&state, "term_a").outer;
    let frame = state.compose(106, 20).unwrap().to_ratatui_buffer().unwrap();
    assert_eq!(
        frame[(original.right(), original.bottom())].bg,
        ratatui::style::Color::Blue
    );
    assert!(!frame[(original.right(), original.bottom())]
        .modifier
        .contains(Modifier::DIM));
    assert!(frame[(moved.right(), moved.bottom())]
        .modifier
        .contains(Modifier::DIM));

    state.floats.drag = None;
    state.compose(106, 20).unwrap();
    let grab = (original.right() - 1, original.bottom() - 1);
    preview_border_drag(&mut state, grab, (grab.0 + 4, grab.1 + 2));
    let resized = hit(&state, "term_a").outer;
    let frame = state.compose(106, 20).unwrap().to_ratatui_buffer().unwrap();
    assert!(frame[(resized.right(), resized.bottom())]
        .modifier
        .contains(Modifier::DIM));
    assert!(!frame[(original.right(), original.bottom())]
        .modifier
        .contains(Modifier::DIM));
}

#[test]
fn pane_patches_under_float_shadows_recompose_but_patches_beside_them_stay_fast() {
    for (enabled, width, height) in [
        (false, 2, 1),
        (true, 1, 1),
        (true, 2, 1),
        (true, 0, 1),
        (true, 2, 0),
        (true, 0, 0),
    ] {
        for (x, y) in [
            (22, 4),
            (23, 4),
            (11, 8),
            (12, 8),
            (22, 8),
            (23, 8),
            (22, 3),
            (10, 8),
        ] {
            let shadow = (x >= 22 && x < 22 + width && y >= 3 + height && y < 8 + height)
                || (y >= 8 && y < 8 + height && x >= 10 + width && x < 22 + width);
            let config: Config = toml::from_str(&format!(
                "[ui.float_shadow]\nenabled = {enabled}\nwidth = {width}\nheight = {height}"
            ))
            .unwrap();
            let mut state = styled_shadow_shell(config, vec![float("term_a", 10, 3, "pip")]);
            let mut pane = state.pane_surface.as_ref().unwrap().panes[0].clone();
            pane.content_revision += 1;
            let patch = crate::protocol::PaneSurfacePatch {
                boot_id: "boot-1".into(),
                projection_revision: 1,
                base_surface_revision: 1,
                surface_revision: 2,
                rows: vec![crate::protocol::PaneSurfacePatchRow {
                    x,
                    y,
                    cells: vec![crate::protocol::CellData {
                        symbol: "N".into(),
                        fg: 0,
                        bg: 0,
                        modifier: 0,
                        skip: false,
                        hyperlink: None,
                    }],
                }],
                panes: vec![pane],
                cursor: None,
            };
            let ClientPaneSurfacePatchOutcome::Applied(fast) =
                state.apply_pane_surface_patch(patch)
            else {
                panic!("patch must apply");
            };
            assert_eq!(
                fast.is_none(),
                enabled && shadow,
                "enabled={enabled}, cell={x},{y}"
            );
            let area = state.layout(106, 20).pane_surface;
            let frame = state.compose(106, 20).unwrap().to_ratatui_buffer().unwrap();
            let cell = &frame[(area.x + x, area.y + y)];
            assert_eq!(cell.symbol(), "N");
            assert_eq!(cell.modifier.contains(Modifier::DIM), enabled && shadow);
        }
    }
}

#[test]
fn float_shadow_cells_do_not_occlude_kitty_graphics() {
    let mut state = shadow_shell(true, vec![float("term_a", 10, 3, "pip")]);
    let area = state.layout(106, 20).pane_surface;
    let outer = hit(&state, "term_a").outer;
    let at = (outer.right(), outer.bottom());
    let mut surface = state.pane_surface.clone().unwrap();
    super::graphics::add_main_image(&mut surface, state.layout(106, 20), at, 48);
    state.set_pane_surface(surface);
    assert!(placed(&mut state, at));
    assert!(super::super::contains(area, at));
}

#[test]
#[ignore = "manual client float shadow render scaling profile"]
fn float_shadow_render_scale_profile() {
    use std::hint::black_box;
    use std::time::Instant;

    for count in [1, 15] {
        let config: Config = toml::from_str("[ui.float_shadow]\nwidth = 4\nheight = 4").unwrap();
        let mut state = styled_shadow_shell(config, vec![float("term_a", 10, 3, "pip")]);
        let mut projection = snapshot();
        let mut surface = state.pane_surface.clone().unwrap();
        let area = state.layout(106, 20).pane_surface;
        let pane = projection.panes[0].clone();
        let geometry = surface.panes[0].clone();
        projection.panes.clear();
        surface.panes.clear();
        for index in 0..count {
            let id = format!("pane_{}", index + 1);
            let mut pane = pane.clone();
            pane.pane_id = id.clone();
            pane.focused = index == 0;
            projection.panes.push(pane);
            let mut geometry = geometry.clone();
            geometry.pane_id = id;
            geometry.focused = index == 0;
            geometry.rect.x = index * area.width / count;
            geometry.rect.width = (index + 1) * area.width / count - geometry.rect.x;
            geometry.inner_rect = geometry.rect;
            surface.panes.push(geometry);
        }
        state.set_snapshot(Box::new(projection));
        state.set_pane_surface(surface);
        let mut samples = [Vec::new(), Vec::new()];
        let outer = hit(&state, "term_a").outer;
        for round in 0..6 {
            for enabled in [false, true] {
                state.config.float_shadow.enabled = enabled;
                let started = Instant::now();
                for _ in 0..40 {
                    black_box(state.compose(106, 20).expect("populated pane frame"));
                }
                let elapsed = started.elapsed().as_nanos() / 40;
                let frame = state.compose(106, 20).unwrap().to_ratatui_buffer().unwrap();
                assert_eq!(
                    frame[(outer.right(), outer.bottom())]
                        .modifier
                        .contains(Modifier::DIM),
                    enabled
                );
                assert_eq!(state.hits.panes.len(), usize::from(count));
                if round > 0 {
                    samples[usize::from(enabled)].push(elapsed);
                }
            }
        }
        for sample in &mut samples {
            sample.sort_unstable();
        }
        println!("float shadow size=4x4 panes={count} fixed=106x20 ns/frame off={} [{}..{}] on={} [{}..{}] runs=5 frames/run=40",
            samples[0][2], samples[0][0], samples[0][4], samples[1][2], samples[1][0], samples[1][4]);
    }
}

#[test]
fn floats_draw_over_the_pane_surface_at_their_server_position() {
    let state = shell_with_floats(vec![float("term_a", 10, 3, "pip-live")]);
    let surface = state.layout(106, 20).pane_surface;
    let a = hit(&state, "term_a");

    assert_eq!(a.outer, Rect::new(surface.x + 10, surface.y + 3, 12, 5));
    assert_eq!(
        a.pane.inner_rect,
        Rect::new(a.outer.x + 1, a.outer.y + 1, 9, 3)
    );
    let mut state = state;
    let frame = state.compose(106, 20).expect("frame");
    assert_eq!(
        cell_symbol_position(&frame, a.outer, "pip-l"),
        (a.outer.x + 1, a.outer.y + 1)
    );
    assert_eq!(
        cell_symbol_position(&frame, a.outer, "term_a"),
        (a.outer.x + 1, a.outer.y)
    );
    assert_eq!(
        frame.cursor.as_ref().map(|cursor| (cursor.x, cursor.y)),
        Some((surface.x + 1, surface.y + 1)),
        "an unfocused float leaves the cursor with the focused pane"
    );
}

#[test]
fn keys_reach_a_float_only_after_it_is_clicked() {
    let mut state = shell_with_floats(vec![float("term_a", 10, 3, "pip")]);

    let before = state.handle_input_bytes(b"x");
    assert!(matches!(
        &before.requests[..],
        [ClientMessage::ClientShellPaneInput { pane_id, .. }] if pane_id == "pane_1"
    ));

    let a = hit(&state, "term_a");
    let down = click(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        a.pane.inner_rect.x + 2,
        a.pane.inner_rect.y,
    );
    assert_eq!(
        float_inputs(&down).len(),
        1,
        "the click itself reaches the float"
    );
    assert!(
        endpoint_methods(&down).is_empty(),
        "the top float needs no raise"
    );

    let typed = state.handle_input_bytes(b"y");
    let inputs = float_inputs(&typed);
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0].float_id, fid("term_a"));

    let frame = state.compose(106, 20).expect("focused frame");
    let a = hit(&state, "term_a");
    assert_eq!(
        frame.cursor.as_ref().map(|cursor| (cursor.x, cursor.y)),
        Some((a.pane.inner_rect.x + 1, a.pane.inner_rect.y)),
        "the focused float owns the cursor"
    );

    let prefix = state.handle_input_bytes(b"\x02");
    assert!(
        prefix.requests.is_empty(),
        "the prefix key stays a shell binding"
    );
    assert_eq!(state.mode, ClientShellMode::Prefix);
}

#[test]
fn clicking_the_tiled_layout_returns_keys_to_the_focused_pane() {
    let mut state = shell_with_floats(vec![float("term_a", 10, 3, "pip")]);
    let a = hit(&state, "term_a");
    click(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        a.pane.inner_rect.x,
        a.pane.inner_rect.y,
    );
    click(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        a.pane.inner_rect.x,
        a.pane.inner_rect.y,
    );
    let pane = state.hits.panes[0].clone();

    click(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        pane.inner_rect.x,
        pane.inner_rect.y,
    );
    click(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        pane.inner_rect.x,
        pane.inner_rect.y,
    );

    let typed = state.handle_input_bytes(b"z");
    assert!(float_inputs(&typed).is_empty());
    assert!(matches!(
        &typed.requests[..],
        [ClientMessage::ClientShellPaneInput { pane_id, .. }] if pane_id == "pane_1"
    ));
}

#[test]
fn clicking_a_lower_float_raises_it() {
    let mut state = shell_with_floats(vec![
        float("term_a", 10, 3, "lower"),
        float("term_b", 40, 3, "upper"),
    ]);
    let a = hit(&state, "term_a");

    let down = click(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        a.pane.inner_rect.x,
        a.pane.inner_rect.y,
    );

    assert!(matches!(
        &endpoint_methods(&down)[..],
        [crate::api::schema::Method::FloatRaise(target)]
            if target.float_id == fid("term_a").to_string()
    ));
    state.compose(106, 20).expect("raised frame");
    let order: Vec<_> = state
        .hits
        .floats
        .iter()
        .map(|hit| hit.pane.pane_id.clone())
        .collect();
    assert_eq!(order, vec!["term_b".to_owned(), "term_a".to_owned()]);
}

#[test]
fn dragging_the_border_previews_then_commits_a_clamped_move() {
    let mut state = shell_with_floats(vec![float("term_a", 10, 3, "pip")]);
    let surface = state.layout(106, 20).pane_surface;
    let a = hit(&state, "term_a");
    let grab = (a.outer.x + 4, a.outer.y);

    let down = click(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        grab.0,
        grab.1,
    );
    assert!(
        float_inputs(&down).is_empty(),
        "border clicks never reach the program"
    );
    click(
        &mut state,
        MouseEventKind::Drag(MouseButton::Left),
        grab.0 + 5,
        grab.1 + 2,
    );
    state.compose(106, 20).expect("drag preview");
    assert_eq!(
        hit(&state, "term_a").outer,
        Rect::new(a.outer.x + 5, a.outer.y + 2, 12, 5)
    );

    let up = click(&mut state, MouseEventKind::Up(MouseButton::Left), 500, 500);
    let max = (surface.width - 12, surface.height - 5);
    assert!(matches!(
        &endpoint_methods(&up)[..],
        [crate::api::schema::Method::FloatMove(params)]
            if params.float_id == fid("term_a").to_string() && (params.x, params.y) == max
    ));
    state.compose(106, 20).expect("held preview");
    assert_eq!(
        hit(&state, "term_a").outer,
        Rect::new(surface.x + max.0, surface.y + max.1, 12, 5),
        "the preview holds until the server confirms"
    );

    state.set_float_layer(FloatsSurface {
        floats: vec![float("term_a", max.0, max.1, "pip")],
    });
    assert!(state.floats.drag.is_none());
}

#[test]
fn modal_popup_takes_input_from_a_focused_float() {
    let mut state = shell_with_floats(vec![float("term_a", 10, 3, "pip")]);
    let a = hit(&state, "term_a");
    click(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        a.pane.inner_rect.x,
        a.pane.inner_rect.y,
    );

    state.set_pane_surface(surface_with_popup());
    state.compose(106, 20).expect("popup frame");
    let typed = state.handle_input_bytes(b"x");

    assert!(float_inputs(&typed).is_empty());
    assert!(matches!(
        &typed.requests[..],
        [ClientMessage::ClientShellPopupInput { terminal_id, .. }] if terminal_id == "terminal-popup"
    ));
}

#[test]
fn a_float_that_leaves_the_layer_gives_focus_back_to_the_layout() {
    let mut state = shell_with_floats(vec![float("term_a", 10, 3, "pip")]);
    let a = hit(&state, "term_a");
    click(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        a.pane.inner_rect.x,
        a.pane.inner_rect.y,
    );

    state.set_float_layer(FloatsSurface::default());
    state.compose(106, 20).expect("empty layer");

    assert!(state.hits.floats.is_empty());
    let typed = state.handle_input_bytes(b"x");
    assert!(matches!(
        &typed.requests[..],
        [ClientMessage::ClientShellPaneInput { pane_id, .. }] if pane_id == "pane_1"
    ));
}

#[test]
fn pane_patches_under_a_float_recompose_instead_of_overwriting_it() {
    let mut state = shell_with_floats(vec![float("term_a", 0, 0, "pip")]);
    let mut updated_pane = surface().panes[0].clone();
    updated_pane.content_revision = 2;
    let patch = crate::protocol::PaneSurfacePatch {
        boot_id: "boot-1".into(),
        projection_revision: 1,
        base_surface_revision: 1,
        surface_revision: 2,
        rows: vec![crate::protocol::PaneSurfacePatchRow {
            x: 0,
            y: 0,
            cells: vec![
                crate::protocol::CellData {
                    symbol: "N".into(),
                    fg: 0,
                    bg: 0,
                    modifier: 0,
                    skip: false,
                    hyperlink: None,
                };
                4
            ],
        }],
        panes: vec![updated_pane],
        cursor: None,
    };

    assert!(matches!(
        state.apply_pane_surface_patch(patch),
        ClientPaneSurfacePatchOutcome::Applied(None)
    ));
}

#[test]
fn clipboard_images_are_not_misrouted_to_the_pane_under_a_focused_float() {
    let mut state = shell_with_floats(vec![float("term_a", 10, 3, "pip")]);
    assert!(state.clipboard_image_target().is_some());
    let a = hit(&state, "term_a");

    click(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        a.pane.inner_rect.x,
        a.pane.inner_rect.y,
    );

    assert_eq!(state.clipboard_image_target(), None);
}

#[test]
fn a_selection_dragged_across_a_float_still_finishes() {
    let mut state = shell_with_floats(vec![float("term_a", 10, 3, "pip")]);
    let pane = state.hits.panes[0].clone();
    let a = hit(&state, "term_a");

    click(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        pane.inner_rect.x,
        pane.inner_rect.y,
    );
    click(
        &mut state,
        MouseEventKind::Drag(MouseButton::Left),
        a.pane.inner_rect.x + 1,
        a.pane.inner_rect.y,
    );
    let up = click(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        a.pane.inner_rect.x + 1,
        a.pane.inner_rect.y,
    );

    assert!(float_inputs(&up).is_empty());
    assert!(matches!(
        &endpoint_methods(&up)[..],
        [crate::api::schema::Method::PaneSelectionRead(params)] if params.pane_id == "pane_1"
    ));
}

#[test]
fn a_sidebar_drag_released_over_a_float_ends_the_chrome_drag() {
    let mut state = shell_with_floats(vec![float("term_a", 10, 3, "pip")]);
    let a = hit(&state, "term_a");
    state.chrome_drag = Some(ClientChromeDrag::SidebarWidth);

    click(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        a.pane.inner_rect.x + 1,
        a.pane.inner_rect.y,
    );

    assert!(state.chrome_drag.is_none());
}

fn release_border_drag(state: &mut ClientShellState, by: (u16, u16)) -> String {
    let a = hit(state, "term_a");
    let grab = (a.outer.x + 4, a.outer.y);
    click(
        state,
        MouseEventKind::Down(MouseButton::Left),
        grab.0,
        grab.1,
    );
    click(
        state,
        MouseEventKind::Drag(MouseButton::Left),
        grab.0 + by.0,
        grab.1 + by.1,
    );
    let up = click(
        state,
        MouseEventKind::Up(MouseButton::Left),
        grab.0 + by.0,
        grab.1 + by.1,
    );
    let [ClientShellAction::Endpoint { request, .. }] = &up.actions[..] else {
        panic!("release sends float.move");
    };
    request.id.clone()
}

fn float_origin(state: &mut ClientShellState) -> (u16, u16) {
    let surface = state.layout(106, 20).pane_surface;
    state.compose(106, 20).expect("frame");
    let outer = hit(state, "term_a").outer;
    (outer.x - surface.x, outer.y - surface.y)
}

#[test]
fn a_held_drag_preview_yields_to_a_move_from_elsewhere() {
    let mut state = shell_with_floats(vec![float("term_a", 10, 3, "pip")]);
    release_border_drag(&mut state, (5, 2));
    assert_eq!(float_origin(&mut state), (15, 5));

    state.set_float_layer(FloatsSurface {
        floats: vec![float("term_a", 30, 1, "pip")],
    });

    assert_eq!(float_origin(&mut state), (30, 1));
}

#[test]
fn a_rejected_move_reverts_the_preview_to_the_server_position() {
    let mut state = shell_with_floats(vec![float("term_a", 10, 3, "pip")]);
    let request_id = release_border_drag(&mut state, (5, 2));

    let (repaint, _) = state.handle_endpoint_result(
        "boot-1",
        &request_id,
        Err(ClientShellEndpointError {
            code: Some("float_not_found".into()),
            message: "float moved away".into(),
        }),
    );

    assert!(repaint);
    assert_eq!(float_origin(&mut state), (10, 3));
}

#[test]
fn an_answered_move_settles_on_the_next_layer_even_when_the_server_placed_it_elsewhere() {
    let mut state = shell_with_floats(vec![float("term_a", 10, 3, "pip")]);
    let request_id = release_border_drag(&mut state, (5, 2));
    state.handle_endpoint_result(
        "boot-1",
        &request_id,
        Ok(crate::api::schema::ResponseResult::Ok {}),
    );
    assert_eq!(float_origin(&mut state), (15, 5));

    state.set_float_layer(FloatsSurface {
        floats: vec![float("term_a", 10, 3, "pip")],
    });

    assert_eq!(float_origin(&mut state), (10, 3));
}

#[test]
fn clicking_away_from_a_float_releases_keys_it_saw_pressed() {
    let mut state = shell_with_floats(vec![float("term_a", 10, 3, "pip")]);
    let a = hit(&state, "term_a");
    click(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        a.pane.inner_rect.x,
        a.pane.inner_rect.y,
    );
    click(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        a.pane.inner_rect.x,
        a.pane.inner_rect.y,
    );
    let press = state.handle_raw_events(vec![RawInputEvent::Key(crate::input::TerminalKey::new(
        KeyCode::Char('x'),
        KeyModifiers::empty(),
    ))]);
    assert_eq!(float_inputs(&press).len(), 1);
    let pane = state.hits.panes[0].clone();

    let away = click(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        pane.inner_rect.x,
        pane.inner_rect.y,
    );

    let released = float_inputs(&away);
    assert!(matches!(
        &released[..],
        [input] if input.float_id == fid("term_a")
            && matches!(
                &input.events[..],
                [ClientPaneInputEvent::Key {
                    kind: crate::protocol::ClientKeyKind::Release,
                    ..
                }]
            )
    ));
}

#[test]
fn a_key_held_across_a_server_focus_change_releases_in_the_float() {
    let mut state = shell_with_floats(vec![float("term_a", 10, 3, "pip")]);
    let a = hit(&state, "term_a");
    click(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        a.pane.inner_rect.x,
        a.pane.inner_rect.y,
    );
    let key = crate::input::TerminalKey::new(KeyCode::Char('x'), KeyModifiers::empty());
    state.handle_raw_events(vec![RawInputEvent::Key(key.clone())]);
    let mut moved = snapshot();
    moved.focused_pane_id = Some("pane_2".into());
    state.set_snapshot(Box::new(moved));

    let release = state.handle_raw_events(vec![RawInputEvent::Key(
        key.with_kind(crossterm::event::KeyEventKind::Release),
    )]);

    assert!(matches!(
        &float_inputs(&release)[..],
        [input] if input.float_id == fid("term_a")
    ));
    let typed = state.handle_input_bytes(b"y");
    assert!(float_inputs(&typed).is_empty(), "focus left the float");
}

#[test]
fn a_focused_float_keeps_the_cursor_while_patches_beside_it_take_the_fast_path() {
    let mut state = shell_with_floats(vec![float("term_a", 10, 3, "pip")]);
    let a = hit(&state, "term_a");
    click(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        a.pane.inner_rect.x,
        a.pane.inner_rect.y,
    );
    let frame = state.compose(106, 20).expect("focused frame");
    let float_cursor = frame.cursor.clone();
    assert!(float_cursor.is_some());
    let mut updated_pane = surface().panes[0].clone();
    updated_pane.content_revision = 2;
    let patch = crate::protocol::PaneSurfacePatch {
        boot_id: "boot-1".into(),
        projection_revision: 1,
        base_surface_revision: 1,
        surface_revision: 2,
        rows: vec![crate::protocol::PaneSurfacePatchRow {
            x: 0,
            y: 1,
            cells: vec![
                crate::protocol::CellData {
                    symbol: "N".into(),
                    fg: 0,
                    bg: 0,
                    modifier: 0,
                    skip: false,
                    hyperlink: None,
                };
                4
            ],
        }],
        panes: vec![updated_pane],
        cursor: Some(crate::protocol::CursorState {
            x: 2,
            y: 1,
            visible: true,
            shape: 1,
        }),
    };

    let ClientPaneSurfacePatchOutcome::Applied(Some(composed)) =
        state.apply_pane_surface_patch(patch)
    else {
        panic!("a patch beside the focused float takes the fast path");
    };
    assert_eq!(composed.cursor, float_cursor);
}

fn surface_with_float_image(
    state: &ClientShellState,
    float_id: &str,
    at: (u16, u16),
    id: u32,
) -> PaneSurfaceFrame {
    let mut surface = state.pane_surface.clone().expect("pane surface");
    let (asset, placement) = super::graphics::image(
        crate::protocol::SurfaceGraphicsTarget::Popup {
            terminal_id: float_id.into(),
        },
        at.0,
        at.1,
        id,
    );
    surface.graphics.assets.push(asset);
    surface.graphics.placements.push(placement);
    surface
}

fn placed(state: &mut ClientShellState, point: (u16, u16)) -> bool {
    let frame = state.compose(106, 20).expect("frame");
    super::graphics::is_placed(&frame.graphics.into_inline_bytes(), point)
}

#[test]
fn float_images_sit_at_the_float_inner_origin_and_follow_a_drag_preview() {
    let mut state = shell_with_floats(vec![float("term_a", 10, 3, "pip")]);
    let inner = hit(&state, "term_a").pane.inner_rect;
    state.set_pane_surface(surface_with_float_image(&state, "term_a", (2, 1), 40));
    assert!(placed(&mut state, (inner.x + 2, inner.y + 1)));

    let grab = (inner.x + 3, inner.y - 1);
    click(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        grab.0,
        grab.1,
    );
    click(
        &mut state,
        MouseEventKind::Drag(MouseButton::Left),
        grab.0 + 5,
        grab.1 + 2,
    );
    let frame = state.compose(106, 20).expect("drag preview");
    let bytes = frame.graphics.into_inline_bytes();
    assert!(super::graphics::is_placed(
        &bytes,
        (inner.x + 7, inner.y + 3)
    ));
    assert!(!super::graphics::is_placed(
        &bytes,
        (inner.x + 2, inner.y + 1)
    ));
}

#[test]
fn float_images_are_cropped_by_whatever_is_drawn_above_them() {
    let mut state = shell_with_floats(vec![
        float("term_a", 10, 3, "low"),
        float("term_b", 14, 5, "high"),
    ]);
    let low = hit(&state, "term_a");
    let high = hit(&state, "term_b");
    let layout = state.layout(106, 20);
    let mut surface = surface_with_float_image(&state, "term_a", (0, 0), 41);
    let (asset, placement) = super::graphics::image(
        crate::protocol::SurfaceGraphicsTarget::Popup {
            terminal_id: "term_a".into(),
        },
        high.outer.x - low.pane.inner_rect.x,
        high.outer.y - low.pane.inner_rect.y,
        42,
    );
    surface.graphics.assets.push(asset);
    surface.graphics.placements.push(placement);
    let under_low = (low.pane.inner_rect.x + 1, low.pane.inner_rect.y + 1);
    super::graphics::add_main_image(&mut surface, layout, under_low, 43);
    let (asset, placement) = super::graphics::image(
        crate::protocol::SurfaceGraphicsTarget::Popup {
            terminal_id: "term_b".into(),
        },
        0,
        0,
        44,
    );
    surface.graphics.assets.push(asset);
    surface.graphics.placements.push(placement);
    state.set_pane_surface(surface);

    let frame = state.compose(106, 20).expect("frame");
    let bytes = frame.graphics.into_inline_bytes();
    let low_origin = (low.pane.inner_rect.x, low.pane.inner_rect.y);
    let high_origin = (high.pane.inner_rect.x, high.pane.inner_rect.y);
    assert!(super::graphics::is_placed(&bytes, low_origin));
    assert!(
        !super::graphics::is_placed(&bytes, (high.outer.x, high.outer.y)),
        "the higher float crops the lower float's image"
    );
    assert!(super::graphics::is_placed(&bytes, high_origin));
    assert!(
        !super::graphics::is_placed(&bytes, under_low),
        "tiled images under a float stay cropped"
    );
}

#[test]
fn the_modal_popup_crops_float_images_under_it() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface_with_popup());
    state.compose(106, 20).expect("popup frame");
    let popup = state.hits.popup.clone().expect("popup hit").rect;
    let surface = state.layout(106, 20).pane_surface;
    let at = (popup.x - surface.x - 4, popup.y - surface.y - 2);
    state.set_float_layer(FloatsSurface {
        floats: vec![float("term_a", at.0, at.1, "pip")],
    });
    state.compose(106, 20).expect("float frame");
    let inner = hit(&state, "term_a").pane.inner_rect;
    let mut surface = surface_with_float_image(&state, "term_a", (0, 0), 46);
    let (asset, placement) = super::graphics::image(
        crate::protocol::SurfaceGraphicsTarget::Popup {
            terminal_id: "term_a".into(),
        },
        popup.x - inner.x,
        popup.y - inner.y,
        47,
    );
    surface.graphics.assets.push(asset);
    surface.graphics.placements.push(placement);
    let mut without_popup = surface.clone();
    state.set_pane_surface(surface);

    let frame = state.compose(106, 20).expect("frame");
    let bytes = frame.graphics.into_inline_bytes();
    assert!(super::graphics::is_placed(&bytes, (inner.x, inner.y)));
    assert!(!super::graphics::is_placed(&bytes, (popup.x, popup.y)));

    without_popup.popup = None;
    state.set_pane_surface(without_popup);
    assert!(
        placed(&mut state, (popup.x, popup.y)),
        "uncovered once the popup closes"
    );
}

#[test]
fn float_images_leave_with_their_float_and_never_draw_without_one() {
    let mut state = shell_with_floats(vec![float("term_a", 10, 3, "pip")]);
    let inner = hit(&state, "term_a").pane.inner_rect;
    state.set_pane_surface(surface_with_float_image(&state, "term_a", (0, 0), 45));
    assert!(placed(&mut state, (inner.x, inner.y)));

    state.set_float_layer(FloatsSurface::default());
    let frame = state.compose(106, 20).expect("frame");
    let bytes = frame.graphics.into_inline_bytes();
    assert!(!super::graphics::is_placed(&bytes, (inner.x, inner.y)));
    assert!(String::from_utf8_lossy(&bytes).contains("a=d"));

    state.set_float_layer(FloatsSurface {
        floats: vec![float("term_a", 10, 3, "pip")],
    });
    assert!(placed(&mut state, (inner.x, inner.y)));
}

/// Presses at `grab`, drags to `to`, and returns the previewed outer rect relative to the
/// pane surface, before release.
fn preview_border_drag(state: &mut ClientShellState, grab: (u16, u16), to: (u16, u16)) -> Rect {
    let down = click(
        state,
        MouseEventKind::Down(MouseButton::Left),
        grab.0,
        grab.1,
    );
    assert!(
        float_inputs(&down).is_empty(),
        "border clicks never reach the program"
    );
    click(state, MouseEventKind::Drag(MouseButton::Left), to.0, to.1);
    let surface = state.layout(106, 20).pane_surface;
    state.compose(106, 20).expect("drag preview");
    let outer = hit(state, "term_a").outer;
    Rect::new(
        outer.x - surface.x,
        outer.y - surface.y,
        outer.width,
        outer.height,
    )
}

#[test]
fn dragging_the_right_border_widens_only() {
    let mut state = shell_with_floats(vec![float("term_a", 10, 3, "pip")]);
    let a = hit(&state, "term_a");
    let grab = (a.outer.right() - 1, a.outer.y + 2);

    let preview = preview_border_drag(&mut state, grab, (grab.0 + 4, grab.1 + 2));

    assert_eq!(preview, Rect::new(10, 3, 16, 5));
}

#[test]
fn dragging_the_bottom_border_heightens_only() {
    let mut state = shell_with_floats(vec![float("term_a", 10, 3, "pip")]);
    let a = hit(&state, "term_a");
    let grab = (a.outer.x + 4, a.outer.bottom() - 1);

    let preview = preview_border_drag(&mut state, grab, (grab.0 + 5, grab.1 + 3));

    assert_eq!(preview, Rect::new(10, 3, 12, 8));
}

#[test]
fn dragging_the_bottom_right_corner_resizes_both_axes() {
    let mut state = shell_with_floats(vec![float("term_a", 10, 3, "pip")]);
    let a = hit(&state, "term_a");
    let grab = (a.outer.right() - 1, a.outer.bottom() - 1);

    let preview = preview_border_drag(&mut state, grab, (grab.0 + 5, grab.1 + 2));

    assert_eq!(preview, Rect::new(10, 3, 17, 7));
}

#[test]
fn a_resize_stops_at_the_float_minimum_and_the_pane_surface_edge() {
    let mut state = shell_with_floats(vec![float("term_a", 10, 3, "pip")]);
    let surface = state.layout(106, 20).pane_surface;
    let a = hit(&state, "term_a");
    let grab = (a.outer.right() - 1, a.outer.bottom() - 1);

    let shrunk = preview_border_drag(&mut state, grab, (a.outer.x, a.outer.y));
    assert_eq!(shrunk, Rect::new(10, 3, 6, 4));

    click(
        &mut state,
        MouseEventKind::Drag(MouseButton::Left),
        500,
        500,
    );
    state.compose(106, 20).expect("grown preview");
    let grown = hit(&state, "term_a").outer;
    assert_eq!(
        grown,
        Rect::new(a.outer.x, a.outer.y, surface.width - 10, surface.height - 3)
    );
}

#[test]
fn the_title_top_border_and_left_border_still_move_without_resizing() {
    let fixture = shell_with_floats(vec![float("term_a", 10, 3, "pip")]);
    let a = hit(&fixture, "term_a");
    let grabs = [
        ("title", (a.outer.x + 4, a.outer.y)),
        ("top-right corner", (a.outer.right() - 1, a.outer.y)),
        ("left border", (a.outer.x, a.outer.y + 2)),
    ];

    for (name, grab) in grabs {
        let mut state = shell_with_floats(vec![float("term_a", 10, 3, "pip")]);
        let preview = preview_border_drag(&mut state, grab, (grab.0 + 3, grab.1 + 1));
        assert_eq!(preview, Rect::new(13, 4, 12, 5), "{name} moves the float");

        let up = click(
            &mut state,
            MouseEventKind::Up(MouseButton::Left),
            grab.0 + 3,
            grab.1 + 1,
        );
        assert!(
            matches!(
                &endpoint_methods(&up)[..],
                [crate::api::schema::Method::FloatMove(params)] if (params.x, params.y) == (13, 4)
            ),
            "{name} commits a move only"
        );
    }
}

#[test]
fn releasing_a_resize_pins_the_top_left_then_commits_through_float_resize() {
    use crate::popup_size::PopupSize;

    let mut state = shell_with_floats(vec![float("term_a", 10, 3, "pip")]);
    let a = hit(&state, "term_a");
    let grab = (a.outer.right() - 1, a.outer.bottom() - 1);
    preview_border_drag(&mut state, grab, (grab.0 + 5, grab.1 + 2));

    let up = click(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        grab.0 + 5,
        grab.1 + 2,
    );
    let id = fid("term_a").to_string();
    assert!(matches!(
        &endpoint_methods(&up)[..],
        [
            crate::api::schema::Method::FloatMove(pin),
            crate::api::schema::Method::FloatResize(resize),
        ] if pin.float_id == id
            && (pin.x, pin.y) == (10, 3)
            && resize.float_id == id
            && resize.width == Some(PopupSize::Cells(17))
            && resize.height == Some(PopupSize::Cells(7))
    ));
    state.compose(106, 20).expect("held preview");
    assert_eq!(
        hit(&state, "term_a").outer,
        Rect::new(a.outer.x, a.outer.y, 17, 7),
        "the preview holds until the server confirms"
    );

    let mut resized = float("term_a", 10, 3, "pip");
    resized.outer.width = 17;
    resized.outer.height = 7;
    state.set_float_layer(FloatsSurface {
        floats: vec![resized],
    });
    assert!(state.floats.drag.is_none());
}

#[test]
fn clicking_outside_a_focused_float_reports_blur_and_passes_through() {
    let mut state = shell_with_floats(vec![float("term_a", 10, 3, "pip")]);
    state.floats.focused = Some(fid("term_a"));
    let pane = state.hits.panes[0].clone();
    let mut outcome = ClientShellInput::default();
    let consumed = state.handle_float_mouse(
        crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: pane.inner_rect.x,
            row: pane.inner_rect.y,
            modifiers: KeyModifiers::empty(),
        },
        &mut outcome,
    );
    assert!(!consumed);
    assert_eq!(float_blurs(&outcome), vec![fid("term_a")]);
    assert!(state.floats.focused.is_none());
}

#[test]
fn clicking_another_float_reports_only_the_previous_float_blur() {
    let mut state = shell_with_floats(vec![
        float("term_a", 10, 3, "a"),
        float("term_b", 40, 3, "b"),
    ]);
    state.floats.focused = Some(fid("term_a"));
    let b = hit(&state, "term_b");
    let down = click(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        b.pane.inner_rect.x,
        b.pane.inner_rect.y,
    );
    assert_eq!(float_blurs(&down), vec![fid("term_a")]);
    assert_eq!(state.floats.focused, Some(fid("term_b")));
}

#[test]
fn clicking_focused_float_content_or_border_never_reports_blur() {
    let mut state = shell_with_floats(vec![float("term_a", 10, 3, "pip")]);
    state.floats.focused = Some(fid("term_a"));
    let a = hit(&state, "term_a");
    let content = click(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        a.pane.inner_rect.x,
        a.pane.inner_rect.y,
    );
    assert!(float_blurs(&content).is_empty());
    assert_eq!(float_inputs(&content)[0].float_id, fid("term_a"));
    let border = click(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        a.outer.x + 4,
        a.outer.y,
    );
    assert!(float_blurs(&border).is_empty());
    assert!(state.floats.drag.is_some());
}

#[test]
fn yielding_float_focus_never_reports_blur() {
    let mut state = shell_with_floats(vec![float("term_a", 10, 3, "pip")]);
    state.floats.focused = Some(fid("term_a"));
    state.yield_float_focus();
    assert!(state.floats.focused.is_none());
    let outcome = state.handle_input_bytes(b"x");
    assert!(float_blurs(&outcome).is_empty());
    assert!(matches!(
        &outcome.requests[..],
        [ClientMessage::ClientShellPaneInput { .. }]
    ));
}

#[test]
fn clicking_outside_an_unfocused_float_never_reports_blur() {
    let mut state = shell_with_floats(vec![float("term_a", 10, 3, "pip")]);
    let pane = state.hits.panes[0].clone();
    let down = click(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        pane.inner_rect.x,
        pane.inner_rect.y,
    );
    assert!(float_blurs(&down).is_empty());
}

fn receive_float_focus(state: &mut ClientShellState, float_id: FloatId) -> ClientShellInput {
    let control = crate::client::endpoint::decode_endpoint_control(
        crate::protocol::floats::FOCUS_KIND,
        &serde_json::json!({"float_id": float_id.to_string()}).to_string(),
    )
    .unwrap();
    let crate::client::endpoint::EndpointControlMessage::FloatFocus(float_focus) = control else {
        panic!("expected float focus control");
    };
    state.set_float_focus(float_focus)
}

#[test]
fn float_focus_control_routes_typed_keys_to_the_float_and_raises_it() {
    let mut state = shell_with_floats(vec![
        float("term_a", 10, 3, "a"),
        float("term_b", 25, 3, "b"),
    ]);
    let outcome = receive_float_focus(&mut state, fid("term_a"));
    assert!(
        matches!(&endpoint_methods(&outcome)[..], [crate::api::schema::Method::FloatRaise(target)] if target.float_id == "f1")
    );
    let typed = state.handle_input_bytes(b"x");
    let inputs = float_inputs(&typed);
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0].float_id, fid("term_a"));
    assert!(matches!(
        &inputs[0].events[..],
        [ClientPaneInputEvent::Key {
            code: crate::protocol::ClientKeyCode::Char('x'),
            kind: crate::protocol::ClientKeyKind::Press,
            ..
        }]
    ));
}

#[test]
fn float_focus_control_before_the_layer_focuses_when_it_arrives() {
    let mut state = shell_with_floats(vec![]);
    let outcome = receive_float_focus(&mut state, fid("term_a"));
    assert!(float_inputs(&outcome).is_empty());
    assert!(float_inputs(&state.handle_input_bytes(b"x")).is_empty());
    let applied = state.set_float_layer(FloatsSurface {
        floats: vec![float("term_a", 10, 3, "a"), float("term_b", 25, 3, "b")],
    });
    assert!(
        matches!(&endpoint_methods(&applied)[..], [crate::api::schema::Method::FloatRaise(target)] if target.float_id == "f1")
    );
    assert_eq!(
        float_inputs(&state.handle_input_bytes(b"y"))[0].float_id,
        fid("term_a")
    );
    assert!(float_blurs(&applied).is_empty());
}

#[test]
fn float_focus_control_is_dropped_on_the_next_layer_without_the_float() {
    let mut state = shell_with_floats(vec![]);
    receive_float_focus(&mut state, fid("term_a"));
    state.set_float_layer(FloatsSurface::default());
    state.set_float_layer(FloatsSurface {
        floats: vec![float("term_a", 10, 3, "a")],
    });
    let typed = state.handle_input_bytes(b"x");
    assert!(float_inputs(&typed).is_empty());
    assert!(
        matches!(&typed.requests[..], [ClientMessage::ClientShellPaneInput { pane_id, .. }] if pane_id == "pane_1")
    );
}

#[test]
fn float_focus_control_never_blurs_the_previously_focused_float() {
    let mut state = shell_with_floats(vec![
        float("term_a", 10, 3, "a"),
        float("term_b", 25, 3, "b"),
    ]);
    receive_float_focus(&mut state, fid("term_a"));
    state.handle_input_bytes(b"x");
    let outcome = receive_float_focus(&mut state, fid("term_b"));
    assert!(float_blurs(&outcome).is_empty());
    assert_eq!(
        float_inputs(&state.handle_input_bytes(b"y"))[0].float_id,
        fid("term_b")
    );
}

fn manifest_button(id: &str, icon: &str) -> crate::api::schema::PluginManifestButton {
    crate::api::schema::PluginManifestButton {
        id: id.to_owned(),
        icon: icon.to_owned(),
        action: format!("{id}-action"),
    }
}

fn buttons_entry(
    float_id: FloatId,
    plugin_id: &str,
    controls_revision: &str,
    buttons: Vec<crate::api::schema::PluginManifestButton>,
) -> crate::protocol::endpoint::EndpointFloatButtonSet {
    crate::protocol::endpoint::EndpointFloatButtonSet {
        float_id,
        workspace_id: crate::api::schema::RequiredNullableString(None),
        plugin_id: plugin_id.to_owned(),
        entrypoint: "shell".to_owned(),
        controls_revision: controls_revision.to_owned(),
        buttons,
    }
}

fn negotiated_shell(
    floats: Vec<FloatSurface>,
    entries: Vec<crate::protocol::endpoint::EndpointFloatButtonSet>,
) -> ClientShellState {
    let mut state = shell_with_floats(floats);
    let boot = state.snapshot.as_ref().unwrap().boot_id.clone();
    state.set_float_buttons(
        &boot,
        Some(crate::protocol::endpoint::EndpointFloatButtons {
            boot_id: boot.clone(),
            revision: 1,
            floats: entries,
        }),
    );
    state.compose(106, 20).expect("composed frame");
    state
}

fn button_icon(hit: &super::super::floats::FloatHit, button_id: &str) -> (u16, u16) {
    let (_, rect) = hit
        .buttons
        .iter()
        .find(|(id, _)| id == button_id)
        .expect("rendered button");
    (rect.x + 1, rect.y)
}

#[test]
fn buttons_render_right_aligned_and_the_hits_match_what_is_drawn() {
    let mut state = negotiated_shell(
        vec![float("term_a", 1, 1, "float")],
        vec![buttons_entry(
            fid("term_a"),
            "plugin-a",
            "rev-1",
            vec![
                manifest_button("inspect", "?"),
                manifest_button("close", "x"),
            ],
        )],
    );
    let a = hit(&state, "term_a");
    let row = a.outer.y;
    // Two one-cell buttons take 6 cells inside the 10-cell interior.
    assert_eq!(
        a.buttons,
        vec![
            (
                "inspect".to_owned(),
                Rect::new(a.outer.right() - 7, row, 3, 1)
            ),
            (
                "close".to_owned(),
                Rect::new(a.outer.right() - 4, row, 3, 1)
            ),
        ]
    );
    let frame = state.compose(106, 20).unwrap().to_ratatui_buffer().unwrap();
    assert_eq!(frame[(a.outer.right() - 6, row)].symbol(), "?");
    assert_eq!(frame[(a.outer.right() - 3, row)].symbol(), "x");
    assert_eq!(frame[(a.outer.right() - 7, row)].symbol(), " ");
    assert_eq!(frame[(a.outer.right() - 5, row)].symbol(), " ");
    assert_eq!(frame[(a.outer.right() - 2, row)].symbol(), " ");
    assert_eq!(frame[(a.outer.right() - 1, row)].symbol(), "┐");
    // The title yields its cells to the buttons and stops before the group.
    assert_eq!(frame[(a.outer.x + 1, row)].symbol(), "t");
    assert_eq!(frame[(a.outer.x + 4, row)].symbol(), "m");
    // The group's left padding cell follows the title directly.
    assert_eq!(frame[(a.outer.x + 5, row)].symbol(), " ");
}

#[test]
fn a_button_click_invokes_exactly_the_clicked_float_and_button() {
    let mut state = negotiated_shell(
        vec![float("term_a", 1, 1, "float")],
        vec![buttons_entry(
            fid("term_a"),
            "plugin-a",
            "rev-1",
            vec![manifest_button("close", "x")],
        )],
    );
    let a = hit(&state, "term_a");
    let icon = button_icon(&a, "close");

    let down = click(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        icon.0,
        icon.1,
    );
    assert!(endpoint_methods(&down).is_empty());
    assert!(float_inputs(&down).is_empty());
    assert!(state.floats.focused.is_none());
    assert!(state.floats.drag.is_none());

    let up = click(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        icon.0,
        icon.1,
    );
    let [crate::api::schema::Method::FloatButtonInvoke(params)] = &endpoint_methods(&up)[..] else {
        panic!("release invokes the button action");
    };
    assert_eq!(params.boot_id, "boot-1");
    assert_eq!(params.float_id, "f1");
    assert!(params.workspace_id.0.is_none());
    assert_eq!(params.plugin_id, "plugin-a");
    assert_eq!(params.entrypoint, "shell");
    assert_eq!(params.controls_revision, "rev-1");
    assert_eq!(params.button_id, "close");
    assert!(state.floats.button_press.is_none());
}

#[test]
fn a_button_press_that_leaves_the_button_never_activates() {
    let mut state = negotiated_shell(
        vec![float("term_a", 1, 1, "float")],
        vec![buttons_entry(
            fid("term_a"),
            "plugin-a",
            "rev-1",
            vec![manifest_button("close", "x")],
        )],
    );
    let a = hit(&state, "term_a");
    let icon = button_icon(&a, "close");

    click(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        icon.0,
        icon.1,
    );
    let drag = click(
        &mut state,
        MouseEventKind::Drag(MouseButton::Left),
        icon.0 + 5,
        icon.1,
    );
    let up = click(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        icon.0 + 5,
        icon.1,
    );

    assert!(endpoint_methods(&drag).is_empty());
    assert!(endpoint_methods(&up).is_empty());
    // The cancelled gesture keeps its release: nothing reaches the pane beneath.
    assert!(float_inputs(&up).is_empty());
    assert!(!endpoint_methods(&up)
        .iter()
        .any(|method| { matches!(method, crate::api::schema::Method::PaneFocus(_)) }));
    assert!(state.floats.button_press.is_none());
}

#[test]
fn non_left_input_on_a_button_cell_is_consumed_without_activation_or_focus() {
    let mut state = negotiated_shell(
        vec![float("term_a", 1, 1, "float")],
        vec![buttons_entry(
            fid("term_a"),
            "plugin-a",
            "rev-1",
            vec![manifest_button("close", "x")],
        )],
    );
    let a = hit(&state, "term_a");
    let icon = button_icon(&a, "close");

    let down = click(
        &mut state,
        MouseEventKind::Down(MouseButton::Right),
        icon.0,
        icon.1,
    );
    assert!(endpoint_methods(&down).is_empty());
    assert!(state.floats.button_press.is_none());
    assert!(state.floats.focused.is_none());
    let up = click(
        &mut state,
        MouseEventKind::Up(MouseButton::Right),
        icon.0,
        icon.1,
    );
    assert!(endpoint_methods(&up).is_empty());
    let wheel = click(&mut state, MouseEventKind::ScrollDown, icon.0, icon.1);
    assert!(endpoint_methods(&wheel).is_empty());
    assert!(float_inputs(&wheel).is_empty());
}

#[test]
fn stacked_floats_route_button_clicks_to_the_topmost_float_at_the_pointer() {
    let mut state = negotiated_shell(
        vec![
            float("term_a", 1, 1, "under"),
            float("term_b", 2, 2, "over"),
        ],
        vec![
            buttons_entry(
                fid("term_a"),
                "plugin-a",
                "rev-a",
                vec![manifest_button("close", "x")],
            ),
            buttons_entry(
                fid("term_b"),
                "plugin-b",
                "rev-b",
                vec![manifest_button("close", "x")],
            ),
        ],
    );
    let a = hit(&state, "term_a");
    let b = hit(&state, "term_b");
    // term_b covers term_a's body but not term_a's title row.
    let under_icon = (a.outer.right() - 3, a.outer.y);
    let over_icon = (b.outer.right() - 3, b.outer.y);

    click(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        under_icon.0,
        under_icon.1,
    );
    let under_up = click(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        under_icon.0,
        under_icon.1,
    );
    let [crate::api::schema::Method::FloatButtonInvoke(under)] = &endpoint_methods(&under_up)[..]
    else {
        panic!("the visible button of the lower float stays clickable");
    };
    assert_eq!(under.float_id, "f1");
    assert_eq!(under.plugin_id, "plugin-a");

    click(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        over_icon.0,
        over_icon.1,
    );
    let over_up = click(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        over_icon.0,
        over_icon.1,
    );
    let [crate::api::schema::Method::FloatButtonInvoke(over)] = &endpoint_methods(&over_up)[..]
    else {
        panic!("the top float owns the click inside its own button");
    };
    assert_eq!(over.float_id, "f2");
    assert_eq!(over.plugin_id, "plugin-b");
}

#[test]
fn a_geometry_change_cancels_an_armed_button_press() {
    let mut state = negotiated_shell(
        vec![float("term_a", 1, 1, "float")],
        vec![buttons_entry(
            fid("term_a"),
            "plugin-a",
            "rev-1",
            vec![manifest_button("close", "x")],
        )],
    );
    let a = hit(&state, "term_a");
    let icon = button_icon(&a, "close");
    click(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        icon.0,
        icon.1,
    );

    state.set_float_layer(FloatsSurface {
        floats: vec![float("term_a", 3, 3, "float")],
    });
    state.compose(106, 20).expect("composed frame");
    let moved = hit(&state, "term_a");
    let moved_icon = button_icon(&moved, "close");

    // Neither the old nor the new button position activates the cancelled press.
    let up_old = click(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        icon.0,
        icon.1,
    );
    assert!(endpoint_methods(&up_old).is_empty());
    click(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        moved_icon.0,
        moved_icon.1,
    );
    let up_new = click(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        moved_icon.0,
        moved_icon.1,
    );
    let [crate::api::schema::Method::FloatButtonInvoke(params)] = &endpoint_methods(&up_new)[..]
    else {
        panic!("a fresh press on the moved button activates");
    };
    assert_eq!(params.float_id, "f1");
}

#[test]
fn a_metadata_revision_change_cancels_an_armed_button_press() {
    let mut state = negotiated_shell(
        vec![float("term_a", 1, 1, "float")],
        vec![buttons_entry(
            fid("term_a"),
            "plugin-a",
            "rev-1",
            vec![manifest_button("close", "x")],
        )],
    );
    let a = hit(&state, "term_a");
    let icon = button_icon(&a, "close");
    click(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        icon.0,
        icon.1,
    );

    let boot = state.snapshot.as_ref().unwrap().boot_id.clone();
    state.set_float_buttons(
        &boot,
        Some(crate::protocol::endpoint::EndpointFloatButtons {
            boot_id: boot.clone(),
            revision: 2,
            floats: vec![buttons_entry(
                fid("term_a"),
                "plugin-a",
                "rev-2",
                vec![manifest_button("close", "x")],
            )],
        }),
    );
    state.compose(106, 20).expect("composed frame");

    let up = click(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        icon.0,
        icon.1,
    );
    assert!(endpoint_methods(&up).is_empty());
    assert!(state.floats.button_press.is_none());
}

#[test]
fn floats_without_buttons_keep_their_existing_appearance_and_gestures() {
    for metadata_buttons in [None, Some(Vec::new())] {
        let entries = metadata_buttons
            .map(|buttons| vec![buttons_entry(fid("term_a"), "plugin-a", "rev-1", buttons)])
            .unwrap_or_default();
        let mut state = negotiated_shell(vec![float("term_a", 10, 3, "pip")], entries);
        let a = hit(&state, "term_a");
        assert!(a.buttons.is_empty());
        let row = a.outer.y;
        let frame = state.compose(106, 20).unwrap().to_ratatui_buffer().unwrap();
        // The title keeps the whole interior, truncated as before.
        let title: String = (a.outer.x + 1..a.outer.right() - 1)
            .map(|col| frame[(col, row)].symbol())
            .collect();
        assert_eq!(title, "term_a tit");
        // The border click still starts a move gesture.
        click(
            &mut state,
            MouseEventKind::Down(MouseButton::Left),
            a.outer.x + 4,
            row,
        );
        assert!(state.floats.drag.is_some());
    }
}
