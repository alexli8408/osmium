//! A tiny in-memory filesystem.
//!
//! Flat namespace, no directories, no persistence — a fixed set of files
//! that live in the kernel heap. It exists to give the shell `ls`/`cat` and
//! to back the file syscalls (open/read/close) with something real, so the
//! per-process file-descriptor machinery has a target. Files are created at
//! boot in `init` and are read-only thereafter.

use alloc::string::String;
use alloc::vec::Vec;

use crate::spinlock::SpinLock;

struct File {
    name: String,
    data: Vec<u8>,
}

struct Fs {
    files: Vec<File>,
}

static FS: SpinLock<Fs> = SpinLock::new("fs", Fs { files: Vec::new() });

/// Populate the filesystem with its built-in files. Called once at boot.
pub fn init() {
    let mut fs = FS.lock();
    fs.files.push(File {
        name: String::from("motd"),
        data: Vec::from(*b"welcome to osmium -- a RISC-V kernel in Rust\n"),
    });
    fs.files.push(File {
        name: String::from("readme"),
        data: Vec::from(
            *b"osmium ramfs\n\
               ------------\n\
               a flat, read-only, in-memory filesystem.\n\
               try: ls, cat motd, cat readme\n",
        ),
    });
    fs.files.push(File {
        name: String::from("cpuinfo"),
        data: Vec::from(*b"isa: rv64gc\nharts: 1\nmachine: qemu-virt\n"),
    });
}

/// Names of all files, for `ls`.
pub fn list() -> Vec<(String, usize)> {
    let fs = FS.lock();
    fs.files
        .iter()
        .map(|f| (f.name.clone(), f.data.len()))
        .collect()
}

/// Index of the file named `name`, or None. This index is the stable "inode"
/// the fd table stores.
pub fn lookup(name: &str) -> Option<usize> {
    let fs = FS.lock();
    fs.files.iter().position(|f| f.name == name)
}

/// Copy up to `buf.len()` bytes of file `idx` starting at `offset` into
/// `buf`. Returns the number of bytes copied (0 at or past EOF).
pub fn read_at(idx: usize, offset: usize, buf: &mut [u8]) -> usize {
    let fs = FS.lock();
    let Some(file) = fs.files.get(idx) else {
        return 0;
    };
    if offset >= file.data.len() {
        return 0;
    }
    let n = buf.len().min(file.data.len() - offset);
    buf[..n].copy_from_slice(&file.data[offset..offset + n]);
    n
}
