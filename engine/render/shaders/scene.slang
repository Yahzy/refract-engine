// Opaque geometry: forward PBR shading with the sun and every local light
// (with their shadows), plus ambient light from the sky.
//
// variant: RAY_TRACED_SHADOWS rt
// With RAY_TRACED_SHADOWS, local lights without a shadow map cast shadows
// through rays against the scene's opaque geometry.

import frame;
import pbr;
import shadows;
import sky;

static const float METERS_PER_UNIT = 0.0254;

[[vk::binding(0, 0)]] ConstantBuffer<View> g_view;
[[vk::binding(1, 0)]] StructuredBuffer<Instance> g_instances;
[[vk::binding(2, 0)]] StructuredBuffer<Material> g_materials;
[[vk::binding(3, 0)]] StructuredBuffer<Light> g_lights;
[[vk::binding(4, 0)]] Texture2DArray<float> g_sun_shadows;
[[vk::binding(5, 0)]] Texture2DArray<float> g_local_shadows;
[[vk::binding(6, 0)]] SamplerComparisonState g_shadow_compare;
// The view of each local shadow map layer.
[[vk::binding(7, 0)]] StructuredBuffer<float4x4> g_local_shadow_views;
// Baked indirect light, x fastest, then y, then z.
[[vk::binding(8, 0)]] StructuredBuffer<Probe> g_probes;
// The lights of each cluster (see light_cull.slang).
[[vk::binding(9, 0)]] StructuredBuffer<uint> g_cluster_counts;
[[vk::binding(10, 0)]] StructuredBuffer<uint> g_cluster_lights;
#ifdef RAY_TRACED_SHADOWS
// The opaque geometry, for shadow rays.
[[vk::binding(15, 0)]] RaytracingAccelerationStructure g_occluders;

// Whether something opaque stands between `origin` and `distance` along `direction`.
bool occluded(float3 origin, float3 direction, float distance)
{
    RayDesc ray;
    ray.Origin = origin;
    ray.Direction = direction;
    ray.TMin = 0.0;
    ray.TMax = distance;
    RayQuery<RAY_FLAG_ACCEPT_FIRST_HIT_AND_END_SEARCH> query;
    query.TraceRayInline(g_occluders, RAY_FLAG_ACCEPT_FIRST_HIT_AND_END_SEARCH, 0xff, ray);
    while (query.Proceed())
    {
    }
    return query.CommittedStatus() == COMMITTED_TRIANGLE_HIT;
}
#endif

// Baked reflection cubemaps; each instance uses the nearest.
[[vk::binding(13, 0)]] TextureCubeArray<float4> g_reflections;
[[vk::binding(14, 0)]] SamplerState g_reflection_sampler;
// Group 1: every texture of the scene (bindless), indexed by materials; a
// group of its own, as wgpu does not mix binding arrays and uniform buffers.
[[vk::binding(0, 1)]] Texture2DArray<float4> g_textures[];
[[vk::binding(1, 1)]] SamplerState g_sampler;

struct VertexIn
{
    [[vk::location(0)]] float3 position : POSITION;
    [[vk::location(1)]] float2 normal : NORMAL;
    [[vk::location(2)]] float4 tangent : TANGENT;
    [[vk::location(3)]] float2 uv : TEXCOORD0;
    [[vk::location(4)]] float4 color : COLOR;
};

struct VertexOut
{
    float4 clip : SV_Position;
    float3 world : WORLD;
    float3 normal : NORMAL;
    // Along increasing U, with the sign of the bitangent (along increasing V)
    // relative to normal × tangent.
    float4 tangent : TANGENT;
    float2 uv : TEXCOORD0;
    float4 color : COLOR;
    nointerpolation uint material : MATERIAL;
    nointerpolation uint reflection : REFLECTION;
};

// The one way positions are transformed, so that the depth prepass and the
// shading pass find exactly the same depths.
float4 world_position(Instance instance, float3 position)
{
    return mul(instance.model, float4(position, 1.0));
}

float4 clip_position(float4 world)
{
    return mul(g_view.view_proj, world);
}

// Depth prepass: opaque surfaces first write their depth, so that the
// shading pass then shades each pixel once.
[shader("vertex")]
float4 depth_vs([[vk::location(0)]] float3 position : POSITION, uint instance_id : SV_VulkanInstanceID) : SV_Position
{
    return clip_position(world_position(g_instances[instance_id], position));
}

// Writes no colour: the prepass shares the scene pass's colour target.
[shader("fragment")]
void depth_fs()
{
}

struct DepthAlphaOut
{
    float4 clip : SV_Position;
    float2 uv : TEXCOORD0;
    nointerpolation uint material : MATERIAL;
};

