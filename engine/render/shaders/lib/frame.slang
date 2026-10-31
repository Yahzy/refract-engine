// Data shared by every pass of a frame. Layouts match `gpu_types.rs`.
module frame;

public static const uint LIGHT_POINT = 0;
public static const uint LIGHT_SPOT = 1;

public static const uint CASCADES = 4;
public static const uint NO_SHADOW = 0xffffffff;

public struct View
{
    public float4x4 view_proj;
    public float4x4 inv_view_proj;
    // The sun's shadow cascades, nearest first.
    public float4x4 cascades[CASCADES];
    // View depth where each cascade ends.
    public float4 cascade_far;
    // World size of one texel of each cascade.
    public float4 cascade_texel;
    public float3 position;
    public float exposure;
    public float3 forward;
    public uint light_count;
    // Towards the sun.
    public float3 sun_direction;
    // Texel size of the local light shadow maps, in texture coordinates.
    public float local_shadow_texel;
    public float3 sun_color;
    public float time;
    public float3 sky_color;
    public float sun_shadow_texel;
    public float3 horizon_color;
    public float _pad0;
    public float3 ground_color;
    public float _pad1;
    public float3 probe_origin;
    // 1 when the probe grid lights the scene, 0 for the sky's light.
    public uint has_probes;
    public uint3 probe_resolution;
    public float probe_cell;
    public float3 flat_ambient;
    // 1 when the sky is a cubemap, 0 for the procedural sky.
    public uint has_sky;
    // Output size in pixels, and the light clusters' depth range: slices
    // spread exponentially from `cluster_near` to `cluster_far`.
    public float2 screen_size;
    public float cluster_near;
    public float cluster_far;
};

// Light clusters: screen tiles times depth slices.
public static const uint CLUSTERS_X = 16;
public static const uint CLUSTERS_Y = 9;
public static const uint CLUSTERS_Z = 24;
public static const uint CLUSTER_MAX_LIGHTS = 128;

// View depth where slice `z` starts.
public float cluster_slice_depth(View view, uint z)
{
    return view.cluster_near * pow(view.cluster_far / view.cluster_near, float(z) / float(CLUSTERS_Z));
}

// The cluster holding a pixel at view depth `depth`.
public uint cluster_index(View view, float2 pixel, float depth)
{
    uint2 tile = min(uint2(pixel / view.screen_size * float2(CLUSTERS_X, CLUSTERS_Y)), uint2(CLUSTERS_X - 1, CLUSTERS_Y - 1));
    float slice = log(max(depth, view.cluster_near) / view.cluster_near) / log(view.cluster_far / view.cluster_near) * float(CLUSTERS_Z);
    uint z = min(uint(slice), CLUSTERS_Z - 1);
    return tile.x + CLUSTERS_X * (tile.y + CLUSTERS_Y * z);
}

public struct Instance
{
    public float4x4 model;
    public uint material;
    // Nearest reflection cubemap, or NO_REFLECTION.
    public uint reflection;
    public uint _pad1;
    public uint _pad2;
};

public struct Material
{
    public float4 base_color;
    public float3 emissive;
    public float metallic;
    public float roughness;
    // Index in the bindless texture array.
    public uint base_texture;
    public float alpha_cutoff;
    public uint flags;
    // How much a non-metal reflects head-on (F0).
    public float reflectance;
    public float _pad0;
    public float _pad1;
    public float _pad2;
};

public static const uint NO_REFLECTION = 0xffffffff;

public static const uint MATERIAL_ALPHA_TEST = 1;

// Layout of refract_asset::map::Probe.
public struct Probe
{
    // RGB9E5 light arriving from +X, -X, +Y, -Y, +Z, -Z.
    public uint light[6];
    // Mean free distance towards the same directions, as pairs of f16.
    public uint distance[3];
    public uint flags;
};

public static const uint PROBE_VALID = 1;

public float3 decode_rgb9e5(uint v)
{
    float scale = exp2(float(int(v >> 27)) - 24.0);
    return float3(float(v & 511), float((v >> 9) & 511), float((v >> 18) & 511)) * scale;
}

// A unit vector stored on an octahedron (see refract-asset's encoding).
public float3 oct_decode(float2 e)
{
    float z = 1.0 - abs(e.x) - abs(e.y);
    float t = max(-z, 0.0);
    float2 xy = e - float2(e.x >= 0.0 ? t : -t, e.y >= 0.0 ? t : -t);
    return normalize(float3(xy, z));
}

public struct Light
{
    public float3 position;
    public float range;
    public float3 color;
    public uint kind;
    public float3 direction;
    public float cos_outer;
    public float cos_inner;
    // First layer of its shadow maps (6 for a point light), or NO_SHADOW.
    public uint shadow;
    public float _pad0;
    public float _pad1;
};

// A triangle covering the screen, for vertex ids 0, 1, 2.
public float4 fullscreen_position(uint id, out float2 ndc)
{
    float2 uv = float2(float((id << 1) & 2), float(id & 2));
    ndc = uv * 2.0 - 1.0;
    return float4(ndc, 0.0, 1.0);
}
