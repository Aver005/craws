// WebGPU viewport: the pipeline output lives in one rgba8 texture; pan/zoom is
// a pure GPU transform (uniforms + one textured quad), so navigation never
// touches the bridge. This is the M3 architecture in miniature — a tile-atlas
// version replaces the single texture when mip streaming lands.

// The fragment shader can apply pointwise ops (exposure, grayscale) itself, in
// linear light, on top of the neutral base frame. This is the "optimistic
// preview": during a slider drag the authoritative engine is NOT consulted —
// only a uniform changes — so interaction runs at the GPU draw rate, and the
// engine reconciles once on release. Exposure/grayscale are exact this way
// (same math as the engine), modulo the 8-bit base quantization.
const SHADER = /* wgsl */ `
struct U {
  canvas:  vec2f, // canvas size, physical px
  size:    vec2f, // image size * zoom, physical px
  offset:  vec2f, // pan, physical px
  stops:   f32,   // preview exposure (0 = show base as-is)
  gray:    f32,   // preview grayscale flag (>0.5 = on)
};
@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var tex: texture_2d<f32>;

struct VSOut {
  @builtin(position) pos: vec4f,
  @location(0) uv: vec2f,
};

@vertex
fn vs(@builtin(vertex_index) i: u32) -> VSOut {
  var corners = array<vec2f, 6>(
    vec2f(0, 0), vec2f(1, 0), vec2f(0, 1),
    vec2f(1, 0), vec2f(1, 1), vec2f(0, 1),
  );
  let c = corners[i];
  let px = c * u.size + u.offset;
  let clip = vec2f(px.x / u.canvas.x * 2.0 - 1.0, 1.0 - px.y / u.canvas.y * 2.0);
  var out: VSOut;
  out.pos = vec4f(clip, 0.0, 1.0);
  out.uv = c;
  return out;
}

fn srgb_to_linear(c: vec3f) -> vec3f {
  let low = c / 12.92;
  let high = pow((c + 0.055) / 1.055, vec3f(2.4));
  return select(high, low, c <= vec3f(0.04045));
}
fn linear_to_srgb(c: vec3f) -> vec3f {
  let low = c * 12.92;
  let high = 1.055 * pow(c, vec3f(1.0 / 2.4)) - 0.055;
  return select(high, low, c <= vec3f(0.0031308));
}

@fragment
fn fs(in: VSOut) -> @location(0) vec4f {
  let s = textureSample(tex, samp, in.uv);
  // fast path: no preview transform → passthrough (authoritative frame already baked)
  if (u.stops == 0.0 && u.gray < 0.5) {
    return s;
  }
  var lin = srgb_to_linear(s.rgb) * exp2(u.stops);
  if (u.gray > 0.5) {
    lin = vec3f(dot(lin, vec3f(0.2126, 0.7152, 0.0722)));
  }
  return vec4f(linear_to_srgb(clamp(lin, vec3f(0.0), vec3f(1.0))), s.a);
}
`;

export interface View {
  zoom: number;
  panX: number;
  panY: number;
  /** preview exposure applied in-shader (0 when the frame is authoritative) */
  previewStops: number;
  /** preview grayscale applied in-shader */
  previewGray: boolean;
}

export class Viewport {
  private device: GPUDevice;
  private context: GPUCanvasContext;
  private pipeline: GPURenderPipeline;
  private uniforms: GPUBuffer;
  private nearest: GPUSampler;
  private linear: GPUSampler;
  private texture: GPUTexture | null = null;
  private bindNearest: GPUBindGroup | null = null;
  private bindLinear: GPUBindGroup | null = null;
  private canvas: HTMLCanvasElement;
  imageWidth = 0;
  imageHeight = 0;

