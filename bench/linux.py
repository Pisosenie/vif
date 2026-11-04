# Run by linux.sh under its own headless sway: time to a window, idle memory and vtebench
# for vif, Alacritty, kitty and foot. Prints a table.
import json, os, statistics, subprocess, sys, time

VIF = os.path.expanduser("~/.cache/vif-target/release/vif")
VTE = os.path.expanduser("~/.cache/vtebench-target/release/vtebench")
BENCH = "/tmp/vtebench-src/benchmarks"
CFG = "/tmp/vif-bench.toml"
os.makedirs(os.path.dirname(CFG), exist_ok=True)
open(CFG, "w").write("[terminal]\nsessions = false\n")
env = dict(os.environ, VIF_CONFIG=CFG)
TERMS = {
    "vif": lambda cmd: [VIF, "-e", *cmd],
    "alacritty": lambda cmd: ["alacritty", "-e", *cmd],
    "kitty": lambda cmd: ["kitty", *cmd],
    "foot": lambda cmd: ["foot", *cmd],
}

def tree_rss(pid):
    out = subprocess.run(["ps", "-e", "-o", "pid=,ppid=,rss="], capture_output=True, text=True).stdout
    rows = [tuple(map(int, l.split())) for l in out.splitlines() if l.strip()]
    keep, total = {pid}, 0
    changed = True
    while changed:
        changed = False
        for p, pp, _ in rows:
            if pp in keep and p not in keep:
                keep.add(p); changed = True
    for p, _, rss in rows:
        if p in keep:
            total += rss
    return total / 1024

def window_time(make):
    sub = subprocess.Popen(["swaymsg", "-t", "subscribe", '["window"]'], stdout=subprocess.PIPE, env=env)
    time.sleep(0.15)
    t = time.perf_counter()
    p = subprocess.Popen(make(["sleep", "5"]), env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    sub.wait(timeout=20)
    dt = (time.perf_counter() - t) * 1000
    return dt, p

result = {}
for name, make in TERMS.items():
    times = []
    for _ in range(10):
        dt, p = window_time(make)
        times.append(dt)
        p.kill(); p.wait(); time.sleep(0.4)
    dt, p = window_time(make)
    time.sleep(1.5)
    mem = tree_rss(p.pid)
    p.kill(); p.wait(); time.sleep(0.4)
    dat = f"/tmp/vb-{name}.dat"
    if os.path.exists(dat):
        os.remove(dat)
    run = subprocess.Popen(make([VTE, "-b", BENCH, "--dat", dat, "--max-secs", "3"]), env=env,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        run.wait(timeout=240)
    except subprocess.TimeoutExpired:
        run.kill()
    bench = {}
    if os.path.exists(dat):
        lines = [l.split() for l in open(dat) if l.strip()]
        head, rows = lines[0], lines[1:]
        for i, col in enumerate(head):
            vals = [float(r[i]) for r in rows if i < len(r) and r[i].replace(".", "").isdigit()]
            if vals:
                bench[col] = statistics.median(vals)
    result[name] = {"start": statistics.median(times), "mem": mem, "bench": bench}
    print(name, round(result[name]["start"]), "ms", round(mem), "MB", len(bench), "benchmarks", file=sys.stderr)

json.dump(result, open("/tmp/vif-bench-linux.json", "w"), indent=1)
names = list(TERMS)
print("| | " + " | ".join(names) + " |")
print("|---|" + "---:|" * len(names))
print("| window on screen, median of 10 | " + " | ".join(f"{result[n]['start']:.0f} ms" for n in names) + " |")
print("| memory when idle | " + " | ".join(f"{result[n]['mem']:.0f} MB" for n in names) + " |")
for b in sorted({k for n in names for k in result[n]["bench"]}):
    print(f"| vtebench {b} | " + " | ".join(f"{result[n]['bench'].get(b, float('nan')):.1f} ms" for n in names) + " |")
