//! Count allocations on the calling test thread only, excluding other tests
//! and PTY/GPU worker threads. Tracking itself never allocates.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    static COUNT: Cell<Option<usize>> = const { Cell::new(None) };
}

struct CountingAllocator;
#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator;

fn record() {
    let _ = COUNT.try_with(|count| {
        if let Some(n) = count.get() { count.set(Some(n + 1)); }
    });
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record();
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record();
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        record();
        unsafe { System.realloc(ptr, layout, size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

pub fn allocations(f: impl FnOnce()) -> usize {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) { COUNT.set(None); }
    }
    COUNT.set(Some(0));
    let _reset = Reset;
    f();
    COUNT.get().unwrap()
}
