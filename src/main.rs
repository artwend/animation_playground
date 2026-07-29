use std::time::Duration;

use bevy::{
    animation::RepeatAnimation, gltf::Gltf, platform::collections::HashMap, prelude::*,
    world_serialization::WorldInstanceReady,
};
use bevy_fsm::{
    Enter, EnumEvent, FSMPlugin, FSMState, FSMTransition, StateChangeRequest, fsm_observer,
};

const CHARACTER_PATH: &str = "models/character.glb";
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
    app.add_plugins(DefaultPlugins)
        .add_plugins(FSMPlugin::<LocomotionState>::default())
        .add_plugins(FSMPlugin::<AirborneState>::default())
        .add_systems(Startup, (spawn_character, setup_camera_and_light))
        .add_systems(
            Update,
            (
                drive_locomotion_input,
                tick_delayed_state_changes::<AirborneState>,
            ),
        )
        .run();
}

fn build_app(app: &mut App) {
    fsm_observer!(app, LocomotionState, on_enter_locomotion);

    fsm_observer!(app, AirborneState, on_enter_grounded);
    fsm_observer!(app, AirborneState, on_enter_jump_start);
    fsm_observer!(app, AirborneState, on_enter_jump_loop);
    fsm_observer!(app, AirborneState, on_enter_jump_land);
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

// ---------- Airborne layer (jump), independent of locomotion ----------

#[derive(Component, EnumEvent, FSMState, Reflect, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[reflect(Component)]
enum AirborneState {
    Grounded,
    JumpStart,
    JumpLoop,
    JumpLand,
}

impl FSMTransition for AirborneState {
    fn can_transition(from: Self, to: Self) -> bool {
        use AirborneState::*;
        matches!(
            (from, to),
            (Grounded, JumpStart)
                | (JumpStart, JumpLoop)
                | (JumpLoop, JumpLand)
                | (JumpLand, Grounded)
        ) || from == to
    }
}

// ---------- Generic timed auto-transition ----------

#[derive(Component)]
struct DelayedStateChange<S: FSMState> {
    next: S,
    timer: Timer,
}

fn tick_delayed_state_changes<S: FSMState>(
    mut commands: Commands,
    time: Res<Time>,
    mut q: Query<(Entity, &mut DelayedStateChange<S>)>,
) {
    for (entity, mut delayed) in &mut q {
        if delayed.timer.tick(time.delta()).just_finished() {
            commands.trigger(StateChangeRequest {
                entity,
                next: delayed.next,
            });
            commands.entity(entity).remove::<DelayedStateChange<S>>();
        }
    }
}

// ---------- Shared animation table ----------

#[derive(Clone, Copy)]
struct AnimationStateInfo {
    node: AnimationNodeIndex,
    repeat: RepeatAnimation,
    blend: Duration,
}

#[derive(Clone, Eq, Hash, PartialEq, Debug)]
pub enum AnimationState {
    Idle,
    WalkLoop,
    JogFwdLoop,
    JumpStart,
    JumpLoop,
    JumpLand,
}

#[derive(Component, Clone)]
struct CharacterAnimations {
    states: HashMap<AnimationState, AnimationStateInfo>,
}

impl CharacterAnimations {
    fn get(&self, key: AnimationState) -> Option<&AnimationStateInfo> {
        self.states.get(&key)
    }
}

#[derive(Component, Clone)]
struct CharacterLink {
    rig: Entity,
    anims: CharacterAnimations,
}

#[derive(Component)]
struct PendingGltf(Handle<Gltf>);

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
    commands
        .spawn((
            WorldAssetRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(CHARACTER_PATH))),
            PendingGltf(gltf_handle),
            Transform::default(),
        ))
        .observe(on_character_ready);
}

fn on_character_ready(
    ready: On<WorldInstanceReady>,
    mut commands: Commands,
    children: Query<&Children>,
    q_players: Query<Entity, With<AnimationPlayer>>,
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

    let mut states = HashMap::new();
    states.insert(
        AnimationState::Idle,
        AnimationStateInfo {
            node: graph.add_clip(gltf.named_animations["Idle_Loop"].clone(), 1.0, root),
            repeat: RepeatAnimation::Forever,
            blend: Duration::from_millis(300),
        },
    );
    states.insert(
        AnimationState::WalkLoop,
        AnimationStateInfo {
            node: graph.add_clip(gltf.named_animations["Walk_Loop"].clone(), 1.0, root),
            repeat: RepeatAnimation::Forever,
            blend: Duration::from_millis(250),
        },
    );
    states.insert(
        AnimationState::JogFwdLoop,
        AnimationStateInfo {
            node: graph.add_clip(gltf.named_animations["Jog_Fwd_Loop"].clone(), 1.0, root),
            repeat: RepeatAnimation::Forever,
            blend: Duration::from_millis(200),
        },
    );
    states.insert(
        AnimationState::JumpStart,
        AnimationStateInfo {
            node: graph.add_clip(gltf.named_animations["Jump_Start"].clone(), 1.0, root),
            repeat: RepeatAnimation::Never,
            blend: Duration::from_millis(80),
        },
    );
    states.insert(
        AnimationState::JumpLoop,
        AnimationStateInfo {
            node: graph.add_clip(gltf.named_animations["Jump_Loop"].clone(), 1.0, root),
            repeat: RepeatAnimation::Forever,
            blend: Duration::from_millis(150),
        },
    );
    states.insert(
        AnimationState::JumpLand,
        AnimationStateInfo {
            node: graph.add_clip(gltf.named_animations["Jump_Land"].clone(), 1.0, root),
            repeat: RepeatAnimation::Never,
            blend: Duration::from_millis(80),
        },
    );

    let graph_handle = graphs.add(graph);

    commands.entity(rig_entity).insert((
        AnimationGraphHandle(graph_handle),
        AnimationTransitions::new(),
    ));

    commands.entity(ready.entity).insert((
        LocomotionState::Idle,
        AirborneState::Grounded,
        CharacterLink {
            rig: rig_entity,
            anims: CharacterAnimations { states },
        },
    ));
}