// Depth prepass of alpha-tested surfaces: holes write no depth.
[shader("vertex")]
DepthAlphaOut depth_alpha_vs([[vk::location(0)]] float3 position : POSITION, [[vk::location(3)]] float2 uv : TEXCOORD0, uint instance_id : SV_VulkanInstanceID)
{
    Instance instance = g_instances[instance_id];
    DepthAlphaOut output;
    output.clip = clip_position(world_position(instance, position));
    output.uv = uv;
    output.material = instance.material;
    return output;
}

[shader("fragment")]
void depth_alpha_fs(DepthAlphaOut input)
{
    Material material = g_materials[input.material];
    float alpha = material.base_color.a * g_textures[NonUniformResourceIndex(material.base_texture)].Sample(g_sampler, float3(input.uv, 0.0)).a;
    if (alpha < material.alpha_cutoff)
        discard;
}

[shader("vertex")]
VertexOut scene_vs(VertexIn input, uint instance_id : SV_VulkanInstanceID)
{
    Instance instance = g_instances[instance_id];
    float4 world = world_position(instance, input.position);
    VertexOut output;
    output.clip = clip_position(world);
    output.world = world.xyz;
    // Instances are scaled uniformly, so the model matrix transforms normals too.
    output.normal = mul((float3x3)instance.model, oct_decode(input.normal));
    output.tangent = float4(mul((float3x3)instance.model, input.tangent.xyz), input.tangent.w);
    output.uv = input.uv;
    output.color = input.color;
    output.material = instance.material;
    output.reflection = instance.reflection;
    return output;
}

float4 sample_texture(uint texture, float2 uv)
{
    return g_textures[NonUniformResourceIndex(texture)].Sample(g_sampler, float3(uv, 0.0));
}

// A tangent-space normal from a normal map; straight up without one.
float3 sample_normal(uint texture, float2 uv)
{
    if (texture == NO_TEXTURE)
        return float3(0.0, 0.0, 1.0);
    return sample_texture(texture, uv).xyz * 2.0 - 1.0;
}

// Like smoothstep, but `low` may equal `high`.
float soft_step(float low, float high, float x)
{
    float t = saturate((x - low) / max(high - low, 1e-4));
    return t * t * (3.0 - 2.0 * t);
}

// What the material looks like at a pixel, before lighting.
struct SurfaceSample
{
    float4 base;
    // Tangent space.
    float3 normal;
    float3 emissive;
};

SurfaceSample sample_surface(Material material, float2 uv, float vertex_alpha)
{
    SurfaceSample s;
    s.base = material.base_color * sample_texture(material.base_texture, uv);
    s.normal = sample_normal(material.normal_texture, uv);
    s.emissive = material.emissive;
    if (material.base_texture2 != NO_TEXTURE)
    {
        float blend = saturate(vertex_alpha);
        if (material.blend_modulate != NO_TEXTURE)
        {
            float2 m = sample_texture(material.blend_modulate, uv).rg;
            blend = soft_step(saturate(m.g - m.r), saturate(m.g + m.r), blend);
        }
        s.base = lerp(s.base, material.base_color * sample_texture(material.base_texture2, uv), blend);
        s.normal = lerp(s.normal, sample_normal(material.normal_texture2, uv), blend);
    }
    if (material.detail_texture != NO_TEXTURE)
    {
        float4 detail = sample_texture(material.detail_texture, uv * material.detail_scale);
        detail.rgb *= material.detail_tint;
        float amount = material.detail_amount;
        switch (material.detail_blend)
        {
        case DETAIL_ADD:
            s.base.rgb += amount * detail.rgb;
            break;
        case DETAIL_OVER:
            s.base.rgb = lerp(s.base.rgb, detail.rgb, amount * detail.a);
            break;
        case DETAIL_UNDER:
            s.base.rgb = lerp(s.base.rgb, detail.rgb, amount * (1.0 - s.base.a));
            break;
        case DETAIL_MULTIPLY:
            s.base.rgb = lerp(s.base.rgb, s.base.rgb * detail.rgb, amount);
            break;
        case DETAIL_GLOW:
            s.emissive += amount * detail.rgb;
            break;
        default:
            // Mid-grey leaves the base unchanged.
            s.base.rgb *= lerp(float3(1.0), 2.0 * detail.rgb, amount);
            break;
        }
    }
    return s;
}

// Smoothly reaches zero at `range`.
float range_falloff(float distance, float range)
{
    float x = distance / range;
    float window = saturate(1.0 - x * x * x * x);
    return window * window;
}

