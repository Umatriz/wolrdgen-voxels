use ash::{ext, khr, vk};
use bevy_app::Plugin;
use bevy_ecs::schedule::IntoScheduleConfigs;

use super::{
    Destroy, Destroyable, Single, Storage, StoragesAppExt, destroy_storage,
    destroy_storage_handled, optional,
};

pub struct CommonStoragesPlugin;

impl Plugin for CommonStoragesPlugin {
    fn build(&self, app: &mut bevy_app::App) {
        app.register_handled_storage::<vk::Framebuffer>()
            .register_handled_storage::<vk::ImageView>()
            .register_handled_storage::<vk::Semaphore>()
            .register_handled_storage::<vk::Fence>()
            .register_handled_storage::<vk::CommandPool>()
            .register_handled_storage::<vk::Pipeline>()
            .register_handled_storage::<vk::PipelineLayout>()
            .register_handled_storage::<vk::RenderPass>();

        app.add_systems(
            Destroy,
            (
                (
                    destroy_storage_handled::<vk::Framebuffer>(),
                    destroy_storage_handled::<vk::ImageView>(),
                    destroy_storage::<SwapchainPack>(),
                ),
                destroy_storage_handled::<vk::Semaphore>(),
                destroy_storage_handled::<vk::Fence>(),
                destroy_storage_handled::<vk::CommandPool>(),
                destroy_storage_handled::<vk::Pipeline>(),
                destroy_storage_handled::<vk::PipelineLayout>(),
                destroy_storage_handled::<vk::RenderPass>(),
                destroy_storage::<ash::Device>(),
                optional(destroy_storage::<DebugUtilsPack>()),
                destroy_storage::<SurfacePack>(),
                destroy_storage::<ash::Instance>(),
            )
                .chain(),
        );
    }
}

impl Destroyable for ash::Instance {
    type Params<'w, 's> = ();

    fn destroy(&mut self, _params: &mut Self::Params<'_, '_>) {
        unsafe { self.destroy_instance(None) };
    }
}

pub type SurfacePack = (khr::surface::Instance, vk::SurfaceKHR);
impl Destroyable for SurfacePack {
    type Params<'w, 's> = ();

    fn destroy(&mut self, _params: &mut Self::Params<'_, '_>) {
        unsafe { self.0.destroy_surface(self.1, None) };
    }
}

pub type DebugUtilsPack = (ext::debug_utils::Instance, vk::DebugUtilsMessengerEXT);
impl Destroyable for DebugUtilsPack {
    type Params<'w, 's> = ();

    fn destroy(&mut self, params: &mut Self::Params<'_, '_>) {
        unsafe { self.0.destroy_debug_utils_messenger(self.1, None) };
    }
}

pub type DeviceStorage<'w> = Storage<'w, Single<ash::Device>>;
impl Destroyable for ash::Device {
    type Params<'w, 's> = ();

    fn destroy(&mut self, _params: &mut Self::Params<'_, '_>) {
        unsafe { self.destroy_device(None) };
    }
}

impl Destroyable for vk::RenderPass {
    type Params<'w, 's> = DeviceStorage<'w>;

    fn destroy(&mut self, params: &mut Self::Params<'_, '_>) {
        unsafe { params.data.destroy_render_pass(*self, None) };
    }
}

impl Destroyable for vk::PipelineLayout {
    type Params<'w, 's> = DeviceStorage<'w>;

    fn destroy(&mut self, params: &mut Self::Params<'_, '_>) {
        unsafe { params.data.destroy_pipeline_layout(*self, None) };
    }
}

impl Destroyable for vk::Pipeline {
    type Params<'w, 's> = DeviceStorage<'w>;

    fn destroy(&mut self, params: &mut Self::Params<'_, '_>) {
        unsafe {
            params.data.destroy_pipeline(*self, None);
        }
    }
}

impl Destroyable for vk::CommandPool {
    type Params<'w, 's> = DeviceStorage<'w>;

    fn destroy(&mut self, params: &mut Self::Params<'_, '_>) {
        unsafe {
            params.data.destroy_command_pool(*self, None);
        }
    }
}

impl Destroyable for vk::Fence {
    type Params<'w, 's> = DeviceStorage<'w>;

    fn destroy(&mut self, params: &mut Self::Params<'_, '_>) {
        unsafe {
            params.data.destroy_fence(*self, None);
        }
    }
}

impl Destroyable for vk::Semaphore {
    type Params<'w, 's> = DeviceStorage<'w>;

    fn destroy(&mut self, params: &mut Self::Params<'_, '_>) {
        unsafe {
            params.data.destroy_semaphore(*self, None);
        }
    }
}

pub type SwapchainPack = (khr::swapchain::Device, vk::SwapchainKHR);
impl Destroyable for SwapchainPack {
    type Params<'w, 's> = ();

    fn destroy(&mut self, _params: &mut Self::Params<'_, '_>) {
        unsafe {
            self.0.destroy_swapchain(self.1, None);
        }
    }
}

impl Destroyable for vk::ImageView {
    type Params<'w, 's> = DeviceStorage<'w>;

    fn destroy(&mut self, params: &mut Self::Params<'_, '_>) {
        unsafe {
            params.data.destroy_image_view(*self, None);
        }
    }
}

impl Destroyable for vk::Framebuffer {
    type Params<'w, 's> = DeviceStorage<'w>;

    fn destroy(&mut self, params: &mut Self::Params<'_, '_>) {
        unsafe {
            params.data.destroy_framebuffer(*self, None);
        }
    }
}