// ---------- Animation reactions ----------

fn play_locomotion(
    entity: Entity,
    state: LocomotionState,
    links: &Query<&CharacterLink>,
    rigs: &mut Query<(&mut AnimationPlayer, &mut AnimationTransitions)>,
) {
    let Ok(link) = links.get(entity) else { return };
    let key = match state {
        LocomotionState::Idle => AnimationState::Idle,
        LocomotionState::Walk => AnimationState::WalkLoop,
        LocomotionState::Run => AnimationState::JogFwdLoop,
    };
    if let Some(info) = link.anims.get(key) {
        if let Ok((mut player, mut transitions)) = rigs.get_mut(link.rig) {
            transitions
                .play(&mut player, info.node, info.blend)
                .set_repeat(info.repeat);
        }
    }
}

fn play_jump(
    entity: Entity,
    state: AirborneState,
    links: &Query<&CharacterLink>,
    rigs: &mut Query<(&mut AnimationPlayer, &mut AnimationTransitions)>,
) {
    let Ok(link) = links.get(entity) else { return };
    let key = match state {
        AirborneState::JumpStart => AnimationState::JumpStart,
        AirborneState::JumpLoop => AnimationState::JumpLoop,
        AirborneState::JumpLand => AnimationState::JumpLand,
        AirborneState::Grounded => return, // handled by on_enter_grounded via locomotion
    };
    if let Some(info) = link.anims.get(key) {
        if let Ok((mut player, mut transitions)) = rigs.get_mut(link.rig) {
            transitions
                .play(&mut player, info.node, info.blend)
                .set_repeat(info.repeat);
        }
    }
}

fn on_enter_locomotion(
    trigger: On<Enter<LocomotionState>>,
    links: Query<&CharacterLink>,
    mut rigs: Query<(&mut AnimationPlayer, &mut AnimationTransitions)>,
) {
    play_locomotion(trigger.entity, trigger.state, &links, &mut rigs);
}

fn on_enter_grounded(
    trigger: On<Enter<airborne_state::Grounded>>,
    locomotion: Query<&LocomotionState>,
    links: Query<&CharacterLink>,
    mut rigs: Query<(&mut AnimationPlayer, &mut AnimationTransitions)>,
) {
    if let Ok(&state) = locomotion.get(trigger.entity) {
        play_locomotion(trigger.entity, state, &links, &mut rigs);
    }
}

fn on_enter_jump_start(
    trigger: On<Enter<airborne_state::JumpStart>>,
    mut commands: Commands,
    links: Query<&CharacterLink>,
    mut rigs: Query<(&mut AnimationPlayer, &mut AnimationTransitions)>,
) {
    play_jump(trigger.entity, AirborneState::JumpStart, &links, &mut rigs);
    commands
        .delayed()
        .secs(JUMP_START_SECS)
        .trigger(StateChangeRequest {
            entity: trigger.entity,
            next: AirborneState::JumpLoop,
        });
}

fn on_enter_jump_loop(
    trigger: On<Enter<airborne_state::JumpLoop>>,
    mut commands: Commands,
    links: Query<&CharacterLink>,
    mut rigs: Query<(&mut AnimationPlayer, &mut AnimationTransitions)>,
) {
    info!("Jump Loop");
    play_jump(trigger.entity, AirborneState::JumpLoop, &links, &mut rigs);
    commands
        .delayed()
        .secs(JUMP_LOOP_SECS)
        .trigger(StateChangeRequest {
            entity: trigger.entity,
            next: AirborneState::JumpLand,
        });
}

fn on_enter_jump_land(
    trigger: On<Enter<airborne_state::JumpLand>>,
    mut commands: Commands,
    links: Query<&CharacterLink>,
    mut rigs: Query<(&mut AnimationPlayer, &mut AnimationTransitions)>,
) {
    info!("Jump Land");
    play_jump(trigger.entity, AirborneState::JumpLand, &links, &mut rigs);
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
