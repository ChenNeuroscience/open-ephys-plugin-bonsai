//! Draws the camera frame with wgpu inside egui (as a paint callback), so a frame costs one 8-bit upload
//! and the threshold highlight and crop mask are applied per pixel on the GPU (see video.wgsl).

use std::sync::Arc;

use eframe::egui;
use eframe::egui_wgpu::{self, wgpu, CallbackResources, CallbackTrait, ScreenDescriptor};
use image::GrayImage;

use crate::tracker::View;

/// What to draw this frame. Cheap to build: the frame and mask are shared, not copied.
pub struct VideoCallback {
    pub target_format: wgpu::TextureFormat,
    pub view: Arc<View>,
    /// `None` means no crop.
    pub mask: Option<Arc<GrayImage>>,
    pub threshold: u8,
    pub highlight: bool,
    pub crop_visible: bool,
}

/// GPU objects, created on first use and kept in egui_wgpu's callback resources.
struct Resources {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    frame: Option<(wgpu::Texture, u64)>, // texture and the id of the frame it holds
    mask: (wgpu::Texture, Option<Arc<GrayImage>>), // texture and the mask it holds
    bind_group: Option<wgpu::BindGroup>,
}

impl Resources {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, target_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("video.wgsl"));
        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("video"),
            entries: &[
                texture_entry(0),
                texture_entry(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("video"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("video"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(target_format.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("video uniforms"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let no_mask = r8_texture(device, 1, 1);
        upload(queue, &no_mask, 1, 1, &[255]);
        Resources { pipeline, layout, uniforms, frame: None, mask: (no_mask, None), bind_group: None }
    }
}

fn r8_texture(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("video r8"),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn upload(queue: &wgpu::Queue, texture: &wgpu::Texture, width: u32, height: u32, data: &[u8]) {
    queue.write_texture(
        texture.as_image_copy(),
        data,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(width), rows_per_image: Some(height) },
        wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
    );
}

impl CallbackTrait for VideoCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let res = resources.entry().or_insert_with(|| Resources::new(device, queue, self.target_format));
        let image = &self.view.frame.image;
        let (w, h) = image.dimensions();

        let mut rebind = res.bind_group.is_none();
        if !res.frame.as_ref().is_some_and(|(t, _)| t.width() == w && t.height() == h) {
            res.frame = Some((r8_texture(device, w, h), u64::MAX));
            rebind = true;
        }
        let (texture, shown_id) = res.frame.as_mut().unwrap();
        if *shown_id != self.view.frame.id {
            upload(queue, texture, w, h, image.as_raw());
            *shown_id = self.view.frame.id;
        }

        let same_mask = match (&self.mask, &res.mask.1) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if !same_mask {
            let texture = match &self.mask {
                Some(m) => {
                    let t = r8_texture(device, m.width(), m.height());
                    upload(queue, &t, m.width(), m.height(), m.as_raw());
                    t
                }
                None => {
                    let t = r8_texture(device, 1, 1);
                    upload(queue, &t, 1, 1, &[255]);
                    t
                }
            };
            res.mask = (texture, self.mask.clone());
            rebind = true;
        }

        if rebind {
            let frame_view = res.frame.as_ref().unwrap().0.create_view(&Default::default());
            let mask_view = res.mask.0.create_view(&Default::default());
            res.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("video"),
                layout: &res.layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&frame_view) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&mask_view) },
                    wgpu::BindGroupEntry { binding: 2, resource: res.uniforms.as_entire_binding() },
                ],
            }));
        }

        let mut uniforms = [0u8; 16];
        uniforms[0..4].copy_from_slice(&(self.threshold as f32).to_le_bytes());
        uniforms[4..8].copy_from_slice(&(self.highlight as u32).to_le_bytes());
        uniforms[8..12].copy_from_slice(&(self.crop_visible as u32).to_le_bytes());
        uniforms[12..16].copy_from_slice(&(self.target_format.is_srgb() as u32).to_le_bytes());
        queue.write_buffer(&res.uniforms, 0, &uniforms);
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        resources: &CallbackResources,
    ) {
        let Some(res) = resources.get::<Resources>() else { return };
        let Some(bind_group) = &res.bind_group else { return };
        render_pass.set_pipeline(&res.pipeline);
        render_pass.set_bind_group(0, bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }
}

pub fn paint(painter: &egui::Painter, rect: egui::Rect, callback: VideoCallback) {
    painter.add(egui_wgpu::Callback::new_paint_callback(rect, callback));
}
