//! The GPU side: a surface, two pipelines and the glyph atlas.
//!
//! Both pipelines draw the same instanced unit quad — one filled with a solid
//! colour, one sampled from the atlas — so a frame is two draw calls however
//! much text is on it.

use crate::quads::{Circle, Frame, Panel, Quad, Rect, Sprite};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::sync::Arc;

pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    rect_pipeline: wgpu::RenderPipeline,
    wave_pipeline: wgpu::RenderPipeline,
    quad_pipeline: wgpu::RenderPipeline,
    sprite_pipeline: wgpu::RenderPipeline,
    circle_pipeline: wgpu::RenderPipeline,
    /// The cards: one that samples the blurred backdrop for what shows
    /// through them, and one for when nothing was blurred.
    panel_pipeline: wgpu::RenderPipeline,
    panel_plain_pipeline: wgpu::RenderPipeline,
    screen: wgpu::Buffer,
    screen_group: wgpu::BindGroup,
    atlas_layout: wgpu::BindGroupLayout,
    atlas: Option<AtlasTexture>,
    /// The two targets the backdrop is drawn into and blurred between, and
    /// what to sample them with. Made on first use and remade on a resize:
    /// a window that never opens a popup never pays for them.
    blur: Option<Blur>,
    blur_pipeline: wgpu::RenderPipeline,
    blit_pipeline: wgpu::RenderPipeline,
    /// The uniform each blur pass reads its direction and spread from. Two,
    /// because both passes are recorded before either runs and a buffer
    /// written twice would have the same value in it both times.
    blur_across: (wgpu::Buffer, wgpu::BindGroup),
    blur_down: (wgpu::Buffer, wgpu::BindGroup),
    sample_layout: wgpu::BindGroupLayout,
    rects: Instances,
    waves: Instances,
    quads: Instances,
    sprites: Instances,
    circles: Instances,
    /// The pictures the buffers stand in for, each a texture of its own
    /// rather than a place in the atlas — it would fill the atlas, and it
    /// is drawn whole. Keyed by whatever the caller keys them by.
    pictures: HashMap<u64, AtlasTexture>,
    /// One sprite per picture drawn this frame, each drawn on its own
    /// because each samples a different texture.
    picture_sprites: Instances,
    /// The cards over everything, and what is written on them.
    panels: Instances,
    over: Instances,
    over_sprites: Instances,
    /// What the window is cleared to before anything is drawn.
    pub background: [f32; 4],
    /// Where the grid's top left corner is, in pixels from the window's:
    /// the padding, when there is any. Everything given to the shaders is
    /// measured from here.
    pub origin: [f32; 2],
}

/// A pair of offscreen targets, ping-ponged between by the two blur passes.
struct Blur {
    width: u32,
    height: u32,
    /// The backdrop is drawn into the first and ends up back in it, so the
    /// first is always the one to sample when compositing.
    targets: [BlurTarget; 2],
}

