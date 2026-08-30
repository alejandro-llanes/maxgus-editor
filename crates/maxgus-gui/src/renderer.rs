//! The GPU side: a surface, two pipelines and the glyph atlas.
//!
//! Both pipelines draw the same instanced unit quad — one filled with a solid
//! colour, one sampled from the atlas — so a frame is two draw calls however
//! much text is on it.

use crate::quads::{Frame, Rect, Sprite};
use anyhow::{Context, Result};
use std::sync::Arc;

pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    rect_pipeline: wgpu::RenderPipeline,
    sprite_pipeline: wgpu::RenderPipeline,
    screen: wgpu::Buffer,
    screen_group: wgpu::BindGroup,
    atlas_layout: wgpu::BindGroupLayout,
    atlas: Option<AtlasTexture>,
    rects: Instances,
    sprites: Instances,
    /// What the window is cleared to before anything is drawn.
    pub background: [f32; 4],
}

struct AtlasTexture {
    group: wgpu::BindGroup,
    texture: wgpu::Texture,
    width: u32,
    height: u32,
}

/// What a frame's worth of instances needs from the buffer holding them.
///
/// Pure, and apart from the device, because the arithmetic is where this went
/// wrong: a buffer was once allocated to fit the frame that grew it while the
/// capacity recorded beside it said `next_power_of_two`, and the first larger
/// frame after that wrote past the end of it. The invariant is one line —
/// the bytes allocated are the capacity's worth, not the frame's — and a
/// test can hold it without a GPU.
#[derive(Debug, PartialEq)]
enum Need {
    /// It fits: write into the buffer that is already there.
    Fits,
    /// A new buffer of `bytes`, which then holds `capacity` instances.
    Grow { capacity: usize, bytes: u64 },
}

fn need(capacity: usize, instances: usize, stride: usize) -> Need {
    if instances <= capacity {
        return Need::Fits;
    }
    // Doubling rather than fitting exactly: a window being resized would
    // otherwise reallocate on every frame of the drag.
    let capacity = instances.next_power_of_two();
    Need::Grow {
        capacity,
        bytes: (capacity * stride) as u64,
    }
}

/// An instance buffer that grows to fit and is rewritten each frame.
struct Instances {
    buffer: wgpu::Buffer,
    /// How many instances the buffer can hold — which is to say, its size in
    /// bytes divided by the stride, and never anything else.
    capacity: usize,
    stride: usize,
    count: u32,
}

impl Instances {
    fn new(device: &wgpu::Device, label: &str, stride: usize) -> Instances {
        let capacity = 4096;
        Instances {
            buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: (capacity * stride) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            capacity,
            stride,
            count: 0,
        }
    }

    fn write<T: bytemuck::Pod>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        label: &str,
        data: &[T],
    ) {
        self.count = data.len() as u32;
        if data.is_empty() {
            return;
        }
        if let Need::Grow { capacity, bytes } = need(self.capacity, data.len(), self.stride) {
            self.capacity = capacity;
            self.buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: bytes,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(data));
    }
}

