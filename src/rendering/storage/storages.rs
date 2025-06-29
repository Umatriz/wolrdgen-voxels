use ash::{ext, khr, vk};
use bevy_app::Plugin;
use bevy_ecs::schedule::IntoScheduleConfigs;

use super::{Destroy, Destroyable, destroy_storage_single};

pub struct CommonStoragesPlugin;

impl Plugin for CommonStoragesPlugin {
    fn build(&self, app: &mut bevy_app::App) {
        app.add_systems(
            Destroy,
            (
                destroy_storage_single::<ash::Device>(),
                destroy_storage_single::<SurfacePack>(),
                destroy_storage_single::<ash::Instance>(),
            )
                .chain(),
        );
    }
}

impl Destroyable for ash::Instance {
    type Params = ();

    fn destroy(&mut self, _params: &mut Self::Params) {
        unsafe { self.destroy_instance(None) };
    }
}

pub type SurfacePack = (khr::surface::Instance, vk::SurfaceKHR);
impl Destroyable for SurfacePack {
    type Params = ();

    fn destroy(&mut self, _params: &mut Self::Params) {
        unsafe { self.0.destroy_surface(self.1, None) };
    }
}

pub type DebugUtilsPack = (ext::debug_utils::Instance, vk::DebugUtilsMessengerEXT);
impl Destroyable for DebugUtilsPack {
    type Params = ();

    fn destroy(&mut self, params: &mut Self::Params) {
        unsafe { self.0.destroy_debug_utils_messenger(self.1, None) };
    }
}

impl Destroyable for ash::Device {
    type Params = ();

    fn destroy(&mut self, _params: &mut Self::Params) {
        unsafe { self.destroy_device(None) };
    }
}
