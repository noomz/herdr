//! Optional floating-terminal layer for client-owned shells.
//!
//! Floats travel beside the frozen v1 pane surface instead of inside it. A client opts in
//! with `EndpointClientHello::floats`; servers advertise [`CAPABILITY`]. Clients without
//! the capability never receive [`SURFACE_KIND`], and old clients ignore unknown kinds.

use base64::{engine::general_purpose::STANDARD_NO_PAD, Engine as _};
use serde::{Deserialize, Serialize};

use super::{ClientMessage, ClientPaneInputEvent, FrameData, ServerMessage, SurfaceRect};

pub(crate) const CAPABILITY: &str = "floats";
pub(crate) const SURFACE_KIND: &str = "endpoint.floats.v1";
pub(crate) const INPUT_KIND: &str = "endpoint.float-input.v1";

/// Every float visible to one client, bottom to top. An empty list clears the layer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FloatsSurface {
    pub floats: Vec<FloatSurface>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FloatSurface {
    pub float_id: String,
    pub title: String,
    /// Bordered rect relative to the pane surface's top-left cell.
    pub outer: SurfaceRect,
    /// Terminal content; it starts one cell inside `outer`.
    pub frame: FrameData,
    pub mouse_reporting: bool,
    pub sgr_pixel_mouse: bool,
    pub pixel_width: u32,
    pub pixel_height: u32,
}

/// Client-classified input for one float, the float counterpart of `ClientShellPopupInput`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FloatInput {
    pub float_id: String,
    pub events: Vec<ClientPaneInputEvent>,
}

pub(crate) fn surface_message(surface: &FloatsSurface) -> Result<ServerMessage, String> {
    let bytes = bincode::serde::encode_to_vec(surface, bincode::config::standard())
        .map_err(|error| error.to_string())?;
    Ok(ServerMessage::EndpointControl {
        kind: SURFACE_KIND.into(),
        data: STANDARD_NO_PAD.encode(bytes),
    })
}

pub(crate) fn decode_surface(data: &str) -> Result<FloatsSurface, String> {
    if data.len() > super::MAX_FRAME_SIZE {
        return Err("float surface exceeds the frame limit".into());
    }
    let bytes = STANDARD_NO_PAD
        .decode(data)
        .map_err(|error| error.to_string())?;
    let (surface, consumed): (FloatsSurface, usize) = bincode::serde::decode_from_slice(
        &bytes,
        bincode::config::standard().with_limit::<{ super::MAX_FRAME_SIZE }>(),
    )
    .map_err(|error| error.to_string())?;
    if consumed != bytes.len() {
        return Err("trailing float surface bytes".into());
    }
    let grids_match = surface.floats.iter().all(|float| {
        usize::from(float.frame.width) * usize::from(float.frame.height) == float.frame.cells.len()
    });
    if !grids_match {
        return Err("float frame does not match its dimensions".into());
    }
    Ok(surface)
}

pub(crate) fn input_message(input: &FloatInput) -> Result<ClientMessage, String> {
    Ok(ClientMessage::EndpointControl {
        kind: INPUT_KIND.into(),
        data: serde_json::to_string(input).map_err(|error| error.to_string())?,
    })
}

pub(crate) fn decode_input(data: &str) -> Result<FloatInput, String> {
    serde_json::from_str(data).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{ClientKeyCode, ClientKeyKind};
    use ratatui::{buffer::Buffer, layout::Rect};

    fn float(id: &str, x: u16) -> FloatSurface {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 4, 2));
        buffer[(1, 1)].set_symbol("x");
        FloatSurface {
            float_id: id.into(),
            title: "peek".into(),
            outer: SurfaceRect {
                x,
                y: 3,
                width: 7,
                height: 4,
            },
            frame: FrameData::from_ratatui_buffer(&buffer, None),
            mouse_reporting: true,
            sgr_pixel_mouse: false,
            pixel_width: 32,
            pixel_height: 32,
        }
    }

    #[test]
    fn surface_round_trips_through_its_named_control() {
        let surface = FloatsSurface {
            floats: vec![float("term_a", 1), float("term_b", 9)],
        };

        let ServerMessage::EndpointControl { kind, data } = surface_message(&surface).unwrap()
        else {
            panic!("floats travel as endpoint control");
        };

        assert_eq!(kind, SURFACE_KIND);
        assert_eq!(decode_surface(&data).unwrap(), surface);
    }

    #[test]
    fn empty_surface_round_trips_to_clear_the_layer() {
        let ServerMessage::EndpointControl { data, .. } =
            surface_message(&FloatsSurface::default()).unwrap()
        else {
            panic!("floats travel as endpoint control");
        };

        assert!(decode_surface(&data).unwrap().floats.is_empty());
    }

    #[test]
    fn decode_rejects_frames_whose_cells_do_not_match_their_size() {
        let mut bad = float("term_a", 1);
        bad.frame.width = 9;
        let ServerMessage::EndpointControl { data, .. } =
            surface_message(&FloatsSurface { floats: vec![bad] }).unwrap()
        else {
            panic!("floats travel as endpoint control");
        };

        assert!(decode_surface(&data).is_err());
        assert!(decode_surface("!!not base64").is_err());
    }

    #[test]
    fn input_round_trips_as_json_control() {
        let input = FloatInput {
            float_id: "term_a".into(),
            events: vec![ClientPaneInputEvent::Key {
                code: ClientKeyCode::Char('q'),
                modifiers: 0,
                kind: ClientKeyKind::Press,
                repeat_count: 1,
                shifted_codepoint: None,
                generated_text: Some("q".into()),
                tracks_release: false,
                physical_key_id: None,
                windows_record: None,
            }],
        };

        let ClientMessage::EndpointControl { kind, data } = input_message(&input).unwrap() else {
            panic!("float input travels as endpoint control");
        };

        assert_eq!(kind, INPUT_KIND);
        assert_eq!(decode_input(&data).unwrap(), input);
    }
}
