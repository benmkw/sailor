use crossbeam_channel::{unbounded, TryRecvError};
use nalgebra_glm::{vec2, vec4};
use notify::{event::ModifyKind, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use osm::*;
use pollster::block_on;
use wgpu::util::{BufferInitDescriptor, DeviceExt};
use wgpu::*;
use wgpu_glyph::{ab_glyph::FontArc, GlyphBrush, GlyphBrushBuilder};
use winit::{dpi::LogicalSize, event_loop::EventLoop, window::Window};

use crate::app_state::AppState;
use crate::drawing::helpers::{load_glsl, ShaderStage};

use crate::config::CONFIG;

pub struct Painter {
    pub window: Window,
    hidpi_factor: f64,
    pub device: Device,
    pub queue: Queue,
    surface: Surface,
    swap_chain_descriptor: SwapChainDescriptor,
    swap_chain: SwapChain,
    blend_pipeline: RenderPipeline,
    noblend_pipeline: RenderPipeline,
    multisampled_framebuffer: TextureView,
    stencil: TextureView,
    uniform_buffer: Buffer,
    tile_transform_buffer: (Buffer, u64),
    bind_group_layout: BindGroupLayout,
    bind_group: BindGroup,
    rx: crossbeam_channel::Receiver<std::result::Result<notify::event::Event, notify::Error>>,
    _watcher: RecommendedWatcher,
    glyph_brush: GlyphBrush<()>,
    staging_belt: wgpu::util::StagingBelt,
    temperature: crate::drawing::weather::Temperature,
}

impl Painter {
    /// Initializes the entire draw machinery.
    pub fn init(event_loop: &EventLoop<()>, width: u32, height: u32, app_state: &AppState) -> Self {
        let window = Window::new(&event_loop).unwrap();
        window.set_inner_size(LogicalSize {
            width: width as f64,
            height: height as f64,
        });
        let factor = window.scale_factor();
        let size = window.inner_size();

        let instance = wgpu::Instance::new(wgpu::BackendBit::PRIMARY);
        let surface = unsafe { instance.create_surface(&window) };

        let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::Default,
            // Request an adapter which can render to our surface
            compatible_surface: Some(&surface),
        }))
        .expect("Failed to find an appropiate adapter");

        let (mut device, mut queue) = block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                features: wgpu::Features::empty(),
                limits: wgpu::Limits::default(),
                shader_validation: true,
            },
            None,
        ))
        .expect("Failed to create device");

        let init_encoder = device.create_command_encoder(&CommandEncoderDescriptor { label: None });

        let (tx, rx) = unbounded();

        let mut watcher: RecommendedWatcher =
            match Watcher::new_immediate(move |res| tx.send(res).unwrap()) {
                Ok(watcher) => watcher,
                Err(err) => {
                    log::info!("Failed to create a watcher for the vertex shader:");
                    log::info!("{}", err);
                    panic!("Unable to load a vertex shader.");
                }
            };

        match watcher.watch(&CONFIG.renderer.vertex_shader, RecursiveMode::Recursive) {
            Ok(_) => {}
            Err(err) => {
                log::info!(
                    "Failed to start watching {}:",
                    &CONFIG.renderer.vertex_shader
                );
                log::info!("{}", err);
            }
        };

        match watcher.watch(&CONFIG.renderer.fragment_shader, RecursiveMode::Recursive) {
            Ok(_) => {}
            Err(err) => {
                log::info!(
                    "Failed to start watching {}:",
                    &CONFIG.renderer.fragment_shader
                );
                log::info!("{}", err);
            }
        };

        let (layer_vs_module, layer_fs_module) = Self::load_shader(
            &device,
            &CONFIG.renderer.vertex_shader,
            &CONFIG.renderer.fragment_shader,
        )
        .expect("Fatal Error. Unable to load shaders.");

        let bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStage::VERTEX,
                    ty: BindingType::UniformBuffer {
                        dynamic: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStage::VERTEX,
                    ty: BindingType::UniformBuffer {
                        dynamic: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let swap_chain_descriptor = SwapChainDescriptor {
            usage: TextureUsage::OUTPUT_ATTACHMENT,
            format: TextureFormat::Bgra8Unorm,
            width: size.width,
            height: size.height,
            present_mode: PresentMode::Immediate,
        };

        let multisampled_framebuffer = Self::create_multisampled_framebuffer(
            &device,
            &swap_chain_descriptor,
            CONFIG.renderer.msaa_samples,
        );
        let stencil = Self::create_stencil(&device, &swap_chain_descriptor);

        let uniform_buffer = Self::create_uniform_buffer(&device);
        let tile_transform_buffer = Self::create_tile_transform_buffer(
            &device,
            &app_state.screen,
            app_state.zoom,
            std::iter::empty::<&VisibleTile>(),
        );

        let blend_pipeline = Self::create_layer_render_pipeline(
            &device,
            &bind_group_layout,
            &layer_vs_module,
            &layer_fs_module,
            BlendDescriptor {
                src_factor: BlendFactor::SrcAlpha,
                dst_factor: BlendFactor::OneMinusSrcAlpha,
                operation: BlendOperation::Add,
            },
            BlendDescriptor {
                src_factor: BlendFactor::One,
                dst_factor: BlendFactor::OneMinusSrcAlpha,
                operation: BlendOperation::Add,
            },
            false,
        );

        let noblend_pipeline = Self::create_layer_render_pipeline(
            &device,
            &bind_group_layout,
            &layer_vs_module,
            &layer_fs_module,
            BlendDescriptor::REPLACE,
            BlendDescriptor::REPLACE,
            true,
        );

        let swap_chain = device.create_swap_chain(&surface, &swap_chain_descriptor);

        let bind_group = Self::create_blend_bind_group(
            &device,
            &bind_group_layout,
            &uniform_buffer,
            &tile_transform_buffer,
        );

        let font =
            FontArc::try_from_slice(include_bytes!("../../../config/Ruda-Bold.ttf")).unwrap();

        let glyph_brush =
            GlyphBrushBuilder::using_font(font).build(&mut device, TextureFormat::Bgra8Unorm);
        let staging_belt = wgpu::util::StagingBelt::new(1024);

        let mut temperature = crate::drawing::weather::Temperature::init(&mut device, &mut queue);

        let init_command_buf = init_encoder.finish();
        queue.submit(vec![init_command_buf]); // TODO this fix is bad

        let width = 64 * 8;
        let height = 64 * 8;

        temperature.generate_texture(&mut device, &mut queue, width, height);

        Self {
            window,
            hidpi_factor: factor,
            device,
            queue,
            surface,
            swap_chain_descriptor,
            swap_chain,
            blend_pipeline,
            noblend_pipeline,
            multisampled_framebuffer,
            uniform_buffer,
            stencil,
            tile_transform_buffer,
            bind_group_layout,
            bind_group,
            _watcher: watcher,
            rx,
            glyph_brush,
            staging_belt,
            temperature,
        }
    }

    fn create_layer_render_pipeline(
        device: &Device,
        bind_group_layout: &BindGroupLayout,
        vs_module: &ShaderModule,
        fs_module: &ShaderModule,
        color_blend: BlendDescriptor,
        alpha_blend: BlendDescriptor,
        depth_write_enabled: bool,
    ) -> RenderPipeline {
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        device.create_render_pipeline(&RenderPipelineDescriptor {
            label: None,
            layout: Some(&pipeline_layout),
            vertex_stage: ProgrammableStageDescriptor {
                module: &vs_module,
                entry_point: "main",
            },
            fragment_stage: Some(ProgrammableStageDescriptor {
                module: &fs_module,
                entry_point: "main",
            }),
            rasterization_state: Some(RasterizationStateDescriptor {
                front_face: FrontFace::Ccw,
                cull_mode: CullMode::None,
                depth_bias: 0,
                depth_bias_slope_scale: 0.0,
                depth_bias_clamp: 0.0,
                clamp_depth: false, // inactive because not enabled in extensions
            }),
            primitive_topology: PrimitiveTopology::TriangleList,
            color_states: &[ColorStateDescriptor {
                format: TextureFormat::Bgra8Unorm,
                color_blend,
                alpha_blend,
                write_mask: ColorWrite::ALL,
            }],
            depth_stencil_state: Some(DepthStencilStateDescriptor {
                format: TextureFormat::Depth24PlusStencil8,
                depth_write_enabled,
                depth_compare: CompareFunction::Greater,
                stencil: wgpu::StencilStateDescriptor {
                    front: StencilStateFaceDescriptor {
                        compare: CompareFunction::NotEqual,
                        fail_op: StencilOperation::Keep,
                        depth_fail_op: StencilOperation::Replace,
                        pass_op: StencilOperation::Replace,
                    },
                    back: StencilStateFaceDescriptor {
                        compare: CompareFunction::NotEqual,
                        fail_op: StencilOperation::Keep,
                        depth_fail_op: StencilOperation::Replace,
                        pass_op: StencilOperation::Replace,
                    },
                    read_mask: std::u32::MAX,
                    write_mask: std::u32::MAX,
                },
            }),
            vertex_state: VertexStateDescriptor {
                index_format: IndexFormat::Uint32,
                vertex_buffers: &[VertexBufferDescriptor {
                    stride: std::mem::size_of::<Vertex>() as BufferAddress,
                    step_mode: InputStepMode::Vertex,
                    attributes: &[
                        VertexAttributeDescriptor {
                            format: VertexFormat::Short2,
                            offset: 0,
                            shader_location: 0,
                        },
                        VertexAttributeDescriptor {
                            format: VertexFormat::Short2,
                            offset: 4,
                            shader_location: 1,
                        },
                        VertexAttributeDescriptor {
                            format: VertexFormat::Uint,
                            offset: 8,
                            shader_location: 2,
                        },
                    ],
                }],
            },
            sample_count: CONFIG.renderer.msaa_samples,
            sample_mask: !0,
            alpha_to_coverage_enabled: false,
        })
    }

    /// Creates a new bind group containing all the relevant uniform buffers.
    fn create_uniform_buffers(
        device: &Device,
        screen: &Screen,
        feature_collection: &FeatureCollection,
    ) -> Vec<(Buffer, usize)> {
        let canvas_size_len = 4 * 4 as usize;
        let canvas_size_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: as_byte_slice(&[screen.width as f32, screen.height as f32, 0.0, 0.0]),
            usage: BufferUsage::UNIFORM | BufferUsage::COPY_SRC,
        });

        let buffer = feature_collection.assemble_style_buffer();
        let len = buffer.len();
        let layer_data_len = len.max(1) * 12 * 4;
        let layer_data_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: if len == 0 {
                &[0; 48]
            } else {
                as_byte_slice(&buffer.as_slice())
            },
            usage: BufferUsage::UNIFORM | BufferUsage::COPY_SRC,
        });

        vec![
            (canvas_size_buffer, canvas_size_len),
            (layer_data_buffer, layer_data_len),
        ]
    }

    fn create_uniform_buffer(device: &Device) -> Buffer {
        let data = vec![0; Self::uniform_buffer_size() as usize];
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: as_byte_slice(&data),
            usage: BufferUsage::UNIFORM | BufferUsage::COPY_DST,
        });
        buffer
    }

    /// Creates a new transform buffer from the tile transforms.
    ///
    /// Ensures that the buffer has the size configured in the config, to match the size configured in the shader.
    fn create_tile_transform_buffer<'a>(
        device: &Device,
        screen: &Screen,
        z: f32,
        visible_tiles: impl Iterator<Item = &'a VisibleTile>,
    ) -> (Buffer, u64) {
        const TILE_DATA_SIZE: usize = 20;
        let tile_data_buffer_byte_size = TILE_DATA_SIZE * 4 * CONFIG.renderer.max_tiles;
        let mut data = vec![0f32; tile_data_buffer_byte_size];

        let mut i = 0;
        for vt in visible_tiles {
            let extent = vt.extent() as f32;
            let matrix = screen.tile_to_global_space(z, &vt.tile_id());
            for float in matrix.as_slice() {
                data[i] = *float;
                i += 1;
            }
            for _ in 0..4 {
                data[i] = extent;
                i += 1;
            }
        }
        (
            {
                let buffer = device.create_buffer_init(&BufferInitDescriptor {
                    label: None,
                    contents: as_byte_slice(data.as_slice()),
                    usage: BufferUsage::UNIFORM | BufferUsage::COPY_DST,
                });
                buffer
            },
            tile_data_buffer_byte_size as u64,
        )
    }

    fn copy_uniform_buffers(
        encoder: &mut CommandEncoder,
        source: &[(Buffer, usize)],
        destination: &Buffer,
    ) {
        let mut total_bytes = 0;
        for (buffer, len) in source {
            encoder.copy_buffer_to_buffer(&buffer, 0, &destination, total_bytes, *len as u64);
            total_bytes += *len as u64;
        }
    }

    fn uniform_buffer_size() -> u64 {
        4 * 4 + 12 * 4 * CONFIG.renderer.max_features
    }

    pub fn create_blend_bind_group(
        device: &Device,
        bind_group_layout: &BindGroupLayout,
        uniform_buffer: &Buffer,
        tile_transform_buffer: &(Buffer, u64),
    ) -> BindGroup {
        device.create_bind_group(&BindGroupDescriptor {
            label: None,
            layout: bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::Buffer(
                        uniform_buffer.slice(0..Self::uniform_buffer_size()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::Buffer(
                        tile_transform_buffer.0.slice(0..tile_transform_buffer.1),
                    ),
                },
            ],
        })
    }

    /// Loads a shader module from a GLSL vertex and fragment shader each.
    fn load_shader(
        device: &Device,
        vertex_shader: &str,
        fragment_shader: &str,
    ) -> Result<(ShaderModule, ShaderModule), std::io::Error> {
        let vertex_shader = std::fs::read_to_string(vertex_shader)?;
        let vs_bytes = load_glsl(&vertex_shader, ShaderStage::Vertex);
        let vs_module = device.create_shader_module(vs_bytes);

        let fragment_shader = std::fs::read_to_string(fragment_shader)?;
        let fs_bytes = load_glsl(&fragment_shader, ShaderStage::Fragment);
        let fs_module = device.create_shader_module(fs_bytes);

        Ok((vs_module, fs_module))
    }

    /// Reloads the shader if the file watcher has detected any change to the shader files.
    pub fn update_shader(&mut self) -> bool {
        self.temperature.update_shader(&self.device);
        match self.rx.try_recv() {
            Ok(Ok(notify::event::Event {
                kind: EventKind::Modify(ModifyKind::Data(_)),
                ..
            })) => {
                if let Ok((vs_module, fs_module)) = Self::load_shader(
                    &self.device,
                    &CONFIG.renderer.vertex_shader,
                    &CONFIG.renderer.fragment_shader,
                ) {
                    self.blend_pipeline = Self::create_layer_render_pipeline(
                        &self.device,
                        &self.bind_group_layout,
                        &vs_module,
                        &fs_module,
                        BlendDescriptor {
                            src_factor: BlendFactor::SrcAlpha,
                            dst_factor: BlendFactor::OneMinusSrcAlpha,
                            operation: BlendOperation::Add,
                        },
                        BlendDescriptor {
                            src_factor: BlendFactor::One,
                            dst_factor: BlendFactor::OneMinusSrcAlpha,
                            operation: BlendOperation::Add,
                        },
                        false,
                    );

                    self.noblend_pipeline = Self::create_layer_render_pipeline(
                        &self.device,
                        &self.bind_group_layout,
                        &vs_module,
                        &fs_module,
                        BlendDescriptor::REPLACE,
                        BlendDescriptor::REPLACE,
                        true,
                    );
                    true
                } else {
                    false
                }
            }
            // Everything is alright but file wasn't actually changed.
            Ok(Ok(_)) => false,
            // This happens all the time when there is no new message.
            Err(TryRecvError::Empty) => false,
            Ok(Err(err)) => {
                log::info!(
                    "Something went wrong with the shader file watcher:\r\n{:?}",
                    err
                );
                false
            }
            Err(err) => {
                log::info!(
                    "Something went wrong with the shader file watcher:\r\n{:?}",
                    err
                );
                false
            }
        }
    }

    pub fn get_hidpi_factor(&self) -> f64 {
        self.hidpi_factor
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.swap_chain_descriptor.width = width;
        self.swap_chain_descriptor.height = height;
        self.swap_chain = self
            .device
            .create_swap_chain(&self.surface, &self.swap_chain_descriptor);
        self.multisampled_framebuffer = Self::create_multisampled_framebuffer(
            &self.device,
            &self.swap_chain_descriptor,
            CONFIG.renderer.msaa_samples,
        );
        self.stencil = Self::create_stencil(&self.device, &self.swap_chain_descriptor);
    }

    fn update_uniforms(
        &mut self,
        encoder: &mut CommandEncoder,
        app_state: &AppState,
        feature_collection: &FeatureCollection,
    ) {
        Self::copy_uniform_buffers(
            encoder,
            &Self::create_uniform_buffers(&self.device, &app_state.screen, feature_collection),
            &self.uniform_buffer,
        );

        self.tile_transform_buffer = Self::create_tile_transform_buffer(
            &self.device,
            &app_state.screen,
            app_state.zoom,
            app_state.visible_tiles().values(),
        );
    }

    fn create_multisampled_framebuffer(
        device: &Device,
        swap_chain_descriptor: &SwapChainDescriptor,
        sample_count: u32,
    ) -> TextureView {
        let multisampled_texture_extent = Extent3d {
            width: swap_chain_descriptor.width,
            height: swap_chain_descriptor.height,
            depth: 1,
        };
        let multisampled_frame_descriptor = &TextureDescriptor {
            label: None,
            size: multisampled_texture_extent,
            // array_layer_count: 1,
            mip_level_count: 1,
            sample_count,
            dimension: TextureDimension::D2,
            format: swap_chain_descriptor.format,
            usage: TextureUsage::OUTPUT_ATTACHMENT | TextureUsage::SAMPLED,
        };

        device
            .create_texture(multisampled_frame_descriptor)
            .create_view(&wgpu::TextureViewDescriptor::default())
    }

    fn create_stencil(device: &Device, swap_chain_descriptor: &SwapChainDescriptor) -> TextureView {
        let texture_extent = Extent3d {
            width: swap_chain_descriptor.width,
            height: swap_chain_descriptor.height,
            depth: 1,
        };
        let frame_descriptor = &TextureDescriptor {
            label: None,
            size: texture_extent,
            // array_layer_count: 1,
            mip_level_count: 1,
            sample_count: CONFIG.renderer.msaa_samples,
            dimension: TextureDimension::D2,
            format: TextureFormat::Depth24PlusStencil8,
            usage: TextureUsage::OUTPUT_ATTACHMENT | TextureUsage::SAMPLED,
        };

        device
            .create_texture(frame_descriptor)
            .create_view(&wgpu::TextureViewDescriptor::default())
    }

    pub fn paint(&mut self, hud: &mut super::ui::HUD, app_state: &mut AppState) {
        let mut encoder = self
            .device
            .create_command_encoder(&CommandEncoderDescriptor { label: None });

        let feature_collection = app_state.feature_collection().read().unwrap().clone();
        self.update_uniforms(&mut encoder, &app_state, &feature_collection);
        self.bind_group = Self::create_blend_bind_group(
            &self.device,
            &self.bind_group_layout,
            &self.uniform_buffer,
            &self.tile_transform_buffer,
        );
        let num_tiles = app_state.visible_tiles().len();
        let features = feature_collection.get_features();
        if !features.is_empty() && num_tiles > 0 {
            if let Ok(frame) = self.swap_chain.get_current_frame() {
                {
                    let mut render_pass = encoder.begin_render_pass(&RenderPassDescriptor {
                        color_attachments: &[RenderPassColorAttachmentDescriptor {
                            attachment: if CONFIG.renderer.msaa_samples > 1 {
                                &self.multisampled_framebuffer
                            } else {
                                &frame.output.view
                            },
                            resolve_target: if CONFIG.renderer.msaa_samples > 1 {
                                Some(&frame.output.view)
                            } else {
                                None
                            },
                            ops: Operations::<wgpu::Color> {
                                load: LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                store: true,
                            },
                        }],
                        depth_stencil_attachment: Some(
                            RenderPassDepthStencilAttachmentDescriptor {
                                attachment: &self.stencil,
                                depth_ops: Some(Operations::<f32> {
                                    load: LoadOp::Clear(0.0),
                                    store: true,
                                }),
                                stencil_ops: Some(Operations::<u32> {
                                    load: LoadOp::Clear(255),
                                    store: true,
                                }),
                            },
                        ),
                    });
                    render_pass.set_bind_group(0, &self.bind_group, &[]);
                    let vec = vec4(0.0, 0.0, 0.0, 1.0);
                    let screen_dimensions = vec2(
                        app_state.screen.width as f32,
                        app_state.screen.height as f32,
                    ) / 2.0;

                    for (i, vt) in app_state.visible_tiles().values().enumerate() {
                        if !vt.is_loaded_to_gpu() {
                            vt.load_to_gpu(&self.device);
                        }
                        let tile_id = vt.tile_id();
                        let matrix = app_state
                            .screen
                            .tile_to_global_space(app_state.zoom, &tile_id);
                        let start = (matrix * vec).xy() + vec2(1.0, 1.0);
                        let s = vec2(
                            {
                                let x = (start.x * screen_dimensions.x).round();
                                if x < 0.0 {
                                    0.0
                                } else {
                                    x
                                }
                            },
                            {
                                let y = (start.y * screen_dimensions.y).round();
                                if y < 0.0 {
                                    0.0
                                } else {
                                    y
                                }
                            },
                        );
                        let matrix = app_state.screen.tile_to_global_space(
                            app_state.zoom,
                            &(tile_id + TileId::new(tile_id.z, 1, 1)),
                        );
                        let end = (matrix * vec).xy() + vec2(1.0, 1.0);
                        let e = vec2(
                            {
                                let x = (end.x * screen_dimensions.x).round();
                                if x < 0.0 {
                                    0.0
                                } else {
                                    x
                                }
                            },
                            {
                                let y = (end.y * screen_dimensions.y).round();
                                if y < 0.0 {
                                    0.0
                                } else {
                                    y
                                }
                            },
                        );

                        render_pass.set_scissor_rect(
                            s.x as u32,
                            s.y as u32,
                            (e.x - s.x) as u32,
                            (e.y - s.y) as u32,
                        );

                        unsafe {
                            let gpu_tile = vt.gpu_tile();
                            let gpu_tile2 = std::mem::transmute(gpu_tile.as_ref());
                            vt.paint(
                                &mut render_pass,
                                &self.blend_pipeline,
                                gpu_tile2,
                                &feature_collection,
                                i as u32,
                            );
                        }

                        // hud.paint(
                        //     app_state,
                        //     &self.window,
                        //     &mut self.device,
                        //     &mut render_pass,
                        //     &self.queue,
                        // );

                        // TODO put hwd.paint here?
                    }
                }

                for (_i, vt) in app_state.visible_tiles().values().enumerate() {
                    vt.queue_text(&mut self.glyph_brush, &app_state.screen, app_state.zoom);
                }

                let _ = self.glyph_brush.draw_queued(
                    &self.device,
                    todo!(), // TODO this needs a stagingbelt, where to get this from?
                    &mut encoder,
                    &frame.output.view,
                    app_state.screen.width,
                    app_state.screen.height,
                );

                // self.temperature.paint(&mut encoder, &frame.view);

                hud.paint(app_state, &self.window, &mut self.device, &self.queue);
                self.queue.submit(vec![encoder.finish()]);
            }
        }
    }
}
