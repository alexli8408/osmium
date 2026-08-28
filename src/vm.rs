//! Sv39 virtual memory.
//!
//! Three-level page tables: a 39-bit VA splits into three 9-bit indices
//! (VPN[2..0]) plus a 12-bit page offset; each table is one 4 KiB page of
//! 512 PTEs. The kernel runs *identity-mapped* (VA == PA) but with real
//! permissions per section — text is R-X, rodata R--, data RW-, and the
//! embedded user-mode section carries the U bit so U-mode can run it while
//! S-mode cannot accidentally execute it.
//!
//! Everything is mapped at 4 KiB granularity so individual pages (user
//! stacks, guard pages) can later change permissions in place.

use crate::kalloc;
use crate::memlayout::{self, PAGE_SIZE, PHYS_TOP, PLIC, PLIC_SIZE, UART0, VIRT_TEST};
use crate::riscv::{satp, sfence_vma};
use core::sync::atomic::{AtomicUsize, Ordering};

// PTE permission/status bits.
pub const PTE_V: usize = 1 << 0;
pub const PTE_R: usize = 1 << 1;
pub const PTE_W: usize = 1 << 2;
pub const PTE_X: usize = 1 << 3;
pub const PTE_U: usize = 1 << 4;
pub const PTE_A: usize = 1 << 6;
pub const PTE_D: usize = 1 << 7;

const PERM_MASK: usize = PTE_R | PTE_W | PTE_X | PTE_U;

const SATP_SV39: usize = 8 << 60;

/// Highest valid Sv39 address: 512 GiB, minus the sign-extension hole above.
pub const VA_MAX: usize = 1 << 38;

#[inline]
fn pa_to_pte(pa: usize) -> usize {
    (pa >> 12) << 10
}

#[inline]
fn pte_to_pa(pte: usize) -> usize {
    (pte >> 10) << 12
}

#[inline]
fn vpn(va: usize, level: usize) -> usize {
    (va >> (12 + 9 * level)) & 0x1ff
}

/// Physical address of the kernel's root page table (0 until kvminit).
static KERNEL_ROOT: AtomicUsize = AtomicUsize::new(0);

pub fn kernel_root() -> usize {
    KERNEL_ROOT.load(Ordering::Relaxed)
}

/// Walk the table rooted at `root` to the level-0 PTE for `va`, allocating
/// intermediate tables when `alloc` is set. Returns a pointer to the PTE.
///
/// # Safety
/// `root` must point at a valid page-table page; identity mapping must make
/// physical page-table addresses dereferenceable (true for this kernel).
unsafe fn walk(root: usize, va: usize, alloc: bool) -> Option<*mut usize> {
    assert!(va < VA_MAX, "walk: va {va:#x} beyond Sv39");
    let mut table = root as *mut usize;
    for level in (1..3).rev() {
        let pte_ptr = unsafe { table.add(vpn(va, level)) };
        let pte = unsafe { *pte_ptr };
        if pte & PTE_V != 0 {
            table = pte_to_pa(pte) as *mut usize;
        } else {
            if !alloc {
                return None;
            }
            let page = kalloc::alloc()?; // zeroed, so all PTEs invalid
            unsafe { *pte_ptr = pa_to_pte(page) | PTE_V };
            table = page as *mut usize;
        }
    }
    Some(unsafe { table.add(vpn(va, 0)) })
}

