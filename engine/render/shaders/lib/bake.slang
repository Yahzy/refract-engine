// Shared by the light baking programs: the scene's triangles and lights,
// ray queries, and how a hit point is lit (sun, nearby lights, probes).
module bake;

public static const float PI = 3.14159265;
public static const float METERS_PER_UNIT = 0.0254;
public static const uint PROBE_VALID = 1;
public static const uint LIGHT_SPOT = 1;

public struct Params
{
    public float3 grid_origin;
    public float cell_size;
    public uint3 resolution;
    public uint pass_index;
    public float3 sun_direction; // the way sunlight travels
    public float has_sun;
    public float3 sun_color;
    public float _pad0;
    public float3 sky_zenith;
    public float _pad1;
    public float3 sky_horizon;
    public float _pad2;
    public float3 sky_ground;
    public float _pad3;
    public float3 light_grid_origin;
    public float light_grid_cell;
    public uint3 light_grid_resolution;
    public uint _pad4;
};

public struct Light
{
    public float3 position;
    public float range;
    public float3 color; // times intensity
    public uint kind;
    public float3 direction;
    public float cos_outer;
    public float cos_inner;
    // Room around the light that shadow rays ignore (its fixture).
    public float clearance;
    public float _pad0;
    public float _pad1;
};

// Layout of refract_asset::map::Probe.
public struct Probe
{
    public uint light[6];
    public uint distance[3]; // six f16
    public uint flags;
};

[[vk::binding(0, 0)]] public RaytracingAccelerationStructure g_scene;
[[vk::binding(1, 0)]] public StructuredBuffer<float> g_positions;
[[vk::binding(2, 0)]] public StructuredBuffer<uint> g_indices;
// Per triangle; alpha 1 for two-sided surfaces.
[[vk::binding(3, 0)]] public StructuredBuffer<float4> g_albedo;
[[vk::binding(4, 0)]] public StructuredBuffer<Light> g_lights;
[[vk::binding(5, 0)]] public StructuredBuffer<uint2> g_light_cells; // first, count
[[vk::binding(6, 0)]] public StructuredBuffer<uint> g_light_indices;
[[vk::binding(7, 0)]] public ConstantBuffer<Params> g_params;
// The probes of the previous pass, or the finished ones.
[[vk::binding(8, 0)]] public StructuredBuffer<Probe> g_previous;


public float3 decode_rgb9e5(uint v)
{
    float scale = exp2(float(int(v >> 27)) - 24.0);
    return float3(float(v & 511), float((v >> 9) & 511), float((v >> 18) & 511)) * scale;
}

public uint encode_rgb9e5(float3 c)
{
    c = clamp(c, 0.0, 65408.0);
    float m = max(c.r, max(c.g, c.b));
    if (m <= 0.0)
        return 0;
    int e = max(int(floor(log2(m))), -16) + 1 + 15;
    float scale = exp2(float(e - 15 - 9));
    if (uint(round(m / scale)) == 512)
    {
        e += 1;
        scale *= 2.0;
    }
    uint3 q = min(uint3(round(c / scale)), 511);
    return q.r | (q.g << 9) | (q.b << 18) | (uint(e) << 27);
}

public float3 vertex(uint index)
{
    uint i = g_indices[index] * 3;
    return float3(g_positions[i], g_positions[i + 1], g_positions[i + 2]);
}

public bool occluded(float3 origin, float3 direction, float distance)
{
    RayDesc ray;
    ray.Origin = origin;
    ray.Direction = direction;
    ray.TMin = 0.0;
    ray.TMax = distance;
    RayQuery<RAY_FLAG_ACCEPT_FIRST_HIT_AND_END_SEARCH> query;
    query.TraceRayInline(g_scene, RAY_FLAG_ACCEPT_FIRST_HIT_AND_END_SEARCH, 0xff, ray);
    while (query.Proceed())
    {
    }
    return query.CommittedStatus() == COMMITTED_TRIANGLE_HIT;
}

public float3 sky(float3 dir)
{
    if (dir.z >= 0.0)
        return lerp(g_params.sky_horizon, g_params.sky_zenith, pow(dir.z, 0.5));
    return lerp(g_params.sky_horizon, g_params.sky_ground, saturate(-dir.z * 4.0));
}

// Light reaching a surface at `p` facing `n` straight from the sun and lights.
public float3 direct_light(float3 p, float3 n)
{
    float3 sum = float3(0.0);
    float3 origin = p + n * 0.5;
    float n_dot_sun = dot(n, -g_params.sun_direction);
    if (g_params.has_sun > 0.0 && n_dot_sun > 0.0 && !occluded(origin, -g_params.sun_direction, 1e6))
        sum += g_params.sun_color * n_dot_sun;

    int3 cell = int3(floor((p - g_params.light_grid_origin) / g_params.light_grid_cell));
    if (all(cell >= 0) && all(cell < int3(g_params.light_grid_resolution)))
    {
        uint2 range = g_light_cells[cell.x + g_params.light_grid_resolution.x * (cell.y + g_params.light_grid_resolution.y * cell.z)];
        for (uint i = 0; i < range.y; i++)
        {
            Light light = g_lights[g_light_indices[range.x + i]];
            float3 to_light = light.position - p;
            float distance = length(to_light);
            if (distance >= light.range || distance < 1e-3)
                continue;
            to_light /= distance;
            float n_dot_l = dot(n, to_light);
            if (n_dot_l <= 0.0)
                continue;
            float x = distance / light.range;
            float window = saturate(1.0 - x * x * x * x);
            float meters = max(distance * METERS_PER_UNIT, 0.05);
            float attenuation = window * window / (meters * meters);
            if (light.kind == LIGHT_SPOT)
            {
                float t = saturate((dot(-to_light, light.direction) - light.cos_outer) / max(light.cos_inner - light.cos_outer, 1e-4));
                attenuation *= t * t * (3.0 - 2.0 * t);
            }
            if (attenuation > 0.0 && !occluded(origin, to_light, distance - min(light.clearance, distance * 0.5)))
                sum += light.color * attenuation * n_dot_l;
        }
    }
    return sum;
}

// The previous pass's light around `p` for a surface facing `n`: trilinear
// over the valid probes of the cell.
public float3 previous_light(float3 p, float3 n)
{
    if (g_params.pass_index == 0)
        return float3(0.0);
    float3 grid = (p - g_params.grid_origin) / g_params.cell_size - 0.5;
    int3 base = int3(floor(grid));
    float3 f = grid - float3(base);
    float3 squared = n * n;
    uint3 faces = uint3(n.x >= 0.0 ? 0 : 1, n.y >= 0.0 ? 2 : 3, n.z >= 0.0 ? 4 : 5);
    float3 sum = float3(0.0);
    float weights = 0.0;
    for (uint corner = 0; corner < 8; corner++)
    {
        int3 offset = int3(corner & 1, (corner >> 1) & 1, (corner >> 2) & 1);
        int3 c = clamp(base + offset, int3(0), int3(g_params.resolution) - 1);
        Probe probe = g_previous[c.x + g_params.resolution.x * (c.y + g_params.resolution.y * c.z)];
        if ((probe.flags & PROBE_VALID) == 0)
            continue;
        float3 w3 = lerp(1.0 - f, f, float3(offset));
        float w = w3.x * w3.y * w3.z + 1e-4;
        sum += w * (squared.x * decode_rgb9e5(probe.light[faces.x]) + squared.y * decode_rgb9e5(probe.light[faces.y]) + squared.z * decode_rgb9e5(probe.light[faces.z]));
        weights += w;
    }
    return weights > 0.0 ? sum / weights : float3(0.0);
}

