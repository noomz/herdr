//! Overlay (float and popup) images delivered end to end: server scene, native upload, client
//! shell, host terminal.
#![cfg(unix)]

use super::*;

use crate::app::floats::{FloatEdit, FloatSpec, FloatVisibility};
use crate::client::{ClientShellConfig, ClientShellState};
use crate::popup_size::{FloatPosition, PopupSize};
use std::collections::HashMap;

const IMAGE_ID: u32 = 7;
const CLIENT: u64 = 7;
const COLS: u16 = 80;
const ROWS: u16 = 24;
/// The float's inner area in cells; the image covers all of it but the header row.
const FLOAT_COLS: u16 = 18;
const FLOAT_ROWS: u16 = 6;
const CELL: (u32, u32) = (10, 20);

fn read_server_messages(bytes: Vec<u8>) -> Vec<ServerMessage> {
    let len = bytes.len() as u64;
    let mut cursor = std::io::Cursor::new(bytes);
    let mut messages = Vec::new();
    while cursor.position() < len {
        messages.push(protocol::read_message(&mut cursor, MAX_FRAME_SIZE).expect("decode message"));
    }
    messages
}

fn image_size() -> (u32, u32) {
    (
        u32::from(FLOAT_COLS) * CELL.0,
        u32::from(FLOAT_ROWS - 1) * CELL.1,
    )
}

/// An RGBA frame whose pixels name the frame number.
fn frame_pixels(frame: u8) -> Vec<u8> {
    let (width, height) = image_size();
    [frame, 7, 9, 255].repeat((width * height) as usize)
}

/// The spike's per-frame output: home the cursor under the header, transmit and place one
/// image under a fixed id in 4 KiB chunks, then redraw the header row.
fn frame_output(frame: u8) -> Vec<Vec<u8>> {
    let (width, height) = image_size();
    let payload = base64::engine::general_purpose::STANDARD.encode(frame_pixels(frame));
    let parts = payload.as_bytes().chunks(4096).collect::<Vec<_>>();
    let mut chunks = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        let mut chunk = Vec::new();
        if index == 0 {
            chunk.extend_from_slice(b"\x1b[2;1H");
        }
        let last = index + 1 == parts.len();
        let control = if index == 0 {
            format!(
                "a=T,i={IMAGE_ID},p=1,q=2,f=32,s={width},v={height},c={FLOAT_COLS},r={},C=1,m={}",
                FLOAT_ROWS - 1,
                u8::from(!last)
            )
        } else {
            format!("m={}", u8::from(!last))
        };
        chunk.extend_from_slice(format!("\x1b_G{control};").as_bytes());
        chunk.extend_from_slice(part);
        chunk.extend_from_slice(b"\x1b\\");
        if last {
            chunk.extend_from_slice(format!("\x1b[1;1H\x1b[7m frame {frame} \x1b[0m").as_bytes());
        }
        chunks.push(chunk);
    }
    chunks
}

/// What a kitty terminal would hold after writing the client's output: image pixels by id,
/// and the image each placement shows.
#[derive(Default)]
struct HostTerminal {
    images: HashMap<u32, Vec<u8>>,
    /// The image a chunked transmit is filling; continuation chunks carry no keys.
    loading: Option<(u32, Option<u32>, Vec<u8>)>,
    /// Placements by (image id, placement id).
    placements: Vec<(u32, u32)>,
}

impl HostTerminal {
    fn shows(&self, pixels: &[u8]) -> bool {
        self.placements
            .iter()
            .any(|(id, _)| self.images.get(id).is_some_and(|image| image == pixels))
    }

    fn accept_native(&mut self, image_id: u32, path: &str) {
        self.images
            .insert(image_id, std::fs::read(path).expect("native export"));
    }

    fn write(&mut self, bytes: &[u8]) {
        let text = String::from_utf8_lossy(bytes);
        let mut rest = text.as_ref();
        while let Some(start) = rest.find("\x1b_G") {
            let body = &rest[start + 3..];
            let Some(end) = body.find("\x1b\\") else {
                break;
            };
            self.apc(&body[..end]);
            rest = &body[end + 2..];
        }
    }

