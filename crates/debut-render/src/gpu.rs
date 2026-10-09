//! wgpu backend: the same ops as [`crate::CpuBackend`], executed as render passes
//! with the WGSL in `/shaders`. Runs on Metal, Vulkan, DX12 natively and WebGPU in
//! the browser (NFR-09, PLT-04). Images are `Rgba32Float` textures; bilinear
//! sampling is done in the shader so results match the CPU reference bit for bit
//! up to float rounding.

use crate::backend::{Backend, BlendMode, Rgba, Transform2D};
use crate::color::{ColorTransform, Grade, Transfer};
use crate::lut::Lut3d;
use bytemuck::{Pod, Zeroable};
use std::sync::Arc;

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;
const COMMON: &str = include_str!("../../../shaders/common.wgsl");
const TRANSFORM: &str = include_str!("../../../shaders/transform.wgsl");
const BLEND: &str = include_str!("../../../shaders/blend.wgsl");
const DISSOLVE: &str = include_str!("../../../shaders/dissolve.wgsl");
const COLOR: &str = include_str!("../../../shaders/color.wgsl");
const COLOR_TRANSFORM: &str = include_str!("../../../shaders/color_transform.wgsl");
const LUT3D: &str = include_str!("../../../shaders/lut3d.wgsl");
const GRADE: &str = include_str!("../../../shaders/grade.wgsl");
const PREMULTIPLY: &str = include_str!("../../../shaders/premultiply.wgsl");
const OUTPUT: &str = include_str!("../../../shaders/output.wgsl");

