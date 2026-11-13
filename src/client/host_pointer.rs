//! Host pointer shapes over the kitty OSC 22 protocol.
//!
//! Herdr overrides the host pointer only over its own float chrome, and only with shapes the
//! host confirmed in the startup support query. It pushes at most one shape onto the host's
//! pointer stack and pops it again, so the host restores whatever shape was current before.
//! Herdr never learns or replays that shape, and it does not project the OSC 22 requests of
//! applications running in panes.

use std::io::{self, Write as _};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

/// The shapes Herdr can request. Their order is the order of the startup support query.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HostPointerShape {
    Pointer,
    Move,
    EwResize,
    NsResize,
    NwseResize,
}

impl HostPointerShape {
    const ALL: [Self; 5] = [
        Self::Pointer,
        Self::Move,
        Self::EwResize,
        Self::NsResize,
        Self::NwseResize,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Pointer => "pointer",
            Self::Move => "move",
            Self::EwResize => "ew-resize",
            Self::NsResize => "ns-resize",
            Self::NwseResize => "nwse-resize",
        }
    }

    fn bit(self) -> u8 {
        1 << (self as u8)
    }

    /// Nonzero code shared with the restore path; zero means no override.
    fn code(self) -> u8 {
        self as u8 + 1
    }

    fn from_code(code: u8) -> Option<Self> {
        Self::ALL.get(usize::from(code.checked_sub(1)?)).copied()
    }
}

/// Asks the host which of [`HostPointerShape::ALL`] it supports. A kitty-protocol host answers
/// `OSC 22 ; 1,0,... ST` with one digit per name, in order, before any later query's reply.
pub(super) const SUPPORT_QUERY: &[u8] =
    b"\x1b]22;?pointer,move,ew-resize,ns-resize,nwse-resize\x1b\\";

/// The reply prefix to [`SUPPORT_QUERY`].
pub(super) const REPLY_PREFIX: &[u8] = b"\x1b]22;";

const POP: &[u8] = b"\x1b]22;<\x1b\\";

/// The shapes the host confirmed. Empty for a host that did not answer, which disables every
/// pointer write.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct HostPointerSupport(u8);

impl HostPointerSupport {
    /// Parses the body of a support reply, between [`REPLY_PREFIX`] and its terminator. Only an
    /// answer with exactly one `0` or `1` per queried name is a reply to Herdr's query.
    pub(super) fn from_reply(body: &[u8]) -> Option<Self> {
        let mut answers = body.split(|byte| *byte == b',');
        let mut support = 0;
        for shape in HostPointerShape::ALL {
            match answers.next()? {
                b"1" => support |= shape.bit(),
                b"0" => {}
                _ => return None,
            }
        }
        answers.next().is_none().then_some(Self(support))
    }

    fn supports(self, shape: HostPointerShape) -> bool {
        self.0 & shape.bit() != 0
    }
}

/// The shape Herdr has pushed onto the host stack, if any.
pub(super) struct HostPointer {
    support: HostPointerSupport,
    applied: Arc<AtomicU8>,
}

impl HostPointer {
    pub(super) fn new(support: HostPointerSupport) -> Self {
        Self {
            support,
            applied: Arc::new(AtomicU8::new(0)),
        }
    }

    /// Moves the host pointer to `desired`. Writes nothing when the shape is unchanged or the
    /// host did not confirm it; an unconfirmed shape means no override.
    pub(super) fn sync(
        &self,
        writer: &mut impl io::Write,
        desired: Option<HostPointerShape>,
    ) -> io::Result<()> {
        let desired = desired.filter(|shape| self.support.supports(*shape));
        let current = HostPointerShape::from_code(self.applied.load(Ordering::Acquire));
        if desired == current {
            return Ok(());
        }
        let mut bytes = Vec::with_capacity(32);
        if current.is_some() {
            bytes.extend_from_slice(POP);
        }
        if let Some(shape) = desired {
            write!(bytes, "\x1b]22;>{}\x1b\\", shape.name())?;
        }
        writer.write_all(&bytes)?;
        writer.flush()?;
        self.applied
            .store(desired.map_or(0, HostPointerShape::code), Ordering::Release);
        Ok(())
    }

    pub(super) fn restorer(&self) -> HostPointerRestore {
        HostPointerRestore(self.applied.clone())
    }
}

