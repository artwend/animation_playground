mod animation;
mod assets;

use animation::{AnimationBlendAssetHandle, LocomotionController};
use assets::{CharacterAssets, PrebuiltAnimationAssets};
use bevy::{
    animation::RepeatAnimation,
    gltf::Gltf,
    log::LogPlugin,
    prelude::*,
    remote::{RemotePlugin, http::RemoteHttpPlugin},
    world_serialization::{WorldAssetRoot, WorldInstanceReady},
};
use bevy_animation_controllers::{
    AnimationBlend, AnimationBlendTime, LabeledAnimationBlend,
    playback::PlayingAnimations,
    retargeting::{AnimationRetargetGroup, AnimationRetargeter, RetargetedAnimations},
};
use bevy_animation_controllers::{
    AnimationControllersPlugin, control::update_animation_controllers,
};
use bevy_asset_loader::prelude::*;
use std::time::Duration;

#[derive(Clone, Eq, PartialEq, Debug, Hash, Default, States)]
enum AppStates {
    #[default]
    AssetLoading,
    Next,
}

const JUMP_START_SECS: f32 = 0.8;
const JUMP_LOOP_SECS: f32 = 0.1; // auto-land timing
const JUMP_LAND_SECS: f32 = 0.6;

pub(crate) const LOWER_BODY_MASK_GROUP: u32 = 1;
pub(crate) const LOWER_BODY_MASK: u64 = 1 << LOWER_BODY_MASK_GROUP;
pub(crate) const UPPER_BODY_MASK_GROUP: u32 = 2;

/// How fast the upper-body blend weight ramps in/out (per second).
const UPPER_BODY_RAMP_SPEED: f32 = 4.0;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(LogPlugin {
            filter: "info,bevy_animation_controllers=debug,animation_playground=debug".into(),
            level: bevy::log::Level::DEBUG,
            ..Default::default()
        }))
        .add_plugins(RemotePlugin::default())
        .add_plugins(RemoteHttpPlugin::default())
        .init_resource::<AnimationBlendAssetHandle>()
        .init_resource::<animation::JumpAnimationClips>()
        .init_resource::<animation::UpperBodyAnimationClips>()
        .add_plugins(AnimationControllersPlugin)
        .init_state::<AppStates>()
        .add_loading_state(
            LoadingState::new(AppStates::AssetLoading)
                .continue_to_state(AppStates::Next)
                .load_collection::<CharacterAssets>()
        )
        .add_systems(
            Update,
            (
                keyboard_input,
                update_locomotion_jump_timers,
                update_upper_body_blend,
                update_animation_controllers::<LocomotionController>,
            )
                .chain(),
        )
        .add_systems(Startup, setup_camera_and_light)
        .add_systems(
            OnEnter(AppStates::Next),
            (assets::build_animation_assets, spawn_character).chain(),
        )
        .run();
}

// ---------- Setup ----------

fn setup_camera_and_light(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 2.0, 5.0).looking_at(Vec3::new(0.0, 1.0, 0.0), Vec3::Y),
    ));

    commands.spawn((
        DirectionalLight {
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(
            EulerRot::ZYX,
            0.0,
            1.0,
            -std::f32::consts::PI / 4.,
        )),
    ));
}

#[derive(Resource)]
pub struct CharacterAnimations {
    pub graph: Handle<AnimationGraph>,
}

fn spawn_character(
    mut commands: Commands,
    character_assets: Res<CharacterAssets>,
    gltf_assets: Res<Assets<Gltf>>,
) {
    let Some(gltf) = gltf_assets.get(&character_assets.ual1) else { return; };
    let Some(character_scene_handle) = gltf.scenes.first() else { return; };

    commands
        .spawn((
            WorldAssetRoot(character_scene_handle.clone()),
            Transform::default(),
        ))
        .observe(on_character_spawned);
}

/// Tracks the upper-body blend node so its weight can be ramped at runtime.
#[derive(Component)]
struct UpperBodyBlend {
    graph: Handle<AnimationGraph>,
    node: AnimationNodeIndex,
    weight: f32,
}

fn on_character_spawned(
    ready: On<WorldInstanceReady>,
    mut commands: Commands,
    children: Query<&Children>,
    q_players: Query<Entity, With<AnimationPlayer>>,
    prebuilt: Res<PrebuiltAnimationAssets>,
) {
    let Some(rig_entity) = children
        .iter_descendants(ready.entity)
        .find(|e| q_players.contains(*e))
    else {
        error!("No AnimationPlayer found on loaded character");
        return;
    };

    // Group 0 = Layer 0 (Locomotion & Jump), whole body.
    let group_locomotion = AnimationRetargetGroup {
        animations: vec![
            (prebuilt.locomotion_blend.id().into(), RepeatAnimation::Forever),
            (prebuilt.jump_start.id().into(), RepeatAnimation::Never),
            (prebuilt.jump_loop.id().into(), RepeatAnimation::Forever),
            (prebuilt.jump_land.id().into(), RepeatAnimation::Never),
        ],
        graph_node: prebuilt.root_node,
    };

    // Group 1 = Layer 1 (Upper Body / Pistol Aim), masked to the upper body.
    let group_upper_body = AnimationRetargetGroup {
        animations: vec![(prebuilt.aim_blend.id().into(), RepeatAnimation::Forever)],
        graph_node: prebuilt.upper_body_node,
    };

    commands.entity(rig_entity).insert((
        AnimationGraphHandle(prebuilt.graph.clone()),
        PlayingAnimations::new(2),
        RetargetedAnimations::default(),
        AnimationRetargeter {
            groups: vec![group_locomotion, group_upper_body],
            dest_root_joint: Name::new("root"),
            include_dest_root_joint_in_path: true,
        },
    ));

    let initial_blend = AnimationBlend::Blend {
        blend: prebuilt.locomotion_blend.id(),
        time: AnimationBlendTime::Blend2d(Vec2::ZERO),
    };

    let initial_state = animation::LocomotionState {
        active: animation::ActiveAnimation::LocomotionBlend {
            speed: 0.0,
            angle: 0.0,
        },
        blend: LabeledAnimationBlend::from(initial_blend),
        transition: Duration::from_millis(150),
    };

    commands.entity(ready.entity).insert(LocomotionController {
        velocity: Vec2::ZERO,
        jump_phase: animation::JumpPhase::Grounded,
        jump_timer: Timer::default(),
        upper_body: animation::UpperBodyState::Unarmed,
        current_locomotion_state: initial_state,
        initialized: false,
    });

    commands.entity(ready.entity).insert(UpperBodyBlend {
        graph: prebuilt.graph.clone(),
        node: prebuilt.upper_body_node,
        weight: 0.0,
    });
}

