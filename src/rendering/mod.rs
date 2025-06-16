use std::ffi::{CStr, CString, c_char, c_void};

use ash::{
    Entry, Instance, ext, khr,
    vk::{self, API_VERSION_1_0, API_VERSION_1_3, DebugReportCallbackEXT, DebugUtilsMessengerEXT},
};
use bevy_app::{Last, MainScheduleOrder, Plugin, Startup};
use bevy_ecs::{prelude::*, schedule::ScheduleLabel};
use itertools::Itertools;
use tracing::{debug, error, info, info_span, trace, warn};

use crate::windowing::AppWindows;

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

#[derive(Resource)]
pub struct VulkanApp {
    _entry: ash::Entry,
    instance: ash::Instance,

    debug_utils_instance_messenger: Option<(ext::debug_utils::Instance, DebugUtilsMessengerEXT)>,
}

impl Drop for VulkanApp {
    fn drop(&mut self) {
        unsafe {
            if let Some((instance, messenger)) = self.debug_utils_instance_messenger.take() {
                instance.destroy_debug_utils_messenger(messenger, None);
            }

            self.instance.destroy_instance(None);
        }
    }
}

impl VulkanApp {
    fn new() -> Self {
        let entry = unsafe { ash::Entry::load().expect("Failed to load entry") };

        let instance = Self::create_instance(&entry);

        let debug_utils_instance_messenger = Self::setup_debug_messenger(&entry, &instance);

        Self {
            _entry: entry,
            instance,
            debug_utils_instance_messenger,
        }
    }

    fn create_instance(entry: &Entry) -> Instance {
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

        let mut extension_names = required_extension_names();

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
}

fn required_extension_names() -> Vec<*const i8> {
    vec![
        khr::surface::NAME.as_ptr(),
        // TODO: it's Windows specific
        khr::win32_surface::NAME.as_ptr(),
    ]
}

fn init_vulkan_app(mut commands: Commands, windows: Res<AppWindows>) {
    let vulkan_app = VulkanApp::new();
    commands.insert_resource(vulkan_app);

    info!("Vulkan app initialized successfully");
}
