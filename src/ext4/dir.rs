use super::Ext4;
use crate::constants::*;
use crate::ext4_defs::*;
use crate::prelude::*;

impl Ext4 {
    /// Find a directory entry that matches a given name under a parent directory
    pub(super) fn dir_find_entry(&self, parent: &InodeRef, name: &str) -> Result<DirEntry> {
        info!("Dir find entry {} under parent {}", name, parent.id);
        let inode_size: u32 = parent.inode.size;
        let total_blocks: u32 = inode_size / BLOCK_SIZE as u32;
        let mut iblock: LBlockId = 0;

        while iblock < total_blocks {
            // Get the fs block id
            let fblock = self.extent_get_pblock(parent, iblock)?;
            // Load block from disk
            let block = self.block_device.read_block(fblock);
            // Find the entry in block
            let res = Self::find_entry_in_block(&block, name);
            if let Ok(r) = res {
                return Ok(r);
            }
            iblock += 1;
        }
        Err(Ext4Error::new(ErrCode::ENOENT))
    }

    /// Add an entry to a directory
    pub(super) fn dir_add_entry(
        &mut self,
        parent: &mut InodeRef,
        child: &InodeRef,
        path: &str,
    ) -> Result<()> {
        info!(
            "Dir add entry: parent {}, child {}, path {}",
            parent.id, child.id, path
        );
        let inode_size = parent.inode.size();
        let total_blocks = inode_size as u32 / BLOCK_SIZE as u32;

        // Try finding a block with enough space
        let mut iblock: LBlockId = 0;
        while iblock < total_blocks {
            // Get the parent physical block id
            let fblock = self.extent_get_pblock(parent, iblock)?;
            // Load the parent block from disk
            let mut block = self.block_device.read_block(fblock);
            // Try inserting the entry to parent block
            if self.insert_entry_to_old_block(&mut block, child, path) {
                return Ok(());
            }
            // Current block has no enough space
            iblock += 1;
        }

        // No free block found - needed to allocate a new data block
        // Append a new data block
        let (_, fblock) = self.inode_append_block(parent)?;
        // Load new block
        let mut new_block = self.block_device.read_block(fblock);
        // Write the entry to block
        self.insert_entry_to_new_block(&mut new_block, child, path);

        Ok(())
    }

    /// Remove a entry from a directory
    pub(super) fn dir_remove_entry(&mut self, parent: &mut InodeRef, path: &str) -> Result<()> {
        info!("Dir remove entry: parent {}, path {}", parent.id, path);
        let inode_size = parent.inode.size();
        let total_blocks = inode_size as u32 / BLOCK_SIZE as u32;

        // Check each block
        let mut iblock: LBlockId = 0;
        while iblock < total_blocks {
            // Get the parent physical block id
            let fblock = self.extent_get_pblock(parent, iblock)?;
            // Load the block from disk
            let mut block = self.block_device.read_block(fblock);
            // Try removing the entry
            if self.remove_entry_from_block(&mut block, path) {
                return Ok(());
            }
            // Current block has no enough space
            iblock += 1;
        }

        // Not found the target entry
        Err(Ext4Error::new(ErrCode::ENOENT))
    }

    /// Find a directory entry that matches a given name in a given block
    fn find_entry_in_block(block: &Block, name: &str) -> Result<DirEntry> {
        info!("Dir find entry {} in block {}", name, block.block_id);
        let mut offset = 0;
        while offset < BLOCK_SIZE {
            let de: DirEntry = block.read_offset_as(offset);
            debug!("Dir entry: {} {:?}", de.rec_len(), de.name());
            offset += de.rec_len() as usize;
            if !de.unused() && de.compare_name(name) {
                return Ok(de);
            }
        }
        Err(Ext4Error::new(ErrCode::ENOENT))
    }

    /// Remove a directory entry that matches a given name from a given block
    fn remove_entry_from_block(&self, block: &mut Block, name: &str) -> bool {
        info!("Dir remove entry {} from block {}", name, block.block_id);
        let mut offset = 0;
        while offset < BLOCK_SIZE {
            let mut de: DirEntry = block.read_offset_as(offset);
            offset += de.rec_len() as usize;
            if !de.unused() && de.compare_name(name) {
                // Mark the target entry as unused
                de.set_unused();
                return true;
            }
        }
        false
    }

    /// Insert a directory entry of a child inode into a new parent block.
    /// A new block must have enough space
    fn insert_entry_to_new_block(&self, dst_blk: &mut Block, child: &InodeRef, name: &str) {
        // Set the entry
        let rec_len = BLOCK_SIZE - size_of::<DirEntryTail>();
        let new_entry = DirEntry::new(
            child.id,
            rec_len as u16,
            name,
            inode_mode2file_type(child.inode.mode()),
        );
        // Write entry to block
        dst_blk.write_offset_as(0, &new_entry);

        // Set tail
        let mut tail = DirEntryTail::default();
        tail.rec_len = size_of::<DirEntryTail>() as u16;
        tail.reserved_ft = 0xDE;
        tail.set_csum(&self.super_block, &new_entry, &dst_blk.data[..]);
        // Copy tail to block
        let tail_offset = BLOCK_SIZE - size_of::<DirEntryTail>();
        dst_blk.write_offset_as(tail_offset, &tail);

        // Sync block to disk
        dst_blk.sync_to_disk(self.block_device.clone());
    }

    /// Try insert a directory entry of child inode into a parent block.
    /// Return true if the entry is successfully inserted.
    fn insert_entry_to_old_block(&self, dst_blk: &mut Block, child: &InodeRef, name: &str) -> bool {
        let required_size = DirEntry::required_size(name.len());
        let mut offset = 0;

        while offset < dst_blk.data.len() {
            let mut de: DirEntry = dst_blk.read_offset_as(offset);
            let rec_len = de.rec_len() as usize;

            // Try splitting dir entry
            // The size that `de` actually uses
            let used_size = de.used_size();
            // The rest size
            let free_size = rec_len - used_size;

            // Compare size
            if free_size < required_size {
                // No enough space, try next dir ent
                offset = offset + rec_len;
                continue;
            }
            // Has enough space
            // Update the old entry
            de.set_rec_len(used_size as u16);
            dst_blk.write_offset_as(offset, &de);
            // Insert the new entry
            let new_entry = DirEntry::new(
                child.id,
                free_size as u16,
                name,
                inode_mode2file_type(child.inode.mode()),
            );
            dst_blk.write_offset_as(offset + used_size, &new_entry);

            // Set tail csum
            let tail_offset = BLOCK_SIZE - size_of::<DirEntryTail>();
            let mut tail = dst_blk.read_offset_as::<DirEntryTail>(tail_offset);
            tail.set_csum(&self.super_block, &de, &dst_blk.data[offset..]);
            // Write tail to blk_data
            dst_blk.write_offset_as(tail_offset, &tail);

            // Sync to disk
            dst_blk.sync_to_disk(self.block_device.clone());
            return true;
        }
        false
    }

    /// Create a new directory. `path` is the absolute path of the new directory.
    pub fn mkdir(&mut self, path: &str) -> Result<()> {
        // get open flags
        let iflags = OpenFlags::from_str("w").unwrap();
        self.generic_open(path, iflags, FileType::Directory, &self.read_root_inode())
            .map(|_| {
                info!("ext4_dir_mk: {} ok", path);
            })
    }
}
