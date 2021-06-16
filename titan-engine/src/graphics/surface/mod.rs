use std::error::Error;

use ash::vk;
use ash_window::create_surface;
use winit::window::Window;

use super::{instance, utils, PhysicalDevice};

pub mod slotmap;

type SurfaceLoader = ash::extensions::khr::Surface;

pub struct Surface {
    handle: vk::SurfaceKHR,
    loader: SurfaceLoader,
    parent_instance: instance::slotmap::Key,
}

impl Surface {
    pub fn new(
        instance_key: instance::slotmap::Key,
        window: &Window,
    ) -> Result<Self, Box<dyn Error>> {
        let slotmap = super::instance::slotmap::read()?;
        let instance = slotmap
            .get(instance_key)
            .ok_or_else(|| utils::make_error("instance not found"))?;

        let loader = SurfaceLoader::new(instance.entry_loader(), instance.loader());
        let handle =
            unsafe { create_surface(instance.entry_loader(), instance.loader(), window, None) }?;
        Ok(Self {
            loader,
            handle,
            parent_instance: instance_key,
        })
    }

    pub fn handle(&self) -> vk::SurfaceKHR {
        self.handle
    }

    pub fn parent_instance(&self) -> instance::slotmap::Key {
        self.parent_instance
    }

    pub fn physical_device_capabilities(
        &self,
        physical_device: &PhysicalDevice,
    ) -> Result<vk::SurfaceCapabilitiesKHR, Box<dyn Error>> {
        let capabilities = unsafe {
            self.loader
                .get_physical_device_surface_capabilities(physical_device.handle(), self.handle)?
        };
        Ok(capabilities)
    }

    pub fn physical_device_formats(
        &self,
        physical_device: &PhysicalDevice,
    ) -> Result<Vec<vk::SurfaceFormatKHR>, Box<dyn Error>> {
        let formats = unsafe {
            self.loader
                .get_physical_device_surface_formats(physical_device.handle(), self.handle)?
        };
        Ok(formats)
    }

    pub fn physical_device_present_modes(
        &self,
        physical_device: &PhysicalDevice,
    ) -> Result<Vec<vk::PresentModeKHR>, Box<dyn Error>> {
        let present_modes = unsafe {
            self.loader
                .get_physical_device_surface_present_modes(physical_device.handle(), self.handle)?
        };
        Ok(present_modes)
    }

    pub fn physical_device_queue_family_properties_support<'a>(
        &'a self,
        physical_device: &'a PhysicalDevice,
    ) -> impl Iterator<Item = (usize, &'a vk::QueueFamilyProperties)> {
        physical_device
            .queue_family_properties()
            .iter()
            .enumerate()
            .filter(move |(index, _queue_family_properties)| unsafe {
                self.loader
                    .get_physical_device_surface_support(
                        physical_device.handle(),
                        *index as u32,
                        self.handle,
                    )
                    .unwrap_or(false)
            })
    }

    pub fn is_suitable(&self, physical_device: &PhysicalDevice) -> Result<bool, Box<dyn Error>> {
        let formats = self.physical_device_formats(physical_device)?;
        let present_modes = self.physical_device_present_modes(physical_device)?;
        Ok(!formats.is_empty() && !present_modes.is_empty())
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe { self.loader.destroy_surface(self.handle, None) };
    }
}