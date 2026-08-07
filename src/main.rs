mod animation;
mod ik;

use animation::{AnimationBlendAssetHandle, LocomotionController};
use bevy::{
    animation::{AnimationTargetId, RepeatAnimation},
    app::AnimationSystems,
    asset::AssetPath,
    gltf::Gltf,
    log::LogPlugin,
    platform::collections::HashMap,
    prelude::*,
    remote::{RemotePlugin, http::RemoteHttpPlugin},
    world_serialization::WorldInstanceReady,
};
use bevy_animation_controllers::{
    AnimationBlend, AnimationBlendAsset, AnimationBlendAssetClipHandle, AnimationBlendAssetRing2d,
    AnimationBlendAssetStop1d, AnimationBlendAssetType, AnimationBlendTime, AnimationLayer,
    LabeledAnimationBlend,
    control::AnimationTransitionMode,
    playback::{self, PlayingAnimation, PlayingAnimations},
    retargeting::{
        AnimationRetargetGroup, AnimationRetargeter, AnimationsRetargetedEvent,
        RetargetedAnimations,
    },
};
use bevy_animation_controllers::{
    AnimationControllersPlugin, control::update_animation_controllers,
};
use bevy_fsm::{
    Enter, EnumEvent, FSMPlugin, FSMState, FSMTransition, StateChangeRequest, fsm_observer,
};
use ik::SpineIkChain;
use smallvec::smallvec;
use std::{borrow::Cow, time::Duration};
use strum::{AsRefStr, IntoStaticStr};

const CHARACTER_PATH: &str = "models/UAL1.glb";
const JUMP_START_SECS: f32 = 0.8;
const JUMP_LOOP_SECS: f32 = 0.1; // auto-land timing
const JUMP_LAND_SECS: f32 = 0.6;

const LOWER_BODY_MASK_GROUP: u32 = 1;
const LOWER_BODY_MASK: u64 = 1 << LOWER_BODY_MASK_GROUP;
const UPPER_BODY_MASK_GROUP: u32 = 2;
const UPPER_BODY_MASK: u64 = 1 << UPPER_BODY_MASK_GROUP;