/// Pops Herdr's shape on detach, shutdown, or panic. The host keeps a separate stack per
/// screen, so this must run before leaving the alternate screen.
#[derive(Clone)]
pub(super) struct HostPointerRestore(Arc<AtomicU8>);

impl HostPointerRestore {
    pub(super) fn restore(&self, writer: &mut impl io::Write) -> io::Result<()> {
        if self.0.swap(0, Ordering::AcqRel) == 0 {
            return Ok(());
        }
        writer.write_all(POP)?;
        writer.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_SUPPORTED: &[u8] = b"1,1,1,1,1";

    fn pointer(reply: &[u8]) -> HostPointer {
        HostPointer::new(HostPointerSupport::from_reply(reply).expect("support reply"))
    }

    fn sync(pointer: &HostPointer, desired: Option<HostPointerShape>) -> Vec<u8> {
        let mut out = Vec::new();
        pointer.sync(&mut out, desired).expect("sync");
        out
    }

    #[test]
    fn support_reply_needs_one_binary_answer_per_queried_name() {
        assert_eq!(
            HostPointerSupport::from_reply(b"1,0,1,0,1"),
            Some(HostPointerSupport(0b10101))
        );
        for malformed in [
            b"".as_slice(),
            b"1,1,1,1",
            b"1,1,1,1,1,1",
            b"1,1,2,1,1",
            b"pointer",
            b"0",
            b"1,1,1,1,1,",
        ] {
            assert_eq!(
                HostPointerSupport::from_reply(malformed),
                None,
                "{malformed:?}"
            );
        }
    }

    #[test]
    fn the_support_query_lists_every_shape_in_reply_order() {
        let names: Vec<_> = HostPointerShape::ALL.iter().map(|s| s.name()).collect();
        let expected = format!("\x1b]22;?{}\x1b\\", names.join(","));
        assert_eq!(SUPPORT_QUERY, expected.as_bytes());
    }

    #[test]
    fn an_unsupported_host_never_receives_a_pointer_sequence() {
        let pointer = HostPointer::new(HostPointerSupport::default());
        for shape in HostPointerShape::ALL {
            assert!(sync(&pointer, Some(shape)).is_empty());
        }
        assert!(sync(&pointer, None).is_empty());
        let mut out = Vec::new();
        pointer.restorer().restore(&mut out).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn overrides_push_once_change_by_pop_and_push_and_clear_by_pop() {
        let pointer = pointer(ALL_SUPPORTED);
        assert_eq!(
            sync(&pointer, Some(HostPointerShape::Move)),
            b"\x1b]22;>move\x1b\\"
        );
        assert!(sync(&pointer, Some(HostPointerShape::Move)).is_empty());
        assert_eq!(
            sync(&pointer, Some(HostPointerShape::NwseResize)),
            b"\x1b]22;<\x1b\\\x1b]22;>nwse-resize\x1b\\"
        );
        assert_eq!(sync(&pointer, None), b"\x1b]22;<\x1b\\");
        assert!(sync(&pointer, None).is_empty());
    }

    #[test]
    fn a_shape_the_host_did_not_confirm_is_no_override() {
        let pointer = pointer(b"1,0,1,1,1");
        assert_eq!(
            sync(&pointer, Some(HostPointerShape::Pointer)),
            b"\x1b]22;>pointer\x1b\\"
        );
        assert_eq!(
            sync(&pointer, Some(HostPointerShape::Move)),
            b"\x1b]22;<\x1b\\",
            "an unconfirmed shape pops Herdr's shape and pushes nothing"
        );
    }

    #[test]
    fn restore_pops_only_a_shape_herdr_pushed_and_only_once() {
        let pointer = pointer(ALL_SUPPORTED);
        let restore = pointer.restorer();
        let mut out = Vec::new();
        restore.restore(&mut out).unwrap();
        assert!(out.is_empty(), "nothing pushed, nothing popped");

        sync(&pointer, Some(HostPointerShape::EwResize));
        restore.restore(&mut out).unwrap();
        restore.restore(&mut out).unwrap();
        assert_eq!(out, b"\x1b]22;<\x1b\\");
        assert_eq!(
            sync(&pointer, None),
            b"",
            "after restore the stack holds nothing of Herdr's"
        );
    }
}
