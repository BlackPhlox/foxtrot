use crate::player_control::actions::{Actions, ActionsFrozen};
use crate::player_control::camera::focus::set_camera_focus;
use crate::util::trait_extension::{Vec2Ext, Vec3Ext};
use crate::GameState;
use bevy::prelude::*;
use bevy::window::CursorGrabMode;
use bevy_rapier3d::prelude::*;
use serde::{Deserialize, Serialize};
use std::f32::consts::{PI, TAU};

pub mod focus;

const MAX_DISTANCE: f32 = 5.0;

pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<UiCamera>()
            .register_type::<MainCamera>()
            .add_startup_system(spawn_ui_camera)
            // Enables the system that synchronizes your `Transform`s and `LookTransform`s.
            .add_system_set(SystemSet::on_enter(GameState::Playing).with_system(despawn_ui_camera))
            .add_system_set(
                SystemSet::on_update(GameState::Playing)
                    .with_system(follow_target.label("follow_target"))
                    .with_system(
                        handle_camera_controls
                            .label("handle_camera_controls")
                            .after("follow_target"),
                    )
                    .with_system(
                        update_camera_transform
                            .label("update_camera_transform")
                            .after("handle_camera_controls"),
                    )
                    .with_system(cursor_grab_system)
                    .with_system(init_camera_eye.before("follow_target"))
                    .with_system(set_camera_focus.before("follow_target")),
            );
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Component, Reflect, Serialize, Deserialize, Default)]
#[reflect(Component, Serialize, Deserialize)]
pub struct UiCamera;

#[derive(Debug, Clone, PartialEq, Component, Reflect, Serialize, Deserialize)]
#[reflect(Component, Serialize, Deserialize)]
pub struct MainCamera {
    current: CameraPosition,
    new: CameraPosition,
    up: Vec3,
}

impl Default for MainCamera {
    fn default() -> Self {
        Self {
            current: default(),
            new: default(),
            up: Vec3::Y,
        }
    }
}

impl MainCamera {
    pub fn set_target(&mut self, target: Vec3) -> &mut Self {
        self.new.target = target;
        self
    }

    pub fn set_up(&mut self, up: Vec3) -> &mut Self {
        self.up = up;
        self
    }

    pub fn forward(&self) -> Vec3 {
        self.new.eye.forward()
    }
}

#[derive(Debug, Clone, PartialEq, Component, Reflect, Serialize, Deserialize, Default)]
#[reflect(Component, Serialize, Deserialize)]
pub struct CameraPosition {
    pub eye: Transform,
    pub target: Vec3,
}

fn spawn_ui_camera(mut commands: Commands) {
    commands.spawn((Camera2dBundle::default(), UiCamera, Name::new("Camera")));
}

fn despawn_ui_camera(mut commands: Commands, query: Query<Entity, With<UiCamera>>) {
    for entity in &query {
        commands.entity(entity).despawn_recursive();
    }
}

fn init_camera_eye(mut camera_query: Query<(&Transform, &mut MainCamera), Added<MainCamera>>) {
    for (transform, mut camera) in &mut camera_query {
        camera.current.eye = *transform;
    }
}

fn follow_target(mut camera_query: Query<&mut MainCamera>) {
    for mut camera in &mut camera_query {
        let target_movement = (camera.new.target - camera.current.target).collapse_approx_zero();
        camera.new.eye.translation = camera.current.eye.translation + target_movement;

        let new_target = camera.new.target;
        if !(new_target - camera.new.eye.translation).is_approx_zero() {
            let up = camera.up;
            camera.new.eye.look_at(new_target, up);
        }
    }
}

fn handle_camera_controls(mut camera_query: Query<&mut MainCamera>, actions: Res<Actions>) {
    let mouse_sensitivity = 1e-2;
    let camera_movement = match actions.camera_movement {
        Some(vector) => vector * mouse_sensitivity,
        None => return,
    };

    if camera_movement.is_approx_zero() {
        return;
    }

    for mut camera in camera_query.iter_mut() {
        let yaw = -camera_movement.x.clamp(-PI, PI);
        let yaw_rotation = Quat::from_axis_angle(camera.up, yaw);

        let pitch = -camera_movement.y;
        let pitch = clamp_pitch(&camera, pitch);
        let pitch_rotation = Quat::from_axis_angle(camera.new.eye.local_x(), pitch);

        let pivot = camera.new.target;
        let rotation = yaw_rotation * pitch_rotation;
        camera.new.eye.rotate_around(pivot, rotation);
    }
}

