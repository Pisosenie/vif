// An old monitor: a slight bulge, scanlines, colors that don't quite line up, darker corners.

const float BULGE = 0.035;

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 c = fragCoord / iResolution.xy * 2.0 - 1.0;
    c *= 1.0 + BULGE * dot(c, c);
    vec2 uv = c * 0.5 + 0.5;
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {
        fragColor = vec4(0.0, 0.0, 0.0, 1.0);
        return;
    }
    vec2 shift = vec2(0.7 / iResolution.x, 0.0);
    vec4 base = texture(iChannel0, uv);
    vec3 color = vec3(texture(iChannel0, uv + shift).r, base.g, texture(iChannel0, uv - shift).b);
    float lines = 0.9 + 0.1 * sin(fragCoord.y * 2.1);
    float corners = mix(0.72, 1.0, smoothstep(1.3, 0.5, length(c)));
    fragColor = vec4(color * lines * corners, base.a);
}
