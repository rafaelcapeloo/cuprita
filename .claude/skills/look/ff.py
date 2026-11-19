#!/usr/bin/env python3
"""ff.py <url> <step>...: drives a headless Firefox over Marionette (no dependencies) to look at
the browser build. Steps, in order:
  wait:<seconds>    shot:<out.png>    click:<x>,<y>    key:<Key>[:ctrl|alt]
  js:<script returning a value>    sh:<shell command run meanwhile>    size:<w>x<h> (a phone's: 390x844)
  pinch:<x>,<y>,<from>,<to> (two fingers about a point, their gap from one width to another)
The page's console errors print at the end. A served project needs no drop:
  cuprita --serve <project> --port 8174 &    ff.py http://127.0.0.1:8174/ wait:7 shot:/tmp/x.png
"""
import base64, json, os, shutil, socket, subprocess, sys, tempfile, time


class Marionette:
    """Firefox's remote protocol: `<length>:<json>` messages, a command `[0, id, name, params]`."""

    def __init__(self, port=2828):
        for _ in range(150):
            try:
                self.s = socket.create_connection(("127.0.0.1", port), timeout=60)
                break
            except OSError:
                time.sleep(0.2)
        else:
            raise SystemExit("Firefox didn't open its Marionette port")
        self.buf, self.id = b"", 0
        self.recv()  # its greeting

    def recv(self):
        while b":" not in self.buf:
            self.buf += self.s.recv(65536)
        n, _, rest = self.buf.partition(b":")
        while len(rest) < int(n):
            rest += self.s.recv(1 << 20)
        self.buf = rest[int(n):]
        return json.loads(rest[: int(n)])

    def cmd(self, name, **params):
        self.id += 1
        msg = json.dumps([0, self.id, name, params])
        self.s.sendall(f"{len(msg)}:{msg}".encode())
        while True:
            kind, of, error, result = self.recv()
            if kind == 1 and of == self.id:
                if error:
                    raise RuntimeError(f"{name}: {error}")
                return result


PREFS = {
    "webgl.force-enabled": True,
    "dom.w3c_touch_events.enabled": 1,
    "gfx.webrender.software": True,
    "devtools.console.stdout.content": True,
    "browser.startup.homepage": "about:blank",
    "browser.startup.page": 0,
    "browser.aboutwelcome.enabled": False,
    "browser.shell.checkDefaultBrowser": False,
    "datareporting.policy.dataSubmissionEnabled": False,
}
EVENT = "{clientX:%s,clientY:%s,button:0,buttons:%d,bubbles:true,cancelable:true,pointerId:1,pointerType:'mouse',isPrimary:true}"


def main():
    if len(sys.argv) < 2:
        raise SystemExit(__doc__)
    url, steps = sys.argv[1], sys.argv[2:]
    profile = tempfile.mkdtemp(prefix="cuprita-ff-")
    with open(os.path.join(profile, "user.js"), "w") as f:
        f.writelines(f"user_pref({json.dumps(k)}, {json.dumps(v)});\n" for k, v in PREFS.items())
    firefox = subprocess.Popen(
        ["firefox", "--headless", "--marionette", "--no-remote", "--profile", profile, "--width", "1440", "--height", "900"],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )
    try:
        m = Marionette()
        m.cmd("WebDriver:NewSession", capabilities={"alwaysMatch": {}})
        m.cmd("WebDriver:SetWindowRect", width=1440, height=900)
        m.cmd("WebDriver:Navigate", url=url)
        run = lambda script: m.cmd("WebDriver:ExecuteScript", script=script, args=[])["value"]
        for step in steps:
            kind, _, arg = step.partition(":")
            if kind == "wait":
                time.sleep(float(arg))
            elif kind == "pinch":
                x, y, a, b = (float(v) for v in arg.split(","))
                finger = lambda name, side: {
                    "type": "pointer",
                    "id": name,
                    "parameters": {"pointerType": "touch"},
                    "actions": [
                        {"type": "pointerMove", "x": int(x + side * a / 2), "y": int(y), "duration": 0},
                        {"type": "pointerDown", "button": 0},
                        {"type": "pause", "duration": 80},
                        {"type": "pointerMove", "x": int(x + side * (a + b) / 4), "y": int(y), "duration": 200},
                        {"type": "pointerMove", "x": int(x + side * b / 2), "y": int(y), "duration": 200},
                        {"type": "pause", "duration": 80},
                        {"type": "pointerUp", "button": 0},
                    ],
                }
                m.cmd("WebDriver:PerformActions", actions=[finger("left", -1), finger("right", 1)])
                m.cmd("WebDriver:ReleaseActions")
            elif kind == "size":
                w, h = arg.split("x")
                m.cmd("WebDriver:SetWindowRect", width=int(w), height=int(h))
            elif kind == "shot":
                with open(arg, "wb") as f:
                    f.write(base64.b64decode(m.cmd("WebDriver:TakeScreenshot", full=False)["value"]))
            elif kind == "js":
                print("js:", json.dumps(run(arg))[:2000])
            elif kind == "sh":
                print("sh:", subprocess.run(arg, shell=True, capture_output=True, text=True).stdout.strip()[:500])
            elif kind == "key":
                key, _, mod = arg.partition(":")
                code = key if len(key) > 1 else ("Digit" + key if key.isdigit() else "Key" + key.upper())
                held = f"ctrlKey:{str(mod == 'ctrl').lower()},altKey:{str(mod == 'alt').lower()}"
                run(
                    "const c=document.getElementById('cuprita');c.focus();for(const t of ['keydown','keyup'])"
                    f"c.dispatchEvent(new KeyboardEvent(t,{{key:'{key}',code:'{code}',{held},bubbles:true,cancelable:true}}));"
                )
            elif kind == "click":
                x, y = arg.split(",")
                down, up = EVENT % (x, y, 1), EVENT % (x, y, 0)
                run(
                    f"const c=document.getElementById('cuprita');c.dispatchEvent(new PointerEvent('pointermove',{down}));"
                    f"c.dispatchEvent(new PointerEvent('pointerdown',{down}));c.dispatchEvent(new PointerEvent('pointerup',{up}));"
                )
            else:
                raise SystemExit(f"no step {kind}\n{__doc__}")
        m.cmd("WebDriver:DeleteSession")
    finally:
        firefox.terminate()
        try:
            said = firefox.communicate(timeout=10)[0].decode(errors="replace")
        except subprocess.TimeoutExpired:
            firefox.kill()
            said = ""
        shutil.rmtree(profile, ignore_errors=True)
        errors = [line for line in said.splitlines() if "console.error" in line or "panicked" in line or "JavaScript error" in line]
        print("\n".join(errors[-20:]))


main()
