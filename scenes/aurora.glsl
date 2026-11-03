// Frontrise built-in scene: Aurora
// Slow ribbons of light over a starry night sky.

// Sine-free hash (Dave Hoskins): stays uniform for large coordinates, where
// sin()-based hashes lose precision and show up as aligned dots.
float hash(vec2 p) {
    vec3 p3 = fract(vec3(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

float noise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    vec2 u = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash(i), hash(i + vec2(1.0, 0.0)), u.x),
               mix(hash(i + vec2(0.0, 1.0)), hash(i + vec2(1.0, 1.0)), u.x), u.y);
}

float fbm(vec2 p) {
    float v = 0.0;
    float a = 0.5;
    for (int i = 0; i < 5; i++) {
        v += a * noise(p);
        p *= 2.03;
        a *= 0.5;
    }
    return v;
}

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = fragCoord / iResolution.xy;
    float t = iTime * 0.05;

    vec3 col = mix(vec3(0.01, 0.015, 0.05), vec3(0.03, 0.05, 0.13), uv.y);

    for (int i = 0; i < 3; i++) {
        float fi = float(i);
        float y = 0.52 + 0.13 * fi
                + 0.07 * sin(uv.x * 3.0 + t * 4.0 + fi * 1.7)
                + 0.12 * (fbm(vec2(uv.x * 2.0 + t * 2.0 + fi, t + fi)) - 0.5);
        float d = (uv.y - y) * 8.0;
        float band = exp(-d * d) * (0.55 + 0.45 * fbm(vec2(uv.x * 7.0 - t * 3.0, fi * 3.1)));
        vec3 tint = mix(vec3(0.10, 0.95, 0.60), vec3(0.55, 0.30, 1.00), fi / 2.0);
        col += tint * band * 0.55;
    }

    vec2 cell = floor(fragCoord / 3.0);
    float star = step(0.9975, hash(cell))
               * (0.55 + 0.45 * sin(iTime * 1.5 + hash(cell + 7.0) * 6.2831))
               * smoothstep(0.25, 0.9, uv.y);
    col += vec3(star);

    fragColor = vec4(col, 1.0);
}
