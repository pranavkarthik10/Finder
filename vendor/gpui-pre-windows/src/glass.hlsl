// Liquid Glass. The renderer copies the backdrop under a glass surface (and
// the band its rim samples) out of the frame, optionally frosts it with a
// separable blur at half resolution, then composites the surface: the rim
// bends the backdrop toward the center, the body is toned like the material,
// and an even hairline rim sits on top, as on macOS.
//
// Optics follow Glass-HQ/liquid-glass (MIT): a circular bezel profile for the
// displacement and its calibrated tone curves.

cbuffer GlassParams: register(b2) {
    float4 g_shape;    // surface origin.xy, size.xy (device px)
    float4 g_clip;     // content mask origin.xy, size.xy
    float4 g_region;   // copied backdrop origin.xy, size.xy
    float4 g_tint;     // straight sRGB rgba; a is strength
    float4 g_optics;   // corner radius, refraction, bezel (device px), dispersion
    float4 g_misc;     // scale factor, highlight, dark, frosted
    float4 g_blur;     // frosting: sigma (source px), step.xy (source px), dest-to-source scale;
                       // composite: x > 0.5 draws only the shadow
    float4 g_tex;      // source texture size.xy; viewport size.xy, or while
                       // frosting, the copied region's size in source px
    float4 g_shadow;   // soft opacity, soft sigma, soft y offset (device px), ring opacity
    float4 g_mode;     // x > 0.5: a scroll edge rather than glass
};

Texture2D<float4> t_source: register(t0);
SamplerState s_clamp: register(s0);

float sat(float x) { return clamp(x, 0.0, 1.0); }

/*
**  Frost: a fullscreen triangle over the destination viewport.
*/

