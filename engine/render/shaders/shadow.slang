// Shadow maps: depth only, from one shadow view at a time. Alpha-tested
// surfaces (foliage, fences) also read their texture, so that light passes
// through their holes.

import frame;

struct ShadowView
{
    float4x4 view_proj;
};

[[vk::binding(0, 0)]] StructuredBuffer<Instance> g_instances;
[[vk::binding(1, 0)]] ConstantBuffer<ShadowView> g_shadow_view;
[[vk::binding(2, 0)]] StructuredBuffer<Material> g_materials;
// Group 1: the scene's textures (bindless), shared with the scene pass.
[[vk::binding(0, 1)]] Texture2DArray<float4> g_textures[];
[[vk::binding(1, 1)]] SamplerState g_sampler;

[shader("vertex")]
float4 shadow_vs([[vk::location(0)]] float3 position : POSITION, uint instance_id : SV_VulkanInstanceID) : SV_Position
{
    float4 world = mul(g_instances[instance_id].model, float4(position, 1.0));
    return mul(g_shadow_view.view_proj, world);
}

struct AlphaOut
{
    float4 clip : SV_Position;
    float2 uv : TEXCOORD0;
    nointerpolation uint material : MATERIAL;
};

[shader("vertex")]
AlphaOut shadow_alpha_vs([[vk::location(0)]] float3 position : POSITION, [[vk::location(3)]] float2 uv : TEXCOORD0, uint instance_id : SV_VulkanInstanceID)
{
    Instance instance = g_instances[instance_id];
    AlphaOut output;
    output.clip = mul(g_shadow_view.view_proj, mul(instance.model, float4(position, 1.0)));
    output.uv = uv;
    output.material = instance.material;
    return output;
}

[shader("fragment")]
void shadow_alpha_fs(AlphaOut input)
{
    Material material = g_materials[input.material];
    float alpha = material.base_color.a * g_textures[NonUniformResourceIndex(material.base_texture)].Sample(g_sampler, float3(input.uv, 0.0)).a;
    if (alpha < material.alpha_cutoff)
        discard;
}