    fn apc(&mut self, command: &str) {
        let (control, payload) = command.split_once(';').unwrap_or((command, ""));
        let keys: HashMap<&str, &str> = control
            .split(',')
            .filter_map(|pair| pair.split_once('='))
            .collect();
        let key = |name: &str| keys.get(name).and_then(|value| value.parse::<u32>().ok());
        let id = key("i");
        let placement = key("p").unwrap_or(0);
        let more = keys.get("m") == Some(&"1");
        let action = keys.get("a").copied();
        let transmit = match (self.loading.take(), action) {
            (Some(loading), None | Some("t") | Some("T")) => Some(loading),
            (None, Some(action @ ("t" | "T"))) => Some((
                id.expect("transmit names an image"),
                (action == "T").then_some(placement),
                Vec::new(),
            )),
            (loading, _) => {
                self.loading = loading;
                None
            }
        };
        if let Some((id, place, mut data)) = transmit {
            data.extend(
                base64::engine::general_purpose::STANDARD
                    .decode(payload)
                    .expect("base64 payload"),
            );
            if more {
                self.loading = Some((id, place, data));
                return;
            }
            self.images.insert(id, data);
            if let Some(placement) = place {
                self.placements.push((id, placement));
            }
            return;
        }
        match action {
            Some("p") => self
                .placements
                .push((id.expect("placement names an image"), placement)),
            Some("d") => match (keys.get("d").copied(), id) {
                (Some("i"), Some(id)) if placement != 0 => {
                    self.placements.retain(|p| *p != (id, placement));
                }
                (Some("i"), Some(id)) => self.placements.retain(|(image, _)| *image != id),
                (Some("I"), Some(id)) => {
                    self.placements.retain(|(image, _)| *image != id);
                    self.images.remove(&id);
                }
                (Some("A"), _) => {
                    self.placements.clear();
                    self.images.clear();
                }
                (_, _) => self.placements.clear(),
            },
            _ => {}
        }
    }
}

struct Harness {
    server: HeadlessServer,
    shell: ClientShellState,
    host: HostTerminal,
    control_rx: std::sync::mpsc::Receiver<Vec<u8>>,
    render_rx: std::sync::mpsc::Receiver<Vec<u8>>,
    /// The test answers uploads itself instead of the harness settling them.
    hold_acks: bool,
    /// Transfers the host has accepted that the shell has not been told about yet.
    pending_acks: Vec<(u64, u32, protocol::SurfaceGraphicsAssetKey)>,
    native_uploads: usize,
    rejected_uploads: usize,
}

impl Harness {
    fn connect(mut server: HeadlessServer) -> Self {
        let (writer, control_rx, render_rx) = test_client_writer();
        assert!(
            server.handle_server_event(ServerEvent::ClientShellConnected {
                surface_reuse: false,
                surface_delta: false,
                surface_scroll: false,
                floats: true,
                client_id: CLIENT,
                surface_cols: COLS,
                surface_rows: ROWS,
                cell_width_px: CELL.0,
                cell_height_px: CELL.1,
                pixel_mouse: true,
                direct_graphics: true,
                endpoint_keybindings: false,
                mouse_capture: false,
                surface_active: true,
                writer,
            })
        );
        let mut shell = ClientShellState::new(ClientShellConfig::from_config(
            &crate::config::Config::default(),
        ));
        shell.set_graphics_cell_size(CELL.0, CELL.1);
        Self {
            server,
            shell,
            host: HostTerminal::default(),
            control_rx,
            render_rx,
            hold_acks: false,
            pending_acks: Vec::new(),
            native_uploads: 0,
            rejected_uploads: 0,
        }
    }

    fn compose(&mut self) {
        if let Some(frame) = self.shell.compose(COLS, ROWS) {
            self.host.write(&frame.graphics.into_inline_bytes());
        }
    }

    fn drain_control(&mut self) {
        while let Ok(bytes) = self.control_rx.try_recv() {
            if let ServerMessage::EndpointControl { kind, data } = read_server_message(bytes) {
                if kind == protocol::endpoint::ENDPOINT_SNAPSHOT_KIND {
                    let snapshot: Box<protocol::ClientShellSnapshot> =
                        serde_json::from_str(&data).expect("snapshot");
                    self.shell.set_snapshot(snapshot);
                    self.compose();
                }
            }
        }
    }

