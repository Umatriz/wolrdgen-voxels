use std::marker::{PhantomData, PhantomPinned};

use bevy_app::{App, Plugin};
use bevy_ecs::{
    entity::EntityHashMap,
    resource::Resource,
    schedule::{IntoScheduleConfigs, IntoSystemSet, Schedule, ScheduleLabel},
    system::{Commands, IntoSystem, ParamSet, Res, ResMut, SystemParam},
};

use uuid::Uuid;

mod storages;

pub struct StoragePlugin;

impl Plugin for StoragePlugin {
    fn build(&self, app: &mut App) {
        app.add_schedule(Schedule::new(Destroy));
    }
}

#[derive(ScheduleLabel, PartialEq, Eq, Hash, Clone, Debug)]
pub struct Destroy;

pub type StorageSingle<'w, T> = Storage<'w, Single<T>>;
pub type StorageSingleMut<'w, T> = StorageMut<'w, Single<T>>;

pub type StorageHandled<'w, T> = Storage<'w, Handled<T>>;
pub type StorageHandledMut<'w, T> = StorageMut<'w, Handled<T>>;

pub fn destroy_storage<T: Destroyable>(mut storage: StorageMut<T>, mut params: T::Params) {
    storage.data_mut().destroy(&mut params);
}

pub fn destroy_storage_handled<T: Destroyable>() -> impl Fn(StorageMut<Handled<T>>, T::Params) {
    destroy_storage::<Handled<T>>
}

pub fn destroy_storage_single<T: Destroyable>() -> impl Fn(StorageMut<Single<T>>, T::Params) {
    destroy_storage::<Single<T>>
}

pub fn optional<T: Destroyable>(
    destroyer: impl Fn(StorageMut<T>, T::Params),
) -> impl Fn(StorageMutOpt<T>, T::Params) {
    move |storage_opt, params| {}
}

mod imp {
    use super::*;

    #[derive(Resource)]
    pub struct Storage<T: Destroyable> {
        pub data: T,
    }
}

#[derive(SystemParam)]
pub struct StorageMut<'w, T: Destroyable>(ResMut<'w, imp::Storage<T>>);

#[derive(SystemParam)]
pub struct Storage<'w, T: Destroyable>(Res<'w, imp::Storage<T>>);

#[derive(SystemParam)]
pub struct StorageMutOpt<'w, T: Destroyable>(Option<ResMut<'w, imp::Storage<T>>>);

#[derive(SystemParam)]
pub struct StorageOpt<'w, T: Destroyable>(Option<Res<'w, imp::Storage<T>>>);

macro_rules! impl_data_getters {
    (
        [$storage:ty] $type:ident,
        data
    ) => {
        impl<T: Destroyable> $storage {
            pub fn data(&self) -> &$type {
                &self.0.data
            }
        }
    };
    (
        [$storage:ty] $type:ident,
        data_mut
    ) => {
        impl<T: Destroyable> $storage {
            pub fn data(&self) -> &$type {
                &self.0.data
            }

            pub fn data_mut(&mut self) -> &$type {
                &mut self.0.data
            }
        }
    };
}

impl_data_getters! {
    [Storage<'_, T>] T,
    data
}

impl_data_getters! {
    [StorageMut<'_, T>] T,
    data_mut
}

impl_data_getters! {
    [StorageOpt<'_, T>] Option<T>,
    data
}

impl_data_getters! {
    [StorageMutOpt<'_, T>] Option<T>,
    data_mut
}

/// A type that must be destroyed at the end of the program execution.
pub trait Destroyable: Send + Sync + 'static {
    type Params: SystemParam;

    fn destroy(&mut self, params: &mut Self::Params);
}

pub struct Single<T>(T);

impl<T: Destroyable> Destroyable for Single<T> {
    type Params = T::Params;

    fn destroy(&mut self, params: &mut T::Params) {
        self.0.destroy(params);
    }
}

pub struct Handled<T> {
    inner: hashbrown::HashMap<Handle<T>, T>,
}

// TODO: Custom `Hash` impl
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Handle<T>(Uuid, PhantomData<T>);

impl<T: Destroyable> Destroyable for Handled<T> {
    type Params = T::Params;

    fn destroy(&mut self, params: &mut T::Params) {
        for (_, val) in &mut self.inner {
            val.destroy(params);
        }
    }
}
