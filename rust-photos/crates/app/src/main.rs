#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
//! Rust Photos entry point. The app itself lives in the library (src/lib.rs) so tests and benchmarks can use it.

// ---- memory diagnostics: counts live heap bytes so Task Manager's number can be split into
// "my Rust heap" and "everything else" (graphics driver, DLLs, GPU textures, thread stacks).
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
struct Counting;
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(l) };
        if !p.is_null() { let n = LIVE.fetch_add(l.size(), Relaxed) + l.size(); PEAK.fetch_max(n, Relaxed); }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) };
        LIVE.fetch_sub(l.size(), Relaxed);
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        let q = unsafe { System.realloc(p, l, new) };
        if !q.is_null() {
            if new >= l.size() { let n = LIVE.fetch_add(new - l.size(), Relaxed) + new - l.size(); PEAK.fetch_max(n, Relaxed); }
            else { LIVE.fetch_sub(l.size() - new, Relaxed); }
        }
        q
    }
}
#[global_allocator]
static GLOBAL: Counting = Counting;
pub fn heap_mb() -> (usize, usize) { (LIVE.load(Relaxed) >> 20, PEAK.load(Relaxed) >> 20) }


fn main() {
    rust_photos::run_app(heap_mb);
}
