import ctypes, ctypes.util, os, subprocess, sys, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from run import cmd_for, env_for
x = ctypes.CDLL(ctypes.util.find_library("X11"))
x.XOpenDisplay.restype = ctypes.c_void_p; x.XOpenDisplay.argtypes = [ctypes.c_char_p]
d = x.XOpenDisplay(None)
x.XDefaultRootWindow.restype = ctypes.c_ulong; x.XDefaultRootWindow.argtypes = [ctypes.c_void_p]
root = x.XDefaultRootWindow(d)
x.XInternAtom.restype = ctypes.c_ulong; x.XInternAtom.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int]
x.XGetWindowProperty.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_ulong, ctypes.c_long, ctypes.c_long, ctypes.c_int, ctypes.c_ulong,
    ctypes.POINTER(ctypes.c_ulong), ctypes.POINTER(ctypes.c_int), ctypes.POINTER(ctypes.c_ulong), ctypes.POINTER(ctypes.c_ulong), ctypes.POINTER(ctypes.c_void_p)]
x.XGetImage.restype = ctypes.c_void_p
x.XGetImage.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_int, ctypes.c_uint, ctypes.c_uint, ctypes.c_ulong, ctypes.c_int]
x.XGetPixel.restype = ctypes.c_ulong; x.XGetPixel.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_int]
x.XDestroyImage = x.XDestroyImage if hasattr(x, "XDestroyImage") else None
class XWA(ctypes.Structure):
    _fields_ = [("x", ctypes.c_int), ("y", ctypes.c_int), ("width", ctypes.c_int), ("height", ctypes.c_int), ("border_width", ctypes.c_int), ("depth", ctypes.c_int),
                ("visual", ctypes.c_void_p), ("root", ctypes.c_ulong), ("class_", ctypes.c_int), ("bit_gravity", ctypes.c_int), ("win_gravity", ctypes.c_int),
                ("backing_store", ctypes.c_int), ("backing_planes", ctypes.c_ulong), ("backing_pixel", ctypes.c_ulong), ("save_under", ctypes.c_int),
                ("colormap", ctypes.c_ulong), ("map_installed", ctypes.c_int), ("map_state", ctypes.c_int), ("all_event_masks", ctypes.c_long),
                ("your_event_mask", ctypes.c_long), ("do_not_propagate_mask", ctypes.c_long), ("override_redirect", ctypes.c_int), ("screen", ctypes.c_void_p)]
x.XGetWindowAttributes.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.POINTER(XWA)]
x.XSetErrorHandler.argtypes = [ctypes.c_void_p]
HANDLER = ctypes.CFUNCTYPE(ctypes.c_int, ctypes.c_void_p, ctypes.c_void_p)(lambda a, b: 0)
x.XSetErrorHandler(ctypes.cast(HANDLER, ctypes.c_void_p))
CL = x.XInternAtom(d, b"_NET_CLIENT_LIST", 0)

def clients():
    t, f, n, a, p = ctypes.c_ulong(), ctypes.c_int(), ctypes.c_ulong(), ctypes.c_ulong(), ctypes.c_void_p()
    x.XGetWindowProperty(d, root, CL, 0, 4096, 0, 0, ctypes.byref(t), ctypes.byref(f), ctypes.byref(n), ctypes.byref(a), ctypes.byref(p))
    if not p.value: return set()
    arr = ctypes.cast(p, ctypes.POINTER(ctypes.c_ulong))
    return {arr[i] for i in range(n.value)}

def magenta(w):
    a = XWA()
    if not x.XGetWindowAttributes(d, w, ctypes.byref(a)) or a.map_state != 2: return False
    img = x.XGetImage(d, w, a.width // 2, a.height * 2 // 3, 1, 1, 0xFFFFFFFF, 2)
    if not img: return False
    px = x.XGetPixel(img, 0, 0)
    r, g, b = (px >> 16) & 255, (px >> 8) & 255, px & 255
    return r > 200 and g < 60 and b > 200

def once(term):
    before = clients()
    script = "printf '\\033[48;2;255;0;255m\\033[2J'; sleep 2"
    t0 = time.perf_counter()
    p = subprocess.Popen(cmd_for(term, script), env=env_for(term), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    res = None
    while time.perf_counter() - t0 < 10:
        new = clients() - before
        if any(magenta(w) for w in new):
            res = (time.perf_counter() - t0) * 1000
            break
        time.sleep(0.002)
    p.wait()
    time.sleep(0.8)
    return res

if __name__ == "__main__":
    for term in sys.argv[1].split(","):
        r = sorted(round(once(term) or -1) for _ in range(5))
        print(f"{term:8} median {r[2]} ms  {r}", flush=True)
