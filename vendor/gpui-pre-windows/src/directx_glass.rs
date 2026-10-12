//! Liquid Glass surfaces. Each one copies the frame beneath it, frosts the
//! copy if its material asks for it, and composites the refracted result back
//! into the frame. See `glass.hlsl` for the optics.

use std::slice;

use anyhow::{Context, Result};
use windows::{
    Win32::Graphics::{
        Direct3D::{Fxc::D3DCompile, *},
        Direct3D11::*,
        Dxgi::Common::*,
    },
    core::PCSTR,
};

use super::{
    RENDER_TARGET_FORMAT, create_blend_state, create_constant_buffer, create_fragment_shader,
    create_vertex_shader, update_buffer,
};
use gpui::{Glass, Rgba};

const SOURCE: &str = include_str!("glass.hlsl");

#[derive(Clone, Copy, Default)]
#[repr(C)]
struct GlassParams {
    shape: [f32; 4],
    clip: [f32; 4],
    region: [f32; 4],
    tint: [f32; 4],
    optics: [f32; 4],
    misc: [f32; 4],
    blur: [f32; 4],
    tex: [f32; 4],
}

struct Target {
    texture: ID3D11Texture2D,
    srv: Option<ID3D11ShaderResourceView>,
    rtv: Option<ID3D11RenderTargetView>,
}

impl Target {
    fn new(device: &ID3D11Device, width: u32, height: u32) -> Result<Self> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: RENDER_TARGET_FORMAT,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut texture = None;
        unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture))? };
        let texture = texture.context("creating glass texture")?;
        let mut srv = None;
        let mut rtv = None;
        unsafe {
            device.CreateShaderResourceView(&texture, None, Some(&mut srv))?;
            device.CreateRenderTargetView(&texture, None, Some(&mut rtv))?;
        }
        Ok(Self { texture, srv, rtv })
    }
}

struct Targets {
    width: u32,
    height: u32,
    /// The backdrop at full resolution, copied out of the frame.
    backdrop: Target,
    /// Half-resolution blur ping-pong.
    horizontal: Target,
    frosted: Target,
}

pub(super) struct GlassRenderer {
    blur_vertex: ID3D11VertexShader,
    blur_fragment: ID3D11PixelShader,
    glass_vertex: ID3D11VertexShader,
    glass_fragment: ID3D11PixelShader,
    params: Option<ID3D11Buffer>,
    sampler: Option<ID3D11SamplerState>,
    blend: ID3D11BlendState,
    opaque: ID3D11BlendState,
    targets: Option<Targets>,
}

fn compile(entry: &str, target: &str) -> Result<ID3DBlob> {
    let entry = format!("{entry}\0");
    let target = format!("{target}\0");
    let mut blob = None;
    let mut errors = None;
    let result = unsafe {
        D3DCompile(
            SOURCE.as_ptr() as *const _,
            SOURCE.len(),
            PCSTR(c"glass.hlsl".as_ptr() as *const u8),
            None,
            None,
            PCSTR(entry.as_ptr()),
            PCSTR(target.as_ptr()),
            0,
            0,
            &mut blob,
            Some(&mut errors),
        )
    };
    if let Err(error) = result {
        let message = errors
            .map(|e| unsafe {
                std::ffi::CStr::from_ptr(e.GetBufferPointer() as *const i8)
                    .to_string_lossy()
                    .into_owned()
            })
            .unwrap_or_default();
        anyhow::bail!("compiling glass shader {entry}: {error:?} {message}");
    }
    blob.context("glass shader blob")
}

fn bytes(blob: &ID3DBlob) -> &[u8] {
    unsafe { slice::from_raw_parts(blob.GetBufferPointer() as *const u8, blob.GetBufferSize()) }
}

