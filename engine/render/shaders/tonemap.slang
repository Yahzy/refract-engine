// HDR to display: exposure, then a tone curve. The output target is sRGB,
// so the hardware applies the transfer function.

import frame;

[[vk::binding(0, 0)]] ConstantBuffer<View> g_view;
[[vk::binding(1, 0)]] Texture2D<float4> g_hdr;

struct VertexOut
{
    float4 clip : SV_Position;
    float2 ndc : NDC;
};

[shader("vertex")]
VertexOut tonemap_vs(uint vertex_id : SV_VulkanVertexID)
{
    VertexOut output;
    output.clip = fullscreen_position(vertex_id, output.ndc);
    return output;
}

// Khronos PBR Neutral: leaves colours as they are up to a high brightness,
// then compresses highlights towards white without shifting their hue, so
// textures look as they were painted.
float3 pbr_neutral(float3 color)
{
    const float start_compression = 0.8 - 0.04;
    const float desaturation = 0.15;
    float x = min(color.r, min(color.g, color.b));
    float offset = x < 0.08 ? x - 6.25 * x * x : 0.04;
    color -= offset;
    float peak = max(color.r, max(color.g, color.b));
    if (peak < start_compression)
        return color;
    const float d = 1.0 - start_compression;
    float new_peak = 1.0 - d * d / (peak + d - start_compression);
    color *= new_peak / peak;
    float g = 1.0 - 1.0 / (desaturation * (peak - new_peak) + 1.0);
    return lerp(color, float3(new_peak), g);
}

[shader("fragment")]
float4 tonemap_fs(VertexOut input) : SV_Target
{
    float3 hdr = g_hdr.Load(int3(int2(input.clip.xy), 0)).rgb;
    return float4(saturate(pbr_neutral(hdr * g_view.exposure)), 1.0);
}
