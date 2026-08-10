use bevy::{animation::AnimationTargetId, gltf::Gltf, prelude::*};
use bevy_animation_controllers::{
    AnimationBlendAsset, AnimationBlendAssetClipHandle, AnimationBlendAssetRing2d,
    AnimationBlendAssetStop1d, AnimationBlendAssetType,
};
use bevy_asset_loader::prelude::*;

use crate::animation;

use super::{LOWER_BODY_MASK_GROUP, UPPER_BODY_MASK_GROUP, LOWER_BODY_MASK, AnimationBlendAssetHandle};

#[derive(AssetCollection, Resource)]
pub(crate) struct CharacterAssets {
    #[asset(path = "models/UAL1.glb")]
    pub ual1: Handle<Gltf>,
    #[asset(path = "models/UAL2.glb")]
    pub ual2: Handle<Gltf>,
}

/// Holds all animation assets built from the loaded gltf files,
/// ready before the character scene is spawned.
#[derive(Resource)]
pub(crate) struct PrebuiltAnimationAssets {
    pub graph: Handle<AnimationGraph>,
    pub upper_body_node: AnimationNodeIndex,
    pub root_node: AnimationNodeIndex,
    pub locomotion_blend: Handle<AnimationBlendAsset>,
    pub aim_blend: Handle<AnimationBlendAsset>,
    pub jump_start: Handle<AnimationClip>,
    pub jump_loop: Handle<AnimationClip>,
    pub jump_land: Handle<AnimationClip>,
}

/// Builds the animation graph, blend assets, and clip resources from the
/// loaded gltf files. Runs once when entering `AppStates::Next`, before the
/// character is spawned.
pub(crate) fn build_animation_assets(
    mut commands: Commands,
    character_assets: Res<CharacterAssets>,
    gltf_assets: Res<Assets<Gltf>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    mut blend_assets: ResMut<Assets<AnimationBlendAsset>>,
    mut blend_handle: ResMut<AnimationBlendAssetHandle>,
) {
    let Some(gltf) = gltf_assets.get(&character_assets.ual1) else {
        error!("UAL1 gltf not loaded");
        return;
    };
    let Some(gltf2) = gltf_assets.get(&character_assets.ual2) else {
        error!("UAL2 gltf not loaded");
        return;
    };

    // -- Graph structure with mask groups --
    let mut graph = AnimationGraph::new();
    let root_node = graph.root;
    let upper_body_node = graph.add_blend_with_mask(LOWER_BODY_MASK, 0.0, root_node);
    add_bones_mask(&mut graph);
    let graph_handle = graphs.add(graph);

    // -- Collect clips --
    let clip = |gltf: &Gltf, name: &str| -> Handle<AnimationClip> {
        gltf.named_animations[name].clone()
    };

    let directional_clips = |gltf: &Gltf, names: &[&str; 8]| -> Vec<Handle<AnimationClip>> {
        names.iter().map(|n| clip(gltf, n)).collect()
    };

    let idle = clip(gltf, "Idle_Loop");

    let walk = directional_clips(gltf2, &[
        "Walk_Fwd_Loop", "Walk_Fwd_R_Loop", "Walk_R_Loop", "Walk_Bwd_R_Loop",
        "Walk_Bwd_Loop", "Walk_Bwd_L_Loop", "Walk_L_Loop", "Walk_Fwd_L",
    ]);

    let jog = directional_clips(gltf, &[
        "Jog_Fwd_Loop", "Jog_Fwd_R_Loop", "Jog_Right_Loop", "Jog_Bwd_R_Loop",
        "Jog_Bwd_Loop", "Jog_Bwd_L_Loop", "Jog_Left_Loop", "Jog_Fwd_L_Loop",
    ]);

    let jump_start = clip(gltf, "Jump_Start");
    let jump_loop = clip(gltf, "Jump_Loop");
    let jump_land = clip(gltf, "Jump_Land");

    let pistol_aim_up = clip(gltf, "Pistol_Aim_Up");
    let pistol_aim_neutral = clip(gltf, "Pistol_Aim_Neutral");
    let pistol_aim_down = clip(gltf, "Pistol_Aim_Down");

    // -- Aim blend (1d) --
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

    // -- Locomotion blend (2d) --
    let ring_stops = |clips: &[Handle<AnimationClip>], time: f32| -> AnimationBlendAssetRing2d {
        AnimationBlendAssetRing2d {
            time,
            stops: clips
                .iter()
                .enumerate()
                .map(|(i, h)| {
                    AnimationBlendAssetStop1d::new(
                        h.clone(),
                        i as f32 * std::f32::consts::FRAC_PI_4,
                    )
                })
                .collect(),
        }
    };

    let locomotion_blend = AnimationBlendAsset {
        blend_type: AnimationBlendAssetType::Blend2d {
            center: AnimationBlendAssetClipHandle(idle),
            rings: vec![
                ring_stops(&walk, 1.0),
                ring_stops(&jog, 2.0),
            ],
        },
    };
    let locomotion_blend_handle = blend_assets.add(locomotion_blend);
    *blend_handle = AnimationBlendAssetHandle(locomotion_blend_handle.clone());

    // -- Insert resources --
    commands.insert_resource(animation::JumpAnimationClips {
        jump_start: jump_start.clone(),
        jump_loop: jump_loop.clone(),
        jump_land: jump_land.clone(),
    });
    commands.insert_resource(animation::UpperBodyAnimationClips {
        pistol_aim_blend: aim_blend_handle.clone(),
    });
    commands.insert_resource(PrebuiltAnimationAssets {
        graph: graph_handle,
        upper_body_node,
        root_node,
        locomotion_blend: locomotion_blend_handle,
        aim_blend: aim_blend_handle,
        jump_start,
        jump_loop,
        jump_land,
    });
}

