use super::utils::*;
use super::Ext4;
use crate::constants::*;
use crate::ext4_defs::*;
use crate::prelude::*;

impl Ext4 {
    /// Find a directory entry that matches a given name under a parent directory
    ///
    /// Save the result in `Ext4DirSearchResult`
    pub fn dir_find_entry(
        &self,
        parent: &mut Ext4InodeRef,
        name: &str,
        result: &mut Ext4DirSearchResult,
    ) -> usize {
        let mut iblock: Ext4LogicBlockId = 0;
        let mut fblock: Ext4FsBlockId = 0;

        let inode_size: u32 = parent.inode.size;
        let total_blocks: u32 = inode_size / BLOCK_SIZE as u32;

        while iblock < total_blocks {
            // Get the fs block id
            self.ext4_fs_get_inode_dblk_idx(parent, &mut iblock, &mut fblock, false);
            // Load block from disk
            let mut data = self.block_device.read_offset(fblock as usize * BLOCK_SIZE);
            let mut ext4_block = Ext4Block {
                logical_block_id: iblock,
                disk_block_id: fblock,
                block_data: &mut data,
                dirty: false,
            };
            // Find the entry in block
            let r = Self::find_entry_in_block(&mut ext4_block, name, result);
            if r {
                return EOK;
            }
            iblock += 1
        }
        ENOENT
    }

    /// Find a directory entry that matches a given name in a given block
    ///
    /// Save the result in `Ext4DirSearchResult`
    fn find_entry_in_block(
        block: &Ext4Block,
        name: &str,
        result: &mut Ext4DirSearchResult,
    ) -> bool {
        let mut offset = 0;
        while offset < block.block_data.len() {
            let de = Ext4DirEntry::try_from(&block.block_data[offset..]).unwrap();
            offset += de.rec_len() as usize;
            // Unused dir entry
            if de.unused() {
                continue;
            }
            // Compare name
            if de.compare_name(name) {
                result.dentry = de;
                return true;
            }
        }
        false
    }

    /// Add an entry to a directory
    pub fn dir_add_entry(
        &self,
        parent: &mut Ext4InodeRef,
        child: &mut Ext4InodeRef,
        path: &str,
    ) -> usize {
        let block_size = self.super_block.block_size();
        let inode_size = parent.inode.size();
        let total_blocks = inode_size as u32 / block_size;

        let mut iblock = 0;
        let mut fblock: Ext4FsBlockId = 0;

        // Try finding a block with enough space
        while iblock < total_blocks {
            // Get the parent fs block id
            self.ext4_fs_get_inode_dblk_idx(parent, &mut iblock, &mut fblock, false);
            // Load the parent block from disk
            let mut data = self.block_device.read_offset(fblock as usize * BLOCK_SIZE);
            let mut ext4_block = Ext4Block {
                logical_block_id: iblock,
                disk_block_id: fblock,
                block_data: &mut data,
                dirty: false,
            };
            // Try inserting the entry to parent block
            let r = self.insert_entry_to_old_block(&mut ext4_block, child, path);
            if r == EOK {
                return EOK;
            }
            // Current block has no enough space
            iblock += 1;
        }

        // No free block found - needed to allocate a new data block
        iblock = 0;
        fblock = 0;
        // Append a new data block
        self.ext4_fs_append_inode_dblk(parent, &mut iblock, &mut fblock);
        // Load new block
        let block_device = self.block_device.clone();
        let mut data = block_device.read_offset(fblock as usize * BLOCK_SIZE);
        let mut new_block = Ext4Block {
            logical_block_id: iblock,
            disk_block_id: fblock,
            block_data: &mut data,
            dirty: false,
        };
        // Write the entry to block
        self.insert_entry_to_new_block(&mut new_block, child, path);

        EOK
    }

