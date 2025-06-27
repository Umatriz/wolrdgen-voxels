use std::{
    collections::HashSet,
    ffi::{CStr, CString, c_char, c_void},
    sync::Arc,
};

use ash::{
    Device, Entry, Instance,
    ext::{self, separate_stencil_usage},
    khr,
    vk::{
        self, API_VERSION_1_0, API_VERSION_1_3, DebugReportCallbackEXT, DebugUtilsMessengerEXT,
        Extent2D, SwapchainDisplayNativeHdrCreateInfoAMD,
    },
};
use bevy_app::{Last, MainScheduleOrder, Plugin, Startup};
use bevy_ecs::{prelude::*, schedule::ScheduleLabel};
use itertools::Itertools;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use tracing::{debug, error, info, info_span, trace, warn};
use winit::{
    dpi::PhysicalSize,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, OwnedDisplayHandle},
    window::Window,
};

use crate::windowing::{AppWindows, RawWnitWindowEvent, WinitOwnedDispayHandle};

mod triangle;

mod storage;

pub struct RenderingPlugin;

impl Plugin for RenderingPlugin {
    fn build(&self, app: &mut bevy_app::App) {
        app.add_schedule(Schedule::new(Render));

        let mut order = app.world_mut().resource_mut::<MainScheduleOrder>();
        order.insert_after(Last, Render);

        app.add_systems(Startup, init_vulkan_app);

        app.add_systems(Render, render_frame);
    }
}

#[derive(ScheduleLabel, Hash, PartialEq, Eq, Clone, Debug)]
pub struct Render;

pub const REQUIRED_LAYERS: &[&str] = &["VK_LAYER_KHRONOS_validation"];
pub const REQUIRED_DEVICE_EXTENSIONS: &[*const i8] = &[khr::swapchain::NAME.as_ptr()];
// TODO: use CLI args instead
pub const ENABLE_VALIDATION_LAYERS: bool = true;
pub const MAX_FRAMES_IN_FLIGHT: usize = 2;

unsafe extern "system" fn vulkan_debug_callback(
    message_severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    message_types: vk::DebugUtilsMessageTypeFlagsEXT,
    p_callback_data: *const vk::DebugUtilsMessengerCallbackDataEXT<'_>,
    _p_user_data: *mut c_void,
) -> u32 {
    let message = unsafe {
        let p_message = (*p_callback_data).p_message;
        CStr::from_ptr(p_message)
    };
    let message = message.to_string_lossy();

    if message_severity == vk::DebugUtilsMessageSeverityFlagsEXT::ERROR {
        error!("{:?} - {}", message_types, message);
    } else if message_severity == vk::DebugUtilsMessageSeverityFlagsEXT::WARNING {
        warn!("{:?} - {}", message_types, message);
    } else if message_severity == vk::DebugUtilsMessageSeverityFlagsEXT::INFO {
        info!("{:?} - {}", message_types, message);
    } else {
        trace!("{:?} - {}", message_types, message)
    };

    vk::FALSE
}

/// The window represented by `window` must be associated with the display connection in `display_handle`.
pub struct VulkanAppCreateInfo {
    pub display_handle: OwnedDisplayHandle,
    pub window: Arc<winit::window::Window>,
}

#[derive(Resource)]
pub struct VulkanApp {
    _entry: ash::Entry,
    instance: ash::Instance,

    debug_utils_instance_messenger: Option<(ext::debug_utils::Instance, DebugUtilsMessengerEXT)>,

    surface_instance: khr::surface::Instance,
    surface: vk::SurfaceKHR,

    physical_device: vk::PhysicalDevice,
    pub device: Device,

    graphics_queue: vk::Queue,
    present_queue: vk::Queue,

    swapchain_device: khr::swapchain::Device,
    swapchain: vk::SwapchainKHR,
    swapchain_images: Vec<vk::Image>,
    swapchain_image_views: Vec<vk::ImageView>,
    swapchain_image_format: vk::Format,
    swapchain_extent: vk::Extent2D,

    render_pass: vk::RenderPass,
    pipeline_layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,

    swapchain_framebuffers: Vec<vk::Framebuffer>,

    command_pool: vk::CommandPool,
    command_buffers: Vec<vk::CommandBuffer>,

    image_available_semaphores: Vec<vk::Semaphore>,
    render_finished_semaphores: Vec<vk::Semaphore>,
    in_flight_fences: Vec<vk::Fence>,

    current_frame: usize,
}

impl Drop for VulkanApp {
    fn drop(&mut self) {
        unsafe {
            self.cleanup_swapchain();

            for semaphore in &self.image_available_semaphores {
                self.device.destroy_semaphore(*semaphore, None);
            }

            for semaphore in &self.render_finished_semaphores {
                self.device.destroy_semaphore(*semaphore, None);
            }

            for fence in &self.in_flight_fences {
                self.device.destroy_fence(*fence, None);
            }

            self.device.destroy_command_pool(self.command_pool, None);

            self.device.destroy_pipeline(self.pipeline, None);

            self.device
                .destroy_pipeline_layout(self.pipeline_layout, None);

            self.device.destroy_render_pass(self.render_pass, None);

            self.device.destroy_device(None);

            if let Some((instance, messenger)) = self.debug_utils_instance_messenger.take() {
                instance.destroy_debug_utils_messenger(messenger, None);
            }

            self.surface_instance.destroy_surface(self.surface, None);

            self.instance.destroy_instance(None);
        }
    }
}

