// A streak from where the cursor was to where it is, gone in a moment.
// Typing doesn't leave one: only a jump of more than a cell and a half.

const float DURATION = 0.18;

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = fragCoord / iResolution.xy;
    vec4 base = texture(iChannel0, uv);
    float t = clamp((iTime - iTimeCursorChange) / DURATION, 0.0, 1.0);
    vec4 now = iCurrentCursor;
    vec4 was = iPreviousCursor;
    // The corners given are top-left with y going up; the middle is half a size right and down.
    vec2 to = now.xy + vec2(now.z, -now.w) * 0.5;
    vec2 from = was.xy + vec2(was.z, -was.w) * 0.5;
    if (t >= 1.0 || length(to - from) < now.z * 1.5) {
        fragColor = base;
        return;
    }
    vec2 tail = mix(from, to, 1.0 - pow(1.0 - t, 3.0));
    vec2 pa = fragCoord - tail;
    vec2 ba = to - tail;
    float along = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-4), 0.0, 1.0);
    float d = length(pa - ba * along);
    float r = min(now.z, now.w) * 0.5 * mix(0.3, 1.0, along);
    float alpha = (1.0 - smoothstep(r - 1.0, r + 1.0, d)) * (1.0 - t) * 0.8;
    fragColor = vec4(mix(base.rgb, iCurrentCursorColor.rgb, alpha), max(base.a, alpha));
}
