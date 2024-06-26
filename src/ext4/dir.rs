use super::Ext4;
use crate::constants::*;
use crate::ext4_defs::*;
use crate::prelude::*;
use crate::return_error;

impl Ext4 {
    /// Find a directory entry that matches a given name under a parent directory
    pub(super) fn dir_find_entry(&self, dir: &InodeRef, name: &str) -> Result<DirEntry> {
        trace!("Dir find entry: dir {}, name {}", dir.id, name);
        let total_blocks: u32 = dir.inode.block_count() as u32;
        let mut iblock: LBlockId = 0;
        while iblock < total_blocks {
            // Get the fs block id
            let fblock = self.extent_query(dir, iblock)?;
            // Load block from disk
            let block = self.read_block(fblock);
            // Find the entry in block
            let res = Self::find_entry_in_block(&block, name);
            if let Some(r) = res {
                return Ok(r);
            }
            iblock += 1;
        }
        return_error!(
            ErrCode::ENOENT,
            "Directory entry not found: dir {}, name {}",
            dir.id,
            name
        );
    }

    /// Add an entry to a directory, memory consistency guaranteed
    pub(super) fn dir_add_entry(
        &self,
        dir: &mut InodeRef,
        child: &InodeRef,
        name: &str,
    ) -> Result<()> {
        trace!(
            "Dir add entry: dir {}, child {}, name {}",
            dir.id,
            child.id,
            name
        );
        let total_blocks: u32 = dir.inode.block_count() as u32;

        // Try finding a block with enough space
        let mut iblock: LBlockId = 0;
        while iblock < total_blocks {
            // Get the parent physical block id
            let fblock = self.extent_query(dir, iblock).unwrap();
            // Load the parent block from disk
            let mut block = self.read_block(fblock);
            // Try inserting the entry to parent block
            if self.insert_entry_to_old_block(dir, child, name, &mut block) {
                return Ok(());
            }
            // Current block has no enough space
            iblock += 1;
        }

        // No free block found - needed to allocate a new data block
        // Append a new data block
        let (_, fblock) = self.inode_append_block(dir)?;
        // Load new block
        let mut new_block = self.read_block(fblock);
        // Write the entry to block
        self.insert_entry_to_new_block(dir, child, name, &mut new_block);
        // Update inode size
        dir.inode.set_size(dir.inode.size() + BLOCK_SIZE as u64);

        Ok(())
    }

    /// Remove a entry from a directory
    pub(super) fn dir_remove_entry(&self, dir: &InodeRef, name: &str) -> Result<()> {
        trace!("Dir remove entry: dir {}, name {}", dir.id, name);
        let total_blocks: u32 = dir.inode.block_count() as u32;
        // Check each block
        let mut iblock: LBlockId = 0;
        while iblock < total_blocks {
            // Get the parent physical block id
            let fblock = self.extent_query(dir, iblock).unwrap();
            // Load the block from disk
            let mut block = self.read_block(fblock);
            // Try removing the entry
            if Self::remove_entry_from_block(&mut block, name) {
                self.write_block(&block);
                return Ok(());
            }
            // Current block has no enough space
            iblock += 1;
        }
        // Not found the target entry
        return_error!(
            ErrCode::ENOENT,
            "Directory entry not found: dir {}, name {}",
            dir.id,
            name
        );
    }

    /// Get all entries under a directory
    pub(super) fn dir_get_all_entries(&self, dir: &InodeRef) -> Vec<DirEntry> {
        let total_blocks = dir.inode.block_count() as u32;
        let mut entries: Vec<DirEntry> = Vec::new();
        let mut iblock: LBlockId = 0;
        while iblock < total_blocks {
            // Get the fs block id
            let fblock = self.extent_query(dir, iblock).unwrap();
            // Load block from disk
            let block = self.read_block(fblock);
            // Get all entries from block
            Self::get_all_entries_from_block(&block, &mut entries);
            iblock += 1;
        }
        entries
    }

    /// Find a directory entry that matches a given name in a given block
    fn find_entry_in_block(block: &Block, name: &str) -> Option<DirEntry> {
        let mut offset = 0;
        while offset < BLOCK_SIZE {
            let de: DirEntry = block.read_offset_as(offset);
            if !de.unused() && de.compare_name(name) {
                return Some(de);
            }
            offset += de.rec_len() as usize;
        }
        None
    }

    /// Remove a directory entry that matches a given name from a given block
    fn remove_entry_from_block(block: &mut Block, name: &str) -> bool {
        let mut offset = 0;
        while offset < BLOCK_SIZE {
            let mut de: DirEntry = block.read_offset_as(offset);
            if !de.unused() && de.compare_name(name) {
                // Mark the target entry as unused
                de.set_unused();
                block.write_offset_as(offset, &de);
                return true;
            }
            offset += de.rec_len() as usize;
        }
        false
    }

    /// Get all directory entries from a given block
    fn get_all_entries_from_block(block: &Block, entries: &mut Vec<DirEntry>) {
        let mut offset = 0;
        while offset < BLOCK_SIZE {
            let de: DirEntry = block.read_offset_as(offset);
            offset += de.rec_len() as usize;
            if !de.unused() {
                trace!("Dir entry: {:?} {}", de.name(), de.inode());
                entries.push(de);
            }
        }
    }

    /// Insert a directory entry of a child inode into a new parent block.
    /// A new block must have enough space
    fn insert_entry_to_new_block(
        &self,
        dir: &InodeRef,
        child: &InodeRef,
        name: &str,
        dst_blk: &mut Block,
    ) {
        // Set the entry
        let rec_len = BLOCK_SIZE - size_of::<DirEntryTail>();
        let new_entry = DirEntry::new(child.id, rec_len as u16, name, child.inode.file_type());
        // Write entry to block
        dst_blk.write_offset_as(0, &new_entry);

        // Set tail
        let mut tail = DirEntryTail::new();
        tail.set_csum(
            &self.read_super_block().uuid(),
            dir.id,
            dir.inode.generation(),
            &dst_blk,
        );
        // Copy tail to block
        let tail_offset = BLOCK_SIZE - size_of::<DirEntryTail>();
        dst_blk.write_offset_as(tail_offset, &tail);

        // Sync block to disk
        self.write_block(&dst_blk);
    }

    /// Try insert a directory entry of child inode into a parent block.
    /// Return true if the entry is successfully inserted.
    fn insert_entry_to_old_block(
        &self,
        dir: &InodeRef,
        child: &InodeRef,
        name: &str,
        dst_blk: &mut Block,
    ) -> bool {
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
            let new_entry =
                DirEntry::new(child.id, free_size as u16, name, child.inode.file_type());
            dst_blk.write_offset_as(offset + used_size, &new_entry);

            // Set tail csum
            let tail_offset = BLOCK_SIZE - size_of::<DirEntryTail>();
            let mut tail = dst_blk.read_offset_as::<DirEntryTail>(tail_offset);
            tail.set_csum(
                &self.read_super_block().uuid(),
                dir.id,
                dir.inode.generation(),
                &dst_blk,
            );
            // Write tail to blk_data
            dst_blk.write_offset_as(tail_offset, &tail);

            // Sync to disk
            self.write_block(&dst_blk);
            return true;
        }
        false
    }
}