/// Assigns bones to mask groups using hierarchical name paths.
///
/// Lower body: root, pelvis, thighs, calves, feet, balls.
/// Upper body: spine, clavicles, arms, hands, fingers, neck, head.
fn add_bones_mask(graph: &mut AnimationGraph) {
    const SPINE: &str = "root/pelvis/spine_01/spine_02/spine_03";
    const HAND_R: &str = "root/pelvis/spine_01/spine_02/spine_03/clavicle_r/upperarm_r/lowerarm_r/hand_r";
    const HAND_L: &str = "root/pelvis/spine_01/spine_02/spine_03/clavicle_l/upperarm_l/lowerarm_l/hand_l";

    // Lower body chains: (prefix, suffix) pairs.
    const LOWER_BODY_CHAINS: &[(&str, &str)] = &[
        ("root", "pelvis"),
        ("root/pelvis", "thigh_r/calf_r/foot_r/ball_r/ball_leaf_r"),
        ("root/pelvis", "thigh_l/calf_l/foot_l/ball_l/ball_leaf_l"),
    ];

    // Upper body chains.
    const UPPER_BODY_CHAINS: &[(&str, &str)] = &[
        ("root/pelvis", "spine_01/spine_02/spine_03"),
        (SPINE, "clavicle_r/upperarm_r/lowerarm_r/hand_r"),
        (HAND_R, "thumb_01_r/thumb_02_r/thumb_03_r/thumb_04_leaf_r"),
        (HAND_R, "ring_01_r/ring_02_r/ring_03_r/ring_04_leaf_r"),
        (HAND_R, "pinky_01_r/pinky_02_r/pinky_03_r/pinky_04_leaf_r"),
        (HAND_R, "middle_01_r/middle_02_r/middle_03_r/middle_04_leaf_r"),
        (HAND_R, "index_01_r/index_02_r/index_03_r/index_04_leaf_r"),
        (SPINE, "clavicle_l/upperarm_l/lowerarm_l/hand_l"),
        (HAND_L, "thumb_01_l/thumb_02_l/thumb_03_l/thumb_04_leaf_l"),
        (HAND_L, "ring_01_l/ring_02_l/ring_03_l/ring_04_leaf_l"),
        (HAND_L, "pinky_01_l/pinky_02_l/pinky_03_l/pinky_04_leaf_l"),
        (HAND_L, "middle_01_l/middle_02_l/middle_03_l/middle_04_leaf_l"),
        (HAND_L, "index_01_l/index_02_l/index_03_l/index_04_leaf_l"),
        (SPINE, "neck_01/Head"),
    ];

    for (prefix, suffix) in LOWER_BODY_CHAINS {
        add_chain_to_mask_group(graph, prefix, suffix, LOWER_BODY_MASK_GROUP);
    }
    for (prefix, suffix) in UPPER_BODY_CHAINS {
        add_chain_to_mask_group(graph, prefix, suffix, UPPER_BODY_MASK_GROUP);
    }
}

/// Adds every bone in a (prefix, suffix) chain to the given mask group.
///
/// The prefix is the parent path, the suffix is the bone chain within it.
/// Every bone from the prefix root through each suffix segment is added.
fn add_chain_to_mask_group(
    graph: &mut AnimationGraph,
    prefix: &str,
    suffix: &str,
    group: u32,
) {
    let prefix_names: Vec<Name> = prefix.split('/').map(|s| Name::new(s.to_string())).collect();
    let suffix_names: Vec<Name> = suffix.split('/').map(|s| Name::new(s.to_string())).collect();

    for chain_length in 0..=suffix_names.len() {
        let target_id = AnimationTargetId::from_names(
            prefix_names.iter().chain(suffix_names[..chain_length].iter()),
        );
        graph.add_target_to_mask_group(target_id, group);
    }
}
