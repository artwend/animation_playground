use std::time::Duration;

use bevy::{
    animation::{RepeatAnimation},
    gltf::Gltf,
    prelude::*,
    scene::prelude::{bsn, CommandsSceneExt},
    world_serialization::WorldInstanceReady,
};
use bevy_gearbox::prelude::*;
use bevy_gearbox::GearboxPlugin;

const CHARACTER_PATH: &str = "models/character.glb";
const JUMP_START_SECS: f32 = 0.8;
const JUMP_LAND_SECS: f32 = 0.6;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(GearboxPlugin::default())
        .add_systems(Startup, (spawn_character, setup_camera_and_light))
        .add_systems(
            Update,
            (
                drive_locomotion_messages.before(GearboxSet),
                play_state_animation.after(GearboxSet),
            ),
        )
        .run();
}

// ---------- Messages that drive transitions ----------
// Each just needs to know which state-machine entity it targets;
// GearboxMessage's derive walks `SubstateOf` up to the root for us.

#[derive(Message, Clone, Reflect, GearboxMessage)]
struct StartWalk {
    #[gearbox(target)]
    machine: Entity,
}
#[derive(Message, Clone, Reflect, GearboxMessage)]
struct StartRun {
    #[gearbox(target)]
    machine: Entity,
}
#[derive(Message, Clone, Reflect, GearboxMessage)]
struct SlowDown {
    #[gearbox(target)]
    machine: Entity,
}
#[derive(Message, Clone, Reflect, GearboxMessage)]
struct Stop {
    #[gearbox(target)]
    machine: Entity,
}
#[derive(Message, Clone, Reflect, GearboxMessage)]
struct Jump {
    #[gearbox(target)]
    machine: Entity,
}

#[derive(Message, Clone, Reflect, GearboxMessage)]
struct Land {
    #[gearbox(target)]
    machine: Entity,
}

// ---------- Linking the state machine to the rig ----------

#[derive(Component, Clone)]
struct CharacterAnimations {
    idle: AnimationNodeIndex,
    walk: AnimationNodeIndex,
    run: AnimationNodeIndex,
    jump_start: AnimationNodeIndex,
    jump_loop: AnimationNodeIndex,
    jump_land: AnimationNodeIndex,
}

/// Tracks what we last *requested*, purely so `drive_locomotion_messages`
/// doesn't spam the same message every frame. The gearbox `Active` markers
/// remain the single source of truth for actual state.
#[derive(Component, Clone, Copy, Default, PartialEq)]
enum Locomotion {
    #[default]
    Idle,
    Walk,
    Run,
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
        Transform::from_rotation(Quat::from_euler(EulerRot::ZYX, 0.0, 1.0, -std::f32::consts::PI / 4.)),
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
    let anims = CharacterAnimations {
        idle: graph.add_clip(gltf.named_animations["Idle_Loop"].clone(), 1.0, root),
        walk: graph.add_clip(gltf.named_animations["Walk_Loop"].clone(), 1.0, root),
        run: graph.add_clip(gltf.named_animations["Jog_Fwd_Loop"].clone(), 1.0, root),
        jump_start: graph.add_clip(gltf.named_animations["Jump_Start"].clone(), 1.0, root),
        jump_loop: graph.add_clip(gltf.named_animations["Jump_Loop"].clone(), 1.0, root),
        jump_land: graph.add_clip(gltf.named_animations["Jump_Land"].clone(), 1.0, root),
    };
    let graph_handle = graphs.add(graph);

    commands.entity(rig_entity).insert((
        AnimationGraphHandle(graph_handle),
        AnimationTransitions::new(),
    ));

