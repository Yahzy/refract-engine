// Bakes reflection cubemaps: one ray per texel from each cubemap's position;
// what it hits is lit like in the probe bake (sun, nearby lights, finished
// probes), and rays that hit nothing see the sky.

import bake;

struct ReflectionParams
{
    // Texels per face side, and how many cubemaps.
    uint size;
    uint count;
    uint _pad0;
    uint _pad1;
};

// Per cubemap, face (+X, -X, +Y, -Y, +Z, -Z), row, column.
[[vk::binding(9, 0)]] RWStructuredBuffer<float4> g_texels;
[[vk::binding(10, 0)]] StructuredBuffer<float4> g_cube_positions;
[[vk::binding(11, 0)]] TextureCube<float4> g_sky;
[[vk::binding(12, 0)]] SamplerState g_sky_sampler;
[[vk::binding(13, 0)]] ConstantBuffer<ReflectionParams> g_reflection;

// The direction through texel `uv` of cube face `face`.
float3 cube_direction(uint face, float2 uv)
{
    float s = uv.x * 2.0 - 1.0, t = uv.y * 2.0 - 1.0;
    switch (face)
    {
    case 0: return float3(1.0, -t, -s);
    case 1: return float3(-1.0, -t, s);
    case 2: return float3(s, 1.0, t);
    case 3: return float3(s, -1.0, -t);
    case 4: return float3(s, -t, 1.0);
    default: return float3(-s, -t, -1.0);
    }
}

[shader("compute")]
[numthreads(8, 8, 1)]
void reflection_bake(uint3 id : SV_DispatchThreadID)
{
    uint size = g_reflection.size;
    if (id.x >= size || id.y >= size || id.z >= g_reflection.count * 6)
        return;
    uint cube = id.z / 6, face = id.z % 6;
    float3 origin = g_cube_positions[cube].xyz;
    float3 dir = normalize(cube_direction(face, (float2(id.xy) + 0.5) / float(size)));

    RayDesc ray;
    ray.Origin = origin;
    ray.Direction = dir;
    ray.TMin = 0.0;
    ray.TMax = 1e6;
    RayQuery<RAY_FLAG_NONE> query;
    query.TraceRayInline(g_scene, RAY_FLAG_NONE, 0xff, ray);
    while (query.Proceed())
    {
    }
    float3 color;
    if (query.CommittedStatus() == COMMITTED_TRIANGLE_HIT)
    {
        uint triangle = query.CommittedPrimitiveIndex();
        float3 a = vertex(triangle * 3), b = vertex(triangle * 3 + 1), c = vertex(triangle * 3 + 2);
        float3 n = normalize(cross(b - a, c - a));
        if (g_albedo[triangle].a > 0.5 && dot(n, dir) > 0.0)
            n = -n; // two-sided: its back is lit like its front
        if (dot(n, dir) > 0.0)
        {
            color = float3(0.0);
        }
        else
        {
            float3 p = origin + dir * query.CommittedRayT();
            color = g_albedo[triangle].rgb * (direct_light(p, n) / PI + previous_light(p + n * 2.0, n));
        }
    }
    else
    {
        color = g_sky.SampleLevel(g_sky_sampler, dir, 0.0).rgb;
    }
    g_texels[((cube * 6 + face) * size + id.y) * size + id.x] = float4(color, 1.0);
}