  private constructor(canvas: HTMLCanvasElement, device: GPUDevice, context: GPUCanvasContext) {
    this.canvas = canvas;
    this.device = device;
    this.context = context;
    const format = navigator.gpu.getPreferredCanvasFormat();
    context.configure({ device, format, alphaMode: "opaque" });

    const module = device.createShaderModule({ code: SHADER });
    this.pipeline = device.createRenderPipeline({
      layout: "auto",
      vertex: { module, entryPoint: "vs" },
      fragment: { module, entryPoint: "fs", targets: [{ format }] },
      primitive: { topology: "triangle-list" },
    });
    this.uniforms = device.createBuffer({
      size: 8 * 4,
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });
    this.nearest = device.createSampler({ magFilter: "nearest", minFilter: "linear" });
    this.linear = device.createSampler({ magFilter: "linear", minFilter: "linear" });
  }

  /** Throws with a readable message when WebGPU is unavailable — that outcome
   *  is itself a spike result and must be loud, not a silent white screen. */
  static async create(canvas: HTMLCanvasElement): Promise<Viewport> {
    if (!navigator.gpu) throw new Error("WebGPU is not available in this WebView2 runtime");
    const adapter = await navigator.gpu.requestAdapter();
    if (!adapter) throw new Error("WebGPU adapter request failed");
    const device = await adapter.requestDevice();
    const context = canvas.getContext("webgpu");
    if (!context) throw new Error("webgpu canvas context unavailable");
    return new Viewport(canvas, device, context);
  }

  setImageSize(width: number, height: number) {
    if (this.texture && width === this.imageWidth && height === this.imageHeight) return;
    this.texture?.destroy();
    this.imageWidth = width;
    this.imageHeight = height;
    this.texture = this.device.createTexture({
      size: { width, height },
      format: "rgba8unorm",
      usage: GPUTextureUsage.TEXTURE_BINDING | GPUTextureUsage.COPY_DST,
    });
    const entries = (samp: GPUSampler): GPUBindGroupEntry[] => [
      { binding: 0, resource: { buffer: this.uniforms } },
      { binding: 1, resource: samp },
      { binding: 2, resource: this.texture!.createView() },
    ];
    this.bindNearest = this.device.createBindGroup({
      layout: this.pipeline.getBindGroupLayout(0),
      entries: entries(this.nearest),
    });
    this.bindLinear = this.device.createBindGroup({
      layout: this.pipeline.getBindGroupLayout(0),
      entries: entries(this.linear),
    });
  }

  /** Full-frame upload. (Changed-tiles-only `writeTexture` per region is the
   *  planned refinement; the spike measures the worst case on purpose.) */
  upload(pixels: Uint8Array, width: number, height: number) {
    this.setImageSize(width, height);
    this.device.queue.writeTexture(
      { texture: this.texture! },
      pixels as BufferSource,
      { bytesPerRow: width * 4, rowsPerImage: height },
      { width, height },
    );
  }

  /** One draw. Costs ~nothing; called from rAF. */
  draw(view: View) {
    if (!this.texture) return;
    const dpr = window.devicePixelRatio || 1;
    const cw = Math.max(1, Math.round(this.canvas.clientWidth * dpr));
    const ch = Math.max(1, Math.round(this.canvas.clientHeight * dpr));
    if (this.canvas.width !== cw || this.canvas.height !== ch) {
      this.canvas.width = cw;
      this.canvas.height = ch;
    }

    const u = new Float32Array([
      cw, ch,
      this.imageWidth * view.zoom, this.imageHeight * view.zoom,
      view.panX, view.panY,
      view.previewStops, view.previewGray ? 1 : 0,
    ]);
    this.device.queue.writeBuffer(this.uniforms, 0, u);

    const encoder = this.device.createCommandEncoder();
    const pass = encoder.beginRenderPass({
      colorAttachments: [
        {
          view: this.context.getCurrentTexture().createView(),
          clearValue: { r: 0.04, g: 0.04, b: 0.04, a: 1 },
          loadOp: "clear",
          storeOp: "store",
        },
      ],
    });
    pass.setPipeline(this.pipeline);
    pass.setBindGroup(0, view.zoom >= 1 ? this.bindNearest! : this.bindLinear!);
    pass.draw(6);
    pass.end();
    this.device.queue.submit([encoder.finish()]);
  }
}