impl Renderer {
    pub async fn new(window: Arc<winit::window::Window>, background: [f32; 4]) -> Result<Renderer> {
        let size = window.inner_size();
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(window)
            .context("this display has no surface the GPU can draw on")?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
                ..Default::default()
            })
            .await
            .context("no GPU adapter can draw on this window")?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("maxgus"),
                required_features: wgpu::Features::empty(),
                // The defaults every adapter meets, so a laptop's integrated
                // GPU is as good as anything else here: this draws rectangles.
                required_limits: wgpu::Limits::downlevel_defaults(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
                ..Default::default()
            })
            .await
            .context("the GPU refused a device")?;

        let capabilities = surface.get_capabilities(&adapter);
        let format = capabilities
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(capabilities.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            // Opaque where the surface offers it. Taking whichever mode
            // came first left the window's alpha up to the driver, and a
            // text editor that is faintly see-through is a text editor
            // nobody can read against a bright wallpaper.
            alpha_mode: match capabilities
                .alpha_modes
                .contains(&wgpu::CompositeAlphaMode::Opaque)
            {
                true => wgpu::CompositeAlphaMode::Opaque,
                false => capabilities.alpha_modes[0],
            },
            view_formats: Vec::new(),
            desired_maximum_frame_latency: 2,
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        surface.configure(&device, &config);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("maxgus"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        let screen = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("screen"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let screen_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("screen"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let screen_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("screen"),
            layout: &screen_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: screen.as_entire_binding(),
            }],
        });
        let atlas_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("atlas"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let blend = Some(wgpu::BlendState::ALPHA_BLENDING);
        let rect_pipeline = pipeline(
            &device,
            &shader,
            &[Some(&screen_layout)],
            "rect_vertex",
            "rect_fragment",
            &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4],
            std::mem::size_of::<Rect>() as u64,
            format,
            blend,
        );
        let sprite_pipeline = pipeline(
            &device,
            &shader,
            &[Some(&screen_layout), Some(&atlas_layout)],
            "sprite_vertex",
            "sprite_fragment",
            &wgpu::vertex_attr_array![
                0 => Float32x2, 1 => Float32x2, 2 => Float32x2, 3 => Float32x2, 4 => Float32x4
            ],
            std::mem::size_of::<Sprite>() as u64,
            format,
            blend,
        );

        let rects = Instances::new(&device, "rects", std::mem::size_of::<Rect>());
        let sprites = Instances::new(&device, "sprites", std::mem::size_of::<Sprite>());
        Ok(Renderer {
            surface,
            device,
            queue,
            config,
            rect_pipeline,
            sprite_pipeline,
            screen,
            screen_group,
            atlas_layout,
            atlas: None,
            rects,
            sprites,
            background,
        })
    }

    pub fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    /// Uploads the glyph atlas, growing the texture if it has changed size.
    pub fn upload_atlas(&mut self, width: u32, height: u32, pixels: &[u8]) {
        let fresh = match &self.atlas {
            Some(atlas) => atlas.width != width || atlas.height != height,
            None => true,
        };
        if fresh {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("atlas"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("atlas"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            });
            let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("atlas"),
                layout: &self.atlas_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                ],
            });
            self.atlas = Some(AtlasTexture {
                group,
                texture,
                width,
                height,
            });
        }
        let atlas = self.atlas.as_ref().expect("just made");
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &atlas.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
    }

    /// Draws one frame. A surface that has gone stale — the window was
    /// resized, the compositor took it back — is reconfigured and skipped
    /// rather than treated as an error.
    pub fn draw(&mut self, frame: &Frame) -> Result<()> {
        let Some(atlas) = self.atlas.as_ref() else {
            return Ok(());
        };
        self.queue.write_buffer(
            &self.screen,
            0,
            bytemuck::cast_slice(&[
                self.config.width as f32,
                self.config.height as f32,
                0.0,
                0.0,
            ]),
        );
        self.rects
            .write(&self.device, &self.queue, "rects", &frame.rects);
        self.sprites
            .write(&self.device, &self.queue, "sprites", &frame.sprites);

        use wgpu::CurrentSurfaceTexture as Acquired;
        let target = match self.surface.get_current_texture() {
            Acquired::Success(target) | Acquired::Suboptimal(target) => target,
            // The window changed under us, or is not on screen. Reconfigure
            // and let the next frame draw: a skipped frame is invisible, and
            // treating this as an error would close the editor over a resize.
            Acquired::Lost | Acquired::Outdated => {
                self.surface.configure(&self.device, &self.config);
                return Ok(());
            }
            Acquired::Timeout | Acquired::Occluded => return Ok(()),
            other => anyhow::bail!("the GPU would not give up a frame: {other:?}"),
        };
        let view = target
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("frame"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: self.background[0] as f64,
                            g: self.background[1] as f64,
                            b: self.background[2] as f64,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if self.rects.count > 0 {
                pass.set_pipeline(&self.rect_pipeline);
                pass.set_bind_group(0, &self.screen_group, &[]);
                pass.set_vertex_buffer(0, self.rects.buffer.slice(..));
                pass.draw(0..4, 0..self.rects.count);
            }
            if self.sprites.count > 0 {
                pass.set_pipeline(&self.sprite_pipeline);
                pass.set_bind_group(0, &self.screen_group, &[]);
                pass.set_bind_group(1, &atlas.group, &[]);
                pass.set_vertex_buffer(0, self.sprites.buffer.slice(..));
                pass.draw(0..4, 0..self.sprites.count);
            }
        }
        self.queue.submit(Some(encoder.finish()));
        self.queue.present(target);
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
fn pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layouts: &[Option<&wgpu::BindGroupLayout>],
    vertex: &str,
    fragment: &str,
    attributes: &[wgpu::VertexAttribute],
    stride: u64,
    format: wgpu::TextureFormat,
    blend: Option<wgpu::BlendState>,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(vertex),
        bind_group_layouts: layouts,
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(vertex),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(vertex),
            compilation_options: Default::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: stride,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes,
            })],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fragment),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const STRIDE: usize = std::mem::size_of::<Rect>();

    #[test]
    fn a_frame_that_fits_reuses_the_buffer() {
        assert_eq!(need(4096, 4096, STRIDE), Need::Fits);
        assert_eq!(need(4096, 1, STRIDE), Need::Fits);
    }

    #[test]
    fn a_grown_buffer_is_as_big_as_the_capacity_beside_it_claims() {
        // The bug this is here for: the buffer was allocated to fit the
        // frame that grew it, while the capacity recorded beside it was
        // rounded up. Every later frame between the two wrote off the end.
        for instances in [4097, 5000, 8191, 100_000] {
            let Need::Grow { capacity, bytes } = need(4096, instances, STRIDE) else {
                panic!("{instances} instances should not fit in 4096");
            };
            assert!(capacity >= instances, "{capacity} cannot hold {instances}");
            assert_eq!(
                bytes,
                (capacity * STRIDE) as u64,
                "a buffer of {bytes} bytes does not hold {capacity} instances"
            );
        }
    }

    #[test]
    fn growing_leaves_room_rather_than_growing_again_next_frame() {
        let Need::Grow { capacity, .. } = need(4096, 4097, STRIDE) else {
            panic!("it should have grown");
        };
        assert_eq!(capacity, 8192);
        // And the frame after, one instance larger again, now fits.
        assert_eq!(need(capacity, 4098, STRIDE), Need::Fits);
    }
}