float sun_shadow(float3 world, float3 normal)
{
    uint cascade = sun_cascade(g_view, world);
    if (cascade >= CASCADES)
        return 1.0;
    // Pushed off the surface by about a texel, against self-shadowing.
    float3 offset = normal * g_view.cascade_texel[cascade] * 1.5;
    float4 clip = mul(g_view.cascades[cascade], float4(world + offset, 1.0));
    return filtered_shadow(g_sun_shadows, g_shadow_compare, cascade, clip, g_view.sun_shadow_texel);
}

float local_shadow(Light light, float3 world, float3 normal, float distance)
{
    if (light.shadow == NO_SHADOW)
        return 1.0;
    uint layer = light.shadow;
    if (light.kind == LIGHT_POINT)
        layer += cube_face(world - light.position);
    // A texel covers about 2 / resolution of the distance to the light.
    float3 offset = normal * distance * g_view.local_shadow_texel * 3.0;
    float4 clip = mul(g_local_shadow_views[layer], float4(world + offset, 1.0));
    return filtered_shadow(g_local_shadows, g_shadow_compare, layer, clip, g_view.local_shadow_texel);
}

float3 local_light(Light light, Surface surface, float3 geometric, float3 world)
{
    float3 to_light = light.position - world;
    float distance = length(to_light);
    if (distance >= light.range)
        return float3(0.0);
    to_light /= distance;

    float meters = max(distance * METERS_PER_UNIT, 0.05);
    float attenuation = range_falloff(distance, light.range) / (meters * meters);
    if (light.kind == LIGHT_SPOT)
    {
        // From the outer edge to the full-brightness cone; both may be equal.
        float cos_angle = dot(-to_light, light.direction);
        float t = saturate((cos_angle - light.cos_outer) / max(light.cos_inner - light.cos_outer, 1e-4));
        attenuation *= t * t * (3.0 - 2.0 * t);
    }
    if (attenuation <= 0.0)
        return float3(0.0);
    attenuation *= local_shadow(light, world, geometric, distance);
#ifdef RAY_TRACED_SHADOWS
    // Lights without a shadow map still do not shine through walls. What is
    // right around the light is ignored: maps put lights inside their
    // fixtures (globes, shades), which must not swallow them.
    float clearance = min(light.clearance, distance * 0.5);
    if (light.shadow == NO_SHADOW && occluded(world + geometric * 1.0, to_light, distance - clearance))
        return float3(0.0);
#endif
    return shade(surface, to_light, light.color * attenuation);
}

// The light an ambient cube sends to a surface facing `n`.
float3 cube_light(Probe probe, float3 n)
{
    float3 squared = n * n;
    return squared.x * decode_rgb9e5(probe.light[n.x >= 0.0 ? 0 : 1])
        + squared.y * decode_rgb9e5(probe.light[n.y >= 0.0 ? 2 : 3])
        + squared.z * decode_rgb9e5(probe.light[n.z >= 0.0 ? 4 : 5]);
}

// Indirect light reaching a surface at `world` facing `normal` (its shading
// normal for the diffuse light) and along `reflected` (specular), blended from the eight probes around it, each
// read once. A probe counts less when it is behind the surface, and hardly at
// all when something stands between it and the surface (it sees less far in
// that direction than the surface is) or when it is inside a wall (the baker
// gave those their open neighbours' light, for surfaces with no open probe
// near). Returns false outside the grid.
bool probe_light(float3 world, float3 normal, float3 shading_normal, float3 reflected, out float3 diffuse, out float3 specular)
{
    diffuse = float3(0.0);
    specular = float3(0.0);
    float cell = g_view.probe_cell;
    float3 p = world + normal * (cell * 0.1);
    float3 grid = (p - g_view.probe_origin) / cell - 0.5;
    int3 res = int3(g_view.probe_resolution);
    if (any(grid < -0.5) || any(grid > float3(res) - 0.5))
        return false;
    int3 base = int3(floor(grid));
    float3 f = grid - float3(base);
    float total = 0.0;
    for (uint corner = 0; corner < 8; corner++)
    {
        int3 offset = int3(corner & 1, (corner >> 1) & 1, (corner >> 2) & 1);
        int3 c = clamp(base + offset, int3(0), res - 1);
        Probe probe = g_probes[c.x + res.x * (c.y + res.y * c.z)];
        float3 probe_position = g_view.probe_origin + (float3(c) + 0.5) * cell;
        float3 to_surface = p - probe_position;
        float distance = length(to_surface);
        float3 d = distance > 1e-3 ? to_surface / distance : normal;

        float3 w3 = lerp(1.0 - f, f, float3(offset));
        float weight = w3.x * w3.y * w3.z;
        // Facing away from the surface.
        float facing = (dot(-d, normal) + 1.0) * 0.5;
        weight *= facing * facing + 0.2;
        // How far the probe sees towards the surface.
        float3 d2 = d * d;
        float2 xs = f16tof32(uint2(probe.distance[0], probe.distance[0] >> 16));
        float2 ys = f16tof32(uint2(probe.distance[1], probe.distance[1] >> 16));
        float2 zs = f16tof32(uint2(probe.distance[2], probe.distance[2] >> 16));
        float free = d2.x * (d.x >= 0.0 ? xs.x : xs.y) + d2.y * (d.y >= 0.0 ? ys.x : ys.y) + d2.z * (d.z >= 0.0 ? zs.x : zs.y);
        // A gradual cut: a sharp one draws the probes' reach as circles.
        weight *= lerp(1.0, 0.02, smoothstep(free + cell * 0.25, free + cell * 0.75, distance));
        weight = max(weight, 1e-5);
        if ((probe.flags & PROBE_VALID) == 0)
            weight *= 1e-4;

        diffuse += weight * cube_light(probe, shading_normal);
        specular += weight * cube_light(probe, reflected);
        total += weight;
    }
    if (total <= 0.0)
        return false;
    diffuse /= total;
    specular /= total;
    return true;
}

