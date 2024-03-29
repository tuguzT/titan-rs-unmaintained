use std::sync::Arc;

use egui::{ClippedMesh, Pos2, Texture, TextureId};
use slotmap::{DefaultKey, Key, KeyData, SlotMap};
use vulkano::buffer::{BufferUsage, CpuBufferPool, TypedBufferAccess};
use vulkano::command_buffer::{
    AutoCommandBufferBuilder, CommandBufferUsage, SecondaryAutoCommandBuffer,
};
use vulkano::descriptor_set::{DescriptorSet, PersistentDescriptorSet};
use vulkano::device::Queue;
use vulkano::format::Format;
use vulkano::image::view::ImageView;
use vulkano::image::{ImageDimensions, ImageViewAbstract, ImmutableImage, MipmapsCount};
use vulkano::pipeline::blend::{AttachmentBlend, BlendFactor};
use vulkano::pipeline::viewport::{Scissor, Viewport};
use vulkano::pipeline::{GraphicsPipeline, PipelineBindPoint};
use vulkano::render_pass::Subpass;
use vulkano::sampler::{Filter, MipmapMode, Sampler, SamplerAddressMode};
use vulkano::sync::GpuFuture;

use crate::{
    graphics::{
        frame::ui_draw::error::{UiDrawError, UiDrawSystemCreationError},
        renderer::error::DescriptorSetCreationError,
        vertex::UiVertex,
    },
    window::Size,
};

pub mod error;

pub struct UiDrawSystem {
    /// Queue to render.
    graphics_queue: Arc<Queue>,

    /// Buffer for all vertices of UI.
    vertex_buffer: Arc<CpuBufferPool<UiVertex>>,

    /// Buffer for all indices of vertices in UI element.
    index_buffer: Arc<CpuBufferPool<u32>>,

    /// Graphics pipeline used for rendering of UI.
    pipeline: Arc<GraphicsPipeline>,

    /// Version of `egui` base texture.
    texture_version: u64,

    /// Descriptor set for `egui` base texture that will be used by shader.
    texture_descriptor_set: Option<Arc<dyn DescriptorSet + Send + Sync>>,

    /// Collection of descriptor sets for user textures to be drawn in UI.
    user_texture_descriptor_sets: SlotMap<DefaultKey, Arc<dyn DescriptorSet + Send + Sync>>,

    /// A sampler for textures used in UI rendering.
    sampler: Arc<Sampler>,
}

impl UiDrawSystem {
    /// Creates new UI draw system.
    pub fn new(
        graphics_queue: Arc<Queue>,
        subpass: Subpass,
    ) -> Result<Self, UiDrawSystemCreationError> {
        // Check queue for graphics support.
        if !graphics_queue.family().supports_graphics() {
            return Err(UiDrawSystemCreationError::QueueFamilyNotSupported);
        }

        let device = graphics_queue.device().clone();
        let pipeline = {
            use crate::graphics::shader::ui::{fragment, vertex};

            let vert_shader_module = vertex::Shader::load(device.clone())?;
            let frag_shader_module = fragment::Shader::load(device.clone())?;

            let blend = AttachmentBlend {
                color_source: BlendFactor::One,
                ..AttachmentBlend::alpha_blending()
            };

            Arc::new(
                GraphicsPipeline::start()
                    .vertex_input_single_buffer::<UiVertex>()
                    .vertex_shader(vert_shader_module.main_entry_point(), ())
                    .fragment_shader(frag_shader_module.main_entry_point(), ())
                    .triangle_list()
                    .viewports_scissors_dynamic(1)
                    .cull_mode_disabled()
                    .blend_collective(blend)
                    .render_pass(subpass)
                    .build(device.clone())?,
            )
        };

        let vertex_buffer = Arc::new(CpuBufferPool::vertex_buffer(device.clone()));
        let index_buffer = Arc::new(CpuBufferPool::new(
            device.clone(),
            BufferUsage::index_buffer(),
        ));

        let sampler = Sampler::new(
            device.clone(),
            Filter::Linear,
            Filter::Linear,
            MipmapMode::Linear,
            SamplerAddressMode::ClampToEdge,
            SamplerAddressMode::ClampToEdge,
            SamplerAddressMode::ClampToEdge,
            0.0,
            1.0,
            0.0,
            0.0,
        )?;

        Ok(Self {
            graphics_queue,
            vertex_buffer,
            index_buffer,
            pipeline,
            sampler,
            texture_version: 0,
            texture_descriptor_set: None,
            user_texture_descriptor_sets: SlotMap::default(),
        })
    }

    fn image_descriptor_set(
        &self,
        image_view: Arc<dyn ImageViewAbstract + Send + Sync>,
    ) -> Result<Arc<PersistentDescriptorSet>, DescriptorSetCreationError> {
        let layout = self.pipeline.layout().descriptor_set_layouts()[0].clone();
        let mut builder = PersistentDescriptorSet::start(layout);
        builder
            .add_sampled_image(image_view, self.sampler.clone())
            .map_err(DescriptorSetCreationError::from)?;
        let set = builder.build().map_err(DescriptorSetCreationError::from)?;
        Ok(Arc::new(set))
    }