impl GlassRenderer {
    pub(super) fn new(device: &ID3D11Device) -> Result<Self> {
        let blur_vertex = create_vertex_shader(device, bytes(&compile("blur_vertex", "vs_5_0")?))?;
        let blur_fragment =
            create_fragment_shader(device, bytes(&compile("blur_fragment", "ps_5_0")?))?;
        let glass_vertex =
            create_vertex_shader(device, bytes(&compile("glass_vertex", "vs_5_0")?))?;
        let glass_fragment =
            create_fragment_shader(device, bytes(&compile("glass_fragment", "ps_5_0")?))?;
        let sampler = unsafe {
            let desc = D3D11_SAMPLER_DESC {
                Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
                AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
                MipLODBias: 0.0,
                MaxAnisotropy: 1,
                ComparisonFunc: D3D11_COMPARISON_ALWAYS,
                BorderColor: [0.0; 4],
                MinLOD: 0.0,
                MaxLOD: D3D11_FLOAT32_MAX,
            };
            let mut output = None;
            device.CreateSamplerState(&desc, Some(&mut output))?;
            output
        };
        let opaque = unsafe {
            let mut desc = D3D11_BLEND_DESC::default();
            desc.RenderTarget[0].RenderTargetWriteMask = D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8;
            let mut state = None;
            device.CreateBlendState(&desc, Some(&mut state))?;
            state.context("glass blend state")?
        };
        Ok(Self {
            blur_vertex,
            blur_fragment,
            glass_vertex,
            glass_fragment,
            params: create_constant_buffer::<GlassParams>(device)?,
            sampler,
            blend: create_blend_state(device)?,
            opaque,
            targets: None,
        })
    }

    fn targets(&mut self, device: &ID3D11Device, width: u32, height: u32) -> Result<&Targets> {
        if self
            .targets
            .as_ref()
            .is_none_or(|t| t.width != width || t.height != height)
        {
            let half = |x: u32| x.div_ceil(2).max(1);
            self.targets = Some(Targets {
                width,
                height,
                backdrop: Target::new(device, width, height)?,
                horizontal: Target::new(device, half(width), half(height))?,
                frosted: Target::new(device, half(width), half(height))?,
            });
        }
        Ok(self.targets.as_ref().unwrap())
    }

