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

/// Array header as laid out by generated code (`func.rs`: `data`, `len`, `cap`, 24 bytes). `data` is
/// stored as an `i64` word (the generated code stores the pointer through an I64 slot), keeping the
/// layout identical on 32-bit and 64-bit targets.
#[repr(C)]
struct ArrayHdr {
    data: u64,
    len: i64,
    cap: i64,
}

/// Replace `hdr[lo..hi]` with the elements of `src` (spec §11.3), returning a fresh header allocated
/// from `arena`. `lo`/`hi` follow the interpreter's slice rules (negative counts from the end, clamped
/// to the length); `i64::MAX` is the "omitted bound" sentinel and clamps to the length. Returns null for
/// an invalid range (`lo > hi`), so the JIT deopts and the interpreter reports the error.
///
/// # Safety
/// `arena` must be a live arena returned by [`pj_arena_new`]; `hdr` and `src` must point to array
/// headers allocated from the same arena; `esize` must be the element size of both arrays.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pj_array_splice(
    arena: *mut Arena,
    hdr: *mut u8,
    lo: i64,
    hi: i64,
    src: *const u8,
    esize: usize,
) -> *mut u8 {
    // SAFETY: the caller (generated code) passes headers from this arena (see the JIT lowering).
    // Copy the fields out first so no shared reference to the arena outlives the mutable borrow below.
    let (h_data, len, s_data, s_len) = unsafe {
        let h = &*(hdr as *const ArrayHdr);
        let s = &*(src as *const ArrayHdr);
        (h.data as *mut u8, h.len, s.data as *mut u8, s.len)
    };
    let clamp = |raw: i64, omitted: i64| {
        let raw = if raw == i64::MAX { omitted } else { raw };
        if raw < 0 {
            (len + raw).max(0)
        } else {
            raw.min(len)
        }
    };
    let lo = clamp(lo, 0);
    let hi = clamp(hi, len);
    if lo > hi || s_len < 0 {
        return std::ptr::null_mut();
    }
    let prefix = lo as usize;
    let suffix = (len - hi) as usize;
    let mid = s_len as usize;
    let new_len = prefix + mid + suffix;

    // SAFETY: `arena` is live for the whole call (see the JIT call protocol).
    let a = unsafe { &mut *arena };
    let new_hdr = a.alloc(std::mem::size_of::<ArrayHdr>());
    let new_data = if new_len == 0 {
        std::ptr::null_mut()
    } else {
        a.alloc(new_len * esize)
    };
    // SAFETY: every segment is valid for its byte count inside the two source buffers.
    unsafe {
        if prefix > 0 {
            std::ptr::copy_nonoverlapping(h_data, new_data, prefix * esize);
        }
        if mid > 0 {
            std::ptr::copy_nonoverlapping(s_data, new_data.add(prefix * esize), mid * esize);
        }
        if suffix > 0 {
            std::ptr::copy_nonoverlapping(
                h_data.add(hi as usize * esize),
                new_data.add((prefix + mid) * esize),
                suffix * esize,
            );
        }
        std::ptr::write(
            new_hdr as *mut ArrayHdr,
            ArrayHdr {
                data: new_data as u64,
                len: new_len as i64,
                cap: new_len as i64,
            },
        );
    }
    new_hdr
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