    /// Registers new user texture to be drawn in UI.
    pub fn register_texture(
        &mut self,
        image_view: Arc<dyn ImageViewAbstract + Send + Sync>,
    ) -> Result<TextureId, DescriptorSetCreationError> {
        let descriptor_set = self.image_descriptor_set(image_view)?;
        let key = self.user_texture_descriptor_sets.insert(descriptor_set);
        let id = key.data().as_ffi();
        Ok(TextureId::User(id))
    }

    /// Unregisters previously registered user texture to be drawn in UI.
    pub fn unregister_texture(&mut self, texture_id: TextureId) {
        if let TextureId::User(id) = texture_id {
            let key_data = KeyData::from_ffi(id);
            let key = DefaultKey::from(key_data);
            self.user_texture_descriptor_sets.remove(key);
        }
    }

    /// Builds a secondary command buffer that draws UI on the current subpass.
    pub fn draw(
        &mut self,
        viewport_size: Size,
        scale_factor: f32,
        meshes: Vec<ClippedMesh>,
        texture: Arc<Texture>,
    ) -> Result<SecondaryAutoCommandBuffer, UiDrawError> {
        use crate::graphics::shader::ui::vertex;

        let mut builder = AutoCommandBufferBuilder::secondary_graphics(
            self.graphics_queue.device().clone(),
            self.graphics_queue.family(),
            CommandBufferUsage::OneTimeSubmit,
            self.pipeline.subpass().clone(),
        )?;

        if texture.version != self.texture_version {
            self.texture_version = texture.version;
            let image = {
                let dimensions = ImageDimensions::Dim2d {
                    width: texture.width as u32,
                    height: texture.height as u32,
                    array_layers: 1,
                };
                let data: Vec<_> = texture.pixels.iter().flat_map(|&r| [r, r, r, r]).collect();

                let (image, image_future) = ImmutableImage::from_iter(
                    data.into_iter(),
                    dimensions,
                    MipmapsCount::One,
                    Format::R8G8B8A8_UNORM,
                    self.graphics_queue.clone(),
                )?;
                image_future.flush()?;
                image
            };

            let image = ImageView::new(image)?;
            let set = self.image_descriptor_set(image)?;
            self.texture_descriptor_set = Some(set);
        }

        let width = viewport_size.width as f32;
        let height = viewport_size.height as f32;
        let push_constants = vertex::ty::PushConstants {
            screen_size: [width / scale_factor, height / scale_factor],
        };

        for ClippedMesh(rect, mesh) in meshes {
            // Nothing to draw if we don't have vertices & indices
            if mesh.vertices.is_empty() || mesh.indices.is_empty() {
                continue;
            }
            let scissor = {
                let min = rect.min;
                let min = Pos2 {
                    x: min.x * scale_factor,
                    y: min.y * scale_factor,
                };
                let min = Pos2 {
                    x: min.x.clamp(0.0, width),
                    y: min.y.clamp(0.0, height),
                };
                let max = rect.max;
                let max = Pos2 {
                    x: max.x * scale_factor,
                    y: max.y * scale_factor,
                };
                let max = Pos2 {
                    x: max.x.clamp(min.x, width),
                    y: max.y.clamp(min.y, height),
                };
                Scissor {
                    origin: [min.x.round() as u32, min.y.round() as u32],
                    dimensions: [
                        (max.x.round() - min.x) as u32,
                        (max.y.round() - min.y) as u32,
                    ],
                }
            };

            let chunk = mesh.vertices.into_iter().map(UiVertex::from);
            let vertex_buffer = self.vertex_buffer.chunk(chunk)?;

            let chunk = mesh.indices.into_iter();
            let index_buffer = self.index_buffer.chunk(chunk)?;

            let viewport = Viewport {
                origin: [0.0, 0.0],
                dimensions: [viewport_size.width as f32, viewport_size.height as f32],
                depth_range: 0.0..1.0,
            };
            let descriptor_sets = match mesh.texture_id {
                TextureId::Egui => self.texture_descriptor_set.as_ref().unwrap().clone(),
                TextureId::User(id) => {
                    let key_data = KeyData::from_ffi(id);
                    let key = DefaultKey::from(key_data);
                    self.user_texture_descriptor_sets
                        .get(key)
                        .expect("User texture was unregistered, but still in use!")
                        .clone()
                }
            };
            builder
                .set_viewport(0, std::iter::once(viewport))
                .set_scissor(0, std::iter::once(scissor))
                .bind_pipeline_graphics(self.pipeline.clone())
                .bind_vertex_buffers(0, vertex_buffer)
                .bind_index_buffer(index_buffer.clone())
                .bind_descriptor_sets(
                    PipelineBindPoint::Graphics,
                    self.pipeline.layout().clone(),
                    0,
                    descriptor_sets,
                )
                .push_constants(self.pipeline.layout().clone(), 0, push_constants)
                .draw_indexed(index_buffer.len() as u32, 1, 0, 0, 0)?;
        }

        Ok(builder.build()?)
    }
}