struct BlurTarget {
    view: wgpu::TextureView,
    group: wgpu::BindGroup,
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

/// The limits to ask a device for, given what the adapter reports.
///
/// The downlevel defaults are the right ask for everything this draws — two
/// pipelines, one texture and a great many rectangles — so a laptop's
/// integrated GPU is as good as anything else here. What they are not is a
/// statement about how big the window may be: they cap a texture at 2048
/// pixels either way, and the surface is a texture. A window filling a 4K
/// display is 3816 across, `Surface::configure` refused it, and the editor
/// panicked before it had drawn a frame — on the one machine where it was
/// most obviously wanted. So the resolution comes from the adapter, and
/// nothing else does.
fn limits(adapter: &wgpu::Limits) -> wgpu::Limits {
    wgpu::Limits::downlevel_defaults().using_resolution(adapter.clone())
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
                required_limits: limits(&adapter.limits()),
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
                // The fragment stage too: a card reads the window's size
                // to find its own pixels in the blurred backdrop.
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
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

        // What a blur pass reads its direction and spread from, and what
        // both blur passes and the final composite sample a target with.
        let blur_uniform_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("blur"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let sample_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sample"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
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
        let blur_buffer = |label: &str| {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: 16,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &blur_uniform_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                }],
            });
            (buffer, group)
        };
        let blur_across = blur_buffer("blur across");
        let blur_down = blur_buffer("blur down");

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
        let wave_pipeline = pipeline(
            &device,
            &shader,
            &[Some(&screen_layout)],
            "wave_vertex",
            "wave_fragment",
            &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4],
            std::mem::size_of::<Rect>() as u64,
            format,
            blend,
        );
        let quad_pipeline = pipeline(
            &device,
            &shader,
            &[Some(&screen_layout)],
            "quad_vertex",
            "rect_fragment",
            &wgpu::vertex_attr_array![
                0 => Float32x2, 1 => Float32x2, 2 => Float32x2, 3 => Float32x2, 4 => Float32x4
            ],
            std::mem::size_of::<Quad>() as u64,
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

        let circle_pipeline = pipeline(
            &device,
            &shader,
            &[Some(&screen_layout)],
            "circle_vertex",
            "circle_fragment",
            &wgpu::vertex_attr_array![
                0 => Float32x2, 1 => Float32, 2 => Float32, 3 => Float32x4
            ],
            std::mem::size_of::<Circle>() as u64,
            format,
            blend,
        );

        let panel_attributes = wgpu::vertex_attr_array![
            0 => Float32x2, 1 => Float32x2, 2 => Float32x2, 3 => Float32x4, 4 => Float32x4
        ];
        let panel_pipeline = pipeline(
            &device,
            &shader,
            &[Some(&screen_layout), Some(&sample_layout)],
            "panel_vertex",
            "panel_fragment",
            &panel_attributes,
            std::mem::size_of::<Panel>() as u64,
            format,
            blend,
        );
        let panel_plain_pipeline = pipeline(
            &device,
            &shader,
            &[Some(&screen_layout)],
            "panel_vertex",
            "panel_plain_fragment",
            &panel_attributes,
            std::mem::size_of::<Panel>() as u64,
            format,
            blend,
        );

        // Both take no vertex buffer: the quad is generated from the vertex
        // index, because a pass over the whole target has nothing to say
        // per instance.
        let full_screen = |vertex: &str, fragment: &str, blend| {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(fragment),
                bind_group_layouts: &[Some(&blur_uniform_layout), Some(&sample_layout)],
                immediate_size: 0,
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(fragment),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vertex),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
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
        };
        let blur_pipeline = full_screen("blit_vertex", "blur_fragment", None);
        let blit_pipeline = full_screen("blit_vertex", "blit_fragment", None);

        let rects = Instances::new(&device, "rects", std::mem::size_of::<Rect>());
        let waves = Instances::new(&device, "waves", std::mem::size_of::<Rect>());
        let quads = Instances::new(&device, "quads", std::mem::size_of::<Quad>());
        let sprites = Instances::new(&device, "sprites", std::mem::size_of::<Sprite>());
        let circles = Instances::new(&device, "circles", std::mem::size_of::<Circle>());
        let picture_sprites = Instances::new(&device, "pictures", std::mem::size_of::<Sprite>());
        let panels = Instances::new(&device, "panels", std::mem::size_of::<Panel>());
        let over = Instances::new(&device, "over", std::mem::size_of::<Rect>());
        let over_sprites = Instances::new(&device, "over sprites", std::mem::size_of::<Sprite>());
        Ok(Renderer {
            surface,
            device,
            queue,
            config,
            rect_pipeline,
            wave_pipeline,
            quad_pipeline,
            sprite_pipeline,
            circle_pipeline,
            panel_pipeline,
            panel_plain_pipeline,
            screen,
            screen_group,
            atlas_layout,
            atlas: None,
            blur: None,
            blur_pipeline,
            blit_pipeline,
            blur_across,
            blur_down,
            sample_layout,
            rects,
            waves,
            quads,
            sprites,
            circles,
            pictures: HashMap::new(),
            picture_sprites,
            panels,
            over,
            over_sprites,
            background,
            origin: [0.0, 0.0],
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
        // They are the size of the window, and the window is not that size
        // any more.
        self.blur = None;
    }

    /// The largest texture the device will take, in pixels a side.
    pub fn max_texture_dimension(&self) -> u32 {
        self.device.limits().max_texture_dimension_2d
    }

    /// Uploads the glyph atlas, growing the texture if it has changed size.
    pub fn upload_atlas(&mut self, width: u32, height: u32, pixels: &[u8]) {
        let fresh = match &self.atlas {
            Some(atlas) => atlas.width != width || atlas.height != height,
            None => true,
        };
        if fresh {
            self.atlas = Some(self.texture("atlas", width, height));
        }
        let atlas = self.atlas.as_ref().expect("just made");
        Renderer::write_texture(&self.queue, atlas, pixels);
    }

    /// Whether a picture is held under `key`, at `width` by `height`.
    pub fn has_picture(&self, key: u64, width: u32, height: u32) -> bool {
        self.pictures
            .get(&key)
            .is_some_and(|held| held.width == width && held.height == height)
    }

    /// Keeps `pixels` as the picture under `key`, replacing whatever was
    /// there. A picture is a texture of its own, so the atlas — sized for
    /// glyphs — is not asked to find room for a photograph.
    pub fn upload_picture(&mut self, key: u64, width: u32, height: u32, pixels: &[u8]) {
        let texture = self.texture("picture", width, height);
        Renderer::write_texture(&self.queue, &texture, pixels);
        self.pictures.insert(key, texture);
    }

    /// Lets go of every picture but those `keep` says to.
    pub fn retain_pictures(&mut self, keep: impl Fn(u64) -> bool) {
        self.pictures.retain(|key, _| keep(*key));
    }

    /// An RGBA texture, sampled linearly, bound the way the sprite shader
    /// wants it.
    fn texture(&self, label: &str, width: u32, height: u32) -> AtlasTexture {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some(label),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
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
        AtlasTexture {
            group,
            texture,
            width,
            height,
        }
    }

    fn write_texture(queue: &wgpu::Queue, target: &AtlasTexture, pixels: &[u8]) {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &target.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(target.width * 4),
                rows_per_image: Some(target.height),
            },
            wgpu::Extent3d {
                width: target.width,
                height: target.height,
                depth_or_array_layers: 1,
            },
        );
    }

    /// The sprites the frame's pictures are drawn as: each its whole
    /// texture, in white so the shader leaves its colours alone. Only the
    /// pictures that are held are drawn; one asked for before it was
    /// uploaded is skipped rather than drawn as something else.
    fn picture_sprites(&self, frame: &Frame) -> Vec<(Sprite, u64)> {
        frame
            .pictures
            .iter()
            .filter_map(|picture| {
                let held = self.pictures.get(&picture.key)?;
                Some((
                    Sprite {
                        position: picture.position,
                        size: picture.size,
                        source: [0.0, 0.0],
                        source_size: [held.width as f32, held.height as f32],
                        color: [1.0, 1.0, 1.0, 1.0],
                    },
                    picture.key,
                ))
            })
            .collect()
    }

    /// Draws the pictures written by [`Renderer::write_pictures`], one draw
    /// each because each binds its own texture.
    fn draw_pictures<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, keys: &[u64]) {
        if keys.is_empty() {
            return;
        }
        pass.set_pipeline(&self.sprite_pipeline);
        pass.set_bind_group(0, &self.screen_group, &[]);
        pass.set_vertex_buffer(0, self.picture_sprites.buffer.slice(..));
        for (index, key) in keys.iter().enumerate() {
            let Some(held) = self.pictures.get(key) else {
                continue;
            };
            pass.set_bind_group(1, &held.group, &[]);
            let index = index as u32;
            pass.draw(0..4, index..index + 1);
        }
    }

    /// Writes the frame's pictures into their instance buffer, and says
    /// which textures they are, in order.
    fn write_pictures(&mut self, frame: &Frame) -> Vec<u64> {
        let sprites = self.picture_sprites(frame);
        let (sprites, keys): (Vec<Sprite>, Vec<u64>) = sprites.into_iter().unzip();
        self.picture_sprites
            .write(&self.device, &self.queue, "pictures", &sprites);
        keys
    }

    /// Makes the pair of offscreen targets, if they are not already the
    /// size of the window.
    fn ready_the_blur(&mut self) {
        let (width, height) = (self.config.width.max(1), self.config.height.max(1));
        if let Some(blur) = &self.blur
            && blur.width == width
            && blur.height == height
        {
            return;
        }
        let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("blur"),
            // Clamped, so a tap that reaches past the edge reads the edge
            // rather than wrapping the other side of the screen into it.
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let mut targets = Vec::new();
        for n in 0..2 {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("blur"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.config.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("blur"),
                layout: &self.sample_layout,
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
            let _ = n;
            targets.push(BlurTarget { view, group });
        }
        let [first, second] = <[BlurTarget; 2]>::try_from(targets).ok().expect("two made");
        self.blur = Some(Blur {
            width,
            height,
            targets: [first, second],
        });
    }

    /// Draws one frame. A surface that has gone stale — the window was
    /// resized, the compositor took it back — is reconfigured and skipped
    /// rather than treated as an error.
    pub fn draw(&mut self, frame: &Frame) -> Result<()> {
        self.draw_over(frame, None, &[], 0.0)
    }

    /// Draws `frame`, over `backdrop` blurred within `areas`.
    ///
    /// The backdrop is the same frame without the things floating over it,
    /// drawn only near where they are. It goes into an offscreen target,
    /// is blurred across and then down, and what comes out is painted into
    /// the popups' rectangles before the frame itself goes on top — whose
    /// popup backgrounds are drawn short of solid, so it shows through.
    ///
    /// A card in the frame reads the blurred backdrop for itself, and so
    /// asks for the blur without an area to paint it into.
    ///
    /// With no backdrop this is one pass, exactly as it was.
    pub fn draw_over(
        &mut self,
        frame: &Frame,
        backdrop: Option<&Frame>,
        areas: &[[f32; 4]],
        radius: f32,
    ) -> Result<()> {
        if self.atlas.is_none() {
            return Ok(());
        }
        self.queue.write_buffer(
            &self.screen,
            0,
            bytemuck::cast_slice(&[
                self.config.width as f32,
                self.config.height as f32,
                self.origin[0],
                self.origin[1],
            ]),
        );

        // The backdrop, blurred, in a submission of its own. It has to be
        // its own submission because both passes read the same instance
        // buffers, and a buffer written twice before either runs would hold
        // the second frame's contents in both of them.
        let wanted = !areas.is_empty() || !frame.panels.is_empty();
        let blurred = match backdrop.filter(|_| wanted && radius > 0.0) {
            Some(backdrop) => {
                self.ready_the_blur();
                self.blur_backdrop(backdrop, radius);
                true
            }
            None => false,
        };

        self.rects
            .write(&self.device, &self.queue, "rects", &frame.rects);
        self.waves
            .write(&self.device, &self.queue, "waves", &frame.waves);
        self.quads
            .write(&self.device, &self.queue, "quads", &frame.quads);
        self.sprites
            .write(&self.device, &self.queue, "sprites", &frame.sprites);
        self.circles
            .write(&self.device, &self.queue, "circles", &frame.circles);
        let pictures = self.write_pictures(frame);
        self.panels
            .write(&self.device, &self.queue, "panels", &frame.panels);
        self.over
            .write(&self.device, &self.queue, "over", &frame.over);
        self.over_sprites.write(
            &self.device,
            &self.queue,
            "over sprites",
            &frame.over_sprites,
        );

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
        let atlas = self.atlas.as_ref().expect("checked above");
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
            // What is behind the popups, blurred, before anything else —
            // and only inside them, which is the whole reason the frame is
            // drawn twice. Scissored rather than clipped in the shader
            // because a rectangle is what the hardware already does.
            if blurred && let Some(blur) = self.blur.as_ref() {
                pass.set_pipeline(&self.blit_pipeline);
                pass.set_bind_group(0, &self.blur_across.1, &[]);
                pass.set_bind_group(1, &blur.targets[0].group, &[]);
                for area in areas {
                    let x = area[0].max(0.0) as u32;
                    let y = area[1].max(0.0) as u32;
                    let width = (area[2].max(0.0) as u32).min(blur.width.saturating_sub(x));
                    let height = (area[3].max(0.0) as u32).min(blur.height.saturating_sub(y));
                    if width == 0 || height == 0 {
                        continue;
                    }
                    pass.set_scissor_rect(x, y, width, height);
                    pass.draw(0..4, 0..1);
                }
                pass.set_scissor_rect(0, 0, self.config.width, self.config.height);
            }
            // The windows' backgrounds, then the pictures, then the rest
            // of the rectangles — a popup's background among them — so a
            // picture is over the window it is in and under what floats.
            let under = (frame.under as u32).min(self.rects.count);
            if self.rects.count > 0 {
                pass.set_pipeline(&self.rect_pipeline);
                pass.set_bind_group(0, &self.screen_group, &[]);
                pass.set_vertex_buffer(0, self.rects.buffer.slice(..));
                pass.draw(0..4, 0..under);
            }
            self.draw_pictures(&mut pass, &pictures);
            if self.rects.count > under {
                pass.set_pipeline(&self.rect_pipeline);
                pass.set_bind_group(0, &self.screen_group, &[]);
                pass.set_vertex_buffer(0, self.rects.buffer.slice(..));
                pass.draw(0..4, under..self.rects.count);
            }
            // The waves under words, like the rules: over the backgrounds,
            // under the cursor and the text.
            if self.waves.count > 0 {
                pass.set_pipeline(&self.wave_pipeline);
                pass.set_bind_group(0, &self.screen_group, &[]);
                pass.set_vertex_buffer(0, self.waves.buffer.slice(..));
                pass.draw(0..4, 0..self.waves.count);
            }
            // Between the two, so the cursor covers the backgrounds and the
            // text is drawn over the cursor rather than under it.
            if self.quads.count > 0 {
                pass.set_pipeline(&self.quad_pipeline);
                pass.set_bind_group(0, &self.screen_group, &[]);
                pass.set_vertex_buffer(0, self.quads.buffer.slice(..));
                pass.draw(0..4, 0..self.quads.count);
            }
            if self.sprites.count > 0 {
                pass.set_pipeline(&self.sprite_pipeline);
                pass.set_bind_group(0, &self.screen_group, &[]);
                pass.set_bind_group(1, &atlas.group, &[]);
                pass.set_vertex_buffer(0, self.sprites.buffer.slice(..));
                pass.draw(0..4, 0..self.sprites.count);
            }
            // Last, so an effect trailing away from the cursor is over the
            // text it is trailing away from rather than under it.
            if self.circles.count > 0 {
                pass.set_pipeline(&self.circle_pipeline);
                pass.set_bind_group(0, &self.screen_group, &[]);
                pass.set_vertex_buffer(0, self.circles.buffer.slice(..));
                pass.draw(0..4, 0..self.circles.count);
            }
            // The cards, over the lot, and what is written on them over
            // the cards. A card shows the blur through itself when there
            // is one, and is solid when there is not.
            if self.panels.count > 0 {
                match self.blur.as_ref().filter(|_| blurred) {
                    Some(blur) => {
                        pass.set_pipeline(&self.panel_pipeline);
                        pass.set_bind_group(0, &self.screen_group, &[]);
                        pass.set_bind_group(1, &blur.targets[0].group, &[]);
                    }
                    None => {
                        pass.set_pipeline(&self.panel_plain_pipeline);
                        pass.set_bind_group(0, &self.screen_group, &[]);
                    }
                }
                pass.set_vertex_buffer(0, self.panels.buffer.slice(..));
                pass.draw(0..4, 0..self.panels.count);
            }
            if self.over.count > 0 {
                pass.set_pipeline(&self.rect_pipeline);
                pass.set_bind_group(0, &self.screen_group, &[]);
                pass.set_vertex_buffer(0, self.over.buffer.slice(..));
                pass.draw(0..4, 0..self.over.count);
            }
            if self.over_sprites.count > 0 {
                pass.set_pipeline(&self.sprite_pipeline);
                pass.set_bind_group(0, &self.screen_group, &[]);
                pass.set_bind_group(1, &atlas.group, &[]);
                pass.set_vertex_buffer(0, self.over_sprites.buffer.slice(..));
                pass.draw(0..4, 0..self.over_sprites.count);
            }
        }
        self.queue.submit(Some(encoder.finish()));
        self.queue.present(target);
        Ok(())
    }
}

