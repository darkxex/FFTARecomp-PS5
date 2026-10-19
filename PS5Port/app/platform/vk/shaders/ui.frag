// FFTARecomp - the interface drawn by the GPU: the texture times the vertex colour.
// A draw whose u coordinates are offset by 2 asks for pixel art: every texel stays a crisp block and
// only the seams between blocks are blended, so uneven scaling does not shimmer. An offset of 4 asks
// for the same through the MMPX 3.2 filter without edge blending, an offset of 6 with it.
// An offset of 8 + 2 * mode asks for the scaling and grid pass of the PC build (grid.wgsl, ported): mode 0 exact
// area average, 1 nearest, 2 sharp bilinear, 3 EPX; the vertex colour's red is the grid strength (0..1).
// SPDX-License-Identifier: GPL-3.0-or-later
// Compiled into shaders_ui_spv.h by tools/make-ui-shaders.sh.
#version 450
#extension GL_GOOGLE_include_directive : require

layout(set = 0, binding = 0) uniform sampler2D picture;
layout(location = 0) in vec2 in_uv;
layout(location = 1) in vec4 in_colour;
layout(location = 0) out vec4 out_color;

#include "mmpx32.glsl"

// ---- the scaling and grid pass of the PC build (crates/screen/src/grid.wgsl) -------------------------------
const float GRID_GAIN = 1.5, GRID_GAMMA = 2.2, GRID_TINT = 0.75, GRID_STRIPE_D = 1.5, GRID_ROW_D = 0.63;

vec3 gfetch(ivec2 c)
{
    ivec2 sz = textureSize(picture, 0);
    return texelFetch(picture, clamp(c, ivec2(0), sz - ivec2(1)), 0).rgb;
}

float smear_f_x(float z)
{
    float z2 = z * z, zn = z, ret = 0.0;
    float co[7] = float[7](1.0, -2.0 / 3.0, -0.2, 4.0 / 7.0, -1.0 / 9.0, -2.0 / 11.0, 1.0 / 13.0);
    for (int i = 0; i < 7; i++) { ret += zn * co[i]; zn *= z2; }
    return ret;
}

float smear_f_y(float z)
{
    float z2 = z * z, zn = z, ret = 0.0;
    float co[7] = float[7](1.0, 0.0, -4.0 / 5.0, 2.0 / 7.0, 4.0 / 9.0, -4.0 / 11.0, 1.0 / 13.0);
    for (int i = 0; i < 7; i++) { ret += zn * co[i]; zn *= z2; }
    return ret;
}

float smear_x(float x, float dx, float d)
{
    float zl = clamp((x - dx * 0.5) / d, -1.0, 1.0);
    float zh = clamp((x + dx * 0.5) / d, -1.0, 1.0);
    return d * (smear_f_x(zh) - smear_f_x(zl)) / dx;
}

float smear_y(float x, float dx, float d)
{
    float zl = clamp((x - dx * 0.5) / d, -1.0, 1.0);
    float zh = clamp((x + dx * 0.5) / d, -1.0, 1.0);
    return d * (smear_f_y(zh) - smear_f_y(zl)) / dx;
}

float box_overlap(float j, float p, float du)
{
    float lo = max(j, p - du * 0.5);
    float hi = min(j + 1.0, p + du * 0.5);
    return max(hi - lo, 0.0) / du;
}

vec3 flat_sample(vec2 p, vec2 du)
{
    int ix = int(floor(p.x)), iy = int(floor(p.y));
    vec3 acc = vec3(0.0);
    for (int dy = -1; dy <= 1; dy++)
    {
        float wy = box_overlap(float(iy + dy), p.y, du.y);
        if (wy <= 0.0) continue;
        for (int dx = -1; dx <= 1; dx++)
        {
            float wx = box_overlap(float(ix + dx), p.x, du.x);
            if (wx <= 0.0) continue;
            acc += gfetch(ivec2(ix + dx, iy + dy)) * wx * wy;
        }
    }
    return acc;
}

