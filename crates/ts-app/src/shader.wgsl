// ThreadSmith viewport shaders.

struct Globals {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    // xyz = direction the key light shines FROM, w unused
    light_dir: vec4<f32>,
    wire_color: vec4<f32>,
    // Soft floor shadow: centre x, floor y, centre z, radius (0 = off)
    shadow: vec4<f32>,
    // Selection outline: rgb, width (fraction of camera distance)
    outline: vec4<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;

// Per mesh part: base colour texture (1x1 white when the part has none).
@group(1) @binding(0) var base_tex: texture_2d<f32>;
@group(1) @binding(1) var base_sampler: sampler;

struct PartLook {
    // hue turn (0..1), saturation, brightness, 1 if textured
    adjust: vec4<f32>,
    // roughness, metallic
    surface: vec4<f32>,
};
@group(1) @binding(2) var<uniform> look: PartLook;

fn to_srgb(c: vec3<f32>) -> vec3<f32> {
    return pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    return pow(max(c, vec3<f32>(0.0)), vec3<f32>(2.2));
}

// Hue / saturation / brightness change on a linear colour (done in sRGB,
// like the export bake in ts-core).
fn adjust_color(c: vec3<f32>) -> vec3<f32> {
    if (abs(look.adjust.x) < 0.00003 && abs(look.adjust.y - 1.0) < 0.001 && abs(look.adjust.z - 1.0) < 0.001) {
        return c;
    }
    let s = to_srgb(c);
    let mx = max(s.r, max(s.g, s.b));
    let mn = min(s.r, min(s.g, s.b));
    let d = mx - mn;
    var h = 0.0;
    if (d > 0.000001) {
        if (mx == s.r) {
            h = (s.g - s.b) / d;
            h = h - 6.0 * floor(h / 6.0);
        } else if (mx == s.g) {
            h = (s.b - s.r) / d + 2.0;
        } else {
            h = (s.r - s.g) / d + 4.0;
        }
        h = h / 6.0;
    }
    var sat = 0.0;
    if (mx > 0.000001) {
        sat = d / mx;
    }
    h = fract(h + look.adjust.x);
    sat = clamp(sat * look.adjust.y, 0.0, 1.0);
    let v = clamp(mx * look.adjust.z, 0.0, 1.0);
    let k = (vec3<f32>(5.0, 3.0, 1.0) + vec3<f32>(h * 6.0)) % vec3<f32>(6.0);
    let rgb = vec3<f32>(v) - v * sat * clamp(min(k, vec3<f32>(4.0) - k), vec3<f32>(0.0), vec3<f32>(1.0));
    return to_linear(rgb);
}

// ---- Shaded meshes ----------------------------------------------------------

struct MeshIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    @location(3) uv: vec2<f32>,
};

struct MeshOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec3<f32>,
    @location(2) world: vec3<f32>,
    @location(3) uv: vec2<f32>,
};

@vertex
fn vs_mesh(v: MeshIn) -> MeshOut {
    var o: MeshOut;
    o.clip = g.view_proj * vec4<f32>(v.pos, 1.0);
    o.normal = v.normal;
    o.color = v.color;
    o.world = v.pos;
    o.uv = v.uv;
    return o;
}

// Simple studio lighting: warm key light, cool fill, sky/ground ambient, rim.
fn studio(albedo_in: vec3<f32>, n: vec3<f32>, world: vec3<f32>) -> vec3<f32> {
    let roughness = clamp(look.surface.x, 0.0, 1.0);
    let metallic = clamp(look.surface.y, 0.0, 1.0);
    // Metal has no diffuse colour; its shine takes the colour instead.
    let albedo = albedo_in * (1.0 - metallic * 0.85);
    let spec_tint = mix(vec3<f32>(0.75, 0.82, 0.95), albedo_in * 1.6 + vec3<f32>(0.05), metallic);
    let view_dir = normalize(g.camera_pos.xyz - world);
    let l = normalize(g.light_dir.xyz);
    let wrap = 0.25;
    let key = max((dot(n, l) + wrap) / (1.0 + wrap), 0.0);
    let fill = max(dot(n, normalize(vec3<f32>(-0.7, 0.25, -0.35))), 0.0);
    let sky = mix(vec3<f32>(0.20, 0.19, 0.18), vec3<f32>(0.42, 0.46, 0.55), 0.5 + 0.5 * n.y);
    let h = normalize(l + view_dir);
    let shine = mix(90.0, 6.0, roughness);
    let spec = pow(max(dot(n, h), 0.0), shine) * mix(0.45, 0.05, roughness) * (1.0 + metallic * 2.0);
    let rim = pow(1.0 - max(dot(n, view_dir), 0.0), 3.0) * 0.18;
    let light = sky * 0.75 + vec3<f32>(1.0, 0.96, 0.9) * key * 0.85 + vec3<f32>(0.55, 0.62, 0.75) * fill * 0.3;
    // A little sky reflection on shiny or metal surfaces.
    let r = reflect(-view_dir, n);
    let env = mix(vec3<f32>(0.12, 0.11, 0.10), vec3<f32>(0.55, 0.6, 0.7), 0.5 + 0.5 * r.y);
    let env_amt = (1.0 - roughness) * (0.08 + metallic * 0.6);
    return albedo * light + spec * spec_tint + vec3<f32>(rim * 0.6) * vec3<f32>(0.75, 0.82, 0.95) + env * env_amt * spec_tint;
}

