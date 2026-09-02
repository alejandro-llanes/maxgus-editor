// One instanced unit quad, used twice: once for solid rectangles and once for
// glyphs sampled from the atlas.

struct Screen {
    size: vec2<f32>,
    // Where the grid starts, in pixels from the window's top left.
    origin: vec2<f32>,
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

// Pixels from the grid's top left into clip space, which has y upwards.
fn to_clip(pixels: vec2<f32>) -> vec4<f32> {
    let at = pixels + screen.origin;
    let ndc = vec2<f32>(
        at.x / screen.size.x * 2.0 - 1.0,
        1.0 - at.y / screen.size.y * 2.0,
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

// A disc, filled or as a ring. The cursor's particle effects are made of
// these, and drawing them as geometry rather than as a texture is what keeps
// their edges smooth at any size.
struct CircleIn {
    @location(0) center: vec2<f32>,
    @location(1) radius: f32,
    // Zero or less fills the disc; above zero draws a ring that thick.
    @location(2) thickness: f32,
    @location(3) color: vec4<f32>,
};

struct CircleOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) shape: vec2<f32>,
};

@vertex
fn circle_vertex(@builtin(vertex_index) vertex: u32, in: CircleIn) -> CircleOut {
    var out: CircleOut;
    // The unit quad, about the centre rather than from a corner.
    let at = corner(vertex) * 2.0 - vec2<f32>(1.0, 1.0);
    // A pixel of margin, so the edge has somewhere to fade out into.
    let reach = in.radius + max(in.thickness, 0.0) * 0.5 + 1.0;
    out.clip = to_clip(in.center + at * reach);
    out.local = at * reach;
    out.color = in.color;
    out.shape = vec2<f32>(in.radius, in.thickness);
    return out;
}

@fragment
fn circle_fragment(in: CircleOut) -> @location(0) vec4<f32> {
    let distance = length(in.local);
    let radius = in.shape.x;
    let thickness = in.shape.y;
    var coverage: f32;
    if thickness <= 0.0 {
        coverage = 1.0 - smoothstep(radius - 1.0, radius + 1.0, distance);
    } else {
        let inner = radius - thickness * 0.5;
        let outer = radius + thickness * 0.5;
        coverage = smoothstep(inner - 1.0, inner + 1.0, distance)
            * (1.0 - smoothstep(outer - 1.0, outer + 1.0, distance));
    }
    return vec4<f32>(in.color.rgb, in.color.a * coverage);
}

// A pass over the whole target, sampling another one. Used twice for the
// blur behind a popup — once across, once down, because a two-dimensional
// gaussian is two one-dimensional ones and doing it that way is a handful of
// samples rather than their square.
struct Blur {
    // How far one texel is, along the axis this pass blurs.
    step: vec2<f32>,
    radius: f32,
    pad: f32,
};

@group(0) @binding(0) var<uniform> blur: Blur;
@group(1) @binding(0) var source: texture_2d<f32>;
@group(1) @binding(1) var source_sampler: sampler;

struct BlitOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn blit_vertex(@builtin(vertex_index) vertex: u32) -> BlitOut {
    var out: BlitOut;
    let at = corner(vertex);
    out.clip = vec4<f32>(at.x * 2.0 - 1.0, 1.0 - at.y * 2.0, 0.0, 1.0);
    out.uv = at;
    return out;
}

@fragment
fn blur_fragment(in: BlitOut) -> @location(0) vec4<f32> {
    // Nine taps, weighted by a gaussian whose spread is the radius asked
    // for. Nine is enough at the sizes a popup border is drawn at, and the
    // cost of a blur is the tap count times two passes times the area.
    var total = vec4<f32>(0.0);
    var weight = 0.0;
    let sigma = max(blur.radius, 0.0001) * 0.5;
    for (var i = -4; i <= 4; i = i + 1) {
        let offset = f32(i) * blur.radius * 0.25;
        let w = exp(-(offset * offset) / (2.0 * sigma * sigma));
        total = total + textureSample(source, source_sampler, in.uv + blur.step * offset) * w;
        weight = weight + w;
    }
    return total / weight;
}

@fragment
fn blit_fragment(in: BlitOut) -> @location(0) vec4<f32> {
    return textureSample(source, source_sampler, in.uv);
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
    // A shape is stored white with its coverage for alpha, so this is the
    // face's foreground cut to the glyph; a picture is stored as it is, and
    // comes with white for a colour, so it is the picture.
    let texel = textureSample(atlas, atlas_sampler, in.uv);
    return vec4<f32>(in.color.rgb * texel.rgb, in.color.a * texel.a);
}

// A card: a rounded rectangle with a border, its fill a tint over whatever
// the blur pass left behind it. The shape is a signed distance, so the
// corners are round at any radius and the edge is a pixel of fade rather
// than a stair.
struct PanelIn {
    @location(0) position: vec2<f32>,
    @location(1) size: vec2<f32>,
    // The corners' radius, and the border's thickness.
    @location(2) shape: vec2<f32>,
    @location(3) fill: vec4<f32>,
    @location(4) border: vec4<f32>,
};

struct PanelOut {
    @builtin(position) clip: vec4<f32>,
    // Pixels from the card's centre.
    @location(0) local: vec2<f32>,
    @location(1) half: vec2<f32>,
    @location(2) shape: vec2<f32>,
    @location(3) fill: vec4<f32>,
    @location(4) border: vec4<f32>,
};

@vertex
fn panel_vertex(@builtin(vertex_index) vertex: u32, in: PanelIn) -> PanelOut {
    var out: PanelOut;
    let at = corner(vertex);
    out.clip = to_clip(in.position + at * in.size);
    out.local = (at - vec2<f32>(0.5, 0.5)) * in.size;
    out.half = in.size * 0.5;
    out.shape = in.shape;
    out.fill = in.fill;
    out.border = in.border;
    return out;
}

// How far outside the rounded rectangle a point is; negative inside.
fn rounded_distance(local: vec2<f32>, half: vec2<f32>, radius: f32) -> f32 {
    let q = abs(local) - (half - vec2<f32>(radius, radius));
    return length(max(q, vec2<f32>(0.0, 0.0))) + min(max(q.x, q.y), 0.0) - radius;
}

// The card's colour at a point, given what is behind it.
fn panel_colour(in: PanelOut, fill: vec4<f32>, behind: vec3<f32>) -> vec4<f32> {
    let distance = rounded_distance(in.local, in.half, in.shape.x);
    let inside = 1.0 - smoothstep(-0.5, 0.5, distance);
    let edge = smoothstep(-in.shape.y - 0.5, -in.shape.y + 0.5, distance);
    let filled = mix(behind, fill.rgb, fill.a);
    let colour = mix(filled, in.border.rgb, edge * in.border.a);
    return vec4<f32>(colour, inside);
}

@fragment
fn panel_fragment(in: PanelOut) -> @location(0) vec4<f32> {
    let behind = textureSample(source, source_sampler, in.clip.xy / screen.size);
    return panel_colour(in, in.fill, behind.rgb);
}

// The same card when nothing was blurred: the tint is the fill, solid.
@fragment
fn panel_plain_fragment(in: PanelOut) -> @location(0) vec4<f32> {
    return panel_colour(in, vec4<f32>(in.fill.rgb, 1.0), in.fill.rgb);
}
