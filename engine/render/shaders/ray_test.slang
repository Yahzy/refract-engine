// Checks that ray queries work: one ray per thread, writes the distance to
// the first hit (or -1).

[[vk::binding(0, 0)]] RaytracingAccelerationStructure g_scene;
[[vk::binding(1, 0)]] StructuredBuffer<float4> g_rays; // origin xyz; direction in the next element
[[vk::binding(2, 0)]] RWStructuredBuffer<float> g_hits;

[shader("compute")]
[numthreads(64, 1, 1)]
void ray_test(uint3 id : SV_DispatchThreadID)
{
    uint count, stride;
    g_hits.GetDimensions(count, stride);
    if (id.x >= count)
        return;
    RayDesc ray;
    ray.Origin = g_rays[id.x * 2].xyz;
    ray.Direction = g_rays[id.x * 2 + 1].xyz;
    ray.TMin = 0.0;
    ray.TMax = 1e9;
    RayQuery<RAY_FLAG_NONE> query;
    query.TraceRayInline(g_scene, RAY_FLAG_NONE, 0xff, ray);
    while (query.Proceed())
    {
    }
    g_hits[id.x] = query.CommittedStatus() == COMMITTED_TRIANGLE_HIT ? query.CommittedRayT() : -1.0;
}
