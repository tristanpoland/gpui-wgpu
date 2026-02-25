/// Example: WgpuSurface with secondary render thread
/// Demonstrates using the WgpuSurface element with a dedicated render thread
use gpui::{
    App, Application, Context, Render, Window, WindowOptions, div, prelude::*, wgpu_surface, WgpuSurfaceHandle, rgb
};
use std::thread;
use std::time::Duration;
use gpui::Styled;
use gpui::AppContext;

struct SurfaceExample {
    surface: WgpuSurfaceHandle,
}

impl Render for SurfaceExample {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // The surface element will display the front buffer
        // Overlay a debug border and label for visibility
        div()
            .w(gpui::px(400.0))
            .h(gpui::px(300.0))
            .border_4()
            .border_color(rgb(0xff00ff))
            .rounded_lg()
            .shadow_xl()
            .bg(rgb(0x151a29))
            .m(gpui::px(8.0))
            .child(
                wgpu_surface(self.surface.clone())
                    .absolute()
                    .inset_0() // Fill parent div
                    
            )
            .child(
                div()
                    .absolute()
                    .top(gpui::px(4.0))
                    .left(gpui::px(8.0))
                    .text_color(rgb(0xff00ff))
                    .text_xl()
                    .child("[WgpuSurface Debug Overlay]")
            )
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        // Open a window
        _ = cx.open_window(WindowOptions::default(), |window: &mut Window, cx: &mut App| {
            // Create a WgpuSurfaceHandle (400x300 RGBA8)
            let surface = window.create_wgpu_surface(400, 300, wgpu::TextureFormat::Rgba8UnormSrgb)
                .expect("WgpuSurface not supported on this platform");
            let surface_thread = surface.clone();

            // Spawn a secondary render thread
            thread::spawn(move || {
                let mut frame: u32 = 0;
                // Wait for surface to be ready
                loop {
                    if surface_thread.back_buffer_view().is_some() {
                        break;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                // high‑performance render loop without sleeps or per‑frame printouts
                let mut last = std::time::Instant::now();
                loop {
                    // throttle producer: wait until the compositor consumes last frame.
                    surface_thread.wait_for_present();
                    // draw to back buffer
                    let device = surface_thread.device();
                    let queue = surface_thread.queue();
                    let view = surface_thread.back_buffer_view();
                    let view = match &view {
                        Some(v) => v,
                        None => {
                            frame = frame.wrapping_add(1);
                            thread::sleep(Duration::from_nanos(500));
                            continue;
                        }
                    };

                    // --- GPU gradient: fullscreen triangle with animated rainbow gradient ---
                    use wgpu::util::DeviceExt;
                    thread_local! {
                        static PIPELINE: std::cell::RefCell<Option<wgpu::RenderPipeline>> = std::cell::RefCell::new(None);
                        static BIND_GROUP_LAYOUT: std::cell::RefCell<Option<wgpu::BindGroupLayout>> = std::cell::RefCell::new(None);
                        static UNIFORM_BUF: std::cell::RefCell<Option<wgpu::Buffer>> = std::cell::RefCell::new(None);
                        static BIND_GROUP: std::cell::RefCell<Option<wgpu::BindGroup>> = std::cell::RefCell::new(None);
                    }

                    let t = frame as f32 * 0.01;
                    let t_arr = [t];
                    let uniform_bytes = bytemuck::cast_slice(&t_arr);

                    let (pipeline, bind_group_layout) = PIPELINE.with(|p| {
                        let mut p = p.borrow_mut();
                        if p.is_none() {
                            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                                label: Some("GradientShader"),
                                source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(r#"
@group(0) @binding(0) var<uniform> time: f32;

struct VSOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) idx: u32) -> VSOut {
    var pos = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(3.0, 1.0),
        vec2<f32>(-1.0, 1.0)
    );
    let uv = (pos[idx] + vec2<f32>(1.0, 1.0)) * 0.5;
    var out: VSOut;
    out.pos = vec4<f32>(pos[idx], 0.0, 1.0);
    out.uv = uv;
    return out;
}

fn hsv2rgb(h: f32, s: f32, v: f32) -> vec3<f32> {
    let c = v * s;
    let x = c * (1.0 - abs((h * 6.0) % 2.0 - 1.0));
    let m = v - c;
    let h6 = h * 6.0;
    var rgb = vec3<f32>(0.0);
    if (h6 < 1.0) {
        rgb = vec3<f32>(c, x, 0.0);
    } else if (h6 < 2.0) {
        rgb = vec3<f32>(x, c, 0.0);
    } else if (h6 < 3.0) {
        rgb = vec3<f32>(0.0, c, x);
    } else if (h6 < 4.0) {
        rgb = vec3<f32>(0.0, x, c);
    } else if (h6 < 5.0) {
        rgb = vec3<f32>(x, 0.0, c);
    } else {
        rgb = vec3<f32>(c, 0.0, x);
    }
    return rgb + vec3<f32>(m);
}

@fragment
fn fs_main(in: VSOut) -> @location(0) vec4<f32> {
    let hue = fract(in.uv.x + time);
    let color = hsv2rgb(hue, 1.0, 1.0);
    return vec4<f32>(color, 1.0);
}
"#)),
                            });
                            let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                                label: Some("GradientBGL"),
                                entries: &[wgpu::BindGroupLayoutEntry {
                                    binding: 0,
                                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                                    ty: wgpu::BindingType::Buffer {
                                        ty: wgpu::BufferBindingType::Uniform,
                                        has_dynamic_offset: false,
                                        min_binding_size: None,
                                    },
                                    count: None,
                                }],
                            });
                            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                                label: Some("GradientPipelineLayout"),
                                bind_group_layouts: &[&bind_group_layout],
                                push_constant_ranges: &[],
                            });
                            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                                label: Some("GradientPipeline"),
                                layout: Some(&pipeline_layout),
                                vertex: wgpu::VertexState {
                                    module: &shader,
                                    entry_point: Some("vs_main"),
                                    buffers: &[],
                                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                                },
                                fragment: Some(wgpu::FragmentState {
                                    module: &shader,
                                    entry_point: Some("fs_main"),
                                    targets: &[Some(wgpu::ColorTargetState {
                                        format: wgpu::TextureFormat::Rgba8UnormSrgb,
                                        blend: Some(wgpu::BlendState::REPLACE),
                                        write_mask: wgpu::ColorWrites::ALL,
                                    })],
                                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                                }),
                                primitive: wgpu::PrimitiveState::default(),
                                depth_stencil: None,
                                multisample: wgpu::MultisampleState::default(),
                                multiview: None,
                                cache: None,
                            });
                            *p = Some(pipeline.clone());
                            (pipeline, bind_group_layout)
                        } else {
                            let pipeline = p.as_ref().unwrap().clone();
                            let bind_group_layout = pipeline.get_bind_group_layout(0);
                            (pipeline, bind_group_layout)
                        }
                    });
                    let uniform_buf = UNIFORM_BUF.with(|u| {
                        let mut u = u.borrow_mut();
                        if u.is_none() {
                            *u = Some(device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                label: Some("GradientUniformBuf"),
                                contents: &[0; 4],
                                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                            }));
                        }
                        u.as_ref().unwrap().clone()
                    });
                    queue.write_buffer(&uniform_buf, 0, uniform_bytes);
                    let bind_group = BIND_GROUP.with(|b| {
                        let mut b = b.borrow_mut();
                        if b.is_none() {
                            *b = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                                label: Some("GradientBindGroup"),
                                layout: &bind_group_layout,
                                entries: &[wgpu::BindGroupEntry {
                                    binding: 0,
                                    resource: uniform_buf.as_entire_binding(),
                                }],
                            }));
                        }
                        b.as_ref().unwrap().clone()
                    });

                    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("SurfaceExample Encoder"),
                    });
                    {
                        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("SurfaceExample Pass"),
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: &view,
                                resolve_target: None,
                                ops: wgpu::Operations {
                                    load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
                                    store: wgpu::StoreOp::Store,
                                },
                            })],
                            depth_stencil_attachment: None,
                            occlusion_query_set: None,
                            timestamp_writes: None,
                        });
                        rpass.set_pipeline(&pipeline);
                        rpass.set_bind_group(0, &bind_group, &[]);
                        rpass.draw(0..3, 0..1);
                    }
                    let _ = queue.submit(Some(encoder.finish()));

                    surface_thread.present();
                    frame = frame.wrapping_add(1);

                    if frame % 1000 == 0 {
                        let now = std::time::Instant::now();
                        let elapsed = now.duration_since(last).as_secs_f64();
                        let fps = 1000.0 / elapsed;
                        println!("[wgpu_surface] {} frames in {:.3}s = {:.1} FPS", 1000, elapsed, fps);
                        last = now;
                    }
                }
            });

            cx.new(|_cx| SurfaceExample { surface })
        });
    });
}