#[derive(Clone)]
pub struct GpuImage {
    texture: Arc<wgpu::Texture>,
    w: u32,
    h: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct TransformParams {
    m: [f32; 4],
    t: [f32; 2],
    _pad: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct BlendParams {
    mode: u32,
    opacity: f32,
    _pad: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct DissolveParams {
    progress: f32,
    _pad: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ColorTransformParams {
    m0: [f32; 4],
    m1: [f32; 4],
    m2: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct LutParams {
    domain_min: [f32; 4],
    domain_max: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GradeParams {
    lift: [f32; 4],
    gamma: [f32; 4],
    gain: [f32; 4],
    wb: [f32; 4],
}

struct Pass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    target: wgpu::TextureFormat,
}

pub struct GpuBackend {
    device: wgpu::Device,
    queue: wgpu::Queue,
    transform: Pass,
    blend: Pass,
    dissolve: Pass,
    color_transform: Pass,
    lut3d: Pass,
    grade: Pass,
    premultiply: Pass,
    output: Pass,
}

impl GpuBackend {
    /// Pick any adapter (software ones included) and build the pipelines.
    /// `None` when the platform has no usable GPU API at all.
    pub fn new() -> Option<Self> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
        }))
        .ok()?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("debut-render"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
        }))
        .ok()?;
        let transform = Self::pass(
            &device,
            "transform",
            "",
            TRANSFORM,
            1,
            false,
            std::mem::size_of::<TransformParams>(),
        );
        let blend = Self::pass(
            &device,
            "blend",
            "",
            BLEND,
            2,
            false,
            std::mem::size_of::<BlendParams>(),
        );
        let dissolve = Self::pass(
            &device,
            "dissolve",
            "",
            DISSOLVE,
            2,
            false,
            std::mem::size_of::<DissolveParams>(),
        );
        let color_transform = Self::pass(
            &device,
            "color_transform",
            COLOR,
            COLOR_TRANSFORM,
            1,
            false,
            std::mem::size_of::<ColorTransformParams>(),
        );
        let lut3d = Self::pass(
            &device,
            "lut3d",
            COLOR,
            LUT3D,
            1,
            true,
            std::mem::size_of::<LutParams>(),
        );
        let grade = Self::pass(
            &device,
            "grade",
            COLOR,
            GRADE,
            1,
            false,
            std::mem::size_of::<GradeParams>(),
        );
        let premultiply = Self::pass(&device, "premultiply", "", PREMULTIPLY, 1, false, 16);
        let output = Self::pass_to(
            &device,
            "output",
            COLOR,
            OUTPUT,
            1,
            false,
            16,
            wgpu::TextureFormat::Rgba8Unorm,
        );
        Some(Self {
            device,
            queue,
            transform,
            blend,
            dissolve,
            color_transform,
            lut3d,
            grade,
            premultiply,
            output,
        })
    }

    /// `textures` 2D inputs at bindings 1.., plus a 3D texture after them if `lut`.
    #[allow(clippy::too_many_arguments)]
    fn pass(
        device: &wgpu::Device,
        name: &str,
        prelude: &str,
        fs: &str,
        textures: u32,
        lut: bool,
        uniform_size: usize,
    ) -> Pass {
        Self::pass_to(
            device,
            name,
            prelude,
            fs,
            textures,
            lut,
            uniform_size,
            FORMAT,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn pass_to(
        device: &wgpu::Device,
        name: &str,
        prelude: &str,
        fs: &str,
        textures: u32,
        lut: bool,
        uniform_size: usize,
        target: wgpu::TextureFormat,
    ) -> Pass {
        let source = format!("{COMMON}\n{prelude}\n{fs}");
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(name),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let mut entries = vec![wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(uniform_size as u64),
            },
            count: None,
        }];
        for i in 0..textures {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: 1 + i,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            });
        }
        if lut {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: 1 + textures,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D3,
                    multisampled: false,
                },
                count: None,
            });
        }
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(name),
            entries: &entries,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(name),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(name),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview: None,
            cache: None,
        });
        Pass {
            pipeline,
            layout,
            target,
        }
    }

    fn texture(&self, w: u32, h: u32, label: &str) -> wgpu::Texture {
        self.texture_with(w, h, FORMAT, label)
    }

    fn texture_with(
        &self,
        w: u32,
        h: u32,
        format: wgpu::TextureFormat,
        label: &str,
    ) -> wgpu::Texture {
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        })
    }

    /// Run one full-screen pass into a new `w x h` image.
    fn run(
        &self,
        pass: &Pass,
        uniforms: &[u8],
        inputs: &[&GpuImage],
        w: u32,
        h: u32,
        label: &str,
    ) -> GpuImage {
        self.run_with(pass, uniforms, inputs, None, w, h, label)
    }

    #[allow(clippy::too_many_arguments)]
    fn run_with(
        &self,
        pass: &Pass,
        uniforms: &[u8],
        inputs: &[&GpuImage],
        extra: Option<&wgpu::TextureView>,
        w: u32,
        h: u32,
        label: &str,
    ) -> GpuImage {
        let out = self.texture_with(w, h, pass.target, label);
        let uniform = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: uniforms.len() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&uniform, 0, uniforms);
        let views: Vec<wgpu::TextureView> = inputs
            .iter()
            .map(|i| {
                i.texture
                    .create_view(&wgpu::TextureViewDescriptor::default())
            })
            .collect();
        let mut entries = vec![wgpu::BindGroupEntry {
            binding: 0,
            resource: uniform.as_entire_binding(),
        }];
        for (i, v) in views.iter().enumerate() {
            entries.push(wgpu::BindGroupEntry {
                binding: 1 + i as u32,
                resource: wgpu::BindingResource::TextureView(v),
            });
        }
        if let Some(v) = extra {
            entries.push(wgpu::BindGroupEntry {
                binding: 1 + views.len() as u32,
                resource: wgpu::BindingResource::TextureView(v),
            });
        }
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: &pass.layout,
            entries: &entries,
        });
        let out_view = out.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some(label) });
        {
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &out_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rp.set_pipeline(&pass.pipeline);
            rp.set_bind_group(0, &bind_group, &[]);
            rp.draw(0..3, 0..1);
        }
        self.queue.submit(Some(encoder.finish()));
        GpuImage {
            texture: Arc::new(out),
            w,
            h,
        }
    }
}

impl GpuBackend {
    /// Copy a texture to the CPU as tightly packed rows of `bpp` bytes per pixel.
    fn read_back(&self, img: &GpuImage, bpp: u32) -> Vec<u8> {
        let unpadded = img.w * bpp;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let bpr = unpadded.div_ceil(align) * align;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("read_back"),
            size: (bpr * img.h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("read_back"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &img.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bpr),
                    rows_per_image: Some(img.h),
                },
            },
            wgpu::Extent3d {
                width: img.w,
                height: img.h,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(encoder.finish()));
        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("device poll");
        rx.recv().expect("map callback").expect("map read");
        let data = slice.get_mapped_range();
        let mut out = Vec::with_capacity((unpadded * img.h) as usize);
        for row in 0..img.h {
            let start = (row * bpr) as usize;
            out.extend_from_slice(&data[start..start + unpadded as usize]);
        }
        drop(data);
        buffer.unmap();
        out
    }
}

