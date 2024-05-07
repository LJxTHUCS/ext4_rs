//! # The Defination of Ext4 Inode Table Entry
//!
//! The inode table is a linear array of struct `Ext4Inode`. The table is sized to have
//! enough blocks to store at least `sb.inode_size * sb.inodes_per_group` bytes.
//!
//! The number of the block group containing an inode can be calculated as
//! `(inode_number - 1) / sb.inodes_per_group`, and the offset into the group's table is
//! `(inode_number - 1) % sb.inodes_per_group`. There is no inode 0.

use super::crc::*;
use super::BlockDevice;
use super::Ext4BlockGroupDesc;
use super::Ext4ExtentHeader;
use super::Ext4Superblock;
use crate::constants::*;
use crate::prelude::*;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Linux2 {
    pub l_i_blocks_high: u16, // 原来是l_i_reserved1
    pub l_i_file_acl_high: u16,
    pub l_i_uid_high: u16,    // 这两个字段
    pub l_i_gid_high: u16,    // 原来是reserved2[0]
    pub l_i_checksum_lo: u16, // crc32c(uuid+inum+inode) LE
    pub l_i_reserved: u16,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Ext4Inode {
    pub mode: u16,
    pub uid: u16,
    pub size: u32,
    pub atime: u32,
    pub ctime: u32,
    pub mtime: u32,
    pub dtime: u32,
    pub gid: u16,
    pub links_count: u16,
    pub blocks: u32,
    pub flags: u32,
    pub osd1: u32,
    pub block: [u32; 15],
    pub generation: u32,
    pub file_acl: u32,
    pub size_hi: u32,
    pub faddr: u32,   /* Obsoleted fragment address */
    pub osd2: Linux2, // 操作系统相关的字段2

    pub i_extra_isize: u16,
    pub i_checksum_hi: u16,  // crc32c(uuid+inum+inode) BE
    pub i_ctime_extra: u32,  // 额外的修改时间（nsec << 2 | epoch）
    pub i_mtime_extra: u32,  // 额外的文件修改时间（nsec << 2 | epoch）
    pub i_atime_extra: u32,  // 额外的访问时间（nsec << 2 | epoch）
    pub i_crtime: u32,       // 文件创建时间
    pub i_crtime_extra: u32, // 额外的文件创建时间（nsec << 2 | epoch）
    pub i_version_hi: u32,   // 64位版本的高32位
}

impl TryFrom<&[u8]> for Ext4Inode {
    type Error = u64;
    fn try_from(data: &[u8]) -> core::result::Result<Self, u64> {
        let data = &data[..size_of::<Ext4Inode>()];
        Ok(unsafe { core::ptr::read(data.as_ptr() as *const _) })
    }
}

impl Ext4Inode {
    pub fn flags(&self) -> u32 {
        self.flags
    }

    pub fn set_flags(&mut self, f: u32) {
        self.flags |= f;
    }

    pub fn mode(&self) -> u16 {
        self.mode
    }

    pub fn set_mode(&mut self, mode: u16) {
        self.mode |= mode;
    }

    pub fn inode_type(&self, super_block: &Ext4Superblock) -> u32{
        let mut v = self.mode;

        if super_block.creator_os() == EXT4_SUPERBLOCK_OS_HURD{
            v |= ((self.osd2.l_i_file_acl_high as u32 ) << 16) as u16;
        }

        (v & EXT4_INODE_MODE_TYPE_MASK) as u32
    }

    pub fn links_cnt(&self) -> u16 {
        self.links_count
    }

    pub fn set_links_cnt(&mut self, cnt: u16) {
        self.links_count = cnt;
    }

    pub fn set_uid(&mut self, uid: u16) {
        self.uid = uid;
    }

    pub fn set_gid(&mut self, gid: u16) {
        self.gid = gid;
    }

    pub fn size(&mut self) -> u64 {
        self.size as u64 | ((self.size_hi as u64) << 32)
    }

    pub fn set_size(&mut self, size: u64) {
        self.size = ((size << 32) >> 32) as u32;
        self.size_hi = (size >> 32) as u32;
    }

    pub fn set_access_time(&mut self, access_time: u32) {
        self.atime = access_time;
    }

    pub fn set_change_inode_time(&mut self, change_inode_time: u32) {
        self.ctime = change_inode_time;
    }

    pub fn set_modif_time(&mut self, modif_time: u32) {
        self.mtime = modif_time;
    }

    pub fn set_del_time(&mut self, del_time: u32) {
        self.dtime = del_time;
    }

    pub fn set_blocks_count(&mut self, blocks_count: u32) {
        self.blocks = blocks_count;
    }

    pub fn set_generation(&mut self, generation: u32) {
        self.generation = generation;
    }

    pub fn set_extra_isize(&mut self, extra_isize: u16) {
        self.i_extra_isize = extra_isize;
    }

    pub fn set_inode_checksum_value(
        &mut self,
        super_block: &Ext4Superblock,
        _inode_id: u32,
        checksum: u32,
    ) {
        let inode_size = super_block.inode_size();

        self.osd2.l_i_checksum_lo = ((checksum << 16) >> 16) as u16;
        if inode_size > 128 {
            self.i_checksum_hi = (checksum >> 16) as u16;
        }
    }

    pub fn extent_header(&mut self) -> *mut Ext4ExtentHeader {
        let header_ptr = (&mut self.block) as *mut [u32; 15] as *mut Ext4ExtentHeader;
        header_ptr
    }

    pub fn extent_tree_init(&mut self) {
        let mut header = Ext4ExtentHeader::default();
        header.set_depth(0);
        header.set_entries_count(0);
        header.set_generation(0);
        header.set_magic();
        header.set_max_entries_count(4 as u16);

        unsafe {
            let header_ptr = &header as *const Ext4ExtentHeader as *const u32;
            let array_ptr = &mut self.block as *mut [u32; 15] as *mut u32;
            core::ptr::copy_nonoverlapping(header_ptr, array_ptr, 3);
        }
    }

    pub fn blocks_count(&self) -> u64 {
        let mut blocks = self.blocks as u64;
        if self.osd2.l_i_blocks_high != 0 {
            blocks |= (self.osd2.l_i_blocks_high as u64) << 32;
        }
        blocks
    }

    /// Find the position of an inode in the block device.
    ///
    /// Each block group contains `sb.inodes_per_group` inodes.
    /// Because inode 0 is defined not to exist, this formula can
    /// be used to find the block group that an inode lives in:
    /// `bg = (inode_id - 1) / sb.inodes_per_group`.
    ///
    /// The particular inode can be found within the block group's
    /// inode table at `index = (inode_id - 1) % sb.inodes_per_group`.
    /// To get the byte address within the inode table, use
    /// `offset = index * sb.inode_size`.
    fn inode_disk_pos(
        super_block: &Ext4Superblock,
        block_device: Arc<dyn BlockDevice>,
        inode_id: u32,
    ) -> usize {
        let inodes_per_group = super_block.inodes_per_group();
        let inode_size = super_block.inode_size();
        let group = (inode_id - 1) / inodes_per_group;
        let index = (inode_id - 1) % inodes_per_group;

        let bg = Ext4BlockGroupDesc::load(block_device, super_block, group as usize).unwrap();
        bg.inode_table_blk_num() as usize * BLOCK_SIZE + (index * inode_size as u32) as usize
    }

    fn read_from_disk(
        super_block: &Ext4Superblock,
        block_device: Arc<dyn BlockDevice>,
        inode_id: u32,
    ) -> Self {
        let pos = Ext4Inode::inode_disk_pos(super_block, block_device.clone(), inode_id);
        let data = block_device.read_offset(pos);
        let inode_data = &data[..core::mem::size_of::<Ext4Inode>()];
        Ext4Inode::try_from(inode_data).unwrap()
    }

    fn copy_to_byte_slice(&self, slice: &mut [u8]) {
        unsafe {
            let inode_ptr = self as *const Ext4Inode as *const u8;
            let array_ptr = slice.as_ptr() as *mut u8;
            core::ptr::copy_nonoverlapping(inode_ptr, array_ptr, 0x9c);
        }
    }

    fn calc_checksum(&mut self, inode_id: u32, super_block: &Ext4Superblock) -> u32 {
        let inode_size = super_block.inode_size();

        let ino_index = inode_id as u32;
        let ino_gen = self.generation;

        // Preparation: temporarily set bg checksum to 0
        self.osd2.l_i_checksum_lo = 0;
        self.i_checksum_hi = 0;

        let mut checksum = ext4_crc32c(
            EXT4_CRC32_INIT,
            &super_block.uuid(),
            super_block.uuid().len() as u32,
        );
        checksum = ext4_crc32c(checksum, &ino_index.to_le_bytes(), 4);
        checksum = ext4_crc32c(checksum, &ino_gen.to_le_bytes(), 4);

        let mut raw_data = [0u8; 0x100];
        self.copy_to_byte_slice(&mut raw_data);

        // inode checksum
        checksum = ext4_crc32c(checksum, &raw_data, inode_size as u32);

        self.set_inode_checksum_value(super_block, inode_id, checksum);

        if inode_size == 128 {
            checksum &= 0xFFFF;
        }

        checksum
    }

    fn set_checksum(&mut self, super_block: &Ext4Superblock, inode_id: u32) {
        let inode_size = super_block.inode_size();
        let checksum = self.calc_checksum(inode_id, super_block);

        self.osd2.l_i_checksum_lo = ((checksum << 16) >> 16) as u16;
        if inode_size > 128 {
            self.i_checksum_hi = (checksum >> 16) as u16;
        }
    }

    fn sync_to_disk_without_csum(
        &self,
        block_device: Arc<dyn BlockDevice>,
        super_block: &Ext4Superblock,
        inode_id: u32,
    ) -> Result<()> {
        let disk_pos = Self::inode_disk_pos(super_block, block_device.clone(), inode_id);
        let data = unsafe {
            core::slice::from_raw_parts(self as *const _ as *const u8, size_of::<Ext4Inode>())
        };
        block_device.write_offset(disk_pos, data);

        Ok(())
    }

    fn sync_to_disk_with_csum(
        &mut self,
        block_device: Arc<dyn BlockDevice>,
        super_block: &Ext4Superblock,
        inode_id: u32,
    ) -> Result<()> {
        self.set_checksum(super_block, inode_id);
        self.sync_to_disk_without_csum(block_device, super_block, inode_id)
    }
}

/// A combination of an `Ext4Inode` and its id
#[derive(Default)]
pub struct Ext4InodeRef {
    pub inode_id: u32,
    pub inode: Ext4Inode,
}

impl Ext4InodeRef {
    pub fn new(inode_id: u32, inode: Ext4Inode) -> Self {
        Self { inode_id, inode }
    }

    pub fn read_from_disk(
        block_device: Arc<dyn BlockDevice>,
        super_block: &Ext4Superblock,
        inode_id: u32,
    ) -> Self {
        Self::new(
            inode_id,
            Ext4Inode::read_from_disk(super_block, block_device, inode_id),
        )
    }

    pub fn sync_to_disk_without_csum(
        &self,
        block_device: Arc<dyn BlockDevice>,
        super_block: &Ext4Superblock,
    ) -> Result<()> {
        self.inode
            .sync_to_disk_without_csum(block_device, super_block, self.inode_id)
    }

    pub fn sync_to_disk_with_csum(
        &mut self,
        block_device: Arc<dyn BlockDevice>,
        super_block: &Ext4Superblock,
    ) -> Result<()> {
        self.inode
            .sync_to_disk_with_csum(block_device, super_block, self.inode_id)
    }
}
