# Sourced: starts a throwaway headless sway, points WAYLAND_DISPLAY and SWAYSOCK at it, and stops
# it when the script that sourced this exits. Linux, needs sway.
runtime=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}
sockets() { ls "$runtime" | grep -E '^wayland-[0-9]+$' || true; }
sway_conf=$(mktemp)
printf 'output HEADLESS-1 resolution %s scale 1\ndefault_border none\n' "${RES:-1440x900}" > "$sway_conf"
before=$(sockets)
WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 WLR_RENDERER=pixman sway -c "$sway_conf" > /dev/null 2>&1 &
sway=$!
trap 'kill $sway 2> /dev/null; rm -f "$sway_conf"' EXIT
for _ in $(seq 50); do
    display=$(comm -13 <(echo "$before") <(sockets) | head -1)
    [ -n "$display" ] && break
    sleep 0.1
done
export WAYLAND_DISPLAY=${display:?sway did not start}
export SWAYSOCK=$(ls -t "$runtime"/sway-ipc.*."$sway".sock 2> /dev/null | head -1)
