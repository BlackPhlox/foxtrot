use crate::spawning::{GameObject, PrimedGameObjectSpawner};
use bevy::gltf::Gltf;
use bevy::prelude::*;

pub const PATH: &str = "scenes/house.glb";

pub fn load_scene(asset_server: &Res<AssetServer>) -> Handle<Gltf> {
    asset_server.load(PATH)
}

impl<'w, 's, 'a, 'b> PrimedGameObjectSpawner<'w, 's, 'a, 'b> {
    pub fn spawn_house(&'a mut self) {
        self.spawn_gltf(GameObject::House, Transform::from_scale(Vec3::splat(3.)));
    }
}
