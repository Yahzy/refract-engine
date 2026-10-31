// Sorts lights into clusters: the view is cut into screen tiles and depth
// slices, and each cluster lists the lights whose range reaches into it.
// One thread per cluster.

import frame;

[[vk::binding(0, 0)]] ConstantBuffer<View> g_view;
[[vk::binding(1, 0)]] StructuredBuffer<Light> g_lights;
// Per cluster: how many lights it lists.
[[vk::binding(2, 0)]] RWStructuredBuffer<uint> g_cluster_counts;
// Per cluster: CLUSTER_MAX_LIGHTS light indices.
[[vk::binding(3, 0)]] RWStructuredBuffer<uint> g_cluster_lights;

// The world point on the view ray through `ndc` at view depth `depth`.
float3 point_at(float2 ndc, float depth)
{
    // Reversed infinite projection: depth d is at NDC z = near / d.
    float4 p = mul(g_view.inv_view_proj, float4(ndc, g_view.cluster_near / depth, 1.0));
    return p.xyz / p.w;
}

[shader("compute")]
[numthreads(64, 1, 1)]
void light_cull(uint3 id : SV_DispatchThreadID)
{
    uint cluster = id.x;
    if (cluster >= CLUSTERS_X * CLUSTERS_Y * CLUSTERS_Z)
        return;
    uint x = cluster % CLUSTERS_X;
    uint y = (cluster / CLUSTERS_X) % CLUSTERS_Y;
    uint z = cluster / (CLUSTERS_X * CLUSTERS_Y);

    // Screen tile (NDC y up, tiles from the top) and depth slice.
    float2 ndc_min = float2(float(x) / CLUSTERS_X * 2.0 - 1.0, 1.0 - float(y + 1) / CLUSTERS_Y * 2.0);
    float2 ndc_max = float2(float(x + 1) / CLUSTERS_X * 2.0 - 1.0, 1.0 - float(y) / CLUSTERS_Y * 2.0);
    float near = cluster_slice_depth(g_view, z);
    float far = cluster_slice_depth(g_view, z + 1);
    float3 lo = float3(1e30), hi = float3(-1e30);
    for (uint corner = 0; corner < 8; corner++)
    {
        float2 ndc = float2((corner & 1) != 0 ? ndc_max.x : ndc_min.x, (corner & 2) != 0 ? ndc_max.y : ndc_min.y);
        float3 p = point_at(ndc, (corner & 4) != 0 ? far : near);
        lo = min(lo, p);
        hi = max(hi, p);
    }

    uint count = 0;
    for (uint i = 0; i < g_view.light_count && count < CLUSTER_MAX_LIGHTS; i++)
    {
        Light light = g_lights[i];
        float3 closest = clamp(light.position, lo, hi);
        float3 offset = closest - light.position;
        if (dot(offset, offset) <= light.range * light.range)
        {
            g_cluster_lights[cluster * CLUSTER_MAX_LIGHTS + count] = i;
            count++;
        }
    }
    g_cluster_counts[cluster] = count;
}