/// Map the range [va, va+size) to [pa, pa+size) with `perm`. Addresses and
/// size must be page-aligned; remapping an existing page is a bug.
///
/// # Safety
/// Caller must ensure the mapping doesn't alias memory in a way that breaks
/// Rust's guarantees, and that `root` is a valid root table.
pub unsafe fn map_pages(root: usize, va: usize, pa: usize, size: usize, perm: usize) {
    assert!(
        va.is_multiple_of(PAGE_SIZE)
            && pa.is_multiple_of(PAGE_SIZE)
            && size.is_multiple_of(PAGE_SIZE)
    );
    assert!(size > 0, "map_pages: empty mapping at {va:#x}");
    assert!(perm & PERM_MASK != 0, "map_pages: no permissions");

    for off in (0..size).step_by(PAGE_SIZE) {
        let pte_ptr = unsafe { walk(root, va + off, true) }.expect("map_pages: out of pages");
        assert!(
            unsafe { *pte_ptr } & PTE_V == 0,
            "map_pages: remap of va {:#x}",
            va + off
        );
        // A|D preset: avoids faults on hardware that traps instead of
        // updating accessed/dirty bits itself.
        unsafe { *pte_ptr = pa_to_pte(pa + off) | perm | PTE_A | PTE_D | PTE_V };
    }
}

/// Rewrite the permission bits of already-mapped leaf PTEs in
/// [va, va+size). Used to grant U-mode access to pages inside the kernel's
/// big RAM mapping (user stacks) without remapping them.
///
/// # Safety
/// Same contract as [`map_pages`]; the range must already be fully mapped.
#[allow(dead_code)] // general VM primitive; not currently on any hot path
pub unsafe fn protect(root: usize, va: usize, size: usize, perm: usize) {
    assert!(va.is_multiple_of(PAGE_SIZE) && size.is_multiple_of(PAGE_SIZE));
    for off in (0..size).step_by(PAGE_SIZE) {
        let pte_ptr = unsafe { walk(root, va + off, false) }.expect("protect: unmapped va");
        let pte = unsafe { *pte_ptr };
        assert!(pte & PTE_V != 0, "protect: invalid pte at {:#x}", va + off);
        unsafe { *pte_ptr = (pte & !PERM_MASK) | perm };
    }
    sfence_vma();
}

/// Software page-table walk: VA -> (PA, leaf PTE). For tests & diagnostics.
pub fn translate(root: usize, va: usize) -> Option<(usize, usize)> {
    let pte_ptr = unsafe { walk(root, va, false) }?;
    let pte = unsafe { *pte_ptr };
    if pte & PTE_V == 0 {
        return None;
    }
    Some((pte_to_pa(pte) | (va % PAGE_SIZE), pte))
}

/// Build the kernel page table: identity mappings with least privilege.
fn kvmmake() -> usize {
    let root = kalloc::alloc().expect("kvmmake: out of pages");

    let text_start = memlayout::text_start();
    let text_end = memlayout::text_end();
    let user_start = memlayout::user_start();
    let user_end = memlayout::user_end();
    let rodata_start = memlayout::rodata_start();
    let rodata_end = memlayout::rodata_end();
    let data_start = memlayout::data_start();
    let kernel_end = memlayout::kernel_end();

    unsafe {
        // Devices.
        map_pages(root, UART0, UART0, PAGE_SIZE, PTE_R | PTE_W);
        map_pages(root, PLIC, PLIC, PLIC_SIZE, PTE_R | PTE_W);
        map_pages(root, VIRT_TEST, VIRT_TEST, PAGE_SIZE, PTE_R | PTE_W);

        // Kernel image, tightest permissions per section.
        map_pages(
            root,
            text_start,
            text_start,
            text_end - text_start,
            PTE_R | PTE_X,
        );
        if user_end > user_start {
            // Embedded user program images: read-only kernel data now — the
            // loader copies them into each process's private address space,
            // so the kernel only needs to *read* them (no U bit, or S-mode
            // couldn't read them with SUM clear; no X, they run elsewhere).
            map_pages(root, user_start, user_start, user_end - user_start, PTE_R);
        }
        map_pages(
            root,
            rodata_start,
            rodata_start,
            rodata_end - rodata_start,
            PTE_R,
        );
        map_pages(
            root,
            data_start,
            data_start,
            kernel_end - data_start,
            PTE_R | PTE_W,
        );

        // Heap + all allocatable DRAM (kernel stacks, page tables, ...).
        map_pages(
            root,
            kernel_end,
            kernel_end,
            PHYS_TOP - kernel_end,
            PTE_R | PTE_W,
        );
    }

    root
}