fn clamp_pitch(camera: &MainCamera, angle: f32) -> f32 {
    const MOST_ACUTE_ALLOWED_FROM_ABOVE: f32 = TAU / 10.;
    const MOST_ACUTE_ALLOWED_FROM_BELOW: f32 = TAU / 7.;

    let forward = camera.new.eye.forward();
    let up = camera.up;
    let angle_to_axis = forward.angle_between(up);
    let (acute_angle_to_axis, most_acute_allowed, sign) = if angle_to_axis > PI / 2. {
        (PI - angle_to_axis, MOST_ACUTE_ALLOWED_FROM_ABOVE, -1.)
    } else {
        (angle_to_axis, MOST_ACUTE_ALLOWED_FROM_BELOW, 1.)
    };
    let new_angle = if acute_angle_to_axis < most_acute_allowed {
        angle - sign * (most_acute_allowed - acute_angle_to_axis)
    } else {
        angle
    };
    if new_angle.abs() < 0.01 {
        0.
    } else {
        new_angle
    }
}

fn update_camera_transform(
    time: Res<Time>,
    mut camera_query: Query<(&mut Transform, &mut MainCamera)>,
    rapier_context: Res<RapierContext>,
) {
    let dt = time.delta_seconds();
    for (mut transform, mut camera) in camera_query.iter_mut() {
        let line_of_sight_result = keep_line_of_sight(&camera, &rapier_context);
        let translation_smoothing =
            if line_of_sight_result.correction == LineOfSightCorrection::Closer {
                25.
            } else {
                10.
            };
        let direction = line_of_sight_result.location - transform.translation;
        let scale = (translation_smoothing * dt).max(1.);
        transform.translation += direction * scale;

        let rotation_smoothing = 15.;
        let scale = (rotation_smoothing * dt).max(1.);
        transform.rotation = transform.rotation.slerp(camera.new.eye.rotation, scale);

        camera.current = camera.new.clone();
    }
}

fn keep_line_of_sight(camera: &MainCamera, rapier_context: &RapierContext) -> LineOfSightResult {
    let origin = camera.new.target;
    let desired_direction = camera.new.eye.translation - camera.new.target;
    let norm_direction = desired_direction.try_normalize().unwrap_or(Vec3::Z);

    let distance = get_raycast_distance(origin, norm_direction, rapier_context, MAX_DISTANCE);
    let location = origin + norm_direction * distance;
    let correction = if distance * distance < desired_direction.length_squared() {
        LineOfSightCorrection::Closer
    } else {
        LineOfSightCorrection::Further
    };
    LineOfSightResult {
        location,
        correction,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct LineOfSightResult {
    pub location: Vec3,
    pub correction: LineOfSightCorrection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineOfSightCorrection {
    Closer,
    Further,
}

pub fn get_raycast_distance(
    origin: Vec3,
    direction: Vec3,
    rapier_context: &RapierContext,
    max_distance: f32,
) -> f32 {
    let max_toi = max_distance;
    let solid = true;
    let mut filter = QueryFilter::only_fixed();
    filter.flags |= QueryFilterFlags::EXCLUDE_SENSORS;

    let min_distance_to_objects = 0.01;
    rapier_context
        .cast_ray(origin, direction, max_toi, solid, filter)
        .map(|(_entity, toi)| toi - min_distance_to_objects)
        .unwrap_or(max_distance)
}

fn cursor_grab_system(
    mut windows: ResMut<Windows>,
    key: Res<Input<KeyCode>>,
    frozen: Option<Res<ActionsFrozen>>,
) {
    let window = windows.get_primary_mut().unwrap();
    if frozen.is_some() {
        window.set_cursor_grab_mode(CursorGrabMode::None);
        window.set_cursor_visibility(true);
        return;
    }
    if key.just_pressed(KeyCode::Escape) {
        if matches!(window.cursor_grab_mode(), CursorGrabMode::None) {
            // if you want to use the cursor, but not let it leave the window,
            // use `Confined` mode:
            window.set_cursor_grab_mode(CursorGrabMode::Confined);

            // for a game that doesn't use the cursor (like a shooter):
            // use `Locked` mode to keep the cursor in one place
            window.set_cursor_grab_mode(CursorGrabMode::Locked);
            // also hide the cursor
            window.set_cursor_visibility(false);
        } else {
            window.set_cursor_grab_mode(CursorGrabMode::None);
            window.set_cursor_visibility(true);
        }
    }
}