    /// Draw each surface in turn, so later ones refract earlier ones.
    pub(super) fn draw(
        &mut self,
        device: &ID3D11Device,
        context: &ID3D11DeviceContext,
        frame: &ID3D11Texture2D,
        frame_view: &Option<ID3D11RenderTargetView>,
        viewport: &D3D11_VIEWPORT,
        glass: &[Glass],
    ) -> Result<()> {
        let (width, height) = (viewport.Width as u32, viewport.Height as u32);
        self.targets(device, width, height)?;
        let targets = self.targets.as_ref().unwrap();
        let params = self.params.as_ref().context("glass params")?;
        let none_srv: [Option<ID3D11ShaderResourceView>; 1] = [None];

        unsafe {
            context.VSSetConstantBuffers(2, Some(slice::from_ref(&self.params)));
            context.PSSetConstantBuffers(2, Some(slice::from_ref(&self.params)));
            context.PSSetSamplers(0, Some(slice::from_ref(&self.sampler)));
            context.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
        }

        for g in glass {
            let scale = g.scale_factor;
            // Copy everything the rim can reach, clamped to the frame.
            let left = (g.bounds.origin.x.0.floor().max(0.0) as u32).min(width);
            let top = (g.bounds.origin.y.0.floor().max(0.0) as u32).min(height);
            let right = ((g.bounds.origin.x.0 + g.bounds.size.width.0).ceil().max(0.0) as u32)
                .min(width);
            let bottom = ((g.bounds.origin.y.0 + g.bounds.size.height.0).ceil().max(0.0) as u32)
                .min(height);
            if right <= left || bottom <= top {
                continue;
            }
            let region = [left as f32, top as f32, (right - left) as f32, (bottom - top) as f32];
            unsafe {
                context.CopySubresourceRegion(
                    &targets.backdrop.texture,
                    0,
                    0,
                    0,
                    0,
                    frame,
                    0,
                    Some(&D3D11_BOX { left, top, front: 0, right, bottom, back: 1 }),
                );
            }

            let frosted = g.blur > 0.0;
            let tint = Rgba::from(g.tint);
            let mut p = GlassParams {
                shape: [
                    g.shape.origin.x.0,
                    g.shape.origin.y.0,
                    g.shape.size.width.0,
                    g.shape.size.height.0,
                ],
                clip: [
                    g.content_mask.bounds.origin.x.0,
                    g.content_mask.bounds.origin.y.0,
                    g.content_mask.bounds.size.width.0,
                    g.content_mask.bounds.size.height.0,
                ],
                region,
                tint: [tint.r, tint.g, tint.b, tint.a],
                optics: [g.corner_radius.0, g.refraction * scale, g.bezel * scale, g.dispersion],
                misc: [scale, g.highlight, g.dark as f32, frosted as u32 as f32],
                blur: [0.0; 4],
                tex: [width as f32, height as f32, viewport.Width, viewport.Height],
            };

            let source = if frosted {
                let half = |x: u32| x.div_ceil(2).max(1);
                let (hw, hh) = (half(width), half(height));
                let half_viewport = D3D11_VIEWPORT {
                    TopLeftX: 0.0,
                    TopLeftY: 0.0,
                    Width: (right - left).div_ceil(2) as f32 + 1.0,
                    Height: (bottom - top).div_ceil(2) as f32 + 1.0,
                    MinDepth: 0.0,
                    MaxDepth: 1.0,
                };
                let sigma = g.blur * scale;
                let (rw, rh) = (region[2], region[3]);
                let passes = [
                    // Horizontal, downsampling the full-resolution backdrop.
                    (&targets.backdrop, &targets.horizontal, [sigma, 2.0, 0.0, 2.0], [width as f32, height as f32, rw, rh]),
                    // Vertical, at half resolution.
                    (&targets.horizontal, &targets.frosted, [sigma * 0.5, 0.0, 1.0, 1.0], [hw as f32, hh as f32, (rw * 0.5).ceil(), (rh * 0.5).ceil()]),
                ];
                for (from, to, blur, tex) in passes {
                    p.blur = blur;
                    p.tex = tex;
                    update_buffer(context, params, &[p])?;
                    unsafe {
                        context.PSSetShaderResources(0, Some(&none_srv));
                        context.OMSetRenderTargets(Some(slice::from_ref(&to.rtv)), None);
                        context.RSSetViewports(Some(slice::from_ref(&half_viewport)));
                        context.VSSetShader(&self.blur_vertex, None);
                        context.PSSetShader(&self.blur_fragment, None);
                        context.OMSetBlendState(&self.opaque, None, 0xFFFFFFFF);
                        context.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
                        context.PSSetShaderResources(0, Some(slice::from_ref(&from.srv)));
                        context.Draw(3, 0);
                    }
                }
                p.tex = [hw as f32, hh as f32, viewport.Width, viewport.Height];
                &targets.frosted
            } else {
                &targets.backdrop
            };

            update_buffer(context, params, &[p])?;
            unsafe {
                context.PSSetShaderResources(0, Some(&none_srv));
                context.OMSetRenderTargets(Some(slice::from_ref(frame_view)), None);
                context.RSSetViewports(Some(slice::from_ref(viewport)));
                context.VSSetShader(&self.glass_vertex, None);
                context.PSSetShader(&self.glass_fragment, None);
                context.OMSetBlendState(&self.blend, None, 0xFFFFFFFF);
                context.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
                context.PSSetShaderResources(0, Some(slice::from_ref(&source.srv)));
                context.Draw(4, 0);
                context.PSSetShaderResources(0, Some(&none_srv));
            }
        }
        Ok(())
    }
}
