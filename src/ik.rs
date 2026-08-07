use bevy::prelude::*;

#[derive(Component)]
pub struct SpineIkChain {
    /// The global 3D position the spine should look at/reach for
    pub target_pos: Vec3,
    /// Influence of IK: 0.0 = pure animation clip, 1.0 = pure IK
    pub weight: f32,
    /// List of entities representing the spine hierarchy (Root to Tip)
    pub bone_entities: Vec<Entity>,
}

fn apply_spine_ik_blend(
    ik_query: Query<&SpineIkChain>,
    mut transform_query: Query<&mut Transform>,
    global_transform_query: Query<&GlobalTransform>,
) {
    for ik in ik_query.iter() {
        // Skip processing if IK is completely turned off
        if ik.weight <= 0.0 { continue; }
        
        let weight = ik.weight.clamp(0.0, 1.0);

        // Iterate through your spine bones (e.g., Spine1, Spine2, Spine3)
        for &bone_entity in &ik.bone_entities {
            // Get the current global position of this bone to calculate the direction
            let Ok(bone_global) = global_transform_query.get(bone_entity) else { continue; };
            let bone_world_pos = bone_global.translation();

            // Calculate the direction from this spine bone to the dynamic target
            let look_dir = (ik.target_pos - bone_world_pos).normalize_or_zero();
            if look_dir == Vec3::ZERO { continue; }

            // 1. Get the current local transform (this holds the data from your animation clip!)
            let Ok(mut bone_transform) = transform_query.get_mut(bone_entity) else { continue; };
            let animated_rotation = bone_transform.rotation;

            // 2. Calculate the target IK rotation
            // Note: Adjust Vec3::Y depending on which axis your model's spine aligns with
            let ik_rotation = Quat::from_rotation_arc(Vec3::Y, look_dir);

            // 3. BLEND THEM TOGETHER
            // Slerp smoothly interpolates between the animation clip and the IK target
            bone_transform.rotation = animated_rotation.slerp(ik_rotation, weight);
        }
    }
}