impl VulkanApp {
    fn new(create_info: VulkanAppCreateInfo) -> Self {
        let entry = unsafe { ash::Entry::load().expect("Failed to load entry") };

        let handle = create_info.display_handle.display_handle().unwrap();
        let raw_display_handle = handle.as_raw();
        let required_extensions =
            ash_window::enumerate_required_extensions(raw_display_handle).unwrap();
        let instance = Self::create_instance(&entry, required_extensions);

        let debug_utils_instance_messenger = Self::setup_debug_messenger(&entry, &instance);

        let window_handle = create_info.window.window_handle().unwrap();
        let raw_window_handle = window_handle.as_raw();
        let (surface_instance, surface) =
            Self::create_surface(&entry, &instance, raw_display_handle, raw_window_handle);

        let (physical_device, queue_family_indices) =
            Self::select_physical_device(&instance, &surface_instance, surface);
        let device = Self::create_logical_device(&instance, physical_device, queue_family_indices);

        let graphics_queue =
            unsafe { device.get_device_queue(queue_family_indices.graphics_family, 0) };
        let present_queue =
            unsafe { device.get_device_queue(queue_family_indices.present_family, 0) };

        let (swapchain_device, swapchain, swapchain_image_format, swapchain_extent) =
            Self::create_swapchain(
                &instance,
                &device,
                physical_device,
                &surface_instance,
                surface,
                create_info.window.inner_size(),
                queue_family_indices,
            );
        let swapchain_images = unsafe { swapchain_device.get_swapchain_images(swapchain).unwrap() };
        let swapchain_image_views =
            Self::create_image_views(&device, &swapchain_images, swapchain_image_format);

        let render_pass = Self::create_render_pass(&device, swapchain_image_format);

        let (pipeline, pipeline_layout) = Self::create_graphics_pipeline(&device, render_pass);

        let swapchain_framebuffers = Self::create_framebuffers(
            &device,
            render_pass,
            &swapchain_image_views,
            swapchain_extent,
        );

        let command_pool = Self::create_command_pool(&device, queue_family_indices);
        let command_buffers = Self::create_command_buffers(&device, command_pool);

        let (image_available_semaphores, render_finished_semaphores, in_flight_fences) =
            Self::create_sync_objects(&device);

        Self {
            _entry: entry,
            instance,
            debug_utils_instance_messenger,
            surface_instance,
            surface,
            physical_device,
            device,
            graphics_queue,
            present_queue,
            swapchain_device,
            swapchain,
            swapchain_images,
            swapchain_image_views,
            swapchain_image_format,
            swapchain_extent,
            render_pass,
            pipeline_layout,
            pipeline,
            swapchain_framebuffers,
            command_pool,
            command_buffers,
            image_available_semaphores,
            render_finished_semaphores,
            in_flight_fences,
            current_frame: 0,
        }
    }

    fn create_instance(entry: &Entry, required_extensions: &[*const c_char]) -> Instance {
        let app_name = CString::new("Vulkan Application").unwrap();
        let engine_name = CString::new("No Engine").unwrap();
        let app_info = vk::ApplicationInfo::default()
            .application_name(app_name.as_c_str())
            .application_version(vk::make_api_version(0, 0, 1, 0))
            .engine_name(engine_name.as_c_str())
            .engine_version(vk::make_api_version(0, 0, 1, 0))
            .api_version(API_VERSION_1_0);

        let extension_properties =
            unsafe { entry.enumerate_instance_extension_properties(None).unwrap() };
        let names = extension_properties
            .iter()
            .flat_map(|prop| {
                prop.extension_name_as_c_str().inspect_err(|_| {
                    error!("Filed to convert extension name to CStr. No null byte was present.")
                })
            })
            .map(|cstr| cstr.to_string_lossy())
            .join("\n\t");

        info!("Available extensions:\n\t{}", names);

        let mut extension_names = Vec::from_iter(required_extensions.into_iter().map(|x| *x));

        if ENABLE_VALIDATION_LAYERS {
            extension_names.push(ext::debug_utils::NAME.as_ptr());
        }

        let mut create_info = vk::InstanceCreateInfo::default()
            .application_info(&app_info)
            .enabled_extension_names(&extension_names);

        // Pointer returned by `CString::as_ptr` does not carry lifetime information
        // because of that we collect the vec twice so `CString` is not immediately
        // deallocated and we aren't left with a dangling pointer.
        let layer_names = REQUIRED_LAYERS
            .iter()
            .map(|s| CString::new(*s).unwrap())
            .collect_vec();
        let layer_name_ptrs = layer_names.iter().map(|s| s.as_ptr()).collect_vec();

        let mut debug_create_info = Self::get_debug_utils_messenger_create_info();

        if ENABLE_VALIDATION_LAYERS {
            Self::check_validation_layer_support(entry);
            create_info = create_info
                .enabled_layer_names(&layer_name_ptrs)
                .push_next(&mut debug_create_info);
        }

        unsafe { entry.create_instance(&create_info, None).unwrap() }
    }