const BYTES_PER_PIXEL: u32 = 16;

impl Backend for GpuBackend {
    type Image = GpuImage;

    fn size(&self, img: &GpuImage) -> (u32, u32) {
        (img.w, img.h)
    }

    fn solid(&mut self, w: u32, h: u32, color: Rgba) -> GpuImage {
        let px = vec![color; (w * h) as usize];
        self.upload(w, h, &px)
    }

    fn upload(&mut self, w: u32, h: u32, pixels: &[Rgba]) -> GpuImage {
        assert_eq!(pixels.len(), (w * h) as usize);
        let texture = self.texture(w, h, "upload");
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(pixels),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * BYTES_PER_PIXEL),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        GpuImage {
            texture: Arc::new(texture),
            w,
            h,
        }
    }

    /// Decoded video is uploaded as an 8-bit texture (a quarter of the bytes) and
    /// premultiplied into the float format by one pass on the GPU.
    fn upload_rgba8(&mut self, w: u32, h: u32, pixels: &[u8]) -> GpuImage {
        assert_eq!(pixels.len(), (w * h * 4) as usize);
        let texture = self.texture_with(w, h, wgpu::TextureFormat::Rgba8Unorm, "upload_rgba8");
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        let raw = GpuImage {
            texture: Arc::new(texture),
            w,
            h,
        };
        self.run(&self.premultiply, &[0u8; 16], &[&raw], w, h, "premultiply")
    }

    fn transform(&mut self, src: &GpuImage, xf: &Transform2D, w: u32, h: u32) -> GpuImage {
        let p = TransformParams {
            m: [xf.m[0][0], xf.m[0][1], xf.m[1][0], xf.m[1][1]],
            t: xf.t,
            _pad: [0.0; 2],
        };
        self.run(
            &self.transform,
            bytemuck::bytes_of(&p),
            &[src],
            w,
            h,
            "transform",
        )
    }

    fn blend(
        &mut self,
        bottom: &GpuImage,
        top: &GpuImage,
        mode: BlendMode,
        opacity: f32,
    ) -> GpuImage {
        assert_eq!(
            (bottom.w, bottom.h),
            (top.w, top.h),
            "blend inputs must match"
        );
        let mode = match mode {
            BlendMode::Normal => 0,
            BlendMode::Add => 1,
            BlendMode::Multiply => 2,
            BlendMode::Screen => 3,
        };
        let p = BlendParams {
            mode,
            opacity,
            _pad: [0.0; 2],
        };
        self.run(
            &self.blend,
            bytemuck::bytes_of(&p),
            &[bottom, top],
            bottom.w,
            bottom.h,
            "blend",
        )
    }

    fn dissolve(&mut self, a: &GpuImage, b: &GpuImage, progress: f32) -> GpuImage {
        assert_eq!((a.w, a.h), (b.w, b.h), "dissolve inputs must match");
        let p = DissolveParams {
            progress: progress.clamp(0.0, 1.0),
            _pad: [0.0; 3],
        };
        self.run(
            &self.dissolve,
            bytemuck::bytes_of(&p),
            &[a, b],
            a.w,
            a.h,
            "dissolve",
        )
    }

    fn color_transform(&mut self, src: &GpuImage, xf: &ColorTransform) -> GpuImage {
        let m = xf.matrix;
        let p = ColorTransformParams {
            m0: [m[0][0], m[0][1], m[0][2], xf.decode as u32 as f32],
            m1: [m[1][0], m[1][1], m[1][2], xf.encode as u32 as f32],
            m2: [m[2][0], m[2][1], m[2][2], 0.0],
        };
        self.run(
            &self.color_transform,
            bytemuck::bytes_of(&p),
            &[src],
            src.w,
            src.h,
            "color_transform",
        )
    }

    fn lut3d(&mut self, src: &GpuImage, lut: &Lut3d) -> GpuImage {
        let n = lut.size as u32;
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("lut3d"),
            size: wgpu::Extent3d {
                width: n,
                height: n,
                depth_or_array_layers: n,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let texels: Vec<Rgba> = lut.data.iter().map(|c| [c[0], c[1], c[2], 1.0]).collect();
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&texels),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(n * BYTES_PER_PIXEL),
                rows_per_image: Some(n),
            },
            wgpu::Extent3d {
                width: n,
                height: n,
                depth_or_array_layers: n,
            },
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let p = LutParams {
            domain_min: [
                lut.domain_min[0],
                lut.domain_min[1],
                lut.domain_min[2],
                lut.size as f32,
            ],
            domain_max: [lut.domain_max[0], lut.domain_max[1], lut.domain_max[2], 0.0],
        };
        self.run_with(
            &self.lut3d,
            bytemuck::bytes_of(&p),
            &[src],
            Some(&view),
            src.w,
            src.h,
            "lut3d",
        )
    }

    fn grade(&mut self, src: &GpuImage, g: &Grade) -> GpuImage {
        let p = GradeParams {
            lift: [g.lift[0], g.lift[1], g.lift[2], g.exposure],
            gamma: [g.gamma[0], g.gamma[1], g.gamma[2], g.contrast],
            gain: [g.gain[0], g.gain[1], g.gain[2], g.saturation],
            wb: [g.temperature, g.tint, 0.0, 0.0],
        };
        self.run(
            &self.grade,
            bytemuck::bytes_of(&p),
            &[src],
            src.w,
            src.h,
            "grade",
        )
    }

    fn download_rgba8(&mut self, img: &GpuImage, transfer: Transfer) -> Vec<u8> {
        let p = [transfer as u32, 0, 0, 0];
        let out = self.run(
            &self.output,
            bytemuck::bytes_of(&p),
            &[img],
            img.w,
            img.h,
            "output",
        );
        self.read_back(&out, 4)
    }

    fn download(&mut self, img: &GpuImage) -> Vec<Rgba> {
        let bytes = self.read_back(img, BYTES_PER_PIXEL);
        bytemuck::cast_slice::<u8, Rgba>(&bytes).to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CpuBackend;

    fn gpu() -> Option<GpuBackend> {
        let g = GpuBackend::new();
        if g.is_none() {
            eprintln!("no GPU adapter available; skipping conformance test");
        }
        g
    }

    /// Deterministic pseudo-random image with partial alpha.
    fn noise(w: u32, h: u32, seed: u32) -> Vec<Rgba> {
        let mut s = seed.wrapping_mul(2654435761) | 1;
        let mut next = move || {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s % 1000) as f32 / 1000.0
        };
        (0..w * h)
            .map(|_| {
                let a = next();
                [next() * a, next() * a, next() * a, a]
            })
            .collect()
    }

    /// Largest difference, relative to the magnitude once values exceed 1 (HDR
    /// transfers like PQ put scene values in the hundreds).
    fn max_diff(a: &[Rgba], b: &[Rgba]) -> f32 {
        assert_eq!(a.len(), b.len());
        a.iter()
            .zip(b)
            .flat_map(|(x, y)| {
                x.iter()
                    .zip(y)
                    .map(|(p, q)| (p - q).abs() / p.abs().max(1.0))
            })
            .fold(0.0, f32::max)
    }

    /// PLT-04: the GPU backend must match the CPU reference within tolerance.
    #[test]
    fn gpu_matches_cpu_reference() {
        let Some(mut gpu) = gpu() else { return };
        let mut cpu = CpuBackend;
        let (w, h) = (37, 23);
        let a = noise(w, h, 1);
        let b = noise(w, h, 2);
        let small = noise(9, 7, 3);

        let (ga, gb, gs) = (
            gpu.upload(w, h, &a),
            gpu.upload(w, h, &b),
            gpu.upload(9, 7, &small),
        );
        let (ca, cb, cs) = (
            cpu.upload(w, h, &a),
            cpu.upload(w, h, &b),
            cpu.upload(9, 7, &small),
        );

        let got = gpu.download(&ga);
        assert_eq!(max_diff(&got, &a), 0.0, "upload/download round trip");
        let bytes: Vec<u8> = (0..w * h * 4).map(|i| (i * 37 % 256) as u8).collect();
        let g8 = gpu.upload_rgba8(w, h, &bytes);
        let c8 = cpu.upload_rgba8(w, h, &bytes);
        let xf0 = Transform2D::IDENTITY;
        let d = max_diff(
            &{
                let t = gpu.transform(&g8, &xf0, w, h);
                gpu.download(&t)
            },
            &{
                let t = cpu.transform(&c8, &xf0, w, h);
                cpu.download(&t)
            },
        );
        assert!(d < 1e-5, "rgba8 upload differs by {d}");

        for mode in [
            BlendMode::Normal,
            BlendMode::Add,
            BlendMode::Multiply,
            BlendMode::Screen,
        ] {
            let g = gpu.blend(&ga, &gb, mode, 0.7);
            let c = cpu.blend(&ca, &cb, mode, 0.7);
            let d = max_diff(&gpu.download(&g), &cpu.download(&c));
            assert!(d < 1e-5, "blend {mode:?} differs by {d}");
        }

        let g = gpu.dissolve(&ga, &gb, 0.3);
        let c = cpu.dissolve(&ca, &cb, 0.3);
        assert!(
            max_diff(&gpu.download(&g), &cpu.download(&c)) < 1e-5,
            "dissolve"
        );

        for xf in [
            ColorTransform::between(
                &debut_core::color::ColorSpace::Rec709,
                &debut_core::color::ColorSpace::Linear709,
            )
            .unwrap(),
            ColorTransform::between(
                &debut_core::color::ColorSpace::SLog3SGamut3Cine,
                &debut_core::color::ColorSpace::Linear709,
            )
            .unwrap(),
            ColorTransform::between(
                &debut_core::color::ColorSpace::Rec2020Pq,
                &debut_core::color::ColorSpace::Srgb,
            )
            .unwrap(),
            ColorTransform::between(
                &debut_core::color::ColorSpace::LogC3Awg3,
                &debut_core::color::ColorSpace::Rec2020Hlg,
            )
            .unwrap(),
            ColorTransform::between(
                &debut_core::color::ColorSpace::VLogVGamut,
                &debut_core::color::ColorSpace::AcesCg,
            )
            .unwrap(),
        ] {
            let g = gpu.color_transform(&ga, &xf);
            let c = cpu.color_transform(&ca, &xf);
            let d = max_diff(&gpu.download(&g), &cpu.download(&c));
            assert!(
                d < 2e-3,
                "color transform {:?}->{:?} differs by {d}",
                xf.decode,
                xf.encode
            );
        }
        let lut = Lut3d::parse_cube(
            "LUT_3D_SIZE 2\n0 0 0\n0 0 1\n0 1 0\n0 1 1\n1 0 0\n1 0 1\n1 1 0\n1 1 1\n",
        )
        .unwrap();
        let g = gpu.lut3d(&ga, &lut);
        let c = cpu.lut3d(&ca, &lut);
        assert!(
            max_diff(&gpu.download(&g), &cpu.download(&c)) < 1e-5,
            "lut3d"
        );
        let grade = Grade {
            exposure: 0.7,
            temperature: 0.3,
            tint: -0.2,
            lift: [0.02, 0.0, -0.01],
            gamma: [1.1, 0.9, 1.0],
            gain: [1.2, 1.0, 0.8],
            contrast: 1.3,
            saturation: 0.6,
        };
        let g = gpu.grade(&ga, &grade);
        let c = cpu.grade(&ca, &grade);
        let d = max_diff(&gpu.download(&g), &cpu.download(&c));
        assert!(d < 1e-4, "grade differs by {d}");

        let g8 = gpu.download_rgba8(&ga, Transfer::Srgb);
        let c8 = cpu.download_rgba8(&ca, Transfer::Srgb);
        let worst = g8
            .iter()
            .zip(&c8)
            .map(|(x, y)| (*x as i32 - *y as i32).abs())
            .max()
            .unwrap();
        assert!(worst <= 1, "rgba8 output differs by {worst}/255");

        let xf = Transform2D::from_srt((9, 7), (w, h), (2.5, 1.75), 0.4, (3.0, -2.0));
        let g = gpu.transform(&gs, &xf, w, h);
        let c = cpu.transform(&cs, &xf, w, h);
        let d = max_diff(&gpu.download(&g), &cpu.download(&c));
        assert!(d < 1e-4, "transform differs by {d}");
    }
}