// ---------- Input ----------

fn keyboard_input(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut query: Query<&mut LocomotionController>,
    time: Res<Time>,
) {
    let mut input = Vec2::ZERO;

    if keys.pressed(KeyCode::KeyW) || keys.pressed(KeyCode::ArrowUp) {
        input.y += 1.0;
    }
    if keys.pressed(KeyCode::KeyS) || keys.pressed(KeyCode::ArrowDown) {
        input.y -= 1.0;
    }
    if keys.pressed(KeyCode::KeyA) || keys.pressed(KeyCode::ArrowLeft) {
        input.x -= 1.0;
    }
    if keys.pressed(KeyCode::KeyD) || keys.pressed(KeyCode::ArrowRight) {
        input.x += 1.0;
    }

    let is_jogging = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    let speed = if is_jogging { 2.0 } else { 1.0 };

    const ACCELERATION: f32 = 4.0;
    let max_delta = ACCELERATION * time.delta_secs();
    let target_velocity = if input != Vec2::ZERO {
        input.normalize() * speed
    } else {
        Vec2::ZERO
    };

    // Condition that drives the upper-body blend: hold RMB or E to aim.
    let aiming = mouse.pressed(MouseButton::Right) || keys.pressed(KeyCode::KeyE);

    for mut controller in &mut query {
        let delta = target_velocity - controller.velocity;
        controller.velocity = if delta.length() <= max_delta {
            target_velocity
        } else {
            controller.velocity + delta.normalize() * max_delta
        };

        controller.upper_body = if aiming {
            animation::UpperBodyState::PistolAim
        } else {
            animation::UpperBodyState::Unarmed
        };

        if keys.just_pressed(KeyCode::Space)
            && controller.jump_phase == animation::JumpPhase::Grounded
        {
            controller.jump_phase = animation::JumpPhase::JumpStart;
            controller.jump_timer = Timer::from_seconds(JUMP_START_SECS, TimerMode::Once);
        }
    }
}

fn update_locomotion_jump_timers(time: Res<Time>, mut query: Query<&mut LocomotionController>) {
    for mut controller in &mut query {
        if controller.jump_phase != animation::JumpPhase::Grounded {
            controller.jump_timer.tick(time.delta());
            if controller.jump_timer.just_finished() {
                match controller.jump_phase {
                    animation::JumpPhase::JumpStart => {
                        controller.jump_phase = animation::JumpPhase::JumpLoop;
                        controller.jump_timer =
                            Timer::from_seconds(JUMP_LOOP_SECS, TimerMode::Once);
                    }
                    animation::JumpPhase::JumpLoop => {
                        controller.jump_phase = animation::JumpPhase::JumpLand;
                        controller.jump_timer =
                            Timer::from_seconds(JUMP_LAND_SECS, TimerMode::Once);
                    }
                    animation::JumpPhase::JumpLand => {
                        controller.jump_phase = animation::JumpPhase::Grounded;
                    }
                    animation::JumpPhase::Grounded => {}
                }
            }
        }
    }
}

// ---------- Upper-body weight blending ----------

/// Ramps the upper-body blend node's weight toward the desired state.
///
/// When the weight is 0 the upper-body layer contributes nothing and the whole
/// body plays the locomotion animation. When it reaches 1 the upper-body aim
/// animation is fully blended in over the upper-body bones.
fn update_upper_body_blend(
    time: Res<Time>,
    mut q: Query<(&LocomotionController, &mut UpperBodyBlend)>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
) {
    for (controller, mut blend) in &mut q {
        let target = if controller.upper_body == animation::UpperBodyState::PistolAim {
            1.0
        } else {
            0.0
        };
        if blend.weight == target { continue; }
        let delta = UPPER_BODY_RAMP_SPEED * time.delta_secs();
        blend.weight = if blend.weight < target {
            (blend.weight + delta).min(target)
        } else {
            (blend.weight - delta).max(target)
        };

        if let Some(mut graph) = graphs.get_mut(&blend.graph) {
            if let Some(node) = graph.get_mut(blend.node) {
                node.weight = blend.weight;
            }
        }
    }
}