fn main() {
    let mut app = App::new();
    build_app(&mut app);
    app.add_plugins(DefaultPlugins.set(LogPlugin {
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
    .add_systems(
        Update,
        (
            keyboard_input,
            update_locomotion_jump_timers,
            update_animation_controllers::<LocomotionController>,
        )
            .chain(),
    )
    // .init_asset::<AnimationBlendAsset>()
    // .add_plugins(FSMPlugin::<LocomotionState>::default())
    // .add_plugins(FSMPlugin::<AirborneState>::default())
    // .add_plugins(FSMPlugin::<UpperBodyState>::default())
    .add_systems(Startup, (spawn_character, setup_camera_and_light))
    // .add_systems(
    //     Update,
    //     (
    //         drive_locomotion_input,
    //         drive_upper_body_input,
    //         detect_falling,
    //         detect_landing,
    //     ),
    // )
    // .add_systems(
    //     PostUpdate,
    //     (
    //         playback::advance_transitions,
    //         playback::expire_completed_transitions,
    //     )
    //         .before(bevy::animation::animate_targets)
    //         .in_set(AnimationSystems),
    // )
    .run();
}

fn build_app(app: &mut App) {
    fsm_observer!(app, LocomotionState, on_enter_locomotion);
    fsm_observer!(app, UpperBodyState, on_enter_upper_body);

    fsm_observer!(app, AirborneState, on_enter_grounded);
    fsm_observer!(app, AirborneState, on_enter_jump_start);
    fsm_observer!(app, AirborneState, on_enter_falling);
    fsm_observer!(app, AirborneState, on_enter_landing);
}

#[derive(Component)]
struct Grounded;

// ---------- Trait abstractions shared by every FSM layer ----------

trait AnimatedState: FSMState {
    /// Key into the shared animation table.
    fn animation_key(self) -> Option<AnimationState>;

    /// Optional animation group for layering.
    fn mask_group(self) -> u32 {
        0
    }

    /// Optional mask for layering.
    fn mask(self) -> u64 {
        u64::MAX
    }
}

// ---------- Locomotion layer (legs) ----------

#[derive(Component, EnumEvent, FSMState, Reflect, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[reflect(Component)]
enum LocomotionState {
    Idle,
    Walk,
    Run,
}

impl FSMTransition for LocomotionState {
    fn can_transition(_from: Self, _to: Self) -> bool {
        true // any of the three to any other is fine
    }
}

impl AnimatedState for LocomotionState {
    fn animation_key(self) -> Option<AnimationState> {
        match self {
            LocomotionState::Idle => Some(AnimationState::Idle),
            LocomotionState::Walk => Some(AnimationState::WalkLoop),
            LocomotionState::Run => Some(AnimationState::JogFwdLoop),
        }
    }

    fn mask_group(self) -> u32 {
        LOWER_BODY_MASK_GROUP
    }

    fn mask(self) -> u64 {
        LOWER_BODY_MASK
    }
}

// ---------- Airborne layer (jump), independent of locomotion ----------

#[derive(Component, EnumEvent, FSMState, Reflect, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[reflect(Component)]
enum AirborneState {
    Grounded,
    JumpStart,
    Falling,
    Landing,
}

impl FSMTransition for AirborneState {
    fn can_transition(from: Self, to: Self) -> bool {
        use AirborneState::*;
        matches!(
            (from, to),
            (Grounded, JumpStart) | (JumpStart, Falling) | (Falling, Landing) | (Landing, Grounded)
        ) || from == to
    }
}

// ---------- Upper body layer ----------

#[derive(Component, EnumEvent, FSMState, Reflect, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[reflect(Component)]
enum UpperBodyState {
    Unarmed,
    PistolIdle,
}

impl FSMTransition for UpperBodyState {
    fn can_transition(_from: Self, _to: Self) -> bool {
        true
    }
}

impl AnimatedState for UpperBodyState {
    fn animation_key(self) -> Option<AnimationState> {
        // match self {
        //     UpperBodyState::Unarmed => None,
        //     UpperBodyState::PistolIdle => Some(AnimationState::PistolIdleLoop),
        // }
        match self {
            UpperBodyState::Unarmed => Some(AnimationState::UnarmedIdleLoop),
            UpperBodyState::PistolIdle => Some(AnimationState::PistolIdleLoop),
        }
    }

    fn mask_group(self) -> u32 {
        UPPER_BODY_MASK_GROUP
    }

    fn mask(self) -> u64 {
        UPPER_BODY_MASK
    }
}

// ---------- Shared animation table ----------

#[derive(Clone, Copy)]
struct AnimationStateInfo {
    node: AnimationNodeIndex,
    clip_id: AssetId<AnimationClip>,
    repeat: RepeatAnimation,
    blend: Duration,
}

#[derive(IntoStaticStr, AsRefStr, Clone, Eq, Hash, PartialEq, Debug)]
pub enum AnimationState {
    Idle,
    WalkLoop,
    JogFwdLoop,
    JumpStart,
    JumpLoop,
    JumpLand,
    UnarmedIdleLoop,
    PistolIdleLoop,
}

#[derive(Component, Clone)]
struct CharacterAnimations {
    anims: HashMap<AnimationState, AnimationStateInfo>,
}

impl CharacterAnimations {
    fn get(&self, key: AnimationState) -> Option<&AnimationStateInfo> {
        self.anims.get(&key)
    }
}

#[derive(Component, Clone)]
struct CharacterLink {
    rig: Entity,
    anims: CharacterAnimations,
    // spine_bone: Option<Entity>,
}

#[derive(Component)]
struct PendingGltf(Handle<Gltf>);

#[derive(Component)]
struct PendingGltfs(Handle<Gltf>, Handle<Gltf>);

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

fn spawn_character(mut commands: Commands, asset_server: Res<AssetServer>) {
    let gltf_handle: Handle<Gltf> = asset_server.load(CHARACTER_PATH);
    let gltf2_handle: Handle<Gltf> = asset_server.load("models/UAL2.glb");

    let node_path = GltfAssetLabel::Node(0).from_asset(CHARACTER_PATH);

    commands
        .spawn((
            WorldAssetRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(CHARACTER_PATH))),
            PendingGltfs(gltf_handle, gltf2_handle),
            Transform::default(),
        ))
        // .observe(on_character_ready);
        .observe(on_character_spawned);
}

fn on_character_spawned(
    ready: On<WorldInstanceReady>,
    mut commands: Commands,
    children: Query<&Children>,
    q_players: Query<Entity, With<AnimationPlayer>>,
    bones: Query<(&Name, &AnimationTargetId)>,
    q_pending: Query<&PendingGltfs>,
    gltf_assets: Res<Assets<Gltf>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    mut blend_handle: ResMut<AnimationBlendAssetHandle>,
    mut blend_assets: ResMut<Assets<AnimationBlendAsset>>,
) {
    let Ok(PendingGltfs(gltf_handle, gltf2_handle)) = q_pending.get(ready.entity) else {
        return;
    };
    let Some(gltf) = gltf_assets.get(gltf_handle) else {
        error!("gltf1 is't loaded");
        return;
    };
    let Some(gltf2) = gltf_assets.get(gltf2_handle) else {
        error!("gltf2 is't loaded");
        return;
    };
    let Some(rig_entity) = children
        .iter_descendants(ready.entity)
        .find(|e| q_players.contains(*e))
    else {
        error!("No AnimationPlayer found on loaded character");
        return;
    };

    let mut graph = AnimationGraph::new();
    let root_node = graph.root;
    let upper_body_node = graph.add_blend(1.0, root_node);
    let graph_handle = graphs.add(graph);

    // Load 8 directional animation clips, jump clips & pistol aim clip
    let idle = gltf.named_animations["Idle_Loop"].clone();

    let walk_n = gltf2.named_animations["Walk_Fwd_Loop"].clone();
    let walk_ne = gltf2.named_animations["Walk_Fwd_R_Loop"].clone();
    let walk_e = gltf2.named_animations["Walk_R_Loop"].clone();
    let walk_se = gltf2.named_animations["Walk_Bwd_R_Loop"].clone();
    let walk_s = gltf2.named_animations["Walk_Bwd_Loop"].clone();
    let walk_sw = gltf2.named_animations["Walk_Bwd_L_Loop"].clone();
    let walk_w = gltf2.named_animations["Walk_L_Loop"].clone();
    let walk_nw = gltf2.named_animations["Walk_Fwd_L"].clone();

    let jog_n = gltf.named_animations["Jog_Fwd_Loop"].clone();
    let jog_ne = gltf.named_animations["Jog_Fwd_R_Loop"].clone();
    let jog_e = gltf.named_animations["Jog_Right_Loop"].clone();
    let jog_se = gltf.named_animations["Jog_Bwd_R_Loop"].clone();
    let jog_s = gltf.named_animations["Jog_Bwd_Loop"].clone();
    let jog_sw = gltf.named_animations["Jog_Bwd_L_Loop"].clone();
    let jog_w = gltf.named_animations["Jog_Left_Loop"].clone();
    let jog_nw = gltf.named_animations["Jog_Fwd_L_Loop"].clone();

    let jump_start = gltf.named_animations["Jump_Start"].clone();
    let jump_loop = gltf.named_animations["Jump_Loop"].clone();
    let jump_land = gltf.named_animations["Jump_Land"].clone();

    let pistol_aim_up = gltf.named_animations["Pistol_Aim_Up"].clone();
    let pistol_aim_neutral = gltf.named_animations["Pistol_Aim_Neutral"].clone();
    let pistol_aim_down = gltf.named_animations["Pistol_Aim_Down"].clone();

    let aim_blend = AnimationBlendAsset {
        blend_type: AnimationBlendAssetType::Blend1d {
            stops: vec![
                AnimationBlendAssetStop1d::new(pistol_aim_down.clone(), -1.0),
                AnimationBlendAssetStop1d::new(pistol_aim_neutral.clone(), 0.0),
                AnimationBlendAssetStop1d::new(pistol_aim_up.clone(), 1.0),
            ],
        },
    };
    let aim_blend_handle = blend_assets.add(aim_blend);

    commands.insert_resource(animation::JumpAnimationClips {
        jump_start: jump_start.clone(),
        jump_loop: jump_loop.clone(),
        jump_land: jump_land.clone(),
    });

    commands.insert_resource(animation::UpperBodyAnimationClips {
        pistol_aim_up: pistol_aim_up.clone(),
        pistol_aim_neutral: pistol_aim_neutral.clone(),
        pistol_aim_down: pistol_aim_down.clone(),
        pistol_aim_blend: aim_blend_handle.clone(),
    });

    let blend = AnimationBlendAsset {
        blend_type: AnimationBlendAssetType::Blend2d {
            center: AnimationBlendAssetClipHandle(idle),
            rings: vec![
                AnimationBlendAssetRing2d {
                    time: 1.0, // radius: full walk speed
                    stops: vec![
                        AnimationBlendAssetStop1d::new(walk_n, 0.0), // North
                        AnimationBlendAssetStop1d::new(walk_ne, std::f32::consts::FRAC_PI_4), // NE
                        AnimationBlendAssetStop1d::new(walk_e, std::f32::consts::FRAC_PI_2), // East
                        AnimationBlendAssetStop1d::new(walk_se, 3.0 * std::f32::consts::FRAC_PI_4), // SE
                        AnimationBlendAssetStop1d::new(walk_s, std::f32::consts::PI), // South
                        AnimationBlendAssetStop1d::new(walk_sw, 5.0 * std::f32::consts::FRAC_PI_4), // SW
                        AnimationBlendAssetStop1d::new(walk_w, 3.0 * std::f32::consts::FRAC_PI_2), // West
                        AnimationBlendAssetStop1d::new(walk_nw, 7.0 * std::f32::consts::FRAC_PI_4), // NW
                    ],
                },
                AnimationBlendAssetRing2d {
                    time: 2.0, // Jog speed radius
                    stops: vec![
                        AnimationBlendAssetStop1d::new(jog_n, 0.0),
                        AnimationBlendAssetStop1d::new(jog_ne, std::f32::consts::FRAC_PI_4),
                        AnimationBlendAssetStop1d::new(jog_e, std::f32::consts::FRAC_PI_2),
                        AnimationBlendAssetStop1d::new(jog_se, 3.0 * std::f32::consts::FRAC_PI_4),
                        AnimationBlendAssetStop1d::new(jog_s, std::f32::consts::PI),
                        AnimationBlendAssetStop1d::new(jog_sw, 5.0 * std::f32::consts::FRAC_PI_4),
                        AnimationBlendAssetStop1d::new(jog_w, 3.0 * std::f32::consts::FRAC_PI_2),
                        AnimationBlendAssetStop1d::new(jog_nw, 7.0 * std::f32::consts::FRAC_PI_4),
                    ],
                },
            ],
        },
    };

    *blend_handle = AnimationBlendAssetHandle(blend_assets.add(blend));

    // Group 0 = Layer 0 (Locomotion & Jump)
    let group_locomotion = AnimationRetargetGroup {
        animations: vec![
            (
                blend_handle.0.id().into(), // AnimationAssetId::Blend
                RepeatAnimation::Forever,
            ),
            (jump_start.id().into(), RepeatAnimation::Never),
            (jump_loop.id().into(), RepeatAnimation::Forever),
            (jump_land.id().into(), RepeatAnimation::Never),
        ],
        graph_node: root_node,
    };

    // Group 1 = Layer 1 (Upper Body / Pistol Aim)
    let group_upper_body = AnimationRetargetGroup {
        animations: vec![(aim_blend_handle.id().into(), RepeatAnimation::Forever)],
        graph_node: upper_body_node,
    };

    commands.entity(rig_entity).insert((
        AnimationGraphHandle(graph_handle),
        PlayingAnimations::new(2),
        RetargetedAnimations::default(),
        AnimationRetargeter {
            groups: vec![group_locomotion, group_upper_body],
            dest_root_joint: Name::new("root"), // or your root bone name
            include_dest_root_joint_in_path: true,
        },
    ));

    let initial_blend = AnimationBlend::Blend {
        blend: blend_handle.0.id(),
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
        current_upper_body_state: animation::UpperBodyState::Unarmed,
        initialized: false,
    });
}

