import ctypes, os, statistics, subprocess, sys, time, zlib
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import visible as V
from run import cmd_for, env_for
x, d = V.x, V.d
xt = ctypes.CDLL(ctypes.util.find_library("Xtst"))
xt.XTestFakeKeyEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]
x.XStringToKeysym.restype = ctypes.c_ulong; x.XStringToKeysym.argtypes = [ctypes.c_char_p]
x.XKeysymToKeycode.restype = ctypes.c_ubyte; x.XKeysymToKeycode.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
x.XSetInputFocus.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_ulong]
x.XRaiseWindow.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
x.XFlush.argtypes = [ctypes.c_void_p]; x.XSync.argtypes = [ctypes.c_void_p, ctypes.c_int]
class XImage(ctypes.Structure):
    _fields_ = [("width", ctypes.c_int), ("height", ctypes.c_int), ("xoffset", ctypes.c_int), ("format", ctypes.c_int), ("data", ctypes.c_void_p),
                ("byte_order", ctypes.c_int), ("bitmap_unit", ctypes.c_int), ("bitmap_bit_order", ctypes.c_int), ("bitmap_pad", ctypes.c_int),
                ("depth", ctypes.c_int), ("bytes_per_line", ctypes.c_int)]

def region_hash(w, W, H):
    img = x.XGetImage(d, w, 0, 0, W, H, 0xFFFFFFFF, 2)
    if not img: return None
    im = ctypes.cast(img, ctypes.POINTER(XImage)).contents
    data = ctypes.string_at(im.data, im.bytes_per_line * im.height)
    h = zlib.crc32(data)
    ctypes.CDLL(ctypes.util.find_library("X11")).XFree(ctypes.c_void_p(im.data)); ctypes.CDLL(ctypes.util.find_library("X11")).XFree(ctypes.c_void_p(img))
    return h

def run(term, n=30):
    before = V.clients()
    p = subprocess.Popen(cmd_for(term, "printf '\\033[H\\033[2J'; stty -icanon; cat"), env=env_for(term), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    w = None
    t0 = time.time()
    while time.time() - t0 < 10 and not w:
        new = V.clients() - before
        w = next(iter(new), None)
        time.sleep(0.01)
    time.sleep(1.5)
    x.XRaiseWindow(d, w); x.XSetInputFocus(d, w, 2, 0); x.XSync(d, 0); time.sleep(0.5)
    a = V.XWA(); x.XGetWindowAttributes(d, w, ctypes.byref(a))
    W, H = min(a.width, 900), min(a.height, 120)
    res = []
    keys = "abcdefghijklmnopqrstuvwxyz" * 3
    for ch in keys[:n]:
        kc = x.XKeysymToKeycode(d, x.XStringToKeysym(ch.encode()))
        h0 = region_hash(w, W, H)
        t = time.perf_counter()
        xt.XTestFakeKeyEvent(d, kc, 1, 0); xt.XTestFakeKeyEvent(d, kc, 0, 0); x.XFlush(d)
        while time.perf_counter() - t < 0.5:
            if region_hash(w, W, H) != h0:
                res.append((time.perf_counter() - t) * 1000); break
        time.sleep(0.12)
    p.terminate()
    try: p.wait(timeout=5)
    except subprocess.TimeoutExpired: p.kill()
    time.sleep(1)
    return res

if __name__ == "__main__":
  for term in sys.argv[1].split(","):
    r = run(term)
    print(f"{term:8} n={len(r)} median {statistics.median(r):.1f} ms  p90 {sorted(r)[int(len(r)*0.9)]:.1f}  min {min(r):.1f}", flush=True)