    /// Delivers one render buffer the way the client loop does.
    fn deliver(&mut self, bytes: Vec<u8>) {
        for message in read_server_messages(bytes) {
            match message {
                ServerMessage::PaneSurface(surface) => {
                    self.shell.set_pane_surface(surface);
                    self.compose();
                }
                ServerMessage::EndpointControl { kind, data }
                    if kind == protocol::floats::SURFACE_KIND =>
                {
                    self.shell
                        .set_float_layer(protocol::floats::decode_surface(&data).expect("layer"));
                    self.compose();
                }
                ServerMessage::GraphicsFile {
                    path,
                    image_id,
                    transfer_id,
                    surface_asset,
                    ..
                } => {
                    self.native_uploads += 1;
                    let asset = surface_asset.expect("surface upload");
                    if self.shell.accepts_direct_graphics_asset(&asset, image_id) {
                        self.server.native_started(CLIENT, transfer_id, image_id);
                        self.host.accept_native(image_id, &path);
                        self.pending_acks.push((transfer_id, image_id, asset));
                    } else {
                        self.rejected_uploads += 1;
                        self.server
                            .native_result(CLIENT, transfer_id, image_id, false);
                    }
                }
                _ => {}
            }
        }
    }

    /// The host answered the upload; the shell settles it like a `DirectGraphicsResponse`.
    fn answer_uploads(&mut self) {
        for (transfer_id, image_id, asset) in std::mem::take(&mut self.pending_acks) {
            let accepted = self.shell.accepts_direct_graphics_asset(&asset, image_id)
                && self.shell.trust_direct_graphics_asset(&asset, image_id);
            if accepted {
                self.compose();
            } else {
                self.rejected_uploads += 1;
            }
            self.server
                .native_result(CLIENT, transfer_id, image_id, accepted);
        }
    }

    fn render_once(&mut self) -> bool {
        self.server.render_and_stream();
        self.drain_control();
        let mut received = false;
        while let Ok(bytes) = self.render_rx.try_recv() {
            received = true;
            self.deliver(bytes);
        }
        self.drain_control();
        if !self.hold_acks {
            self.answer_uploads();
        }
        received
    }

    /// Renders until the server has nothing more to send for the current state.
    fn settle(&mut self) {
        for _ in 0..24 {
            let received = self.render_once();
            let deferred = self.server.clients[&CLIENT].deferred_render() != DeferredRender::None;
            if !received && !deferred && (self.hold_acks || self.pending_acks.is_empty()) {
                return;
            }
        }
        panic!("server never settled");
    }
}

fn float_runtime<'a>(
    server: &'a HeadlessServer,
    terminal_id: &crate::terminal::TerminalId,
) -> &'a crate::terminal::TerminalRuntime {
    server
        .app
        .terminal_runtimes
        .get(terminal_id)
        .expect("float runtime")
}

fn install_float(server: &mut HeadlessServer) -> crate::terminal::TerminalId {
    let (runtime, _input) =
        crate::terminal::TerminalRuntime::test_with_channel_and_scrollback_bytes(
            FLOAT_COLS, FLOAT_ROWS, 0, b"", 4,
        );
    let (_, terminal_id, _) = server.app.install_test_float_runtime(
        runtime,
        FloatSpec {
            width: Some(PopupSize::Cells(FLOAT_COLS + 2)),
            height: Some(PopupSize::Cells(FLOAT_ROWS + 2)),
            position: FloatPosition::At { x: 50, y: 2 },
            hide_while_focused: None,
        },
    );
    terminal_id
}

/// The modal popup is the other overlay terminal whose images reach the client.
fn install_popup(server: &mut HeadlessServer) -> crate::terminal::TerminalId {
    let (runtime, _input) =
        crate::terminal::TerminalRuntime::test_with_channel_and_scrollback_bytes(
            FLOAT_COLS, FLOAT_ROWS, 0, b"", 4,
        );
    let (_, terminal_id) = server.app.install_test_popup_runtime(runtime);
    terminal_id
}

