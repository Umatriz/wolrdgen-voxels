use std::{
    collections::HashSet,
    ffi::{CStr, CString, c_char, c_void},
    sync::Arc,
};

use ash::{
    Device, Entry, Instance, ext, khr,
    vk::{
        self, API_VERSION_1_0, API_VERSION_1_3, DebugReportCallbackEXT, DebugUtilsMessengerEXT,
        Extent2D,
    },
};
use bevy_app::{Last, MainScheduleOrder, Plugin, Startup};
use bevy_ecs::{prelude::*, schedule::ScheduleLabel};
use itertools::Itertools;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use tracing::{debug, error, info, info_span, trace, warn};
use winit::{
    event_loop::{ActiveEventLoop, OwnedDisplayHandle},
    window::Window,
};

use crate::windowing::{AppWindows, WinitOwnedDispayHandle};

mod triangle;

pub struct RenderingPlugin;

impl Plugin for RenderingPlugin {
    fn build(&self, app: &mut bevy_app::App) {
        app.add_schedule(Schedule::new(Render));

        let mut order = app.world_mut().resource_mut::<MainScheduleOrder>();
        order.insert_after(Last, Render);

        app.add_systems(Startup, init_vulkan_app);
    }
}

#[derive(ScheduleLabel, Hash, PartialEq, Eq, Clone, Debug)]
pub struct Render;

pub const REQUIRED_LAYERS: &[&str] = &["VK_LAYER_KHRONOS_validation"];
pub const REQUIRED_DEVICE_EXTENSIONS: &[*const i8] = &[khr::swapchain::NAME.as_ptr()];
// TODO: use CLI args instead
pub const ENABLE_VALIDATION_LAYERS: bool = true;

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
    device: Device,

    graphics_queue: vk::Queue,
    present_queue: vk::Queue,

    swapchain_device: khr::swapchain::Device,
    swapchain: vk::SwapchainKHR,
    swapchain_images: Vec<vk::Image>,
    swapchain_image_views: Vec<vk::ImageView>,
    swapchain_image_format: vk::Format,
    swapchain_extent: vk::Extent2D,
}

impl Drop for VulkanApp {
    fn drop(&mut self) {
        unsafe {
            self.swapchain_device
                .destroy_swapchain(self.swapchain, None);

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
                &create_info.window,
                queue_family_indices,
            );
        let swapchain_images = unsafe { swapchain_device.get_swapchain_images(swapchain).unwrap() };
        let swapchain_image_views =
            Self::create_image_views(&device, &swapchain_images, swapchain_image_format);

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
        window: &Window,
    ) -> vk::Extent2D {
        if capabilities.current_extent.width != u32::MAX {
            capabilities.current_extent
        } else {
            let size = window.inner_size();

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
        window: &Window,
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
        let extent = Self::choose_swapchain_extent(swapchain_support.capabilities, window);

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
                        .layer_count(0),
                );
            let image_view = unsafe { device.create_image_view(&create_info, None).unwrap() };
            image_views.push(image_view);
        }

        image_views
    }

    fn create_graphics_pipeline() {
        let vertex = include_bytes!("../../shaders/out/triangle.vert.spv");
        let fragment = include_bytes!("../../shaders/out/triangle.frag.spv");
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