    // The state machine is its own scene, linked back to the rig via `template`.
    commands.spawn_scene(bsn! {
        #Character
            // template(move |_| Ok((AnimationRig(rig_entity), anims.clone(), Locomotion::default())))
            StateMachine InitialState(#Locomotion)
        Substates [
            #Locomotion History InitialState(#Idle) Substates [
            #Idle Transitions [
                (Target(#Walk) MessageEdge::<StartWalk>),
                (Target(#Run)  MessageEdge::<StartRun>),
                (Target(#Jump) MessageEdge::<Jump>),
            ],
            #Walk Transitions [
                (Target(#Run)  MessageEdge::<StartRun>),
                (Target(#Idle) MessageEdge::<Stop>),
                (Target(#Jump) MessageEdge::<Jump>),
            ],
            #Run Transitions [
                (Target(#Walk) MessageEdge::<StartWalk>),
                (Target(#Idle) MessageEdge::<Stop>),
                (Target(#Jump) MessageEdge::<Jump>),
            ],
        ],
        #Jump InitialState(#Jump_Start) Substates [
            #Jump_Start Transitions [
                (Target(#Jump_Loop) AlwaysEdge Delay::from_secs_f32(JUMP_START_SECS)),
            ],
            #Jump_Loop Transitions [
                // (Target(#Jump_Land) MessageEdge::<Land>),
                (Target(#Jump_Land) AlwaysEdge Delay::from_secs_f32(0.1)), // auto-land after 0.1s
            ],
            #Jump_Land Transitions [
                (Target(#Locomotion) AlwaysEdge Delay::from_secs_f32(JUMP_LAND_SECS)),
            ],
        ],
        ]
    })
    .insert((
        CharacterLink { rig: rig_entity, anims },
        Locomotion::default(),
    ));
}

// ---------- Gameplay -> messages ----------

fn drive_locomotion_messages(
    keys: Res<ButtonInput<KeyCode>>,
    mut characters: Query<(Entity, &mut Locomotion), With<StateMachine>>,
    mut walk_w: MessageWriter<StartWalk>,
    mut run_w: MessageWriter<StartRun>,
    mut stop_w: MessageWriter<Stop>,
    mut jump_w: MessageWriter<Jump>,
) {
    let moving = keys.any_pressed([KeyCode::KeyW, KeyCode::KeyA, KeyCode::KeyS, KeyCode::KeyD]);
    let sprinting = keys.pressed(KeyCode::ShiftLeft);
    let desired = match (moving, sprinting) {
        (false, _) => Locomotion::Idle,
        (true, false) => Locomotion::Walk,
        (true, true) => Locomotion::Run,
    };

    for (machine, mut locomotion) in &mut characters {
        if desired != *locomotion {
            match &desired {
                Locomotion::Walk => { walk_w.write(StartWalk { machine }); }
                Locomotion::Run => { run_w.write(StartRun { machine }); }
                Locomotion::Idle => { stop_w.write(Stop { machine }); }
            }
            *locomotion = desired;
        }

        if keys.just_pressed(KeyCode::Space) {
            jump_w.write(Jump { machine });
        }
    }
}

// ---------- Gearbox state -> animation ----------

fn animation_for_state(name: &str, anims: &CharacterAnimations) -> Option<(AnimationNodeIndex, RepeatAnimation, Duration)> {
    match name {
        "Idle" => Some((anims.idle, RepeatAnimation::Forever, Duration::from_millis(300))),
        "Walk" => Some((anims.walk, RepeatAnimation::Forever, Duration::from_millis(250))),
        "Run"  => Some((anims.run,  RepeatAnimation::Forever, Duration::from_millis(200))),
        "Jump_Start" => Some((anims.jump_start, RepeatAnimation::Never,   Duration::from_millis(80))),
        "Jump_Loop" => Some((anims.jump_loop, RepeatAnimation::Forever,   Duration::from_millis(250))),
        "Jump_Land" => Some((anims.jump_land, RepeatAnimation::Never,   Duration::from_millis(80))),
        _ => None,
    }
}

fn play_state_animation(
    entered: Query<(&Name, &Active), Added<Active>>,
    characters: Query<&CharacterLink>,
    mut rigs: Query<(&mut AnimationPlayer, &mut AnimationTransitions)>,
) {
    for (name, active) in &entered {
         let Ok(link) = characters.get(active.machine) else {
            continue;
        };
         let Ok((mut player, mut transitions)) = rigs.get_mut(link.rig) else {
            continue;
        };

        let Some((node, repeat, blend)) = animation_for_state(name.as_str(), &link.anims) else {
            continue;
        };

        transitions.play(&mut player, node, blend).set_repeat(repeat);
    }
}