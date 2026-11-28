//! `Vm` frame/span helpers shared by the dispatch loop and `step_vm`.

use std::rc::Rc;

use prima_core::Value;

use super::{Frame, Vm};
use crate::vm::op::Chunk;

impl<'a> Vm<'a> {
    /// Push a frame for `chunk`, reserving its slot array.
    pub(super) fn push_frame(&mut self, chunk: Rc<Chunk>) {
        let slots = vec![Value::Nil; chunk.slot_count as usize];
        self.frames.push(Frame {
            chunk,
            ip: 0,
            slots,
        });
    }

    /// The active (top) frame's chunk.
    pub(super) fn active_chunk(&self) -> &Chunk {
        &self.frames.last().expect("unreachable").chunk
    }

    pub(super) fn pop_frame(&mut self) {
        self.frames.pop();
    }

    /// Current instruction's source line for diagnostics.
    pub(super) fn current_line(&self) -> u32 {
        let f = self.frames.last().expect("unreachable");
        f.chunk
            .lines
            .get(f.ip.saturating_sub(1))
            .copied()
            .unwrap_or(0)
    }

    /// Current instruction's source span (a degenerate line span) for diagnostics.
    pub(super) fn current_span(&self) -> prima_syntax::Span {
        let l = self.current_line();
        prima_syntax::Span::new(l, l)
    }
}
