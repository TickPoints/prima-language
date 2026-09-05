//! Bytecode VM executor (spec §19.5, Milestone B).
//!
//! A call/return stack machine over `Value`-typed slots. Each `Frame` references a compiled
//! `Chunk`, a program-counter, its slot array, and its captured upvalues; the VM owns a frame stack
//! and the operand stack. Class/collection/builtin ops dispatch back into the `Evaluator` so the VM
//! shares one semantic engine with the AST interpreter.
//!
//! Phase 1 is wired behind the `vm` policy; when a function has no compiled chunk the caller falls
//! back to the AST interpreter.

/// A single active call frame.
#[derive(Debug, Clone)]
pub struct Frame {
    /// Owned chunk (function body). Cloned for simplicity; later moved into a shared `Arc<Chunk>`.
    pub chunk: super::op::Chunk,
    /// Program counter (index into `chunk.code`).
    pub ip: usize,
    /// Stack slots for locals/self/upvalues.
    pub slots: Vec<prima_core::Value>,
    /// Line of the most recently executed instruction, for diagnostics.
    pub line: u32,
}

/// The VM: a stack of frames plus the operand stack.
pub struct Vm {
    pub frames: Vec<Frame>,
    pub stack: Vec<prima_core::Value>,
}

impl Vm {
    pub fn new() -> Vm {
        Vm {
            frames: Vec::new(),
            stack: Vec::new(),
        }
    }

    /// Push a frame for a freshly-compiled chunk, reserving its slots.
    pub fn push_frame(&mut self, chunk: super::op::Chunk) {
        let slots = vec![prima_core::Value::Nil; chunk.slot_count as usize];
        self.frames.push(Frame {
            chunk,
            ip: 0,
            slots,
            line: 0,
        });
    }

    /// Pop the top frame (on return).
    pub fn pop_frame(&mut self) -> Option<Frame> {
        self.frames.pop()
    }

    /// The current frame, if any.
    pub fn current_frame(&mut self) -> Option<&mut Frame> {
        self.frames.last_mut()
    }
}

impl Default for Vm {
    fn default() -> Self {
        Self::new()
    }
}
