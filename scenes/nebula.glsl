// Frontrise built-in scene: Nebula
// Drifting clouds of colour that brighten with the music.

float hash(vec2 p) { return fract(sin(dot(p, vec2(41.3, 289.1))) * 17853.1123); }

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
    mat2 rot = mat2(0.8, 0.6, -0.6, 0.8);
    for (int i = 0; i < 6; i++) {
        v += a * noise(p);
        p = rot * p * 2.0;
        a *= 0.5;
    }
    return v;
}

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = (fragCoord - 0.5 * iResolution.xy) / iResolution.y;
    float t = iTime * 0.03;

    vec2 q = vec2(fbm(uv * 2.0 + t), fbm(uv * 2.0 - t + 4.0));
    float n = fbm(uv * 1.6 + 1.8 * q + vec2(t * 2.0, -t));

    vec3 col = mix(vec3(0.02, 0.01, 0.06), vec3(0.35, 0.10, 0.55), n);
    col = mix(col, vec3(0.95, 0.45, 0.25), smoothstep(0.55, 0.95, n) * 0.7);
    col *= 0.75 + 0.6 * iLevel + 0.4 * iBass;

    vec2 cell = floor(fragCoord / 2.0);
    col += vec3(step(0.9985, hash(cell))) * 0.8;

    fragColor = vec4(col, 1.0);
}
