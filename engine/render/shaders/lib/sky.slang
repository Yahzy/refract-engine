// The sky: the map's cubemap when it has one, otherwise a procedural
// gradient from the horizon to the zenith with a darker ground below and the
// sun's disk. Also the source of ambient light where no probes reach.
module sky;

import frame;

// Bound by every pass that shows the sky (group 0).
[[vk::binding(11, 0)]] public TextureCube<float4> g_sky;
[[vk::binding(12, 0)]] public SamplerState g_sky_sampler;

// As a light source, a sky texture counts for less than it shows: one texel
// stands for the whole blurred hemisphere around it.
static const float SKY_TEXTURE_LIGHT = 0.4;

// Light coming from direction `dir` (world space, normalized): the sky as
// seen when `with_sun`, otherwise as a source of ambient light.
public float3 sky_radiance(View view, float3 dir, bool with_sun)
{
    if (view.has_sky != 0)
        return g_sky.SampleLevel(g_sky_sampler, dir, 0.0).rgb * (with_sun ? 1.0 : SKY_TEXTURE_LIGHT);
    float up = dir.z;
    float3 color;
    if (up >= 0.0)
        color = lerp(view.horizon_color, view.sky_color, pow(up, 0.5));
    else
        color = lerp(view.horizon_color, view.ground_color, saturate(-up * 4.0));

    if (with_sun)
    {
        float cos_angle = dot(dir, view.sun_direction);
        // About half a degree wide, with a soft glow around it.
        float disk = smoothstep(0.99996, 0.99999, cos_angle);
        float glow = pow(saturate(cos_angle), 64.0) * 0.15;
        color += view.sun_color * (disk * 400.0 + glow);
    }
    return color;
}