[shader("fragment")]
float4 scene_fs(VertexOut input, bool front_facing : SV_IsFrontFace) : SV_Target
{
    Material material = g_materials[input.material];
    SurfaceSample s = sample_surface(material, input.uv, input.color.a);
    float4 base = s.base;
    if ((material.flags & MATERIAL_ALPHA_TEST) != 0 && base.a < material.alpha_cutoff)
        discard;
    // The geometric normal places shadow lookups and weighs probes; the
    // normal map only shades.
    float3 geometric = normalize(input.normal);
    float3 tangent = input.tangent.xyz - geometric * dot(input.tangent.xyz, geometric);
    tangent = dot(tangent, tangent) > 1e-8 ? normalize(tangent) : float3(0.0);
    float3 bitangent = cross(geometric, tangent) * input.tangent.w;
    float3 normal = normalize(tangent * s.normal.x + bitangent * s.normal.y + geometric * max(s.normal.z, 1e-3));
    if (!front_facing)
    {
        geometric = -geometric;
        normal = -normal;
    }

    Surface surface;
    surface.albedo = base.rgb;
    surface.metallic = material.metallic;
    surface.roughness = material.roughness;
    surface.reflectance = material.reflectance;
    surface.normal = normal;
    surface.view = normalize(g_view.position - input.world);

    float3 color = float3(0.0);
    if (dot(geometric, g_view.sun_direction) > 0.0)
        color += shade(surface, g_view.sun_direction, g_view.sun_color * sun_shadow(input.world, geometric));
    uint cluster = cluster_index(g_view, input.clip.xy, dot(input.world - g_view.position, g_view.forward));
    uint cluster_lights = g_cluster_counts[cluster];
    for (uint i = 0; i < cluster_lights; i++)
        color += local_light(g_lights[g_cluster_lights[cluster * CLUSTER_MAX_LIGHTS + i]], surface, geometric, input.world);

    // Ambient: from the baked probes when there are any, otherwise the sky seen
    // along the normal (diffuse) and along the reflection (specular, blurred
    // towards the normal as roughness grows).
    float n_dot_v = max(dot(normal, surface.view), 1e-4);
    float3 reflected = reflect(-surface.view, normal);
    float3 specular_dir = normalize(lerp(reflected, normal, surface.roughness * surface.roughness));
    float3 diffuse_ambient, specular_ambient;
    // Outside the grid (the 3D skybox, for one), the sky lights the surface.
    if (g_view.has_probes == 0 || !probe_light(input.world, geometric, normal, specular_dir, diffuse_ambient, specular_ambient))
    {
        diffuse_ambient = sky_radiance(g_view, normal, false);
        specular_ambient = sky_radiance(g_view, specular_dir, false);
    }
    // Reflective materials (more than the usual 4% head-on) mirror the
    // nearest cubemap, blurrier as roughness grows; others keep the probes'.
    if (input.reflection != NO_REFLECTION && material.reflectance > 0.041)
    {
        uint width, height, elements, levels;
        g_reflections.GetDimensions(0, width, height, elements, levels);
        specular_ambient = g_reflections.SampleLevel(g_reflection_sampler, float4(reflected, float(input.reflection)), surface.roughness * float(levels - 1)).rgb;
    }
    diffuse_ambient += g_view.flat_ambient;
    specular_ambient += g_view.flat_ambient;
    float2 env = environment_brdf(surface.roughness, n_dot_v);
    color += diffuse_ambient * surface.albedo * (1.0 - surface.metallic);
    color += specular_ambient * (specular_color(surface) * env.x + env.y);

    color += s.emissive;
    return float4(color, base.a);
}
