"""Drives the interactive picker through a pty and checks it renders, filters,
toggles and exits cleanly, leaving the terminal usable.

Kept out of check.sh because it needs `gh` and the network. Run it directly:
    python3 scripts/check-picker.py /tmp/picker-scratch
"""
import os, pty, sys, time, select, re, fcntl, termios, struct
TD = sys.argv[1] if len(sys.argv) > 1 else "/tmp/repo-metrics-picker-check"
os.makedirs(TD, exist_ok=True)
BIN = os.environ.get("REPO_METRICS_BIN", os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "target", "release", "repo-metrics"))
pid, fd = pty.fork()
if pid == 0:
    os.environ["TERM"] = "xterm-256color"
    os.execv(BIN, [BIN, "sync", "getsentry", "--dir", TD, "--limit", "60"])
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 0, 0))

buf = b""
def pump(secs=1.2):
    global buf
    end = time.time() + secs
    while time.time() < end:
        r, _, _ = select.select([fd], [], [], 0.15)
        if r:
            try: d = os.read(fd, 65536)
            except OSError: break
            if not d: break
            buf += d
def send(s, wait=0.8):
    os.write(fd, s.encode() if isinstance(s, str) else s)
    pump(wait)
def clean(s): return re.sub(r'\x1b\[[0-9;?]*[a-zA-Z]|\x1b[()][B0]|\r', '', s)

pump(15)                       # listing may need a live fetch on a cold cache
snap1 = clean(buf.decode(errors="replace"))

# Derive the filter from a repo that is actually on screen, so the test doesn't
# rot as the org's recently-pushed list changes.
names = re.findall(r'getsentry/([A-Za-z0-9._-]+)', snap1)
term = names[0][:6] if names else "sentry"
send(term)
snap2 = clean(buf.decode(errors="replace"))
send(" ")                      # toggle the highlighted row
snap3 = clean(buf.decode(errors="replace"))
send("\r"); pump(2)            # confirm -> leaves alt screen, asks to proceed
send("n\n"); pump(2)           # decline, so nothing is cloned
try:
    os.close(fd)
except OSError:
    pass
os.waitpid(pid, 0)
cf = clean(buf.decode(errors="replace"))

checks = [
  ("picker drew the header",      "getsentry →" in snap1),
  ("listed repos",                bool(names)),
  ("showed keybinding footer",    "space toggle" in snap1),
  ("filter narrowed the list",    f"filter {term}" in snap2 and " shown" in snap2),
  ("filter matched something",    "nothing matches" not in snap2.split(f"filter {term}")[-1][:120]),
  ("name not over-truncated",     len(names[0]) < 30 if names else False),
  ("checkbox got ticked",         "[x]" in snap3),
  ("left alternate screen",       "Proceed?" in cf),
  ("declining cancelled cleanly", "cancelled" in cf),
  ("no panic",                    "panicked" not in cf),
]
for name, ok in checks:
    print(("  PASS  " if ok else "  FAIL  ") + name)
failed = sum(1 for _, ok in checks if not ok)
if failed:
    print("\n--- last frame ---")
    print(cf.split("\x1b[2J")[-1][-700:])
sys.exit(1 if failed else 0)
