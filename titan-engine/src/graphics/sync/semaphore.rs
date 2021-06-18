use std::error::Error;

use ash::version::DeviceV1_0;
use ash::vk;

use proc_macro::SlotMappable;

use super::super::{
    super::slotmap::SlotMappable,
    device::{self, Device},
    utils,
};

slotmap::new_key_type! {
    pub struct Key;
}

#[derive(SlotMappable)]
pub struct Semaphore {
    key: Key,
    handle: vk::Semaphore,
    parent_device: device::Key,
}

impl Semaphore {
    pub fn new(key: Key, device_key: device::Key) -> Result<Self, Box<dyn Error>> {
        let slotmap_device = Device::slotmap().read()?;
        let device = slotmap_device
            .get(device_key)
            .ok_or_else(|| utils::make_error("device not found"))?;

        let create_info = vk::SemaphoreCreateInfo::builder();
        let handle = unsafe { device.loader().create_semaphore(&create_info, None)? };
        Ok(Self {
            key,
            handle,
            parent_device: device_key,
        })
    }

    pub fn handle(&self) -> vk::Semaphore {
        self.handle
    }

    pub fn parent_device(&self) -> device::Key {
        self.parent_device
    }
}

impl Drop for Semaphore {
    fn drop(&mut self) {
        let slotmap_device = match Device::slotmap().read() {
            Ok(value) => value,
            Err(_) => return,
        };
        let device = match slotmap_device.get(self.parent_device()) {
            None => return,
            Some(value) => value,
        };
        unsafe { device.loader().destroy_semaphore(self.handle, None) }
    }
}