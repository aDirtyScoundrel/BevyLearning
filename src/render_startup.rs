//! Clear unrendered startup surfaces on the Bevy 0.19 render path.
//! Remove this workaround after validating an upgrade that fixes Bevy issue 22733.
//! https://github.com/bevyengine/bevy/issues/22733
use bevy::camera::{CameraOutputMode, NormalizedRenderTarget};
use bevy::core_pipeline::blit::{BlitPipeline, BlitPipelineKey};
use bevy::prelude::*;
use bevy::render::{
    RenderApp,
    camera::ExtractedCamera,
    render_resource::{
        BlendState, CommandEncoderDescriptor, LoadOp, Operations, PipelineCache,
        RenderPassColorAttachment, RenderPassDescriptor, SpecializedRenderPipelines, StoreOp,
    },
    renderer::{RenderDevice, RenderGraph, RenderGraphSystems, RenderQueue},
    view::{ViewTarget, window::ExtractedWindows},
};

pub struct StartupSurfacePlugin;

impl Plugin for StartupSurfacePlugin {
    fn build(&self, app: &mut App) {
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app.add_systems(
                RenderGraph,
                clear_pending_surface.in_set(RenderGraphSystems::Begin),
            );
        }
    }
}

// Bevy system: the argument list IS the dependency injection.
#[allow(clippy::too_many_arguments)]
fn clear_pending_surface(
    windows: Res<ExtractedWindows>,
    cameras: Query<(&ViewTarget, &ExtractedCamera)>,
    pipeline_cache: Res<PipelineCache>,
    mut pipelines: ResMut<SpecializedRenderPipelines<BlitPipeline>>,
    blit_pipeline: Res<BlitPipeline>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    clear_color: Res<ClearColor>,
    mut log_count: Local<u32>,
) {
    for window in windows.values() {
        let Some(swap_chain_view) = &window.swap_chain_texture_view else {
            continue;
        };

        let mut has_ready_output = false;

        for (view_target, camera) in cameras.iter() {
            if !matches!(camera.output_mode, CameraOutputMode::Write { .. }) {
                continue;
            }

            if !matches!(camera.target, Some(NormalizedRenderTarget::Window(w)) if w.entity() == window.entity)
            {
                continue;
            }

            let Some(target_format) = view_target.out_texture_view_format() else {
                continue;
            };

            let blend_state =
                output_blend(&camera.output_mode, camera.sorted_camera_index_for_target);
            let Some(blend_state) = blend_state else {
                continue;
            };

            let key = BlitPipelineKey {
                target_format,
                blend_state,
                samples: 1,
                source_space: view_target.compositing_space,
            };

            let id = pipelines.specialize(&pipeline_cache, &blit_pipeline, key);
            if pipeline_cache.get_render_pipeline(id).is_some() {
                has_ready_output = true;
                break;
            }
        }

        if has_ready_output {
            continue;
        }

        let clear_value = clear_color.0.to_linear().into();
        let attachment = RenderPassColorAttachment {
            view: swap_chain_view,
            depth_slice: None,
            resolve_target: None,
            ops: Operations {
                load: LoadOp::Clear(clear_value),
                store: StoreOp::Store,
            },
        };

        let pass_descriptor = RenderPassDescriptor {
            label: Some("startup-surface-clear"),
            color_attachments: &[Some(attachment)],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        };

        let mut encoder = render_device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("startup-surface-clear-encoder"),
        });
        encoder.begin_render_pass(&pass_descriptor);
        let command_buffer = encoder.finish();
        render_queue.submit([command_buffer]);

        if *log_count < 4 {
            info!(
                "[render-startup] cleared pending surface for window {:?}",
                window.entity
            );
            *log_count += 1;
        }
    }
}

fn output_blend(mode: &CameraOutputMode, index: usize) -> Option<Option<BlendState>> {
    match mode {
        CameraOutputMode::Skip => None,
        CameraOutputMode::Write { blend_state, .. } => Some(match blend_state {
            None => {
                if index > 0 {
                    Some(BlendState::ALPHA_BLENDING)
                } else {
                    None
                }
            }
            Some(state) => Some(*state),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headless_app_does_not_require_render_resources() {
        let mut app = App::new();
        app.add_plugins(StartupSurfacePlugin);
        app.update();
    }

    #[test]
    fn test_skip_mode() {
        let mode = CameraOutputMode::Skip;
        assert_eq!(output_blend(&mode, 0), None);
    }

    #[test]
    fn test_index0_default_replace() {
        let mode = CameraOutputMode::Write {
            blend_state: None,
            clear_color: ClearColorConfig::Default,
        };
        assert_eq!(output_blend(&mode, 0), Some(None));
    }

    #[test]
    fn test_later_default_alpha() {
        let mode = CameraOutputMode::Write {
            blend_state: None,
            clear_color: ClearColorConfig::Default,
        };
        assert_eq!(
            output_blend(&mode, 1),
            Some(Some(BlendState::ALPHA_BLENDING))
        );
    }

    #[test]
    fn test_explicit_blend_preserved() {
        let mode = CameraOutputMode::Write {
            blend_state: Some(BlendState::REPLACE),
            clear_color: ClearColorConfig::Default,
        };
        assert_eq!(output_blend(&mode, 3), Some(Some(BlendState::REPLACE)));
    }
}