impl Renderer {
    /// Draws `backdrop` into the first offscreen target and blurs it, in
    /// its own submission.
    fn blur_backdrop(&mut self, backdrop: &Frame, radius: f32) {
        if self.atlas.is_none() || self.blur.is_none() {
            return;
        }
        let pictures = self.write_pictures(backdrop);
        let Some(atlas) = self.atlas.as_ref() else {
            return;
        };
        let Some(blur) = self.blur.as_ref() else {
            return;
        };
        let (width, height) = (blur.width as f32, blur.height as f32);
        // One pass across and one down, each told how far a texel is along
        // the axis it works on.
        self.queue.write_buffer(
            &self.blur_across.0,
            0,
            bytemuck::cast_slice(&[1.0 / width, 0.0, radius, 0.0]),
        );
        self.queue.write_buffer(
            &self.blur_down.0,
            0,
            bytemuck::cast_slice(&[0.0, 1.0 / height, radius, 0.0]),
        );
        self.rects
            .write(&self.device, &self.queue, "backdrop", &backdrop.rects);
        self.sprites
            .write(&self.device, &self.queue, "backdrop", &backdrop.sprites);

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("blur"),
            });
        let clear = wgpu::Color {
            r: self.background[0] as f64,
            g: self.background[1] as f64,
            b: self.background[2] as f64,
            a: 1.0,
        };
        {
            let mut pass = onto(&mut encoder, &blur.targets[0].view, clear);
            if self.rects.count > 0 {
                pass.set_pipeline(&self.rect_pipeline);
                pass.set_bind_group(0, &self.screen_group, &[]);
                pass.set_vertex_buffer(0, self.rects.buffer.slice(..));
                pass.draw(0..4, 0..self.rects.count);
            }
            self.draw_pictures(&mut pass, &pictures);
            if self.sprites.count > 0 {
                pass.set_pipeline(&self.sprite_pipeline);
                pass.set_bind_group(0, &self.screen_group, &[]);
                pass.set_bind_group(1, &atlas.group, &[]);
                pass.set_vertex_buffer(0, self.sprites.buffer.slice(..));
                pass.draw(0..4, 0..self.sprites.count);
            }
        }
        // Across into the second, then back down into the first, so what
        // the composite samples is always the first.
        for (uniform, from, to) in [
            (&self.blur_across.1, 0usize, 1usize),
            (&self.blur_down.1, 1, 0),
        ] {
            let mut pass = onto(&mut encoder, &blur.targets[to].view, clear);
            pass.set_pipeline(&self.blur_pipeline);
            pass.set_bind_group(0, uniform, &[]);
            pass.set_bind_group(1, &blur.targets[from].group, &[]);
            pass.draw(0..4, 0..1);
        }
        self.queue.submit(Some(encoder.finish()));
    }
}

