use super::crc::*;
use super::BlockDevice;
use super::Ext4Block;
use super::Ext4Superblock;
use crate::constants::*;
use crate::prelude::*;

#[repr(C)]
pub union Ext4DirEnInner {
    pub name_length_high: u8, // 高8位的文件名长度
    pub inode_type: u8,       // 引用的inode的类型（在rev >= 0.5中）
}

impl Debug for Ext4DirEnInner {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        unsafe {
            write!(
                f,
                "Ext4DirEnInternal {{ name_length_high: {:?} }}",
                self.name_length_high
            )
        }
    }
}

impl Default for Ext4DirEnInner {
    fn default() -> Self {
        Self {
            name_length_high: 0,
        }
    }
}

#[repr(C)]
#[derive(Debug)]
pub struct Ext4DirEntry {
    pub inode: u32,            // 该目录项指向的inode的编号
    pub entry_len: u16,        // 到下一个目录项的距离
    pub name_len: u8,          // 低8位的文件名长度
    pub inner: Ext4DirEnInner, // 联合体成员
    pub name: [u8; 255],       // 文件名
}

impl Default for Ext4DirEntry {
    fn default() -> Self {
        Self {
            inode: 0,
            entry_len: 0,
            name_len: 0,
            inner: Ext4DirEnInner::default(),
            name: [0; 255],
        }
    }
}

impl<T> TryFrom<&[T]> for Ext4DirEntry {
    type Error = u64;
    fn try_from(data: &[T]) -> core::result::Result<Self, u64> {
        let data = data;
        Ok(unsafe { core::ptr::read(data.as_ptr() as *const _) })
    }
}

impl Ext4DirEntry {
    pub fn get_name(&self) -> String {
        let name_len = self.name_len as usize;
        let name = &self.name[..name_len];
        let name = core::str::from_utf8(name).unwrap();
        name.to_string()
    }

    pub fn get_name_len(&self) -> usize {
        let name_len = self.name_len as usize;
        name_len
    }

    pub fn ext4_dir_get_csum(&self, s: &Ext4Superblock, blk_data: &[u8]) -> u32 {
        let ino_index = self.inode;
        let ino_gen = 0 as u32;

        let uuid = s.uuid();

        let mut csum = ext4_crc32c(EXT4_CRC32_INIT, &uuid, uuid.len() as u32);
        csum = ext4_crc32c(csum, &ino_index.to_le_bytes(), 4);
        csum = ext4_crc32c(csum, &ino_gen.to_le_bytes(), 4);
        let mut data = [0u8; 0xff4];
        unsafe {
            core::ptr::copy_nonoverlapping(blk_data.as_ptr(), data.as_mut_ptr(), blk_data.len());
        }
        csum = ext4_crc32c(csum, &data[..], 0xff4);
        csum
    }

    pub fn write_de_to_blk(&self, dst_blk: &mut Ext4Block, offset: usize) {
        let count = core::mem::size_of::<Ext4DirEntry>() / core::mem::size_of::<u8>();
        let data = unsafe { core::slice::from_raw_parts(self as *const _ as *const u8, count) };
        dst_blk.block_data.splice(
            offset..offset + core::mem::size_of::<Ext4DirEntry>(),
            data.iter().cloned(),
        );
        // assert_eq!(dst_blk.block_data[offset..offset + core::mem::size_of::<Ext4DirEntry>()], data[..]);
    }
}

pub fn copy_dir_entry_to_array(header: &Ext4DirEntry, array: &mut [u8], offset: usize) {
    unsafe {
        let de_ptr = header as *const Ext4DirEntry as *const u8;
        let array_ptr = array as *mut [u8] as *mut u8;
        let count = core::mem::size_of::<Ext4DirEntry>() / core::mem::size_of::<u8>();
        core::ptr::copy_nonoverlapping(de_ptr, array_ptr.add(offset), count);
    }
}

