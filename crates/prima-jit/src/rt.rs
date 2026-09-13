//! Runtime support called from JIT-generated code (spec §19.2).
//!
//! Generated functions allocate their dense arrays from a per-call [`Arena`] and free the whole
//! arena at return, so no individual array needs explicit cleanup. Everything is a plain
//! `extern "C"` trampoline registered in the engine's symbol table (see `engine.rs`), so the
//! generated code never depends on platform symbol names.

/// A bump arena: allocations are 8-byte aligned and freed together at the end of a JIT call.
/// Chunk buffers are heap `Vec`s whose storage does not move when the `Arena` itself is moved, so
/// the raw pointers handed to generated code stay valid.
#[derive(Default)]
pub struct Arena {
    chunks: Vec<Vec<u8>>,
    used: usize,
}

impl Arena {
    fn alloc(&mut self, size: usize) -> *mut u8 {
        let size = (size + 7) & !7;
        let fits = self
            .chunks
            .last()
            .is_some_and(|c| c.len().saturating_sub(self.used) >= size);
        if !fits {
            let cap = size.max(1 << 16);
            self.chunks.push(vec![0u8; cap]);
            self.used = 0;
        }
        let chunk = self.chunks.last_mut().expect("chunk was just ensured");
        // SAFETY: `used + size <= chunk.len()` by the check above; the pointer is valid for the
        // chunk's lifetime (the arena outlives every allocation).
        let p = unsafe { chunk.as_mut_ptr().add(self.used) };
        self.used += size;
        p
    }
}

/// Create a new arena for one JIT call.
#[unsafe(no_mangle)]
pub extern "C" fn pj_arena_new() -> *mut Arena {
    Box::into_raw(Box::new(Arena::default()))
}

/// Allocate `size` bytes from the arena (8-byte aligned).
///
/// # Safety
/// `arena` must be a pointer returned by [`pj_arena_new`] and not yet freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pj_arena_alloc(arena: *mut Arena, size: usize) -> *mut u8 {
    unsafe { (*arena).alloc(size) }
}

/// Free the arena and every buffer allocated from it.
///
/// # Safety
/// `arena` must be a pointer returned by [`pj_arena_new`] that has not already been freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pj_arena_free(arena: *mut Arena) {
    drop(unsafe { Box::from_raw(arena) });
}

/// Copy `n` bytes from `src` to `dst` (non-overlapping; the JIT only copies array buffers).
///
/// # Safety
/// `dst` and `src` must each be valid for `n` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pj_memcpy(dst: *mut u8, src: *const u8, n: usize) {
    unsafe { std::ptr::copy_nonoverlapping(src, dst, n) };
}

/// Fill `count` `u8` elements with `val` (bool arrays).
///
/// # Safety
/// `dst` must be valid for `count` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pj_fill_u8(dst: *mut u8, count: usize, val: u8) {
    unsafe { std::ptr::write_bytes(dst, val, count) };
}

/// Fill `count` `i64` elements with `val`.
///
/// # Safety
/// `dst` must be valid for `count * 8` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pj_fill_i64(dst: *mut i64, count: usize, val: i64) {
    for i in 0..count {
        unsafe { *dst.add(i) = val };
    }
}

/// Fill `count` `f64` elements with `val`.
///
/// # Safety
/// `dst` must be valid for `count * 8` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pj_fill_f64(dst: *mut f64, count: usize, val: f64) {
    for i in 0..count {
        unsafe { *dst.add(i) = val };
    }
}

/// The language's integer modulo (sign follows the divisor), matching `number_mod`. The JIT only
/// calls this with a non-zero divisor (it deopts on zero).
#[unsafe(no_mangle)]
pub extern "C" fn pj_i64_rem(a: i64, b: i64) -> i64 {
    let r = a % b;
    if r != 0 && (r < 0) != (b < 0) {
        r + b
    } else {
        r
    }
}

/// Host-provided cancellation predicate (spec §16): the runtime registers
/// `Evaluator::is_cancelled` so JIT loop back-edges can poll it and deopt, keeping the same
/// interruption behavior as the interpreter.
static CANCEL_CHECK: std::sync::atomic::AtomicPtr<()> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

/// Register the cancellation predicate (idempotent; called once by the runtime).
pub fn set_cancel_check(f: fn() -> bool) {
    CANCEL_CHECK.store(f as *mut (), std::sync::atomic::Ordering::Release);
}

/// Poll the cancellation predicate; returns 1 when cancellation has been requested. With no
/// predicate registered it always returns 0.
#[unsafe(no_mangle)]
pub extern "C" fn pj_check_cancel() -> u8 {
    let p = CANCEL_CHECK.load(std::sync::atomic::Ordering::Acquire);
    if p.is_null() {
        return 0;
    }
    // SAFETY: `p` was stored from a `fn() -> bool` pointer in `set_cancel_check`.
    let f: fn() -> bool = unsafe { std::mem::transmute(p) };
    u8::from(f())
}