    /// Insert a directory entry of a child inode into a new parent block.
    /// A new block must have enough space
    fn insert_entry_to_new_block(
        &self,
        dst_blk: &mut Ext4Block,
        child: &mut Ext4InodeRef,
        name: &str,
    ) {
        // Set the entry
        let mut new_entry = Ext4DirEntry::default();
        let rec_len = BLOCK_SIZE - size_of::<Ext4DirEntryTail>();
        Self::set_dir_entry(&mut new_entry, rec_len as u16, &child, name);

        // Write to block
        new_entry.copy_to_byte_slice(&mut dst_blk.block_data, 0);

        // Set tail
        let mut tail = Ext4DirEntryTail::default();
        tail.rec_len = size_of::<Ext4DirEntryTail>() as u16;
        tail.reserved_ft = 0xDE;
        tail.reserved_zero1 = 0;
        tail.reserved_zero2 = 0;
        tail.set_csum(&self.super_block, &new_entry, &dst_blk.block_data[..]);

        // Copy to block
        let tail_offset = BLOCK_SIZE - size_of::<Ext4DirEntryTail>();
        tail.copy_to_byte_slice(&mut dst_blk.block_data, tail_offset);

        // Sync to disk
        dst_blk.sync_to_disk(self.block_device.clone());
    }

    /// Try insert a directory entry of child inode into a parent block.
    /// Return `ENOSPC` if parent block has no enough space.
    fn insert_entry_to_old_block(
        &self,
        dst_blk: &mut Ext4Block,
        child: &mut Ext4InodeRef,
        name: &str,
    ) -> usize {
        let required_size = Ext4DirEntry::required_size(name.len());
        let mut offset = 0;

        while offset < dst_blk.block_data.len() {
            let mut de = Ext4DirEntry::try_from(&dst_blk.block_data[offset..]).unwrap();
            if de.unused() {
                continue;
            }
            // Split valid dir entry
            let rec_len = de.rec_len();

            // The actual size that `de` uses
            let used_size = de.used_size();
            // The rest size
            let free_size = rec_len as usize - used_size;
            // Compare size
            if free_size < required_size {
                // No enough space, try next dir ent
                offset = offset + rec_len as usize;
                continue;
            }
            // Has enough space
            // Set the entry
            de.set_rec_len(free_size as u16);
            let mut new_entry = Ext4DirEntry::default();
            Self::set_dir_entry(&mut new_entry, free_size as u16, &child, name);

            // Write dir entries to blk_data
            de.copy_to_byte_slice(&mut dst_blk.block_data, offset);
            new_entry.copy_to_byte_slice(&mut dst_blk.block_data, offset + used_size);

            // Set tail csum
            let mut tail = Ext4DirEntryTail::from(&mut dst_blk.block_data, BLOCK_SIZE).unwrap();
            tail.set_csum(&self.super_block, &de, &dst_blk.block_data[offset..]);
            let parent_de = Ext4DirEntry::try_from(&dst_blk.block_data[..]).unwrap();
            tail.set_csum(&self.super_block, &parent_de, &dst_blk.block_data[..]);

            // Write tail to blk_data
            let tail_offset = BLOCK_SIZE - size_of::<Ext4DirEntryTail>();
            tail.copy_to_byte_slice(&mut dst_blk.block_data, tail_offset);

            // Sync to disk
            dst_blk.sync_to_disk(self.block_device.clone());

            return EOK;
        }
        ENOSPC
    }

    /// Set the directory entry for an inode
    fn set_dir_entry(en: &mut Ext4DirEntry, rec_len: u16, child: &Ext4InodeRef, name: &str) {
        en.set_inode(child.inode_id);
        en.set_rec_len(rec_len);
        en.set_entry_type(child.inode.mode());
        en.set_name(name);
    }

    pub fn ext4_dir_mk(&self, path: &str) -> Result<usize> {
        let mut file = Ext4File::new();
        let flags = "w";

        let filetype = FileType::Directory;

        // get mount point
        let mut ptr = Box::new(self.mount_point.clone());
        file.mp = Box::as_mut(&mut ptr) as *mut Ext4MountPoint;

        // get open flags
        let iflags = ext4_parse_flags(flags).unwrap();

        if iflags & O_CREAT != 0 {
            self.ext4_trans_start();
        }

        let mut root_inode_ref = self.get_root_inode_ref();

        let r = self.ext4_generic_open(&mut file, path, iflags, filetype, &mut root_inode_ref);
        r
    }
}