/// A pass onto one offscreen target, cleared first.
///
/// A function rather than a closure because the pass borrows the encoder
/// and a closure cannot say so.
fn onto<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    view: &'a wgpu::TextureView,
    clear: wgpu::Color,
) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("blur"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(clear),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    })
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
    fn a_surface_may_be_as_wide_as_the_display_the_window_is_on() {
        // The bug this is here for: the ask was the downlevel defaults
        // whole, so a window filling a 4K display asked for a 3816-pixel
        // surface against a 2048-pixel cap and the editor died configuring
        // it. Nothing had been drawn yet, so all it left was a wgpu
        // validation error where a window should have been.
        let adapter = wgpu::Limits::default();
        let asked = limits(&adapter);
        assert_eq!(
            asked.max_texture_dimension_2d,
            adapter.max_texture_dimension_2d
        );
        assert!(
            asked.max_texture_dimension_2d >= 3840,
            "a 4K window will not fit in {} pixels",
            asked.max_texture_dimension_2d
        );
    }

    #[test]
    fn nothing_but_the_resolution_is_asked_for_beyond_the_downlevel_defaults() {
        // Raising the rest would turn away the modest GPU this is careful
        // to run on, and none of it is anything a rectangle needs.
        let downlevel = wgpu::Limits::downlevel_defaults();
        let asked = limits(&wgpu::Limits::default());
        assert_eq!(
            wgpu::Limits {
                max_texture_dimension_1d: downlevel.max_texture_dimension_1d,
                max_texture_dimension_2d: downlevel.max_texture_dimension_2d,
                max_texture_dimension_3d: downlevel.max_texture_dimension_3d,
                ..asked.clone()
            },
            downlevel
        );
    }

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