/// Build the kernel page table (once, at boot, before enabling paging).
pub fn init() {
    let root = kvmmake();
    KERNEL_ROOT.store(root, Ordering::Relaxed);
}

/// Switch this hart onto the kernel page table.
pub fn init_hart() {
    let root = kernel_root();
    assert!(root != 0, "init_hart before vm::init");
    switch_to(root);
    println!("  vm: sv39 paging on (root table {root:#x})");
}

/// Point satp at `root` and flush the TLB. Every table this kernel builds
/// contains the full kernel mapping, so switching is always safe: whatever
/// kernel code runs next is addressable under the new table.
pub fn switch_to(root: usize) {
    assert!(root != 0, "switch_to null root");
    unsafe {
        // Order matters: all table writes must be visible before satp.
        sfence_vma();
        satp::write(SATP_SV39 | (root >> 12));
        sfence_vma();
    }
}

// --- Per-process address spaces ---------------------------------------------
//
// Each user process gets its own root table. The kernel occupies top-level
// slots 0 (devices + low physical) and 2 (DRAM at 2 GiB); those entries are
// copied into every process table so the kernel map — trap vector, kernel
// code/data, all kernel stacks — is reachable with the process's satp
// active, which is what lets a trap from user mode run kernel code without
// first swapping page tables. Slot 1 (the 0x4000_0000..0x8000_0000 GiB) is
// left for user mappings, so a process's private code/stack live in a
// subtree the kernel never shares.

/// Base virtual address of a user program's code.
pub const USER_TEXT: usize = 0x4000_0000;
/// One past the top of a user program's stack (grows down from here).
pub const USER_STACK_TOP: usize = 0x4100_0000;

/// Top-level (level-2) index reserved for user mappings.
const USER_L2_INDEX: usize = USER_TEXT >> 30;

/// Create a fresh process page table that shares the kernel mapping. User
/// mappings added later land in the private slot-1 subtree.
pub fn make_user_table() -> usize {
    let root = kalloc::alloc().expect("make_user_table: out of pages");
    let kroot = kernel_root() as *const usize;
    let uroot = root as *mut usize;
    // Copy all top-level entries; the kernel's are shared, and slot 1 is
    // invalid in the kernel table, so user mappings allocate a fresh
    // subtree there rather than touching anything shared.
    for i in 0..512 {
        unsafe { *uroot.add(i) = *kroot.add(i) };
    }
    debug_assert!(
        unsafe { *kroot.add(USER_L2_INDEX) } & PTE_V == 0,
        "user slot collides with a kernel mapping"
    );
    root
}

/// Free a process page table: the private slot-1 subtree (its level-1 and
/// level-0 tables and every leaf frame — the user code and stack) plus the
/// root page. Shared kernel subtrees are left untouched.
///
/// # Safety
/// `root` must be a table from [`make_user_table`] that is not the active
/// satp on any hart.
pub unsafe fn free_user_table(root: usize) {
    let l2 = root as *const usize;
    let e2 = unsafe { *l2.add(USER_L2_INDEX) };
    if e2 & PTE_V != 0 {
        let l1 = pte_to_pa(e2);
        let l1p = l1 as *const usize;
        for i in 0..512 {
            let e1 = unsafe { *l1p.add(i) };
            if e1 & PTE_V != 0 {
                let l0 = pte_to_pa(e1);
                let l0p = l0 as *const usize;
                for j in 0..512 {
                    let e0 = unsafe { *l0p.add(j) };
                    if e0 & PTE_V != 0 {
                        unsafe { kalloc::free(pte_to_pa(e0)) }; // leaf frame
                    }
                }
                unsafe { kalloc::free(l0) }; // level-0 table
            }
        }
        unsafe { kalloc::free(l1) }; // level-1 table
    }
    unsafe { kalloc::free(root) };
}
