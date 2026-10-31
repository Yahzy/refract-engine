// Shadow lookups: how much of a light reaches a point, from 0 (shadowed) to 1.
module shadows;

import frame;

// Shadow maps hold the nearest depth seen from the light; the comparison
// sampler returns 1 where the point is no farther than that.
public float filtered_shadow(Texture2DArray<float> maps, SamplerComparisonState compare, uint layer, float4 clip, float texel)
{
    float3 ndc = clip.xyz / clip.w;
    float2 uv = ndc.xy * float2(0.5, -0.5) + 0.5;
    if (any(uv < 0.0) || any(uv > 1.0) || ndc.z > 1.0)
        return 1.0;
    // 3x3 bilinear comparisons: a 4x4 texel tent filter.
    float sum = 0.0;
    for (int y = -1; y <= 1; y++)
        for (int x = -1; x <= 1; x++)
            sum += maps.SampleCmpLevelZero(compare, float3(uv + float2(x, y) * texel, float(layer)), ndc.z);
    return sum / 9.0;
}

// The cascade covering `world`, or CASCADES when it is beyond the last one.
public uint sun_cascade(View view, float3 world)
{
    float depth = dot(world - view.position, view.forward);
    for (uint i = 0; i < CASCADES; i++)
        if (depth <= view.cascade_far[i])
            return i;
    return CASCADES;
}

// The cube face of a point light that sees `offset` (light to surface):
// +X, -X, +Y, -Y, +Z, -Z.
public uint cube_face(float3 offset)
{
    float3 a = abs(offset);
    if (a.x >= a.y && a.x >= a.z)
        return offset.x > 0.0 ? 0 : 1;
    if (a.y >= a.z)
        return offset.y > 0.0 ? 2 : 3;
    return offset.z > 0.0 ? 4 : 5;
}
