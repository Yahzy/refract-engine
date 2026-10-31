// The background: the sky behind everything, drawn first over the whole screen.

import fog;
import frame;
import sky;

[[vk::binding(0, 0)]] ConstantBuffer<View> g_view;

struct VertexOut
{
    float4 clip : SV_Position;
    float2 ndc : NDC;
};

[shader("vertex")]
VertexOut background_vs(uint vertex_id : SV_VulkanVertexID)
{
    VertexOut output;
    output.clip = fullscreen_position(vertex_id, output.ndc);
    return output;
}

[shader("fragment")]
float4 background_fs(VertexOut input) : SV_Target
{
    // A point on the near plane (depth 1 with reversed depth) under this pixel.
    float4 near = mul(g_view.inv_view_proj, float4(input.ndc, 1.0, 1.0));
    float3 dir = normalize(near.xyz / near.w - g_view.position);
    return float4(fog_sky(g_view, sky_radiance(g_view, dir, true), dir), 1.0);
}
