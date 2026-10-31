// Physically based shading: metallic-roughness materials, GGX specular with a
// height-correlated Smith visibility term, Lambert diffuse.
module pbr;

static const float PI = 3.14159265;

public struct Surface
{
    public float3 albedo;
    public float metallic;
    public float roughness;
    // F0 of the non-metallic part.
    public float reflectance;
    public float3 normal;
    // Towards the eye.
    public float3 view;
};

float3 fresnel_schlick(float3 f0, float v_dot_h)
{
    return f0 + (1.0 - f0) * pow(1.0 - v_dot_h, 5.0);
}

float distribution_ggx(float n_dot_h, float a2)
{
    float d = n_dot_h * n_dot_h * (a2 - 1.0) + 1.0;
    return a2 / (PI * d * d);
}

float visibility_smith_ggx(float n_dot_v, float n_dot_l, float a2)
{
    float gv = n_dot_l * sqrt(n_dot_v * n_dot_v * (1.0 - a2) + a2);
    float gl = n_dot_v * sqrt(n_dot_l * n_dot_l * (1.0 - a2) + a2);
    return 0.5 / max(gv + gl, 1e-5);
}

public float3 specular_color(Surface s)
{
    return lerp(float3(s.reflectance), s.albedo, s.metallic);
}

// Light reflected towards the eye from light arriving along `to_light` with
// irradiance `radiance` (already scaled by distance and cone falloff).
public float3 shade(Surface s, float3 to_light, float3 radiance)
{
    float n_dot_l = saturate(dot(s.normal, to_light));
    if (n_dot_l <= 0.0)
        return float3(0.0);
    float3 h = normalize(to_light + s.view);
    float n_dot_v = max(dot(s.normal, s.view), 1e-4);
    float n_dot_h = saturate(dot(s.normal, h));
    float v_dot_h = saturate(dot(s.view, h));
    // Clamped so that mirror-like surfaces keep a visible highlight.
    float a = max(s.roughness * s.roughness, 0.002);
    float a2 = a * a;

    float3 f = fresnel_schlick(specular_color(s), v_dot_h);
    float3 specular = f * (distribution_ggx(n_dot_h, a2) * visibility_smith_ggx(n_dot_v, n_dot_l, a2));
    float3 diffuse = (1.0 - f) * (1.0 - s.metallic) * s.albedo / PI;
    return (diffuse + specular) * radiance * n_dot_l;
}

// Scale and bias applied to F0 for light from the environment, integrated over
// the hemisphere (analytic fit of the split-sum lookup table).
public float2 environment_brdf(float roughness, float n_dot_v)
{
    float4 c0 = float4(-1.0, -0.0275, -0.572, 0.022);
    float4 c1 = float4(1.0, 0.0425, 1.04, -0.04);
    float4 r = roughness * c0 + c1;
    float a004 = min(r.x * r.x, exp2(-9.28 * n_dot_v)) * r.x + r.y;
    return float2(-1.04, 1.04) * a004 + r.zw;
}