pub fn copy_diren_tail_to_array(dir_en: &Ext4DirEntryTail, array: &mut [u8], offset: usize) {
    unsafe {
        let de_ptr = dir_en as *const Ext4DirEntryTail as *const u8;
        let array_ptr = array as *mut [u8] as *mut u8;
        let count = core::mem::size_of::<Ext4DirEntryTail>();
        core::ptr::copy_nonoverlapping(de_ptr, array_ptr.add(offset), count);
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Ext4DirEntryTail {
    pub reserved_zero1: u32,
    pub rec_len: u16,
    pub reserved_zero2: u8,
    pub reserved_ft: u8,
    pub checksum: u32, // crc32c(uuid+inum+dirblock)
}

impl Ext4DirEntryTail {
    pub fn from(data: &mut [u8], blocksize: usize) -> Option<Self> {
        unsafe {
            let ptr = data as *mut [u8] as *mut u8;
            let t = *(ptr.add(blocksize - core::mem::size_of::<Ext4DirEntryTail>())
                as *mut Ext4DirEntryTail);
            if t.reserved_zero1 != 0 || t.reserved_zero2 != 0 {
                log::info!("t.reserved_zero1");
                return None;
            }
            if t.rec_len.to_le() != core::mem::size_of::<Ext4DirEntryTail>() as u16 {
                log::info!("t.rec_len");
                return None;
            }
            if t.reserved_ft != 0xDE {
                log::info!("t.reserved_ft");
                return None;
            }
            Some(t)
        }
    }

    pub fn ext4_dir_set_csum(&mut self, s: &Ext4Superblock, diren: &Ext4DirEntry, blk_data: &[u8]) {
        let csum = diren.ext4_dir_get_csum(s, blk_data);
        self.checksum = csum;
    }

    #[allow(unused)]
    pub fn write_de_tail_to_blk(&self, dst_blk: &mut Ext4Block, offset: usize) {
        let data = unsafe { core::slice::from_raw_parts(self as *const _ as *const u8, 0x20) };
        dst_blk.block_data.splice(
            offset..offset + core::mem::size_of::<Ext4DirEntryTail>(),
            data.iter().cloned(),
        );
        assert_eq!(
            dst_blk.block_data[offset..offset + core::mem::size_of::<Ext4DirEntryTail>()],
            data[..]
        );
    }

    #[allow(unused)]
    pub fn sync_de_tail_to_disk(
        &self,
        block_device: Arc<dyn BlockDevice>,
        dst_blk: &mut Ext4Block,
    ) {
        let offset = BASE_OFFSET as usize - core::mem::size_of::<Ext4DirEntryTail>();

        let data = unsafe {
            core::slice::from_raw_parts(
                self as *const _ as *const u8,
                core::mem::size_of::<Ext4DirEntryTail>(),
            )
        };
        dst_blk.block_data.splice(
            offset..offset + core::mem::size_of::<Ext4DirEntryTail>(),
            data.iter().cloned(),
        );
        assert_eq!(
            dst_blk.block_data[offset..offset + core::mem::size_of::<Ext4DirEntryTail>()],
            data[..]
        );
        block_device.write_offset(
            dst_blk.disk_block_id as usize * BLOCK_SIZE,
            &dst_blk.block_data,
        );
    }
}

#[allow(unused)]
pub fn copy_diren_to_array(diren: &Ext4DirEntry, array: &mut [u8]) {
    unsafe {
        let diren_ptr = diren as *const Ext4DirEntry as *const u8;
        let array_ptr = array as *mut [u8] as *mut u8;
        core::ptr::copy_nonoverlapping(diren_ptr, array_ptr, core::mem::size_of::<Ext4DirEntry>());
    }
}

pub struct Ext4DirSearchResult<'a> {
    pub block: Ext4Block<'a>,
    pub dentry: Ext4DirEntry,
}

impl<'a> Ext4DirSearchResult<'a> {
    pub fn new(block: Ext4Block<'a>, dentry: Ext4DirEntry) -> Self {
        Self { block, dentry }
    }
}

/// fake dir entry
#[repr(C)]
pub struct Ext4FakeDirEntry {
    inode: u32,
    entry_length: u16,
    name_length: u8,
    inode_type: u8,
}

bitflags! {
    #[derive(PartialEq, Eq)]
    pub struct DirEntryType: u8 {
        const EXT4_DE_UNKNOWN = 0;
        const EXT4_DE_REG_FILE = 1;
        const EXT4_DE_DIR = 2;
        const EXT4_DE_CHRDEV = 3;
        const EXT4_DE_BLKDEV = 4;
        const EXT4_DE_FIFO = 5;
        const EXT4_DE_SOCK = 6;
        const EXT4_DE_SYMLINK = 7;
    }
}

pub fn ext4_fs_correspond_inode_mode(filetype: u8) -> u32 {
    let file_type = DirEntryType::from_bits(filetype).unwrap();
    match file_type {
        DirEntryType::EXT4_DE_DIR => EXT4_INODE_MODE_DIRECTORY as u32,
        DirEntryType::EXT4_DE_REG_FILE => EXT4_INODE_MODE_FILE as u32,
        DirEntryType::EXT4_DE_SYMLINK => EXT4_INODE_MODE_SOFTLINK as u32,
        DirEntryType::EXT4_DE_CHRDEV => EXT4_INODE_MODE_CHARDEV as u32,
        DirEntryType::EXT4_DE_BLKDEV => EXT4_INODE_MODE_BLOCKDEV as u32,
        DirEntryType::EXT4_DE_FIFO => EXT4_INODE_MODE_FIFO as u32,
        DirEntryType::EXT4_DE_SOCK => EXT4_INODE_MODE_SOCKET as u32,
        _ => {
            // FIXME: unsupported filetype
            EXT4_INODE_MODE_FILE as u32
        }
    }
}
