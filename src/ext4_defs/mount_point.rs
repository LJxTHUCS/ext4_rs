use crate::prelude::*;

/// Mount point descriptor
#[derive(Clone)]
pub struct Ext4MountPoint {
    /**@brief   Mount done flag.*/
    pub mounted: bool,
    /**@brief   Mount point name (@ref ext4_mount)*/
    pub mount_name: String,
    // pub mount_name_string: String,
}

impl Ext4MountPoint {
    pub fn new(name: &str) -> Self {
        Self {
            mounted: false,
            mount_name: name.to_owned(),
            // mount_name_string: name.to_string(),
        }
    }
}

impl Debug for Ext4MountPoint {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        write!(f, "Ext4MountPoint {{ mount_name: {:?} }}", self.mount_name)
    }
}