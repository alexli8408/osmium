//! Kernel heap: a first-fit, address-ordered free-list allocator backing
//! Rust's `alloc` crate (`Box`, `Vec`, `String`, ...).
//!
//! The heap owns the fixed 8 MiB region right after the kernel image
//! (memlayout::heap_start()). Free blocks carry an intrusive header {size,
//! next} and are kept sorted by address so freeing can coalesce with both
//! neighbors, keeping fragmentation bounded.
//!
//! Each live allocation is preceded by a 16-byte header recording the block
//! it was carved from, so `dealloc` returns exactly what `alloc` took —
//! including any alignment padding or absorbed slack — no matter what
//! layout the caller hands back.

use core::alloc::{GlobalAlloc, Layout};
use core::ptr;

use crate::memlayout;
use crate::spinlock::SpinLock;

/// Minimum granule: every block size/address is a multiple of this, and it
/// is the smallest chunk worth keeping on the free list.
const ALIGN: usize = 16;
/// A free block must at least hold its header with room to be useful.
const MIN_BLOCK: usize = 32;

/// Header of a *free* block, stored at the block's first bytes.
#[repr(C)]
struct FreeBlock {
    size: usize,
    next: *mut FreeBlock,
}

/// Header preceding every *allocated* pointer: which block to give back.
#[repr(C)]
struct AllocHeader {
    block_start: usize,
    block_size: usize,
}

struct Heap {
    head: *mut FreeBlock,
    free_bytes: usize,
}

unsafe impl Send for Heap {}

static HEAP: SpinLock<Heap> = SpinLock::new(
    "heap",
    Heap {
        head: ptr::null_mut(),
        free_bytes: 0,
    },
);

const fn align_up(x: usize, align: usize) -> usize {
    (x + align - 1) & !(align - 1)
}

/// Seed the heap with its whole region as one free block.
pub fn init() {
    let start = memlayout::heap_start();
    let size = memlayout::HEAP_SIZE;
    assert!(start % ALIGN == 0 && size % ALIGN == 0);

    let block = start as *mut FreeBlock;
    let mut heap = HEAP.lock();
    assert!(heap.head.is_null(), "heap::init called twice");
    unsafe {
        (*block).size = size;
        (*block).next = ptr::null_mut();
    }
    heap.head = block;
    heap.free_bytes = size;
}

/// Bytes currently free (not accounting for per-allocation headers).
pub fn free_bytes() -> usize {
    HEAP.lock().free_bytes
}

fn alloc_inner(layout: Layout) -> *mut u8 {
    let need = align_up(layout.size().max(1), ALIGN);
    let align = layout.align().max(ALIGN);

    let mut heap = HEAP.lock();
    // First fit: walk the address-ordered list for a block that can hold
    // header + aligned payload.
    let mut prev: *mut FreeBlock = ptr::null_mut();
    let mut cur = heap.head;
    while !cur.is_null() {
        let block_start = cur as usize;
        let block_size = unsafe { (*cur).size };
        let block_end = block_start + block_size;

        let user_start = align_up(block_start + size_of::<AllocHeader>(), align);
        let user_end = user_start + need;

        if user_end <= block_end {
            let next = unsafe { (*cur).next };

            // Front remainder: keep on the list if big enough, otherwise
            // absorb it into the allocation (the header records the truth).
            let front_gap = user_start - size_of::<AllocHeader>() - block_start;
            let (alloc_start, keep_front) = if front_gap >= MIN_BLOCK {
                (block_start + front_gap, true)
            } else {
                (block_start, false)
            };

            // Back remainder, same policy.
            let back_gap = block_end - user_end;
            let (alloc_end, keep_back) = if back_gap >= MIN_BLOCK {
                (user_end, true)
            } else {
                (block_end, false)
            };

            // Rewire the free list around the carved-out region.
            let mut replacement = next;
            if keep_back {
                let back = alloc_end as *mut FreeBlock;
                unsafe {
                    (*back).size = block_end - alloc_end;
                    (*back).next = replacement;
                }
                replacement = back;
            }
            if keep_front {
                unsafe {
                    (*cur).size = front_gap;
                    (*cur).next = replacement;
                }
                replacement = cur;
            }
            if prev.is_null() {
                heap.head = replacement;
            } else {
                unsafe { (*prev).next = replacement };
            }

            heap.free_bytes -= alloc_end - alloc_start;

            let header = (user_start - size_of::<AllocHeader>()) as *mut AllocHeader;
            unsafe {
                (*header).block_start = alloc_start;
                (*header).block_size = alloc_end - alloc_start;
            }
            return user_start as *mut u8;
        }

        prev = cur;
        cur = unsafe { (*cur).next };
    }
    ptr::null_mut() // out of heap; Rust's OOM path panics
}

fn dealloc_inner(user_ptr: *mut u8) {
    let header = unsafe { &*((user_ptr as usize - size_of::<AllocHeader>()) as *const AllocHeader) };
    let start = header.block_start;
    let size = header.block_size;
    let end = start + size;

    let mut heap = HEAP.lock();
    heap.free_bytes += size;

    // Find the address-ordered position, then coalesce with both sides.
    let mut prev: *mut FreeBlock = ptr::null_mut();
    let mut cur = heap.head;
    while !cur.is_null() && (cur as usize) < start {
        prev = cur;
        cur = unsafe { (*cur).next };
    }

    let block = start as *mut FreeBlock;
    unsafe {
        (*block).size = size;
        (*block).next = cur;

        // Merge forward into `cur`.
        if !cur.is_null() && end == cur as usize {
            (*block).size += (*cur).size;
            (*block).next = (*cur).next;
        }

        // Merge backward into `prev`, or link in.
        if !prev.is_null() && (prev as usize) + (*prev).size == start {
            (*prev).size += (*block).size;
            (*prev).next = (*block).next;
        } else if prev.is_null() {
            heap.head = block;
        } else {
            (*prev).next = block;
        }
    }
}

struct KernelAllocator;

unsafe impl GlobalAlloc for KernelAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        alloc_inner(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        dealloc_inner(ptr)
    }
}

#[global_allocator]
static ALLOCATOR: KernelAllocator = KernelAllocator;
