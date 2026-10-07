import os, subprocess, sys, time, json, signal, glob

HERE = os.path.dirname(os.path.abspath(__file__))
S = os.path.join(HERE, "out")
os.makedirs(S, exist_ok=True)
Z = os.environ.get("ZHELL_BIN_DIR", os.path.join(HERE, "..", "target", "release"))

RUNTIME = f"/run/user/{os.getuid()}/zhell-bench"
os.makedirs(RUNTIME, mode=0o700, exist_ok=True)
os.makedirs(S + "/zstate", exist_ok=True)
os.makedirs(S + "/zcfg", exist_ok=True)
if not os.path.exists(S + "/zcfg/zhell.toml"):
    open(S + "/zcfg/zhell.toml", "w").write("[window]\nwidth = 1000\nheight = 640\n")
ZENV = {"XDG_RUNTIME_DIR": RUNTIME, "XDG_STATE_HOME": S + "/zstate", "ZHELL_CONFIG": S + "/zcfg/zhell.toml"}

def cmd_for(term, script):
    sh = ["sh", "-c", script]
    return {
        "zhell": [Z + "/zhell"] + sh,
        "wezterm": ["wezterm", "--config", "initial_cols=120", "--config", "initial_rows=35", "start", "--always-new-process", "--"] + sh,
        "kitty": ["kitty", "-o", "remember_window_size=no", "-o", "initial_window_width=120c", "-o", "initial_window_height=35c"] + sh,
        "konsole": ["konsole", "--separate", "--nofork", "-e"] + sh,
        "xterm": ["xterm", "-geometry", "120x35", "-e"] + sh,
    }[term]

def env_for(term):
    e = dict(os.environ)
    if term == "zhell":
        e.update(ZENV)
    return e

def tree(pid):
    out = [pid]
    for p in glob.glob(f"/proc/{pid}/task/*/children"):
        try:
            for c in open(p).read().split():
                out += tree(int(c))
        except OSError:
            pass
    return out

def pss_kb(pids):
    total = 0
    for p in pids:
        try:
            for line in open(f"/proc/{p}/smaps_rollup"):
                if line.startswith("Pss:"):
                    total += int(line.split()[1])
        except OSError:
            pass
    return total

def cpu_ticks(pids):
    t = 0
    for p in pids:
        try:
            f = open(f"/proc/{p}/stat").read().rsplit(")", 1)[1].split()
            t += int(f[11]) + int(f[12])
        except OSError:
            pass
    return t

def zhelld():
    out = subprocess.run(["pgrep", "-x", "zhelld"], capture_output=True, text=True).stdout.split()
    res = []
    for p in out:
        try:
            if ("XDG_RUNTIME_DIR=" + RUNTIME).encode() in open(f"/proc/{p}/environ", "rb").read():
                res.append(int(p))
        except OSError:
            pass
    return res

def throughput(term, file, runs):
    times = []
    for _ in range(runs):
        res = S + "/result"
        for f in (res,):
            if os.path.exists(f): os.remove(f)
        script = f't0=$(date +%s%N); cat {file}; t1=$(date +%s%N); echo $((t1-t0)) > {res}'
        p = subprocess.Popen(cmd_for(term, script), env=env_for(term), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        deadline = time.time() + 300
        while not os.path.exists(res) and time.time() < deadline:
            time.sleep(0.05)
        time.sleep(0.3)
        try:
            p.wait(timeout=10)
        except subprocess.TimeoutExpired:
            p.kill()
        times.append(int(open(res).read()) / 1e9 if os.path.exists(res) else None)
        time.sleep(1)
    return times

def startup(term, runs):
    out = []
    for _ in range(runs):
        res = S + "/started"
        if os.path.exists(res): os.remove(res)
        t0 = time.time_ns()
        p = subprocess.Popen(cmd_for(term, f"date +%s%N > {res}"), env=env_for(term), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        while not os.path.exists(res) or not open(res).read().strip():
            time.sleep(0.002)
        out.append((int(open(res).read()) - t0) / 1e6)
        try:
            p.wait(timeout=10)
        except subprocess.TimeoutExpired:
            p.kill()
        time.sleep(0.8)
    return out

def idle(term):
    p = subprocess.Popen(cmd_for(term, "sleep 30"), env=env_for(term), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(8)
    pids = tree(p.pid) + (zhelld() if term == "zhell" else [])
    pids = [x for x in pids if open(f"/proc/{x}/comm").read().strip() not in ("sh", "sleep")]
    mem = pss_kb(pids)
    c0 = cpu_ticks(pids); time.sleep(10); c1 = cpu_ticks(pids)
    p.terminate()
    try: p.wait(timeout=10)
    except subprocess.TimeoutExpired: p.kill()
    time.sleep(1)
    return {"pss_mb": round(mem / 1024, 1), "idle_cpu_pct": round((c1 - c0) / os.sysconf("SC_CLK_TCK") / 10 * 100, 2)}

if __name__ == "__main__":
    terms = sys.argv[1].split(",")
    what = sys.argv[2]
    result = {}
    for t in terms:
        if what == "plain": result[t] = throughput(t, S + "/plain.txt", 3)
        elif what == "color": result[t] = throughput(t, S + "/color.txt", 3)
        elif what == "startup": result[t] = startup(t, 5)
        elif what == "idle": result[t] = idle(t)
        print(t, result[t], flush=True)
    json.dump(result, open(f"{S}/res-{what}-{'_'.join(terms)}.json", "w"))
