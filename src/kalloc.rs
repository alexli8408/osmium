//! Physical page allocator.
//!
//! Manages all of DRAM above the kernel image + heap region as a free list
//! of 4 KiB pages. The list nodes live inside the free pages themselves, so
//! the allocator needs no metadata storage of its own.

use core::ptr;

use crate::memlayout::{self, PAGE_SIZE, PHYS_TOP};
use crate::spinlock::SpinLock;

struct FreeList {
    head: *mut FreeNode,
    free_pages: usize,
}

// Raw pointers aren't Send by default; the free list only ever dereferences
// them under the lock, which is what makes moving it across harts sound.
unsafe impl Send for FreeList {}

#[repr(C)]
struct FreeNode {
    next: *mut FreeNode,
}

static KMEM: SpinLock<FreeList> = SpinLock::new(
    "kmem",
    FreeList {
        head: ptr::null_mut(),
        free_pages: 0,
    },
);

/// Hand every page between the end of the kernel heap and the top of DRAM
/// to the free list. Called once at boot.
pub fn init() {
    let start = memlayout::alloc_start().next_multiple_of(PAGE_SIZE);
    let mut page = start;
    while page + PAGE_SIZE <= PHYS_TOP {
        unsafe { free(page) };
        page += PAGE_SIZE;
    }
}

/// Allocate one zeroed 4 KiB page, returning its physical address (which is
/// also its kernel virtual address — the kernel runs identity-mapped).
pub fn alloc() -> Option<usize> {
    let mut kmem = KMEM.lock();
    let node = kmem.head;
    if node.is_null() {
        return None;
    }
    unsafe {
        kmem.head = (*node).next;
    }
    kmem.free_pages -= 1;
    drop(kmem);

    // Zero outside the lock; page tables and fresh stacks rely on this.
    unsafe { ptr::write_bytes(node as *mut u8, 0, PAGE_SIZE) };
    Some(node as usize)
}

/// Return a page to the allocator.
///
/// # Safety
/// `pa` must be a page-aligned physical page previously handed out by
/// [`alloc`] (or DRAM being seeded at init), with no live references into it.
pub unsafe fn free(pa: usize) {
    assert!(
        pa.is_multiple_of(PAGE_SIZE),
        "kalloc::free: unaligned page {pa:#x}"
    );
    assert!(
        pa >= memlayout::alloc_start() && pa + PAGE_SIZE <= PHYS_TOP,
        "kalloc::free: page {pa:#x} outside allocator range"
    );

    // Poison to catch use-after-free before the node pointer goes in.
    unsafe { ptr::write_bytes(pa as *mut u8, 0x5a, PAGE_SIZE) };

    let node = pa as *mut FreeNode;
    let mut kmem = KMEM.lock();
    unsafe { (*node).next = kmem.head };
    kmem.head = node;
    kmem.free_pages += 1;
}

/// Number of free pages currently available.
pub fn free_pages() -> usize {
    KMEM.lock().free_pages
}