    fn check_validation_layer_support(entry: &Entry) {
        let layer_properties = unsafe { entry.enumerate_instance_layer_properties().unwrap() };
        for required in REQUIRED_LAYERS {
            let found = layer_properties.iter().any(|layer| {
                let name = layer.layer_name_as_c_str().unwrap();
                let name = name.to_str().unwrap();
                *required == name
            });

            if !found {
                panic!("Validation layer not supported: {}", required);
            }
        }
    }

    fn setup_debug_messenger(
        entry: &Entry,
        instance: &Instance,
    ) -> Option<(ext::debug_utils::Instance, DebugUtilsMessengerEXT)> {
        if !ENABLE_VALIDATION_LAYERS {
            return None;
        }

        let create_info = Self::get_debug_utils_messenger_create_info();

        let debug_utils_instance = ext::debug_utils::Instance::new(entry, instance);

        let messenger = unsafe {
            debug_utils_instance
                .create_debug_utils_messenger(&create_info, None)
                .unwrap()
        };

        Some((debug_utils_instance, messenger))
    }

    fn get_debug_utils_messenger_create_info() -> vk::DebugUtilsMessengerCreateInfoEXT<'static> {
        vk::DebugUtilsMessengerCreateInfoEXT::default()
            .message_severity(
                vk::DebugUtilsMessageSeverityFlagsEXT::VERBOSE
                    | vk::DebugUtilsMessageSeverityFlagsEXT::WARNING
                    | vk::DebugUtilsMessageSeverityFlagsEXT::ERROR,
            )
            .message_type(
                vk::DebugUtilsMessageTypeFlagsEXT::GENERAL
                    | vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION
                    | vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE,
            )
            .pfn_user_callback(Some(vulkan_debug_callback))
    }

    // TODO: select gpu from all available
    fn select_physical_device(
        instance: &Instance,
        surface_instance: &khr::surface::Instance,
        surface: vk::SurfaceKHR,
    ) -> (vk::PhysicalDevice, QueueFamilyIndices) {
        let physical_devices = unsafe { instance.enumerate_physical_devices().unwrap() };

        if physical_devices.is_empty() {
            panic!("Failed to find GPUs with Vulkan support");
        }

        for physical_device in physical_devices {
            let properties = unsafe { instance.get_physical_device_properties(physical_device) };
            let features = unsafe { instance.get_physical_device_features(physical_device) };

            if let Some(queue_families_data) = Self::is_device_suitable(
                instance,
                physical_device,
                properties,
                features,
                surface_instance,
                surface,
            ) {
                info!(
                    "Selected physical device: {}",
                    properties.device_name_as_c_str().unwrap().to_string_lossy()
                );
                return (physical_device, queue_families_data);
            }
        }

        panic!("Failed to find a suitable GPU")
    }

    fn is_device_suitable(
        instance: &Instance,
        physical_device: vk::PhysicalDevice,
        properties: vk::PhysicalDeviceProperties,
        features: vk::PhysicalDeviceFeatures,
        surface_instance: &khr::surface::Instance,
        surface: vk::SurfaceKHR,
    ) -> Option<QueueFamilyIndices> {
        let queue_family_indices =
            Self::find_queue_families(instance, physical_device, surface_instance, surface);

        let extensions_supported = Self::check_device_extension_support(instance, physical_device);
        if !extensions_supported {
            return None;
        }

        let mut swapchain_support = false;
        if extensions_supported {
            let swapchain_support_details =
                Self::query_swapchain_support(physical_device, surface_instance, surface);
            swapchain_support = !swapchain_support_details.formats.is_empty()
                && !swapchain_support_details.present_modes.is_empty();
        }

        if !swapchain_support {
            return None;
        }

        queue_family_indices
    }

    fn check_device_extension_support(
        instance: &Instance,
        physical_device: vk::PhysicalDevice,
    ) -> bool {
        let extension_properties = unsafe {
            instance
                .enumerate_device_extension_properties(physical_device)
                .unwrap()
        };

        for name in REQUIRED_DEVICE_EXTENSIONS {
            let found = extension_properties.iter().any(|ext_prop| {
                ext_prop.extension_name_as_c_str().unwrap() == unsafe { CStr::from_ptr(*name) }
            });

            if !found {
                return false;
            }
        }

        true
    }

    fn find_queue_families(
        instance: &Instance,
        physical_device: vk::PhysicalDevice,
        surface_instance: &khr::surface::Instance,
        surface: vk::SurfaceKHR,
    ) -> Option<QueueFamilyIndices> {
        let properties =
            unsafe { instance.get_physical_device_queue_family_properties(physical_device) };

        let mut graphics_family_index = None;
        let mut present_family_index = None;

        for (i, queue_family) in properties.iter().enumerate() {
            let i = i as u32;

            if graphics_family_index.is_none()
                && queue_family.queue_flags.contains(vk::QueueFlags::GRAPHICS)
            {
                graphics_family_index = Some(i)
            };

            let surface_support = unsafe {
                surface_instance
                    .get_physical_device_surface_support(physical_device, i, surface)
                    .unwrap()
            };
            if present_family_index.is_none() && surface_support {
                present_family_index = Some(i)
            }

            if graphics_family_index.is_some() && present_family_index.is_some() {
                break;
            }
        }

        graphics_family_index.and_then(|graphics| {
            present_family_index.map(|present| QueueFamilyIndices {
                graphics_family: graphics,
                present_family: present,
            })
        })
    }

    fn create_logical_device(
        instance: &Instance,
        physical_device: vk::PhysicalDevice,
        queue_families_data: QueueFamilyIndices,
    ) -> Device {
        let mut queue_create_infos = vec![];

        let unique_queue_families = HashSet::from([
            queue_families_data.graphics_family,
            queue_families_data.present_family,
        ]);

        let queue_priorities = &[1.0];
        for queue_family in unique_queue_families {
            let queue_create_info = vk::DeviceQueueCreateInfo::default()
                .queue_family_index(queue_family)
                .queue_priorities(queue_priorities);
            queue_create_infos.push(queue_create_info);
        }

        let features = vk::PhysicalDeviceFeatures::default();
        let device_create_info = vk::DeviceCreateInfo::default()
            .queue_create_infos(&queue_create_infos)
            .enabled_features(&features)
            .enabled_extension_names(REQUIRED_DEVICE_EXTENSIONS);

        unsafe {
            instance
                .create_device(physical_device, &device_create_info, None)
                .unwrap()
        }
    }

    fn create_surface(
        entry: &Entry,
        instance: &Instance,
        raw_display_handle: RawDisplayHandle,
        raw_window_handle: RawWindowHandle,
    ) -> (khr::surface::Instance, vk::SurfaceKHR) {
        let surface = unsafe {
            ash_window::create_surface(entry, instance, raw_display_handle, raw_window_handle, None)
                .inspect_err(|err| error!(error = %err, "Failed to create surface"))
                .unwrap()
        };

        let instance = khr::surface::Instance::new(entry, instance);

        (instance, surface)
    }

    fn query_swapchain_support(
        physical_device: vk::PhysicalDevice,
        surface_instance: &khr::surface::Instance,
        surface: vk::SurfaceKHR,
    ) -> SwapchainSupportDetails {
        unsafe {
            let capabilities = surface_instance
                .get_physical_device_surface_capabilities(physical_device, surface)
                .unwrap();

            let formats = surface_instance
                .get_physical_device_surface_formats(physical_device, surface)
                .unwrap();

            let present_modes = surface_instance
                .get_physical_device_surface_present_modes(physical_device, surface)
                .unwrap();

            SwapchainSupportDetails {
                capabilities,
                formats,
                present_modes,
            }
        }
    }

    fn choose_swapchain_surface_format(
        available_formats: &[vk::SurfaceFormatKHR],
    ) -> vk::SurfaceFormatKHR {
        for available_format in available_formats {
            if available_format.format == vk::Format::B8G8R8A8_SRGB
                && available_format.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
            {
                return *available_format;
            }
        }

        available_formats[0]
    }

    fn choose_swapchain_present_mode(
        available_present_modes: &[vk::PresentModeKHR],
    ) -> vk::PresentModeKHR {
        for available_present_mode in available_present_modes {
            if *available_present_mode == vk::PresentModeKHR::MAILBOX {
                return *available_present_mode;
            }
        }

        vk::PresentModeKHR::FIFO
    }

    fn choose_swapchain_extent(
        capabilities: vk::SurfaceCapabilitiesKHR,
        size: PhysicalSize<u32>,
    ) -> vk::Extent2D {
        if capabilities.current_extent.width != u32::MAX {
            dbg!(capabilities.current_extent)
        } else {
            let width = size.width.clamp(
                capabilities.min_image_extent.width,
                capabilities.max_image_extent.width,
            );
            let height = size.height.clamp(
                capabilities.min_image_extent.height,
                capabilities.max_image_extent.height,
            );

            Extent2D::default().width(width).height(height)
        }
    }

    fn create_swapchain(
        instance: &Instance,
        device: &Device,
        physical_device: vk::PhysicalDevice,
        surface_instance: &khr::surface::Instance,
        surface: vk::SurfaceKHR,
        size: PhysicalSize<u32>,
        queue_family_indices: QueueFamilyIndices,
    ) -> (
        khr::swapchain::Device,
        vk::SwapchainKHR,
        vk::Format,
        vk::Extent2D,
    ) {
        let swapchain_support =
            Self::query_swapchain_support(physical_device, surface_instance, surface);

        let surface_format = Self::choose_swapchain_surface_format(&swapchain_support.formats);
        let present_mode = Self::choose_swapchain_present_mode(&swapchain_support.present_modes);
        let extent = Self::choose_swapchain_extent(swapchain_support.capabilities, size);

        let mut image_count = swapchain_support.capabilities.min_image_count + 1;
        if swapchain_support.capabilities.max_image_count > 0
            && image_count > swapchain_support.capabilities.max_image_count
        {
            image_count = swapchain_support.capabilities.max_image_count;
        }

        let mut create_info = vk::SwapchainCreateInfoKHR::default()
            .surface(surface)
            .min_image_count(image_count)
            .image_format(surface_format.format)
            .image_color_space(surface_format.color_space)
            .image_extent(extent)
            .image_array_layers(1)
            .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT);

        let indices = &[
            queue_family_indices.graphics_family,
            queue_family_indices.present_family,
        ];

        if queue_family_indices.graphics_family != queue_family_indices.present_family {
            create_info = create_info
                .image_sharing_mode(vk::SharingMode::CONCURRENT)
                .queue_family_indices(indices)
        } else {
            create_info = create_info.image_sharing_mode(vk::SharingMode::EXCLUSIVE)
        };

        create_info = create_info
            .pre_transform(swapchain_support.capabilities.current_transform)
            .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
            .present_mode(present_mode)
            .clipped(false);

        let swapchain_device = khr::swapchain::Device::new(instance, device);
        let swapchain = unsafe {
            swapchain_device
                .create_swapchain(&create_info, None)
                .unwrap()
        };

        (swapchain_device, swapchain, surface_format.format, extent)
    }

    fn create_image_views(
        device: &Device,
        swapchain_images: &[vk::Image],
        format: vk::Format,
    ) -> Vec<vk::ImageView> {
        let mut image_views = Vec::with_capacity(swapchain_images.len());
        for image in swapchain_images {
            let create_info = vk::ImageViewCreateInfo::default()
                .image(*image)
                .view_type(vk::ImageViewType::TYPE_2D)
                .format(format)
                .subresource_range(
                    vk::ImageSubresourceRange::default()
                        .aspect_mask(vk::ImageAspectFlags::COLOR)
                        .base_mip_level(0)
                        .level_count(1)
                        .base_array_layer(0)
                        .layer_count(1),
                );
            let image_view = unsafe { device.create_image_view(&create_info, None).unwrap() };
            image_views.push(image_view);
        }

        image_views
    }

    fn create_render_pass(device: &Device, swapchain_image_format: vk::Format) -> vk::RenderPass {
        let color_attachment = vk::AttachmentDescription::default()
            .format(swapchain_image_format)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(vk::ImageLayout::PRESENT_SRC_KHR);

        let color_attachment_ref = vk::AttachmentReference::default()
            .attachment(0)
            .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);

        let color_attachments = &[color_attachment_ref];
        let subpass = vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .color_attachments(color_attachments);

        let dependency = vk::SubpassDependency::default()
            .src_subpass(vk::SUBPASS_EXTERNAL)
            .dst_subpass(0)
            .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
            .src_access_mask(vk::AccessFlags::empty())
            .dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
            .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE);

        let attachments = &[color_attachment];
        let subpasses = &[subpass];
        let dependencies = &[dependency];
        let render_pass_create_info = vk::RenderPassCreateInfo::default()
            .attachments(attachments)
            .subpasses(subpasses)
            .dependencies(dependencies);

        unsafe {
            device
                .create_render_pass(&render_pass_create_info, None)
                .unwrap()
        }
    }

    fn create_graphics_pipeline(
        device: &Device,
        render_pass: vk::RenderPass,
    ) -> (vk::Pipeline, vk::PipelineLayout) {
        let vertex = include_bytes!("../../shaders/out/triangle.vert.spv");
        let fragment = include_bytes!("../../shaders/out/triangle.frag.spv");

        let vertex_shader_module = Self::create_shader_module(device, vertex);
        let fragment_shader_module = Self::create_shader_module(device, fragment);

        let vertex_stage_info = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::VERTEX)
            .module(vertex_shader_module)
            .name(c"main");
        let fragment_stage_info = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::FRAGMENT)
            .module(fragment_shader_module)
            .name(c"main");
        let shader_stages = &[vertex_stage_info, fragment_stage_info];

        let vertex_input_create_info = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_attribute_descriptions(&[])
            .vertex_binding_descriptions(&[]);

        let input_assembly_create_info = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST)
            .primitive_restart_enable(false);

        // let viewport = vk::Viewport::default()
        //     .x(0.0)
        //     .y(0.0)
        //     .width(swapchain_extent.width as f32)
        //     .height(swapchain_extent.height as f32)
        //     .min_depth(0.0)
        //     .max_depth(1.0);

        // let scissor = vk::Rect2D::default()
        //     .offset(vk::Offset2D::default().x(0).y(0))
        //     .extent(swapchain_extent);

        let dynamic_states = &[vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let dynamic_state_create_info =
            vk::PipelineDynamicStateCreateInfo::default().dynamic_states(dynamic_states);

        let viewport_state_create_info = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);

        let rasterizer_create_info = vk::PipelineRasterizationStateCreateInfo::default()
            .depth_clamp_enable(false)
            .rasterizer_discard_enable(false)
            .polygon_mode(vk::PolygonMode::FILL)
            .line_width(1.0)
            .cull_mode(vk::CullModeFlags::BACK)
            .front_face(vk::FrontFace::CLOCKWISE)
            .depth_bias_enable(false);

        let multisampling_create_info = vk::PipelineMultisampleStateCreateInfo::default()
            .sample_shading_enable(false)
            .rasterization_samples(vk::SampleCountFlags::TYPE_1)
            .min_sample_shading(1.0);

        let color_blend_attachment = vk::PipelineColorBlendAttachmentState::default()
            .color_write_mask(
                vk::ColorComponentFlags::R
                    | vk::ColorComponentFlags::G
                    | vk::ColorComponentFlags::B
                    | vk::ColorComponentFlags::A,
            )
            .blend_enable(false);

        let attachments = &[color_blend_attachment];
        let color_blending_create_info = vk::PipelineColorBlendStateCreateInfo::default()
            .logic_op_enable(false)
            .logic_op(vk::LogicOp::COPY)
            .attachments(attachments);

        let pipeline_layout_create_info = vk::PipelineLayoutCreateInfo::default();

        let pipeline_layout = unsafe {
            device
                .create_pipeline_layout(&pipeline_layout_create_info, None)
                .unwrap()
        };

        let pipeline_create_info = vk::GraphicsPipelineCreateInfo::default()
            .stages(shader_stages)
            .vertex_input_state(&vertex_input_create_info)
            .input_assembly_state(&input_assembly_create_info)
            .viewport_state(&viewport_state_create_info)
            .rasterization_state(&rasterizer_create_info)
            .multisample_state(&multisampling_create_info)
            .color_blend_state(&color_blending_create_info)
            .dynamic_state(&dynamic_state_create_info)
            .layout(pipeline_layout)
            .render_pass(render_pass)
            .subpass(0);

        let pipeline = unsafe {
            device
                .create_graphics_pipelines(vk::PipelineCache::null(), &[pipeline_create_info], None)
                .unwrap()[0]
        };

        unsafe {
            device.destroy_shader_module(vertex_shader_module, None);
            device.destroy_shader_module(fragment_shader_module, None);
        }

        (pipeline, pipeline_layout)
    }

    fn create_shader_module(device: &Device, buf: &[u8]) -> vk::ShaderModule {
        let create_info = vk::ShaderModuleCreateInfo::default().code(bytemuck::cast_slice(buf));
        unsafe { device.create_shader_module(&create_info, None).unwrap() }
    }

    fn create_framebuffers(
        device: &Device,
        render_pass: vk::RenderPass,
        swapchain_image_views: &[vk::ImageView],
        swapchain_extent: Extent2D,
    ) -> Vec<vk::Framebuffer> {
        let mut swapchain_framebuffers = Vec::with_capacity(swapchain_image_views.len());

        for image_view in swapchain_image_views {
            let attachments = &[*image_view];

            let framebuffer_create_info = vk::FramebufferCreateInfo::default()
                .render_pass(render_pass)
                .attachments(attachments)
                .width(swapchain_extent.width)
                .height(swapchain_extent.height)
                .layers(1);

            let framebuffer = unsafe {
                device
                    .create_framebuffer(&framebuffer_create_info, None)
                    .unwrap()
            };

            swapchain_framebuffers.push(framebuffer);
        }

        swapchain_framebuffers
    }

    fn create_command_pool(
        device: &Device,
        queue_family_indices: QueueFamilyIndices,
    ) -> vk::CommandPool {
        let command_pool_info = vk::CommandPoolCreateInfo::default()
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER)
            .queue_family_index(queue_family_indices.graphics_family);

        unsafe {
            device
                .create_command_pool(&command_pool_info, None)
                .unwrap()
        }
    }

    fn create_command_buffers(
        device: &Device,
        command_pool: vk::CommandPool,
    ) -> Vec<vk::CommandBuffer> {
        let allocate_info = vk::CommandBufferAllocateInfo::default()
            .command_pool(command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(2);

        unsafe { device.allocate_command_buffers(&allocate_info).unwrap() }
    }

    fn record_command_buffer(
        device: &Device,
        command_buffer: vk::CommandBuffer,
        render_pass: vk::RenderPass,
        swapchain_framebuffers: &[vk::Framebuffer],
        image_index: usize,
        swapchain_extent: Extent2D,
        graphics_pipeline: vk::Pipeline,
    ) {
        let begin_info = vk::CommandBufferBeginInfo::default();

        unsafe {
            device.begin_command_buffer(command_buffer, &begin_info);

            let render_pass_info = vk::RenderPassBeginInfo::default()
                .render_pass(render_pass)
                .framebuffer(swapchain_framebuffers[image_index])
                .render_area(vk::Rect2D {
                    offset: vk::Offset2D { x: 0, y: 0 },
                    extent: swapchain_extent,
                })
                .clear_values(&[vk::ClearValue {
                    color: vk::ClearColorValue {
                        float32: [1.0, 1.0, 1.0, 1.0],
                    },
                }]);

            device.cmd_begin_render_pass(
                command_buffer,
                &render_pass_info,
                vk::SubpassContents::INLINE,
            );

            device.cmd_bind_pipeline(
                command_buffer,
                vk::PipelineBindPoint::GRAPHICS,
                graphics_pipeline,
            );

            let viewport = vk::Viewport::default()
                .x(0.0)
                .y(0.0)
                .width(swapchain_extent.width as f32)
                .height(swapchain_extent.height as f32)
                .min_depth(0.0)
                .max_depth(1.0);
            device.cmd_set_viewport(command_buffer, 0, &[viewport]);

            let scissor = vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: swapchain_extent,
            };
            device.cmd_set_scissor(command_buffer, 0, &[scissor]);

            device.cmd_draw(command_buffer, 3, 1, 0, 0);

            device.cmd_end_render_pass(command_buffer);

            device.end_command_buffer(command_buffer)
        };
    }

    fn create_sync_objects(
        device: &Device,
    ) -> (Vec<vk::Semaphore>, Vec<vk::Semaphore>, Vec<vk::Fence>) {
        let semaphore_info = vk::SemaphoreCreateInfo::default();

        let fence_info = vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED);

        let mut objects = Vec::new();

        for _ in 0..MAX_FRAMES_IN_FLIGHT {
            let frame_objects = unsafe {
                (
                    device.create_semaphore(&semaphore_info, None).unwrap(),
                    device.create_semaphore(&semaphore_info, None).unwrap(),
                    device.create_fence(&fence_info, None).unwrap(),
                )
            };
            objects.push(frame_objects);
        }

        objects.into_iter().multiunzip()
    }

    // TODO: Replace bool with custom error type
    fn draw_frame(&mut self, swapchain_ok: &mut bool) {
        unsafe {
            self.device
                .wait_for_fences(&[self.in_flight_fences[self.current_frame]], true, u64::MAX)
                .unwrap();

            // FIXME: nesting
            let image_index = if *swapchain_ok {
                match self.swapchain_device.acquire_next_image(
                    self.swapchain,
                    u64::MAX,
                    self.image_available_semaphores[self.current_frame],
                    vk::Fence::null(),
                ) {
                    Ok((index, _)) => index,
                    Err(err) if err == vk::Result::ERROR_OUT_OF_DATE_KHR => {
                        // self.recreate_swapchain(window);
                        *swapchain_ok = false;
                        return;
                    }
                    Err(err) => panic!("{}", err),
                }
            } else {
                return;
            };

            self.device
                .reset_fences(&[self.in_flight_fences[self.current_frame]])
                .unwrap();

            self.device
                .reset_command_buffer(
                    self.command_buffers[self.current_frame],
                    vk::CommandBufferResetFlags::empty(),
                )
                .unwrap();

            Self::record_command_buffer(
                &self.device,
                self.command_buffers[self.current_frame],
                self.render_pass,
                &self.swapchain_framebuffers,
                image_index as usize,
                self.swapchain_extent,
                self.pipeline,
            );

            let wait_semaphores = &[self.image_available_semaphores[self.current_frame]];
            let wait_stages = &[vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];
            let command_buffers = &[self.command_buffers[self.current_frame]];
            let signal_semaphores = &[self.render_finished_semaphores[self.current_frame]];

            let submit_info = vk::SubmitInfo::default()
                .wait_semaphores(wait_semaphores)
                .wait_dst_stage_mask(wait_stages)
                .command_buffers(command_buffers)
                .signal_semaphores(signal_semaphores);

            self.device.queue_submit(
                self.graphics_queue,
                &[submit_info],
                self.in_flight_fences[self.current_frame],
            );

            let swapchains = &[self.swapchain];
            let image_indices = &[image_index];
            let present_info = vk::PresentInfoKHR::default()
                .wait_semaphores(signal_semaphores)
                .swapchains(swapchains)
                .image_indices(image_indices);

            match self
                .swapchain_device
                .queue_present(self.present_queue, &present_info)
            {
                /* Ok(true) | */
                Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                    // self.recreate_swapchain(window);
                    *swapchain_ok = false;
                }
                // Err(_) | Ok(_) if was_resized => {
                //     self.recreate_swapchain(window);
                // }
                Ok(_) => {}
                Err(_) => panic!("Failed to present swapchain image"),
            };
        };

        self.current_frame = (self.current_frame + 1) % MAX_FRAMES_IN_FLIGHT;
    }

    // TODO: Handle minimization/maximization
    fn recreate_swapchain(&mut self, window: &Window) {
        unsafe { self.device.device_wait_idle().unwrap() };

        self.cleanup_swapchain();

        info!("Swapchain is cleaned and is ready to be recreated");

        let queue_family_indices = Self::find_queue_families(
            &self.instance,
            self.physical_device,
            &self.surface_instance,
            self.surface,
        )
        .unwrap();

        let (swapchain_device, swapchain, swapchain_image_format, swapchain_extent) =
            Self::create_swapchain(
                &self.instance,
                &self.device,
                self.physical_device,
                &self.surface_instance,
                self.surface,
                window.inner_size(),
                queue_family_indices,
            );

        let swapchain_images = unsafe { swapchain_device.get_swapchain_images(swapchain).unwrap() };
        let swapchain_image_views =
            Self::create_image_views(&self.device, &swapchain_images, swapchain_image_format);

        let swapchain_framebuffers = Self::create_framebuffers(
            &self.device,
            self.render_pass,
            &swapchain_image_views,
            swapchain_extent,
        );

        self.swapchain_device = swapchain_device;
        self.swapchain = swapchain;
        self.swapchain_extent = swapchain_extent;
        self.swapchain_images = swapchain_images;
        self.swapchain_image_views = swapchain_image_views;
        self.swapchain_framebuffers = swapchain_framebuffers;
    }

    fn cleanup_swapchain(&mut self) {
        unsafe {
            for framebuffer in &self.swapchain_framebuffers {
                self.device.destroy_framebuffer(*framebuffer, None);
            }

            for image_view in &self.swapchain_image_views {
                self.device.destroy_image_view(*image_view, None);
            }

            self.swapchain_device
                .destroy_swapchain(self.swapchain, None);
        }
    }

    fn resize(&mut self, swapchain_ok: &mut bool, size: PhysicalSize<u32>) {
        unsafe {
            self.device.device_wait_idle();

            let old_swapchain = self.swapchain;

            self.cleanup_swapchain();

            let queue_family_indices = Self::find_queue_families(
                &self.instance,
                self.physical_device,
                &self.surface_instance,
                self.surface,
            )
            .unwrap();

            let (swapchain_device, swapchain, swapchain_image_format, swapchain_extent) =
                Self::create_swapchain(
                    &self.instance,
                    &self.device,
                    self.physical_device,
                    &self.surface_instance,
                    self.surface,
                    size,
                    queue_family_indices,
                );

            let swapchain_images = swapchain_device.get_swapchain_images(swapchain).unwrap();
            let swapchain_image_views =
                Self::create_image_views(&self.device, &swapchain_images, swapchain_image_format);

            let swapchain_framebuffers = Self::create_framebuffers(
                &self.device,
                self.render_pass,
                &swapchain_image_views,
                swapchain_extent,
            );

            self.swapchain_device = swapchain_device;
            self.swapchain = swapchain;
            self.swapchain_extent = swapchain_extent;
            self.swapchain_images = swapchain_images;
            self.swapchain_image_views = swapchain_image_views;
            self.swapchain_framebuffers = swapchain_framebuffers;

            *swapchain_ok = true;

            self.draw_frame(swapchain_ok);
        }
    }
}

