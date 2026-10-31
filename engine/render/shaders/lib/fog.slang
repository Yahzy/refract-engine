// Height fog in closed form: its density falls exponentially with height,
// so the light it takes away along a straight ray integrates exactly. No
// marching, no extra pass: a few instructions per pixel.
module fog;

import frame;

// Share of the fog's light that is sunlight scattered towards the eye; the
// rest is the sky's, the fog's own colour.
static const float SUN_SHARE = 0.3;
// How much the fog scatters light forwards (Henyey-Greenstein), as haze does:
// it glows around the sun.
static const float ANISOTROPY = 0.6;
// Distance the sky counts as, world units.
static const float SKY_DISTANCE = 200000.0;

// Fraction of what lies `distance` away along `dir` (normalized) that the
// fog hides, for a fog of `density` at the fog's height.
public float fog_opacity(View view, float3 dir, float distance, float density)
{
    float k = 1.0 / view.fog_falloff;
    float at_eye = density * exp(clamp((view.fog_height - view.position.z) * k, -30.0, 30.0));
    // The density along the ray, averaged: (1 - e^-x) / x for a rise of x falloffs.
    float rise = clamp(dir.z * distance * k, -30.0, 30.0);
    float average = abs(rise) > 1e-3 ? (1.0 - exp(-rise)) / rise : 1.0 - 0.5 * rise;
    return view.fog_max_opacity * (1.0 - exp(-at_eye * distance * average));
}

// Light the fog sends towards the eye along `dir` (from the eye).
public float3 fog_light(View view, float3 dir)
{
    float sun = dot(view.sun_color, float3(0.2126, 0.7152, 0.0722));
    if (sun <= 0.0)
        return view.fog_color;
    float g = ANISOTROPY;
    // Times 4 pi: 1 on average over all directions.
    float phase = (1.0 - g * g) / pow(1.0 + g * g - 2.0 * g * dot(dir, view.sun_direction), 1.5);
    return view.fog_color * ((1.0 - SUN_SHARE) + SUN_SHARE * phase * view.sun_color / sun);
}

// `color` of a surface at `world` seen through the fog. Distant scenery takes
// the sky's density; additive surfaces only fade (the fog behind them is
// already there).
public float3 apply_fog(View view, float3 color, float3 world, bool distant, bool additive)
{
    if (view.fog_max_opacity <= 0.0)
        return color;
    float3 to_surface = world - view.position;
    float distance = length(to_surface);
    float3 dir = to_surface / max(distance, 1e-4);
    float opacity = fog_opacity(view, dir, distance, distant ? view.fog_sky_density : view.fog_density);
    return additive ? color * (1.0 - opacity) : lerp(color, fog_light(view, dir), opacity);
}

// The sky seen along `dir` through the fog.
public float3 fog_sky(View view, float3 color, float3 dir)
{
    if (view.fog_max_opacity <= 0.0)
        return color;
    return lerp(color, fog_light(view, dir), fog_opacity(view, dir, SKY_DISTANCE, view.fog_sky_density));
}