@fragment
fn fs_mesh(i: MeshOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    var n = normalize(i.normal);
    if (!front) {
        n = -n; // show the inside of open garments properly
    }
    let tex = textureSample(base_tex, base_sampler, i.uv);
    var albedo: vec3<f32>;
    if (look.adjust.w > 0.5) {
        albedo = i.color * adjust_color(tex.rgb);
    } else {
        albedo = adjust_color(i.color * tex.rgb);
    }
    if (!front) {
        albedo = albedo * 0.6; // insides a bit darker, like real cloth
    }
    return vec4<f32>(studio(albedo, n, i.world), 1.0);
}

// Selected garment outline: the mesh pushed out a little, back faces only.
@vertex
fn vs_outline(v: MeshIn) -> MeshOut {
    var o: MeshOut;
    let dist = length(g.camera_pos.xyz - v.pos);
    let p = v.pos + normalize(v.normal) * dist * g.outline.w;
    o.clip = g.view_proj * vec4<f32>(p, 1.0);
    o.normal = v.normal;
    o.color = g.outline.rgb;
    o.world = p;
    o.uv = v.uv;
    return o;
}

@fragment
fn fs_flat(i: MeshOut) -> @location(0) vec4<f32> {
    return vec4<f32>(i.color, 1.0);
}

// Wireframe drawn with the mesh vertex buffer, nudged toward the camera.
@vertex
fn vs_wire(v: MeshIn) -> MeshOut {
    var o: MeshOut;
    o.clip = g.view_proj * vec4<f32>(v.pos, 1.0);
    o.clip.z = o.clip.z - 0.00015 * o.clip.w;
    o.normal = v.normal;
    o.color = g.wire_color.rgb;
    o.world = v.pos;
    o.uv = v.uv;
    return o;
}

@fragment
fn fs_wire(i: MeshOut) -> @location(0) vec4<f32> {
    return vec4<f32>(i.color, g.wire_color.a);
}

// ---- Backdrop: soft studio gradient -------------------------------------------

struct BgOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_bg(@builtin(vertex_index) i: u32) -> BgOut {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: BgOut;
    o.clip = vec4<f32>(p * 2.0 - 1.0, 1.0, 1.0);
    o.uv = vec2<f32>(p.x, 1.0 - p.y);
    return o;
}

@fragment
fn fs_bg(i: BgOut) -> @location(0) vec4<f32> {
    // Lighter in the middle (behind the character), darker at the top and edges.
    let top = vec3<f32>(0.055, 0.062, 0.075);
    let mid = vec3<f32>(0.20, 0.225, 0.26);
    let bottom = vec3<f32>(0.035, 0.038, 0.045);
    let y = i.uv.y;
    var c = mix(top, mid, smoothstep(0.0, 0.62, y));
    c = mix(c, bottom, smoothstep(0.62, 1.0, y));
    let d = distance(i.uv, vec2<f32>(0.5, 0.55));
    c = c * (1.0 - 0.45 * smoothstep(0.25, 0.85, d));
    return vec4<f32>(c, 1.0);
}

// ---- Soft floor shadow under the character --------------------------------------

struct ShadowOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) local: vec2<f32>,
};

@vertex
fn vs_shadow(@builtin(vertex_index) i: u32) -> ShadowOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    let c = corners[i];
    let world = vec3<f32>(g.shadow.x + c.x * g.shadow.w, g.shadow.y + 0.001, g.shadow.z + c.y * g.shadow.w);
    var o: ShadowOut;
    o.clip = g.view_proj * vec4<f32>(world, 1.0);
    o.local = c;
    return o;
}

@fragment
fn fs_shadow(i: ShadowOut) -> @location(0) vec4<f32> {
    let r = length(i.local);
    let a = (1.0 - smoothstep(0.0, 1.0, r)) * 0.55;
    return vec4<f32>(0.0, 0.0, 0.0, a);
}

// ---- Lines (grid, skeleton) -------------------------------------------------

struct LineIn {
    @location(0) pos: vec3<f32>,
    @location(1) color: vec4<f32>,
};

struct LineOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_line(v: LineIn) -> LineOut {
    var o: LineOut;
    o.clip = g.view_proj * vec4<f32>(v.pos, 1.0);
    o.color = v.color;
    return o;
}

@fragment
fn fs_line(i: LineOut) -> @location(0) vec4<f32> {
    return i.color;
}
