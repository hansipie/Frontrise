// Frontrise built-in scene: Pulse
// A ring of light that dances with the sound your computer plays.
// Uses iSpectrum (32 bands, 0..1), iBass and iLevel.

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = (fragCoord - 0.5 * iResolution.xy) / iResolution.y;
    float r = length(uv);
    float a = atan(uv.y, uv.x);

    // Mirror the spectrum around the circle: low notes at the bottom.
    float s = abs(a) / 3.14159265;
    int band = int(clamp((1.0 - s) * 31.0, 0.0, 31.0));
    float mag = iSpectrum[band];

    float idle = 0.03 * sin(a * 6.0 + iTime * 0.8);
    float radius = 0.20 + 0.06 * iBass + 0.18 * mag + idle;
    float line = 0.004 / (abs(r - radius) + 0.004);

    vec3 base = mix(vec3(0.04, 0.02, 0.10), vec3(0.01, 0.05, 0.10), uv.y + 0.5);
    vec3 hue = 0.5 + 0.5 * cos(6.2831 * (s * 0.6 + iTime * 0.03 + vec3(0.0, 0.33, 0.67)));
    vec3 col = base + hue * line * (0.6 + 1.4 * iLevel);

    // Soft halo inside the ring, breathing with the bass.
    col += hue * 0.25 * smoothstep(radius, 0.0, r) * (0.15 + iBass);

    fragColor = vec4(col, 1.0);
}
