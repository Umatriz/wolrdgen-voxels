use bevy_app::{App, Plugin};
use bevy_ecs::{
    resource::Resource,
    schedule::IntoScheduleConfigs,
    system::{ParamSet, ResMut, SystemParam},
};

pub struct StoragePlugin;

impl Plugin for StoragePlugin {
    fn build(&self, app: &mut App) {}
}

mod imp {
    use super::*;

    // TODO: Make it a SystemParam wrapper around imp::Storage that holds actuall data.
    #[derive(Resource)]
    pub struct Storage<S: StorageType> {
        pub data: S::Inner,
    }
}

#[derive(SystemParam)]
pub struct Storage<'w, S: StorageType>(ResMut<'w, imp::Storage<S>>);

impl<S: StorageType> Storage<'_, S> {
    pub fn data(&self) -> &S::Inner {
        &self.0.data
    }
}

pub trait StorageType: Sized + 'static {
    type Inner: Send + Sync + 'static;
    type Params: SystemParam;

    fn destory(params: Self::Params, data: &mut Self::Inner);
}

fn destroy_storage<S: StorageType + 'static>(mut params: (Storage<S>, S::Params)) {
    // S::destory(params.1, &mut params.0.data);
}