vec3 nearest_sample(vec2 p) { return gfetch(ivec2(floor(p))); }

vec3 sharp_sample(vec2 p, vec2 scale)
{
    vec2 e = floor(p + 0.5);
    vec2 w = clamp((p - e) * scale + vec2(0.5), vec2(0.0), vec2(1.0));
    ivec2 ei = ivec2(e);
    vec3 top = mix(gfetch(ei + ivec2(-1, -1)), gfetch(ei + ivec2(0, -1)), w.x);
    vec3 bot = mix(gfetch(ei + ivec2(-1, 0)), gfetch(ei), w.x);
    return mix(top, bot, w.y);
}

vec3 epx_px(int x, int y)
{
    int ix = x >> 1, iy = y >> 1, qx = x & 1, qy = y & 1;
    vec3 m = gfetch(ivec2(ix, iy));
    vec3 a = gfetch(ivec2(ix, iy - 1)); // up
    vec3 b = gfetch(ivec2(ix + 1, iy)); // right
    vec3 c = gfetch(ivec2(ix - 1, iy)); // left
    vec3 d = gfetch(ivec2(ix, iy + 1)); // down
    vec3 r = m;
    if (qx == 0 && qy == 0) { if (all(equal(c, a)) && any(notEqual(c, d)) && any(notEqual(a, b))) r = a; }
    else if (qx == 1 && qy == 0) { if (all(equal(a, b)) && any(notEqual(a, c)) && any(notEqual(b, d))) r = b; }
    else if (qx == 0 && qy == 1) { if (all(equal(d, c)) && any(notEqual(d, b)) && any(notEqual(c, a))) r = c; }
    else { if (all(equal(b, d)) && any(notEqual(b, a)) && any(notEqual(d, c))) r = d; }
    return r;
}

vec3 epx_sample(vec2 p, vec2 scale)
{
    vec2 p2 = p * 2.0, s2 = scale * 0.5;
    vec2 e = floor(p2 + 0.5);
    vec2 w = clamp((p2 - e) * s2 + vec2(0.5), vec2(0.0), vec2(1.0));
    ivec2 ei = ivec2(e);
    vec3 top = mix(epx_px(ei.x - 1, ei.y - 1), epx_px(ei.x, ei.y - 1), w.x);
    vec3 bot = mix(epx_px(ei.x - 1, ei.y), epx_px(ei.x, ei.y), w.x);
    return mix(top, bot, w.y);
}

vec3 grid_sample(vec2 p, vec2 du)
{
    float sx = p.x * 3.0, dux = du.x * 3.0;
    int ix = int(floor(p.x)), iy0 = int(floor(p.y - 0.5));
    vec3 acc = vec3(0.0);
    for (int ry = 0; ry <= 1; ry++)
    {
        int jy = iy0 + ry;
        float wy = smear_y((float(jy) + 0.5) - p.y, du.y, GRID_ROW_D);
        if (wy <= 0.0) continue;
        for (int rx = -1; rx <= 1; rx++)
        {
            int jx = ix + rx;
            vec3 lin = pow(max(gfetch(ivec2(jx, jy)) * GRID_GAIN, vec3(0.0)), vec3(GRID_GAMMA));
            for (int ch = 0; ch < 3; ch++)
            {
                float stripe = float(2 - ch); // the panel's stripe order is BGR
                float center = float(jx) * 3.0 + stripe + 0.5;
                acc[ch] += lin[ch] * smear_x(center - sx, dux, GRID_STRIPE_D) * wy;
            }
        }
    }
    return pow(max(acc * GRID_TINT, vec3(0.0)), vec3(1.0 / GRID_GAMMA));
}