/// Streams whole frames into the overlay terminal, settling after each one.
fn stream(frames: std::ops::RangeInclusive<u8>) -> (Harness, crate::terminal::TerminalId) {
    let mut server = test_headless_server();
    let _pane_input = install_focused_test_runtime(&mut server, b"base-pane");
    let terminal_id = install_float(&mut server);
    let mut harness = Harness::connect(server);
    harness.settle();
    for frame in frames {
        for chunk in frame_output(frame) {
            float_runtime(&harness.server, &terminal_id).test_process_pty_bytes(&chunk);
        }
        harness.settle();
        assert!(
            harness.host.shows(&frame_pixels(frame)),
            "frame {frame} never reached the host; uploads={} rejected={} resident={:?} placements={:?}",
            harness.native_uploads,
            harness.rejected_uploads,
            harness.host.images.keys().collect::<Vec<_>>(),
            harness.host.placements,
        );
    }
    (harness, terminal_id)
}

/// The spike's steady state on a loaded host: the host answers a frame's upload only after
/// the next frame has started arriving, and the server renders between that frame's chunks.
fn frames_acked_after_the_next_frame_starts_stay_visible(
    mut server: HeadlessServer,
    terminal_id: &crate::terminal::TerminalId,
) {
    let _pane_input = install_focused_test_runtime(&mut server, b"base-pane");
    let mut harness = Harness::connect(server);
    harness.hold_acks = true;
    harness.settle();

    let mut missing = Vec::new();
    for frame in 1..=6u8 {
        for (index, chunk) in frame_output(frame).into_iter().enumerate() {
            float_runtime(&harness.server, terminal_id).test_process_pty_bytes(&chunk);
            if index == 0 {
                harness.render_once();
                harness.answer_uploads();
                harness.render_once();
                if frame > 1 && !harness.host.shows(&frame_pixels(frame - 1)) {
                    missing.push(frame - 1);
                }
            }
        }
        harness.settle();
    }
    harness.answer_uploads();
    harness.settle();
    assert!(
        harness.host.shows(&frame_pixels(6)),
        "the last frame never reached the host; resident={:?} placements={:?}",
        harness.host.images.keys().collect::<Vec<_>>(),
        harness.host.placements
    );
    assert!(
        missing.is_empty(),
        "frames {missing:?} were not visible once their upload was acknowledged; uploads={} rejected={}",
        harness.native_uploads,
        harness.rejected_uploads
    );
    shutdown_test_runtimes(&mut harness.server);
}

#[tokio::test]
async fn a_float_frame_acked_after_the_next_frame_starts_stays_visible() {
    let mut server = test_headless_server();
    let terminal_id = install_float(&mut server);
    frames_acked_after_the_next_frame_starts_stay_visible(server, &terminal_id);
}

#[tokio::test]
async fn a_popup_frame_acked_after_the_next_frame_starts_stays_visible() {
    let mut server = test_headless_server();
    let terminal_id = install_popup(&mut server);
    frames_acked_after_the_next_frame_starts_stay_visible(server, &terminal_id);
}

#[tokio::test]
async fn a_float_streaming_frames_uses_native_uploads_when_negotiated() {
    let (mut harness, _terminal_id) = stream(1..=4);
    assert!(
        harness.native_uploads >= 4,
        "uploads={}",
        harness.native_uploads
    );
    assert_eq!(harness.rejected_uploads, 0);
    shutdown_test_runtimes(&mut harness.server);
}

#[tokio::test]
async fn a_float_hidden_and_shown_while_streaming_shows_the_next_frame() {
    let (mut harness, terminal_id) = stream(1..=2);
    let float_id = harness.server.app.state.floats[0].id;

    harness
        .server
        .app
        .edit_float(float_id, FloatEdit::Visibility(FloatVisibility::Hidden));
    harness.settle();
    for chunk in frame_output(3) {
        float_runtime(&harness.server, &terminal_id).test_process_pty_bytes(&chunk);
    }
    harness
        .server
        .app
        .edit_float(float_id, FloatEdit::Visibility(FloatVisibility::Shown));
    harness.settle();
    for frame in 4..=6u8 {
        for chunk in frame_output(frame) {
            float_runtime(&harness.server, &terminal_id).test_process_pty_bytes(&chunk);
        }
        harness.settle();
        assert!(
            harness.host.shows(&frame_pixels(frame)),
            "frame {frame} never reached the host terminal after show"
        );
    }
    shutdown_test_runtimes(&mut harness.server);
}
