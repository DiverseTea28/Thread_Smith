// ThreadSmith viewport shaders.

struct Globals {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    // xyz = direction the key light shines FROM, w unused
    light_dir: vec4<f32>,
    wire_color: vec4<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;

// ---- Shaded meshes ----------------------------------------------------------

struct MeshIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
};

struct MeshOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec3<f32>,
    @location(2) world: vec3<f32>,
};

@vertex
fn vs_mesh(v: MeshIn) -> MeshOut {
    var o: MeshOut;
    o.clip = g.view_proj * vec4<f32>(v.pos, 1.0);
    o.normal = v.normal;
    o.color = v.color;
    o.world = v.pos;
    return o;
}

@fragment
fn fs_mesh(i: MeshOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    var n = normalize(i.normal);
    if (!front) {
        n = -n; // show the inside of open garments properly
    }
    let view_dir = normalize(g.camera_pos.xyz - i.world);
    let key = max(dot(n, normalize(g.light_dir.xyz)), 0.0);
    let fill = max(dot(n, normalize(vec3<f32>(-0.6, 0.2, -0.4))), 0.0) * 0.35;
    let sky = 0.5 + 0.5 * n.y; // soft light from above
    let rim = pow(1.0 - max(dot(n, view_dir), 0.0), 3.0) * 0.25;
    let light = 0.18 * sky + 0.75 * key + fill;
    let c = i.color * light + vec3<f32>(rim);
    return vec4<f32>(c, 1.0);
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
    return o;
}

@fragment
fn fs_wire(i: MeshOut) -> @location(0) vec4<f32> {
    return vec4<f32>(i.color, g.wire_color.a);
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
