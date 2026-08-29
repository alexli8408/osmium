//! Physical memory map of QEMU's `virt` machine, plus the kernel's own
//! layout as exported by linker.ld.
//!
//!   0x0c00_0000  PLIC          platform-level interrupt controller
//!   0x1000_0000  UART0         NS16550A serial port
//!   0x8000_0000  DRAM          first 2 MiB: OpenSBI firmware (PMP-guarded,
//!                              never mapped); 0x8020_0000: kernel image,
//!                              then heap, then free pages

pub const PAGE_SIZE: usize = 4096;

pub const PLIC: usize = 0x0c00_0000;
pub const PLIC_SIZE: usize = 0x40_0000;
pub const UART0: usize = 0x1000_0000;
pub const UART0_IRQ: u32 = 10;

pub const DRAM_BASE: usize = 0x8000_0000;
/// Matches -m 128M in the QEMU invocation.
pub const PHYS_TOP: usize = DRAM_BASE + 128 * 1024 * 1024;

/// Fixed-size kernel heap carved out directly after the kernel image; the
/// page allocator manages everything from the heap's end to PHYS_TOP.
pub const HEAP_SIZE: usize = 8 * 1024 * 1024;

// Section boundaries defined in linker.ld. Declared as opaque bytes; only
// their addresses are meaningful.
unsafe extern "C" {
    static __kernel_start: u8;
    static __text_start: u8;
    static __text_end: u8;
    static __user_start: u8;
    static __user_end: u8;
    static __rodata_start: u8;
    static __rodata_end: u8;
    static __data_start: u8;
    static __kernel_end: u8;
}

macro_rules! link_addr {
    ($fn_name:ident, $sym:ident) => {
        #[inline]
        pub fn $fn_name() -> usize {
            (&raw const $sym) as usize
        }
    };
}

link_addr!(kernel_start, __kernel_start);
link_addr!(text_start, __text_start);
link_addr!(text_end, __text_end);
link_addr!(user_start, __user_start);
link_addr!(user_end, __user_end);
link_addr!(rodata_start, __rodata_start);
link_addr!(rodata_end, __rodata_end);
link_addr!(data_start, __data_start);
link_addr!(kernel_end, __kernel_end);

/// First byte of the kernel heap.
#[inline]
pub fn heap_start() -> usize {
    kernel_end()
}

/// First page the physical page allocator owns.
#[inline]
pub fn alloc_start() -> usize {
    heap_start() + HEAP_SIZE
}