vec4 scaled_grid(vec2 uv, int mode, float strength)
{
    vec2 size = vec2(textureSize(picture, 0));
    vec2 p = uv * size;
    vec2 step_texels = max(fwidth(in_uv * size), vec2(1e-4)); // texels per screen pixel
    vec2 scale = max(1.0 / step_texels, vec2(1.0)); // screen pixels per texel
    vec2 du = 1.0 / scale;
    vec3 flat_c;
    if (mode == 3) flat_c = epx_sample(p, scale);
    else if (mode == 2) flat_c = sharp_sample(p, scale);
    else if (mode == 1) flat_c = nearest_sample(p);
    else flat_c = flat_sample(p, du);
    // below about 2x the grid is sub-Nyquist: faded out; EPX replaces the pixels themselves, so it has none
    float st = strength * smoothstep(1.5, 2.5, min(scale.x, scale.y));
    if (mode == 3) st = 0.0;
    vec3 rgb = flat_c;
    if (st > 0.0)
    {
        float gk = mode == 2 ? 0.5 : (mode == 1 ? 0.05 : 1.0);
        rgb = mix(flat_c, grid_sample(p, du * gk), st);
    }
    return vec4(rgb, 1.0);
}

void main()
{
    vec2 uv = in_uv;
    if (uv.x > 7.5)
    {
        int mode = int(floor((uv.x - 8.0) * 0.5));
        uv.x -= 8.0 + 2.0 * float(mode);
        out_color = scaled_grid(uv, mode, in_colour.r);
        return;
    }
    vec2 size = vec2(textureSize(picture, 0));
    vec2 step_texels = fwidth(vec2(fract(in_uv.x * 0.25) * 4.0, in_uv.y) * size); // texels per screen pixel
    if (uv.x > 3.5)
    {
        bool blend = uv.x > 5.5; // MMPX 3.2 with edge blending; otherwise without
        vec4 colour;
        float alpha;
        uv.x -= blend ? 6.0 : 4.0;
        vec2 coords = uv * size;
        MMPXCore(coords, colour, alpha);
        if (blend)
        {
            // the edges of a shape on a transparent background are blended with the four texels around
            vec2 bc = coords - vec2(0.5);
            vec2 b0 = floor(bc);
            vec2 f = bc - b0;
            vec4 t00 = mmpxFetch(b0);
            vec4 t10 = mmpxFetch(b0 + vec2(1.0, 0.0));
            vec4 t01 = mmpxFetch(b0 + vec2(0.0, 1.0));
            vec4 t11 = mmpxFetch(b0 + vec2(1.0, 1.0));
            float w00 = (1.0 - f.x) * (1.0 - f.y);
            float w10 = f.x * (1.0 - f.y);
            float w01 = (1.0 - f.x) * f.y;
            float w11 = f.x * f.y;
            bool n00 = any(notEqual(t00, vec4(0.0)));
            bool n10 = any(notEqual(t10, vec4(0.0)));
            bool n01 = any(notEqual(t01, vec4(0.0)));
            bool n11 = any(notEqual(t11, vec4(0.0)));
            float a = (n00 ? w00 : 0.0) + (n10 ? w10 : 0.0) + (n01 ? w01 : 0.0) + (n11 ? w11 : 0.0);
            if (alpha <= 0.0 && a > 0.0)
            {
                float best = -1.0;
                if (n00 && w00 > best) { best = w00; colour = t00; }
                if (n10 && w10 > best) { best = w10; colour = t10; }
                if (n01 && w01 > best) { best = w01; colour = t01; }
                if (n11 && w11 > best) { best = w11; colour = t11; }
            }
            alpha = a;
        }
        out_color = vec4(colour.rgb, colour.a * alpha) * in_colour;
        return;
    }
    if (uv.x > 1.5)
    {
        uv.x -= 2.0;
        vec2 texel = uv * size;
        vec2 scale = max(1.0 / max(step_texels, vec2(1e-4)), vec2(1.0)); // screen pixels per texel
        vec2 base = floor(texel);
        vec2 offset = fract(texel) - 0.5;
        vec2 region = 0.5 - 0.5 / scale;
        vec2 f = (offset - clamp(offset, -region, region)) * scale + 0.5;
        uv = (base + f) / size;
    }
    out_color = texture(picture, uv) * in_colour;
}