float4 blur_vertex(uint vertex_id: SV_VertexID): SV_Position {
    float2 uv = float2((vertex_id << 1) & 2, vertex_id & 2);
    return float4(uv * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
}

float4 blur_fragment(float4 position: SV_Position): SV_Target {
    // Pixel centers of a half-size destination land between source texel
    // pairs, so each bilinear tap already averages two of them.
    float2 center = position.xy * g_blur.w;
    float2 step_px = g_blur.yz;
    float sigma = max(g_blur.x, 0.001);
    float step_len = length(step_px);
    int taps = min((int)ceil(3.0 * sigma / step_len), 64);
    float4 sum = 0.0;
    float weight_sum = 0.0;
    for (int i = -taps; i <= taps; i++) {
        float x = i * step_len;
        float w = exp(-x * x / (2.0 * sigma * sigma));
        float2 at = clamp(center + step_px * i, 0.5, g_tex.zw - 0.5);
        sum += w * t_source.SampleLevel(s_clamp, at / g_tex.xy, 0);
        weight_sum += w;
    }
    return sum / weight_sum;
}

/*
**  Composite
*/

struct GlassVertexOutput {
    float4 position: SV_Position;
};

GlassVertexOutput glass_vertex(uint vertex_id: SV_VertexID) {
    float2 unit = float2(float(vertex_id & 1u), 0.5 * float(vertex_id & 2u));
    // Past the outline far enough for its antialiased edge and its shadow.
    float margin = g_shadow.x > 0.0 ? 3.0 * g_shadow.y + g_shadow.z : 2.0;
    float2 origin = g_shape.xy - margin;
    float2 size = g_shape.zw + 2.0 * margin;
    float2 p = origin + unit * size;
    GlassVertexOutput output;
    output.position = float4(p / g_tex.zw * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
    return output;
}

// Signed distance to the outline (negative inside) and the outward normal of
// its nearest point. Corners are continuous, like Apple's: a superellipse
// that starts curving earlier than the circular radius but cuts less off the
// diagonal. Capsules have no room for that and keep circular caps.
float3 shape_field(float2 p, float2 half_size, float radius) {
    float inner = min(half_size.x, half_size.y);
    float reach = min(radius * 1.35, inner);
    float exponent = 2.0 + 2.0 * sat((reach - radius) / max(radius * 0.35, 0.001));
    float2 q = abs(p) - (half_size - reach);
    float d;
    float2 n;
    if (q.x > 0.0 && q.y > 0.0) {
        float2 qn = pow(q / reach, exponent);
        float norm = pow(qn.x + qn.y, 1.0 / exponent);
        // Gradient of the p-norm; its length is below 1 off the axes, so
        // dividing by it turns the level set into a distance.
        float2 grad = pow(q / reach, exponent - 1.0) / pow(max(norm, 0.0001), exponent - 1.0);
        d = (norm - 1.0) * reach / max(length(grad), 0.0001);
        n = normalize(grad);
    } else if (q.x > q.y) {
        d = q.x - reach;
        n = float2(1.0, 0.0);
    } else {
        d = q.y - reach;
        n = float2(0.0, 1.0);
    }
    float2 s = float2(p.x < 0.0 ? -1.0 : 1.0, p.y < 0.0 ? -1.0 : 1.0);
    return float3(d, n * s);
}

float3 sample_backdrop(float2 device_px) {
    // Only the copied region is valid; the rest of the texture is stale.
    float2 local = clamp(device_px - g_region.xy, 0.5, g_region.zw - 0.5);
    // Frosted backdrops live at half resolution.
    if (g_misc.w > 0.5) local *= 0.5;
    return t_source.SampleLevel(s_clamp, local / g_tex.xy, 0).rgb;
}

static const float3 LUMA = float3(0.2126, 0.7152, 0.0722);

float3 saturate_color(float3 c, float amount) {
    float l = dot(c, LUMA);
    return l + (c - l) * amount;
}

float3 tone(float3 v) {
    bool dark = g_misc.z > 0.5;
    if (g_misc.w > 0.5) {
        // Regular: a calibrated tone curve, then a chroma boost.
        float3 curve = dark ? float3(0.105, 0.83, -0.2091) : float3(0.36, 0.67, -0.1113);
        float weight = dark ? 0.90 : 0.80;
        float3 toned = curve.x + curve.y * v + curve.z * v * v;
        float3 c = weight * toned + (1.0 - weight) * v;
        c = saturate(saturate_color(c, dark ? 1.0648 : 1.1031));
        // macOS toolbar glass is milkier than iOS: near white over white
        // content in the light, a deep gray in the dark.
        return lerp(c, dark ? float3(0.16, 0.16, 0.17) : float3(1.0, 1.0, 1.0), 0.45);
    }
    // Clear: lift the shadows, keep a faint broad veil.
    float lift = dark ? 0.15 : 0.18;
    float rolloff = dark ? 0.20 : 0.0;
    float3 t = v + lift * pow(1.0 - v, 3.0) - rolloff * v * v * v;
    float3 veil = dark ? 0.38 * v + 0.20 : 0.78 * v + 0.26;
    float veil_weight = dark ? 0.14 : 0.06;
    return saturate((1.0 - veil_weight) * t + veil_weight * veil);
}

float4 glass_fragment(GlassVertexOutput input): SV_Target {
    float2 px = input.position.xy;
    if (px.x < g_clip.x || px.y < g_clip.y || px.x > g_clip.x + g_clip.z || px.y > g_clip.y + g_clip.w)
        discard;

    if (g_mode.x > 0.5) {
        // Scroll edge: a flat, milky veil over blurred content, as macOS
        // lays under a toolbar once content scrolls beneath it.
        if (px.x < g_shape.x || px.x > g_shape.x + g_shape.z || px.y < g_shape.y || px.y > g_shape.y + g_shape.w)
            discard;
        float3 c = lerp(sample_backdrop(px), g_tint.rgb, g_tint.a);
        return float4(c, 1.0);
    }

    float scale = g_misc.x;
    float2 half_size = g_shape.zw * 0.5;
    float radius = min(g_optics.x, min(half_size.x, half_size.y));
    float3 field = shape_field(px - (g_shape.xy + half_size), half_size, radius);
    // The light model below is tuned in logical pixels.
    float d = field.x / scale;
    float2 n = field.yz;
    float aa = 1.0 / scale;
    float coverage = sat(0.5 - d / aa);

    // The shadow, under and around the outline.
    float2 lowered = px - float2(0.0, g_shadow.z) - (g_shape.xy + half_size);
    float sd = max(shape_field(lowered, half_size, radius).x, 0.0);
    float soft = g_shadow.x * exp(-sd * sd / (2.0 * g_shadow.y * g_shadow.y));
    float ring = g_shadow.w * sat(1.0 - max(field.x, 0.0) / scale);
    float shadow = sat(soft + ring) * (1.0 - coverage);
    // Shadows are drawn for every surface in a batch before any glass.
    if (g_blur.x > 0.5) {
        if (shadow <= 0.0) discard;
        return float4(0.0, 0.0, 0.0, shadow);
    }
    if (coverage <= 0.0) discard;

    // Circular bezel: full displacement at the outline, none past the bezel.
    // Small controls are thin glass: neither may reach past the middle.
    float inner = min(half_size.x, half_size.y);
    float bezel = min(g_optics.z, inner);
    float refraction = min(g_optics.y, 0.75 * inner);
    float depth = sat(-field.x / max(bezel, 0.001));
    float edge = 1.0 - sat(sqrt((2.0 - depth) * depth));
    float2 offset = -n * refraction * edge;
    float spread = g_optics.w;
    float3 color = float3(
        sample_backdrop(px + offset * (1.0 + spread)).r,
        sample_backdrop(px + offset).g,
        sample_backdrop(px + offset * (1.0 - spread)).b);
    color = tone(color);

    // Pigment is transmitted by luminance, so pale backdrops keep its chroma.
    if (g_tint.a > 0.0) {
        float body = g_misc.w > 0.5 ? 0.78 : 0.68;
        float3 tinted = g_tint.rgb * (body + (1.0 - body) * dot(color, LUMA));
        color = lerp(color, tinted, g_tint.a);
    }

    // The rim: one even hairline just inside the outline, as macOS draws
    // its toolbar glass (darker in the light, lighter in the dark), rather
    // than iOS's directional specular and side shade.
    bool dark = g_misc.z > 0.5;
    float rim = sat((d + 1.0) / aa + 0.5) * coverage;
    color = lerp(color, dark ? float3(1.0, 1.0, 1.0) : float3(0.0, 0.0, 0.0), rim * (dark ? 0.16 : 0.10) * g_misc.y);

    return float4(color, coverage);
}
