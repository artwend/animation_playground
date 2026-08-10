use bevy::prelude::*;
use bevy_animation_controllers::{
    AnimationBlend, AnimationBlendAsset, AnimationBlendTime, AnimationLayer,
    LabeledAnimationBlend,
    control::{AnimationControl, AnimationTransitionMode},
};
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum JumpPhase {
    #[default]
    Grounded,
    JumpStart,
    JumpLoop,
    JumpLand,
}

/// Whether the upper-body layer should be blended in.
///
/// This is only a *desired* state. The actual blend amount is ramped toward
/// this state by `update_upper_body_blend` in `main.rs`, which adjusts the
/// weight of the upper-body blend node in the animation graph.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum UpperBodyState {
    #[default]
    Unarmed,
    PistolAim,
}

#[derive(Component)]
pub(crate) struct LocomotionController {
    pub velocity: Vec2, // x = right/left, y = forward/backward
    pub jump_phase: JumpPhase,
    pub jump_timer: Timer,
    pub upper_body: UpperBodyState,
    pub current_locomotion_state: LocomotionState,
    pub initialized: bool,
}

#[derive(Clone, Resource, Default)]
pub(crate) struct AnimationBlendAssetHandle(pub Handle<AnimationBlendAsset>);

#[derive(Clone, Resource, Default)]
pub(crate) struct JumpAnimationClips {
    pub jump_start: Handle<AnimationClip>,
    pub jump_loop: Handle<AnimationClip>,
    pub jump_land: Handle<AnimationClip>,
}

#[derive(Clone, Resource, Default)]
pub(crate) struct UpperBodyAnimationClips {
    pub pistol_aim_blend: Handle<AnimationBlendAsset>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum ActiveAnimation {
    LocomotionBlend { speed: f32, angle: f32 },
    JumpStart,
    JumpLoop,
    JumpLand,
}

#[derive(Clone)]
pub(crate) struct LocomotionState {
    pub active: ActiveAnimation,
    pub blend: LabeledAnimationBlend,
    pub transition: Duration,
}

#[derive(Clone)]
pub(crate) struct CharacterAnimState {
    pub locomotion: LocomotionState,
}

impl AnimationControl for LocomotionController {
    type AnimationState = CharacterAnimState;
    type SystemParam = (
        Res<'static, AnimationBlendAssetHandle>,
        Res<'static, JumpAnimationClips>,
        Res<'static, UpperBodyAnimationClips>,
    );

    // Layer 0 = locomotion/jump (whole body), Layer 1 = upper body (aim).
    const LAYER_COUNT: u32 = 2;

    fn compute_new_animation_state(
        &self,
        _entity: Entity,
        param: &bevy::ecs::system::StaticSystemParam<'_, '_, Self::SystemParam>,
    ) -> Self::AnimationState {
        let blend_handle = &param.0;
        let jump_clips = &param.1;

        let locomotion = match self.jump_phase {
            JumpPhase::JumpStart => LocomotionState {
                active: ActiveAnimation::JumpStart,
                blend: LabeledAnimationBlend::from(AnimationBlend::Single {
                    clip: jump_clips.jump_start.id(),
                }),
                transition: Duration::from_millis(80),
            },
            JumpPhase::JumpLoop => LocomotionState {
                active: ActiveAnimation::JumpLoop,
                blend: LabeledAnimationBlend::from(AnimationBlend::Single {
                    clip: jump_clips.jump_loop.id(),
                }),
                transition: Duration::from_millis(50),
            },
            JumpPhase::JumpLand => LocomotionState {
                active: ActiveAnimation::JumpLand,
                blend: LabeledAnimationBlend::from(AnimationBlend::Single {
                    clip: jump_clips.jump_land.id(),
                }),
                transition: Duration::from_millis(80),
            },
            JumpPhase::Grounded => {
                let speed = self.velocity.length();
                let dir = self.velocity.normalize_or_zero();
                let angle = dir.x.atan2(dir.y);
                let angle = if angle < 0.0 {
                    angle + std::f32::consts::TAU
                } else {
                    angle
                };

                let blend = AnimationBlend::Blend {
                    blend: blend_handle.0.id(),
                    time: AnimationBlendTime::Blend2d(Vec2::new(speed, angle)),
                };

                LocomotionState {
                    active: ActiveAnimation::LocomotionBlend { speed, angle },
                    blend: LabeledAnimationBlend::from(blend),
                    transition: Duration::from_millis(250),
                }
            }
        };

        CharacterAnimState { locomotion }
    }

    fn compute_animation_action(
        &self,
        group: AnimationLayer,
        new_state: &Self::AnimationState,
    ) -> AnimationTransitionMode {
        if !self.initialized {
            return AnimationTransitionMode::ChangeAndRestart;
        }

        match group.0 {
            0 => {
                let old = &self.current_locomotion_state;
                if new_state.locomotion.active != old.active {
                    AnimationTransitionMode::ChangeAndRestart
                } else
                if let (
                    ActiveAnimation::LocomotionBlend {
                        speed: new_speed,
                        angle: new_angle,
                    },
                    ActiveAnimation::LocomotionBlend {
                        speed: old_speed,
                        angle: old_angle,
                    },
                ) = (new_state.locomotion.active, old.active)
                {
                    if (new_speed < 0.01) != (old_speed < 0.01) {
                        AnimationTransitionMode::ChangeAndRestart
                    } else
                    if (new_speed - old_speed).abs() > 0.05 || (new_angle - old_angle).abs() > 0.1 {
                        AnimationTransitionMode::ChangeTime
                    } else {
                        AnimationTransitionMode::NoChange
                    }
                } else {
                    AnimationTransitionMode::NoChange
                }
            }
            // The upper-body layer always keeps the aim blend playing; its
            // visibility is controlled by the blend node weight instead.
            1 => AnimationTransitionMode::NoChange,
            _ => AnimationTransitionMode::NoChange,
        }
    }

    fn animation_for_state(
        group: AnimationLayer,
        state: &Self::AnimationState,
        param: &bevy::ecs::system::StaticSystemParam<'_, '_, Self::SystemParam>,
    ) -> (Option<LabeledAnimationBlend>, Duration) {
        match group.0 {
            0 => (
                Some(state.locomotion.blend.clone()),
                state.locomotion.transition,
            ),
            1 => {
                let upper_body_clips = &param.2;
                (
                    Some(LabeledAnimationBlend::from(AnimationBlend::Blend {
                        blend: upper_body_clips.pistol_aim_blend.id(),
                        time: AnimationBlendTime::Blend1d(0.0),
                    })),
                    Duration::from_millis(150),
                )
            }
            _ => (None, Duration::ZERO),
        }
    }

    fn set_current_animation_state(&mut self, new_state: &Self::AnimationState) {
        self.current_locomotion_state = new_state.locomotion.clone();
        self.initialized = true;
    }
}
