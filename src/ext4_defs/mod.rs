//! # The Defination of Ext4 File System Data Structures
//!
//! The layout of a standard block group is approximately as follows:
//!
//! - Group 0 Padding: 1024 bytes
//! - Superblock: 1 block
//! - Group Descriptors: many blocks
//! - Reserved GDT Blocks: many blocks
//! - Block Bitmap: 1 block
//! - inode Bitmap: 1 block
//! - inode Table: many blocks
//! - Data Blocks: many more blocks
//!
//! For the special case of block group 0, the first 1024 bytes are unused.
//! For all other block groups, there is no padding.

mod bitmap;
mod block_device;
mod block_group;
mod crc;
mod dir_entry;
mod extent;
mod file;
mod inode;
mod mount_point;
mod super_block;
mod xattr;

pub use bitmap::*;
pub use block_device::*;
pub use block_group::*;
pub use dir_entry::*;
pub use extent::*;
pub use file::*;
pub use inode::*;
pub use super_block::*;
pub use xattr::*;

/// All file types. Also matches the defination in directory entries.
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
#[repr(u8)]
pub enum FileType {
    Unknown,
    RegularFile,
    Directory,
    CharacterDev,
    BlockDev,
    Fifo,
    Socket,
    SymLink,
}