#[derive(Clone, Copy, Default)]
struct QueueFamilyIndices {
    graphics_family: u32,
    present_family: u32,
}

#[derive(Default)]
struct SwapchainSupportDetails {
    capabilities: vk::SurfaceCapabilitiesKHR,
    formats: Vec<vk::SurfaceFormatKHR>,
    present_modes: Vec<vk::PresentModeKHR>,
}

fn init_vulkan_app(
    mut commands: Commands,
    windows: Res<AppWindows>,
    display_handle: Res<WinitOwnedDispayHandle>,
) {
    let create_info = VulkanAppCreateInfo {
        display_handle: display_handle.0.clone(),
        window: windows.primary.clone(),
    };

    let vulkan_app = VulkanApp::new(create_info);
    commands.insert_resource(vulkan_app);
}

fn render_frame(
    mut vulkan_app: ResMut<VulkanApp>,
    windows: Res<AppWindows>,
    mut raw_winit_events: EventReader<RawWnitWindowEvent>,
    mut maximization_state: Local<Option<bool>>,
    mut swapchain_ok: Local<Option<bool>>,
) {
    let swapchain_ok = swapchain_ok.get_or_insert(true);

    let primary_window = &windows.primary;
    // let was_resized = raw_winit_events
    //     .read()
    //     .any(|RawWnitWindowEvent { event, window_id }| {
    //         let is_resize =
    //             matches!(event, WindowEvent::Resized(..)) && *window_id == primary_window.id();
    //         if is_resize {
    //             info!(event = ?event);
    //         }
    //         is_resize
    //     });

    for event in raw_winit_events.read() {
        let WindowEvent::Resized(size) = event.event else {
            continue;
        };

        vulkan_app.resize(swapchain_ok, size);
    }

    let is_maximized = primary_window.is_maximized();
    let was_maximized = match *maximization_state {
        Some(previous_state) => previous_state ^ is_maximized,
        None => is_maximized,
    };
    *maximization_state = Some(is_maximized);

    if was_maximized {
        info!("Maximized");
    }

    vulkan_app.draw_frame(swapchain_ok);
}
