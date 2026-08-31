// One instanced unit quad, used twice: once for solid rectangles and once for
// glyphs sampled from the atlas.

struct Screen {
    size: vec2<f32>,
};

@group(0) @binding(0) var<uniform> screen: Screen;

struct RectIn {
    @location(0) position: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) color: vec4<f32>,
};

struct RectOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
};

// The four corners of a unit quad, as a triangle strip.
fn corner(index: u32) -> vec2<f32> {
    let x = f32(index & 1u);
    let y = f32((index >> 1u) & 1u);
    return vec2<f32>(x, y);
}

// Pixels from the top left into clip space, which has y upwards.
fn to_clip(pixels: vec2<f32>) -> vec4<f32> {
    let ndc = vec2<f32>(
        pixels.x / screen.size.x * 2.0 - 1.0,
        1.0 - pixels.y / screen.size.y * 2.0,
    );
    return vec4<f32>(ndc, 0.0, 1.0);
}

@vertex
fn rect_vertex(@builtin(vertex_index) vertex: u32, in: RectIn) -> RectOut {
    var out: RectOut;
    out.clip = to_clip(in.position + corner(vertex) * in.size);
    out.color = in.color;
    return out;
}

@fragment
fn rect_fragment(in: RectOut) -> @location(0) vec4<f32> {
    return in.color;
}

// A quadrilateral given as its four corners rather than as a position and a
// size, because the cursor's smear is not upright: while it travels its
// corners are at different points along the journey, and that shape has no
// width and height to be given.
struct QuadIn {
    @location(0) top_left: vec2<f32>,
    @location(1) top_right: vec2<f32>,
    @location(2) bottom_left: vec2<f32>,
    @location(3) bottom_right: vec2<f32>,
    @location(4) color: vec4<f32>,
};

@vertex
fn quad_vertex(@builtin(vertex_index) vertex: u32, in: QuadIn) -> RectOut {
    var out: RectOut;
    // The same winding `corner` gives, so the triangle strip covers the same
    // shape: x along the bottom bit, y along the one above it.
    var at: vec2<f32>;
    switch vertex {
        case 0u: { at = in.top_left; }
        case 1u: { at = in.top_right; }
        case 2u: { at = in.bottom_left; }
        default: { at = in.bottom_right; }
    }
    out.clip = to_clip(at);
    out.color = in.color;
    return out;
}

struct SpriteIn {
    @location(0) position: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) source: vec2<f32>,
    @location(3) source_size: vec2<f32>,
    @location(4) color: vec4<f32>,
};

struct SpriteOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@group(1) @binding(0) var atlas: texture_2d<f32>;
@group(1) @binding(1) var atlas_sampler: sampler;

@vertex
fn sprite_vertex(@builtin(vertex_index) vertex: u32, in: SpriteIn) -> SpriteOut {
    var out: SpriteOut;
    let at = corner(vertex);
    out.clip = to_clip(in.position + at * in.size);
    let atlas_size = vec2<f32>(textureDimensions(atlas));
    out.uv = (in.source + at * in.source_size) / atlas_size;
    out.color = in.color;
    return out;
}

@fragment
fn sprite_fragment(in: SpriteOut) -> @location(0) vec4<f32> {
    // The atlas holds coverage, not colour: the glyph's shape times the
    // face's foreground.
    let coverage = textureSample(atlas, atlas_sampler, in.uv).r;
    return vec4<f32>(in.color.rgb, in.color.a * coverage);
}
