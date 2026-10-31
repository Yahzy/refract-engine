// Bakes light probes: each workgroup traces RAYS rays from one probe, lights
// what they hit (sun, nearby lights, and the previous pass's probes for
// further bounces) and folds the result into the probe's ambient cube.

import bake;

static const uint RAYS = 64;

// Distance a ray that hits nothing counts as, for visibility.
static const float FAR = 2048.0;
// Share of rays hitting back faces above which a probe is inside geometry.
static const float INSIDE = 0.25;

[[vk::binding(9, 0)]] RWStructuredBuffer<Probe> g_probes;

static const float3 AXES[6] = { float3(1, 0, 0), float3(-1, 0, 0), float3(0, 1, 0), float3(0, -1, 0), float3(0, 0, 1), float3(0, 0, -1) };

// Evenly spread directions on the sphere (spherical Fibonacci), turned by a
// per-probe, per-pass angle so passes do not repeat the same pattern.
float3 ray_direction(uint i, uint seed)
{
    float golden = 2.39996323;
    float z = 1.0 - (2.0 * float(i) + 1.0) / float(RAYS);
    float r = sqrt(max(0.0, 1.0 - z * z));
    float phi = float(i) * golden + float(seed % 1024u) * 0.0061359;
    return float3(r * cos(phi), r * sin(phi), z);
}

groupshared float3 s_light[RAYS];
groupshared float3 s_dir[RAYS];
groupshared float s_distance[RAYS];
groupshared uint s_back[RAYS];
groupshared float s_face_distance[6];

[shader("compute")]
[numthreads(RAYS, 1, 1)]
void probe_bake(uint3 group : SV_GroupID, uint thread : SV_GroupIndex)
{
    uint probe_index = group.x + group.y * 65535;
    uint3 res = g_params.resolution;
    if (probe_index >= res.x * res.y * res.z)
        return;
    uint3 cell = uint3(probe_index % res.x, (probe_index / res.x) % res.y, probe_index / (res.x * res.y));
    float3 origin = g_params.grid_origin + (float3(cell) + 0.5) * g_params.cell_size;

    float3 dir = ray_direction(thread, probe_index * 7919u + g_params.pass_index * 104729u);
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

    float3 light;
    float distance = FAR;
    uint back = 0;
    if (query.CommittedStatus() == COMMITTED_TRIANGLE_HIT)
    {
        uint triangle = query.CommittedPrimitiveIndex();
        float3 a = vertex(triangle * 3), b = vertex(triangle * 3 + 1), c = vertex(triangle * 3 + 2);
        float3 n = normalize(cross(b - a, c - a));
        distance = min(query.CommittedRayT(), FAR);
        if (dot(n, dir) > 0.0)
        {
            // The back of a surface: this probe is (partly) inside something.
            back = 1;
            light = float3(0.0);
        }
        else
        {
            float3 p = origin + dir * query.CommittedRayT();
            float3 albedo = g_albedo[triangle].rgb;
            light = albedo * (direct_light(p, n) / PI + previous_light(p + n * 2.0, n));
        }
    }
    else
    {
        light = sky(dir);
    }
    s_light[thread] = light;
    s_dir[thread] = dir;
    s_distance[thread] = distance;
    s_back[thread] = back;
    GroupMemoryBarrierWithGroupSync();

    // Six threads fold the rays into the six directions.
    if (thread < 6)
    {
        float3 axis = AXES[thread];
        float3 sum = float3(0.0);
        float distance_sum = 0.0;
        float weight_sum = 0.0;
        for (uint i = 0; i < RAYS; i++)
        {
            float w = max(0.0, dot(s_dir[i], axis));
            sum += s_light[i] * w;
            distance_sum += s_distance[i] * w;
            weight_sum += w;
        }
        // Irradiance over pi, from a uniform sampling of the sphere.
        g_probes[probe_index].light[thread] = encode_rgb9e5(sum * (4.0 / float(RAYS)));
        s_face_distance[thread] = distance_sum / max(weight_sum, 1e-4);
    }
    GroupMemoryBarrierWithGroupSync();
    if (thread == 0)
    {
        uint backs = 0;
        for (uint i = 0; i < RAYS; i++)
            backs += s_back[i];
        for (uint k = 0; k < 3; k++)
            g_probes[probe_index].distance[k] = f32tof16(s_face_distance[2 * k]) | (f32tof16(s_face_distance[2 * k + 1]) << 16);
        g_probes[probe_index].flags = float(backs) < INSIDE * float(RAYS) ? PROBE_VALID : 0;
    }
}