fn keyboard_input(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut query: Query<&mut LocomotionController>,
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

    let velocity = if input != Vec2::ZERO {
        input.normalize() * speed
    } else {
        Vec2::ZERO
    };

    let aiming = mouse.pressed(MouseButton::Right) || keys.pressed(KeyCode::KeyE);

    for mut controller in &mut query {
        controller.velocity = velocity;

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

fn is_lower_body_bone(name: &str) -> bool {
    ["def-hips", "def-thigh", "def-shin", "def-foot", "def-toe"]
        .iter()
        .any(|kw| name.to_lowercase().contains(kw))
}

fn on_character_ready(
    ready: On<WorldInstanceReady>,
    mut commands: Commands,
    children: Query<&Children>,
    q_players: Query<Entity, With<AnimationPlayer>>,
    bones: Query<(&Name, &AnimationTargetId)>,
    q_pending: Query<&PendingGltf>,
    gltf_assets: Res<Assets<Gltf>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
) {
    let Ok(PendingGltf(gltf_handle)) = q_pending.get(ready.entity) else {
        return;
    };
    let Some(gltf) = gltf_assets.get(gltf_handle) else {
        return;
    };
    let Some(rig_entity) = children
        .iter_descendants(ready.entity)
        .find(|e| q_players.contains(*e))
    else {
        error!("No AnimationPlayer found on loaded character");
        return;
    };

    let mut graph = AnimationGraph::new();
    let root = graph.root;
    // let upper_body_blend = graph.add_blend_with_mask(LOWER_BODY_MASK, 1.0, root);
    // let aim_add = graph.add_additive_blend(1.0, upper_body_blend);
    let add_node = graph.add_additive_blend(1.0, graph.root);
    // let lower_body_blend = graph.add_blend(1.0, add_node);
    let upper_body_blend = graph.add_blend(1.0, add_node);
    // let full_body = graph.add_blend(1.0, graph.root);
    let mut anims = HashMap::new();

    let mut add_anim = |name, clip: Handle<AnimationClip>, repeat, blend, parent, mask| {
        let clip_id = clip.id();
        let node = graph.add_clip_with_mask(clip, mask, 1.0, parent);
        anims.insert(
            name,
            AnimationStateInfo {
                node,
                clip_id,
                repeat,
                blend,
            },
        );
    };

    add_anim(
        AnimationState::Idle,
        gltf.named_animations["Idle_Loop"].clone(),
        RepeatAnimation::Forever,
        Duration::from_millis(300),
        root,
        UPPER_BODY_MASK,
    );
    add_anim(
        AnimationState::WalkLoop,
        gltf.named_animations["Walk_Loop"].clone(),
        RepeatAnimation::Forever,
        Duration::from_millis(250),
        root,
        UPPER_BODY_MASK,
    );
    add_anim(
        AnimationState::JogFwdLoop,
        gltf.named_animations["Jog_Fwd_Loop"].clone(),
        RepeatAnimation::Forever,
        Duration::from_millis(200),
        root,
        UPPER_BODY_MASK,
    );
    add_anim(
        AnimationState::JumpStart,
        gltf.named_animations["Jump_Start"].clone(),
        RepeatAnimation::Never,
        Duration::from_millis(80),
        root,
        UPPER_BODY_MASK,
    );
    add_anim(
        AnimationState::JumpLoop,
        gltf.named_animations["Jump_Loop"].clone(),
        RepeatAnimation::Forever,
        Duration::from_millis(50),
        root,
        UPPER_BODY_MASK,
    );
    add_anim(
        AnimationState::JumpLand,
        gltf.named_animations["Jump_Land"].clone(),
        RepeatAnimation::Never,
        Duration::from_millis(80),
        root,
        UPPER_BODY_MASK,
    );

    // Upper-body layer: masked to exclude the lower body group.
    add_anim(
        AnimationState::PistolIdleLoop,
        gltf.named_animations["Pistol_Idle_Loop"].clone(),
        RepeatAnimation::Forever,
        Duration::from_millis(250),
        upper_body_blend,
        0,
    );
    add_anim(
        AnimationState::UnarmedIdleLoop,
        gltf.named_animations["Idle_Loop"].clone(),
        RepeatAnimation::Forever,
        Duration::from_millis(250),
        upper_body_blend,
        0,
    );

    dbg!(&graph);

    for (name, target_id) in children
        .iter_descendants(rig_entity)
        .filter_map(|e| bones.get(e).ok())
    {
        let group = if is_lower_body_bone(name.as_str()) {
            LOWER_BODY_MASK_GROUP
        } else {
            UPPER_BODY_MASK_GROUP
        };
        graph.add_target_to_mask_group(*target_id, group);
    }

    let graph_handle = graphs.add(graph);

    commands.entity(rig_entity).insert((
        AnimationGraphHandle(graph_handle),
        PlayingAnimations::new(2),
    ));

    commands.entity(ready.entity).insert((
        LocomotionState::Idle,
        AirborneState::Grounded,
        UpperBodyState::Unarmed,
        CharacterLink {
            rig: rig_entity,
            anims: CharacterAnimations { anims },
        },
    ));
}

// ---------- Animation reactions ----------

fn play_animation(
    entity: Entity,
    layer: AnimationLayer,
    key: AnimationState,
    links: &Query<&CharacterLink>,
    playing: &mut Query<&mut PlayingAnimations>,
    players: &mut Query<&mut AnimationPlayer>,
    blend_assets: &Assets<AnimationBlendAsset>,
) {
    let Ok(link) = links.get(entity) else { return };
    let Some(info) = link.anims.get(key.clone()) else {
        return;
    };
    let Ok(mut playing_animations) = playing.get_mut(link.rig) else {
        return;
    };
    let Ok(mut player) = players.get_mut(link.rig) else {
        return;
    };

    playing_animations.group_mut(layer).play(
        &mut player,
        PlayingAnimation {
            nodes: smallvec![info.node],
            blend: LabeledAnimationBlend {
                blend: AnimationBlend::Single { clip: info.clip_id },
                label: Cow::Borrowed(key.into()),
            },
        },
        info.blend,
        info.repeat,
        AnimationTransitionMode::ChangeAndRestart,
        blend_assets,
    );
}

fn play_jump(
    entity: Entity,
    state: AirborneState,
    links: &Query<&CharacterLink>,
    playing: &mut Query<&mut PlayingAnimations>,
    players: &mut Query<&mut AnimationPlayer>,
    blend_assets: &Assets<AnimationBlendAsset>,
) {
    let key = match state {
        AirborneState::JumpStart => AnimationState::JumpStart,
        AirborneState::Falling => AnimationState::JumpLoop,
        AirborneState::Landing => AnimationState::JumpLand,
        AirborneState::Grounded => return, // handled by on_enter_grounded via locomotion
    };
    play_animation(
        entity,
        AnimationLayer(0),
        key,
        links,
        playing,
        players,
        blend_assets,
    );
}

fn on_enter_locomotion(
    trigger: On<Enter<LocomotionState>>,
    links: Query<&CharacterLink>,
    mut playing: Query<&mut PlayingAnimations>,
    mut players: Query<&mut AnimationPlayer>,
    blend_assets: Res<Assets<AnimationBlendAsset>>,
) {
    if let Some(key) = trigger.state.animation_key() {
        play_animation(
            trigger.entity,
            AnimationLayer(0),
            key,
            &links,
            &mut playing,
            &mut players,
            &blend_assets,
        );
    }
}

fn on_enter_upper_body(
    trigger: On<Enter<UpperBodyState>>,
    links: Query<&CharacterLink>,
    mut playing: Query<&mut PlayingAnimations>,
    mut players: Query<&mut AnimationPlayer>,
    blend_assets: Res<Assets<AnimationBlendAsset>>,
) {
    match trigger.state.animation_key() {
        Some(key) => {
            play_animation(
                trigger.entity,
                AnimationLayer(1),
                key,
                &links,
                &mut playing,
                &mut players,
                &blend_assets,
            );
        }
        None => {
            let Ok(link) = links.get(trigger.entity) else {
                return;
            };
            let Ok(mut playing_animations) = playing.get_mut(link.rig) else {
                return;
            };
            let Ok(mut player) = players.get_mut(link.rig) else {
                return;
            };

            playing_animations
                .group_mut(AnimationLayer(1))
                .stop(&mut player, Duration::from_millis(200));
        }
    }
}

fn on_enter_grounded(
    trigger: On<Enter<airborne_state::Grounded>>,
    locomotion: Query<&LocomotionState>,
    links: Query<&CharacterLink>,
    mut playing: Query<&mut PlayingAnimations>,
    mut players: Query<&mut AnimationPlayer>,
    blend_assets: Res<Assets<AnimationBlendAsset>>,
) {
    if let Ok(&state) = locomotion.get(trigger.entity) {
        if let Some(key) = state.animation_key() {
            play_animation(
                trigger.entity,
                AnimationLayer(0),
                key,
                &links,
                &mut playing,
                &mut players,
                &blend_assets,
            );
        }
    }
}

fn on_enter_jump_start(
    trigger: On<Enter<airborne_state::JumpStart>>,
    mut commands: Commands,
    links: Query<&CharacterLink>,
    mut playing: Query<&mut PlayingAnimations>,
    mut players: Query<&mut AnimationPlayer>,
    blend_assets: Res<Assets<AnimationBlendAsset>>,
) {
    play_jump(
        trigger.entity,
        AirborneState::JumpStart,
        &links,
        &mut playing,
        &mut players,
        &blend_assets,
    );
    commands
        .delayed()
        .secs(JUMP_START_SECS)
        .trigger(StateChangeRequest {
            entity: trigger.entity,
            next: AirborneState::Falling,
        });
}

fn on_enter_falling(
    trigger: On<Enter<airborne_state::Falling>>,
    mut commands: Commands,
    links: Query<&CharacterLink>,
    mut playing: Query<&mut PlayingAnimations>,
    mut players: Query<&mut AnimationPlayer>,
    blend_assets: Res<Assets<AnimationBlendAsset>>,
) {
    info!("Jump Loop");
    play_jump(
        trigger.entity,
        AirborneState::Falling,
        &links,
        &mut playing,
        &mut players,
        &blend_assets,
    );
    commands
        .delayed()
        .secs(JUMP_LOOP_SECS)
        .trigger(StateChangeRequest {
            entity: trigger.entity,
            next: AirborneState::Landing,
        });
}

fn on_enter_landing(
    trigger: On<Enter<airborne_state::Landing>>,
    mut commands: Commands,
    links: Query<&CharacterLink>,
    mut playing: Query<&mut PlayingAnimations>,
    mut players: Query<&mut AnimationPlayer>,
    blend_assets: Res<Assets<AnimationBlendAsset>>,
) {
    info!("Jump Land");
    play_jump(
        trigger.entity,
        AirborneState::Landing,
        &links,
        &mut playing,
        &mut players,
        &blend_assets,
    );
    commands
        .delayed()
        .secs(JUMP_LAND_SECS)
        .trigger(StateChangeRequest {
            entity: trigger.entity,
            next: AirborneState::Grounded,
        });
}

// ---------- Input ----------

fn drive_locomotion_input(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    characters: Query<(Entity, &LocomotionState, &AirborneState)>,
) {
    let moving = keys.any_pressed([KeyCode::KeyW, KeyCode::KeyA, KeyCode::KeyS, KeyCode::KeyD]);
    let sprinting = keys.pressed(KeyCode::ShiftLeft);
    let desired = match (moving, sprinting) {
        (false, _) => LocomotionState::Idle,
        (true, false) => LocomotionState::Walk,
        (true, true) => LocomotionState::Run,
    };

    for (entity, &locomotion, &airborne) in &characters {
        if airborne == AirborneState::Grounded {
            if locomotion != desired {
                commands.trigger(StateChangeRequest {
                    entity,
                    next: desired,
                });
            }
            if keys.just_pressed(KeyCode::Space) {
                commands.trigger(StateChangeRequest {
                    entity,
                    next: AirborneState::JumpStart,
                });
            }
        }
    }
}

fn drive_upper_body_input(
    mut commands: Commands,
    mouse: Res<ButtonInput<MouseButton>>,
    characters: Query<(Entity, &UpperBodyState)>,
) {
    let desired = if mouse.pressed(MouseButton::Right) {
        UpperBodyState::PistolIdle
    } else {
        UpperBodyState::Unarmed
    };

    for (entity, &current) in &characters {
        if current != desired {
            commands.trigger(StateChangeRequest {
                entity,
                next: desired,
            });
        }
    }
}

fn detect_falling(
    mut removed: RemovedComponents<Grounded>,
    mut commands: Commands,
    airborne: Query<&AirborneState>,
) {
    for entity in removed.read() {
        if airborne.get(entity) == Ok(&AirborneState::Grounded) {
            commands.trigger(StateChangeRequest {
                entity,
                next: AirborneState::Falling,
            });
        }
    }
}

fn detect_landing(
    grounded_added: Query<Entity, Added<Grounded>>,
    mut commands: Commands,
    airborne: Query<&AirborneState>,
) {
    for entity in &grounded_added {
        match airborne.get(entity) {
            Ok(&AirborneState::Falling) => commands.trigger(StateChangeRequest {
                entity,
                next: AirborneState::Landing,
            }),
            _ => {}
        }
    }
}
