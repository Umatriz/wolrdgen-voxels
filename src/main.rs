use bevy_app::{App, AppExit, Plugin, PluginsState};
use tracing::{error, info};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};
use winit::{
    application::ApplicationHandler,
    event_loop::{ControlFlow, EventLoop},
};

use crate::{rendering::RenderingPlugin, windowing::WindowingPlugin};

pub mod dense_storage;
mod rendering;
pub mod utils;
mod windowing;

fn main() {
    tracing_subscriber::registry()
        .with(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| format!("{}=debug", env!("CARGO_CRATE_NAME")).into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!("Logging is successfully initialized");

    App::new()
        .add_plugins((WindowingPlugin, RenderingPlugin))
        .run();
}
