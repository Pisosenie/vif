// Bright text glows a little into what's around it.

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = fragCoord / iResolution.xy;
    vec2 px = 2.0 / iResolution.xy;
    vec4 base = texture(iChannel0, uv);
    vec3 glow = vec3(0.0);
    float total = 0.0;
    for (int i = -2; i <= 2; i++) {
        for (int j = -2; j <= 2; j++) {
            float w = exp(-float(i * i + j * j) / 4.0);
            glow += max(texture(iChannel0, uv + vec2(float(i), float(j)) * px).rgb - 0.5, 0.0) * w;
            total += w;
        }
    }
    fragColor = vec4(base.rgb + glow / total * 1.8, base.a);
}
