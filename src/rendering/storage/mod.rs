use std::marker::{PhantomData, PhantomPinned};

use bevy_app::{App, Plugin, Startup};
use bevy_ecs::{
    entity::EntityHashMap,
    resource::Resource,
    schedule::{IntoScheduleConfigs, IntoSystemSet, Schedule, ScheduleLabel, SystemSet},
    system::{Commands, IntoSystem, ParamSet, Res, ResMut, SystemParam},
};

use derive_more::{Deref, DerefMut};
use uuid::Uuid;

pub mod common;

pub struct StoragePlugin;

impl Plugin for StoragePlugin {
    fn build(&self, app: &mut App) {
        app.add_schedule(Schedule::new(Destroy));
    }
}

#[derive(SystemSet, PartialEq, Eq, Debug, Clone, Hash)]
pub enum StorageInitSet {
    InitHandledStorages,
}

pub trait StoragesAppExt {
    fn app_mut(&mut self) -> &mut App;

    fn register_handled_storage<T: Send + Sync + 'static>(&mut self) -> &mut App {
        let app = self.app_mut();
        app.add_systems(
            Startup,
            init_handled_storage_system::<T>.in_set(StorageInitSet::InitHandledStorages),
        );
        app
    }
}

impl StoragesAppExt for App {
    fn app_mut(&mut self) -> &mut App {
        self
    }
}

fn init_handled_storage_system<T: Send + Sync + 'static>(mut commands: Commands) {
    commands.insert_storage(Handled::<T>::default());
}

#[derive(ScheduleLabel, PartialEq, Eq, Hash, Clone, Debug)]
pub struct Destroy;

pub fn destroy_storage_system<T: Destroyable>(
    mut storage: StorageMut<T>,
    mut params: T::Params<'_, '_>,
) {
    storage.data.destroy(&mut params);
}

pub fn destroy_storage_handled<T: Destroyable>()
-> impl Fn(StorageMut<Handled<T>>, T::Params<'_, '_>) {
    destroy_storage_system::<Handled<T>>
}

pub fn destroy_storage<T: Destroyable>() -> impl Fn(StorageMut<T>, T::Params<'_, '_>) {
    destroy_storage_system::<T>
}

pub fn optional<'w, 's, T: Destroyable>(
    destroyer: impl Fn(StorageMut<T>, T::Params<'w, 's>),
) -> impl Fn(StorageMutOpt<T>, T::Params<'w, 's>) {
    move |storage_opt, params| {
        if let Some(storage) = storage_opt {
            destroyer(storage, params);
        }
    }
}

pub type StorageMut<'w, T> = ResMut<'w, RawStorage<T>>;
pub type Storage<'w, T> = Res<'w, RawStorage<T>>;

pub type StorageMutOpt<'w, T> = Option<ResMut<'w, RawStorage<T>>>;
pub type StorageOpt<'w, T> = Option<Res<'w, RawStorage<T>>>;

pub type StorageHandled<'w, T> = Storage<'w, Handled<T>>;
pub type StorageHandledMut<'w, T> = StorageMut<'w, Handled<T>>;

#[derive(Resource, Deref, DerefMut)]
pub struct RawStorage<T> {
    pub data: T,
}

pub fn storage<T>(data: T) -> RawStorage<T> {
    RawStorage { data }
}

pub trait InsertStorageCommandsExt<'w, 's> {
    fn commands_mut(&mut self) -> &mut Commands<'w, 's>;

    fn insert_storage<T: Send + Sync + 'static>(&mut self, data: T) {
        self.commands_mut().insert_resource(RawStorage { data });
    }
}

impl<'w, 's> InsertStorageCommandsExt<'w, 's> for Commands<'w, 's> {
    fn commands_mut(&mut self) -> &mut Commands<'w, 's> {
        self
    }
}

/// A type that must be destroyed at the end of the program execution.
pub trait Destroyable: Send + Sync + 'static {
    type Params<'w, 's>: SystemParam;

    fn destroy(&mut self, params: &mut Self::Params<'_, '_>);
}

pub struct Handled<T> {
    inner: hashbrown::HashMap<Handle<T>, T>,
}

impl<T> Default for Handled<T> {
    fn default() -> Self {
        Self {
            inner: Default::default(),
        }
    }
}

// TODO: Custom `Hash` impl
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Handle<T>(Uuid, PhantomData<T>);

impl<T: Destroyable> Destroyable for Handled<T> {
    type Params<'w, 's> = T::Params<'w, 's>;

    fn destroy(&mut self, params: &mut T::Params<'_, '_>) {
        for (_, val) in &mut self.inner {
            val.destroy(params);
        }
    }
}
