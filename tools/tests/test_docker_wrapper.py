"""tools/docker/scene-render: the docker command it builds, and providers answered on the host through its relay.

A fake `docker` stands in for the container: it keeps only the variables given with `-e`, works in the `-w`
folder and runs a fake engine that calls a provider as sr-resolve does (the command split on whitespace, the
request on standard input, the last line of output as the answer, the result written in a scratch folder under
TMPDIR). A variable set only on the host, HOST_ONLY, shows where the provider ran.
"""
import json
import os
import re
import shutil
import signal
import stat
import subprocess
import tempfile
import textwrap
import time
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
WRAPPER = ROOT / "tools" / "docker" / "scene-render"
# what the wrapper, its relay and the fakes run; PATH holds only these, so no host GPU tool is found by accident
COMMANDS = ["bash", "sh", "python3", "cat", "dirname", "id", "mkdir", "mkfifo", "mktemp", "rm", "setsid", "sleep",
            "timeout"]

# The container runs in a session of its own, as one owned by the daemon does, so it outlives a killed client;
# `docker kill NAME` ends it, and the client hands HUP, INT and TERM to it as --sig-proxy does.
FAKE_DOCKER = r'''#!/usr/bin/env python3
import json, os, signal, subprocess, sys
args = sys.argv[1:]
log = os.environ["FAKE_DOCKER_LOG"]
state = os.environ["FAKE_DOCKER_STATE"]
if args[0] == "kill":
    with open(log + ".kill", "a") as f:
        f.write(args[1] + "\n")
    try:
        os.kill(int(open(os.path.join(state, args[1])).read()), signal.SIGKILL)
    except (FileNotFoundError, ProcessLookupError):
        sys.exit(1)
    sys.exit(0)
with open(log, "w") as f:
    json.dump(args, f)
assert args[0] == "run", args
open(os.environ["FAKE_ENGINE_REPORT"] + ".client", "w").write(str(os.getpid()))
env = {"PATH": os.environ["PATH"], "FAKE_ENGINE_REPORT": os.environ["FAKE_ENGINE_REPORT"]}
cwd = name = None
i = 1
valued = {"--user", "-w", "-v", "--gpus", "-e", "--name"}
while args[i].startswith("-"):
    opt = args[i]
    if opt in valued:
        value = args[i + 1]
        i += 2
        if opt == "-w":
            cwd = value
        elif opt == "--name":
            name = value
        elif opt == "-e":
            var, eq, val = value.partition("=")
            env[var] = val if eq else os.environ.get(var, "")
    else:
        i += 1
image, command = args[i], args[i + 1:]
if command and command[0] == "exit":
    sys.exit(int(command[1]))
container = subprocess.Popen(["scene-render", *command], executable=os.environ["FAKE_ENGINE"], env=env, cwd=cwd,
                             start_new_session=True)
open(os.path.join(state, name), "w").write(str(container.pid))
for sig in (signal.SIGHUP, signal.SIGINT, signal.SIGTERM):
    signal.signal(sig, lambda s, _: container.send_signal(s))
status = container.wait()
os.remove(os.path.join(state, name))
sys.exit(status if status >= 0 else 128 - status)
'''

FAKE_ENGINE = r'''#!/usr/bin/env python3
import json, os, signal, subprocess, sys, tempfile, time
mode = sys.argv[1]
if mode == "stdin":
    open(os.environ["FAKE_ENGINE_REPORT"], "w").write(sys.stdin.read())
    sys.exit(0)
if mode == "hold":
    pid = os.environ["FAKE_ENGINE_REPORT"] + ".engine"
    open(pid + ".new", "w").write(str(os.getpid()))
    os.replace(pid + ".new", pid)
    time.sleep(60)
    sys.exit(0)
name = sys.argv[2]
command = os.environ["SR_PROVIDER_" + name.upper().replace("-", "_")].split()
work = tempfile.mkdtemp(prefix="scene-render-resolve-")
request = {"kind": "speech", "provider": name, "prompt": "hello", "output": os.path.join(work, "result.wav"),
           "workdir": work}
child = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                         start_new_session=True)
if mode == "abandon":
    child.stdin.write(json.dumps(request).encode()); child.stdin.close()
    deadline = time.monotonic() + 10
    while not os.path.exists(os.environ["FAKE_ENGINE_REPORT"] + ".pid") and time.monotonic() < deadline:
        time.sleep(0.05)
    os.killpg(child.pid, signal.SIGKILL)
    sys.exit(1)
out, err = child.communicate(json.dumps(request).encode())
lines = [l for l in out.decode().splitlines() if l.strip()]
report = {"command": command, "status": child.returncode, "answer": json.loads(lines[-1]) if lines else None,
          "stderr": err.decode(), "result": open(request["output"]).read() if os.path.exists(request["output"]) else None,
          "tmpdir": os.environ.get("TMPDIR"), "env": {k: v for k, v in os.environ.items() if k.startswith(("SR_", "ELEVEN"))}}
with open(os.environ["FAKE_ENGINE_REPORT"], "w") as f:
    json.dump(report, f)
'''

PROVIDER = r'''#!/usr/bin/env python3
import json, os, sys, time
request = json.load(sys.stdin)
report = os.environ["FAKE_ENGINE_REPORT"]
if "--hang" in sys.argv:
    open(report + ".pid", "w").write(str(os.getpid()))
    time.sleep(30)
with open(request["output"], "w") as f:
    f.write(json.dumps({"host_only": os.environ.get("HOST_ONLY"), "cwd": os.getcwd(), "argv": sys.argv[1:],
                        "prompt": request["prompt"]}))
print("progress", file=sys.stderr)
print("noise")
if "--fail" in sys.argv:
    print(json.dumps({"ok": False, "error": "voice not found"}))
    sys.exit(3)
print(json.dumps({"ok": True, "version": "p1"}))
'''


def executable(path, text):
    path.write_text(text)
    path.chmod(path.stat().st_mode | stat.S_IXUSR)
    return path


class Fixture:
    def __init__(self, base):
        self.base = Path(base)
        self.bin = self.base / "bin"
        self.bin.mkdir()
        executable(self.bin / "docker", FAKE_DOCKER)
        self.commands = self.base / "commands"
        self.commands.mkdir()
        for name in COMMANDS:
            (self.commands / name).symlink_to(shutil.which(name))
        self.engine = executable(self.base / "engine", FAKE_ENGINE)
        self.project = self.base / "project"
        self.project.mkdir()
        executable(self.project / "voice.py", PROVIDER)
        self.home = self.base / "home"
        self.home.mkdir()
        self.log = self.base / "docker.json"
        self.report = self.base / "report.json"
        self.containers = self.base / "containers"
        self.containers.mkdir()

    def env(self, env=None, path_dirs=()):
        return {"PATH": os.pathsep.join([*map(str, path_dirs), str(self.bin), str(self.commands)]),
                "HOME": str(self.home), "FAKE_DOCKER_LOG": str(self.log), "FAKE_DOCKER_STATE": str(self.containers),
                "FAKE_ENGINE": str(self.engine), "FAKE_ENGINE_REPORT": str(self.report), "HOST_ONLY": "host",
                "TMPDIR": str(self.base), "SCENE_RENDER_GPUS": "none", **(env or {})}

    def run(self, *args, env=None, path_dirs=(), input=None):
        return subprocess.run([str(WRAPPER), *args], cwd=self.project, env=self.env(env, path_dirs), input=input,
                              capture_output=True, text=True, timeout=60)

    def start(self, *args, env=None):
        """The wrapper in a session of its own, once its container runs; returns it and the container's pid."""
        wrapper = subprocess.Popen([str(WRAPPER), *args], cwd=self.project, env=self.env(env),
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
        engine = Path(str(self.report) + ".engine")
        deadline = time.monotonic() + 20
        while not engine.exists() and time.monotonic() < deadline:
            time.sleep(0.05)
        return wrapper, int(engine.read_text())

    def kills(self):
        log = Path(str(self.log) + ".kill")
        return log.read_text().split() if log.exists() else []

    def argv(self):
        return json.loads(self.log.read_text())

    def engine_report(self):
        return json.loads(self.report.read_text())


def options(argv, flag):
    return [argv[i + 1] for i, a in enumerate(argv) if a == flag and i + 1 < len(argv)]


class RelayTests(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.f = Fixture(tmp.name)

    def test_a_provider_variable_runs_on_the_host_through_the_relay(self):
        done = self.f.run("call", "voice", env={"SR_PROVIDER_VOICE": "python3 voice.py --lang pt"})
        self.assertEqual(done.returncode, 0, done.stderr)
        report = self.f.engine_report()
        self.assertEqual(report["command"][0], "sh")
        self.assertTrue(report["command"][1].endswith("/relay") and report["command"][2:] == ["VOICE"])
        self.assertEqual((report["status"], report["answer"]), (0, {"ok": True, "version": "p1"}))
        self.assertIn("progress", report["stderr"])
        result = json.loads(report["result"])
        self.assertEqual(result, {"host_only": "host", "cwd": str(self.f.project), "argv": ["--lang", "pt"],
                                  "prompt": "hello"})
        self.assertTrue(report["tmpdir"].startswith(str(self.f.base)))
        # the relay folder is gone once the container exits
        self.assertFalse(Path(report["tmpdir"]).exists())

    def test_a_failing_provider_returns_its_status_and_answer(self):
        self.f.run("call", "voice", env={"SR_PROVIDER_VOICE": "./voice.py --fail"})
        report = self.f.engine_report()
        self.assertEqual((report["status"], report["answer"]), (3, {"ok": False, "error": "voice not found"}))

    def test_a_provider_that_does_not_exist_on_the_host_fails_with_127(self):
        self.f.run("call", "voice", env={"SR_PROVIDER_VOICE": "./missing-provider"})
        report = self.f.engine_report()
        self.assertEqual((report["status"], report["answer"]), (127, None))
        self.assertIn("missing-provider", report["stderr"])

    def test_path_programs_are_relayed_and_a_variable_overrides_them(self):
        programs = self.f.base / "programs"
        programs.mkdir()
        executable(programs / "scene-render-provider-my-music", PROVIDER)
        executable(programs / "scene-render-provider-voice", "#!/bin/sh\nexit 9\n")
        self.f.run("call", "my-music", path_dirs=[programs])
        report = self.f.engine_report()
        self.assertEqual(report["command"][2:], ["MY_MUSIC"])
        self.assertEqual(report["answer"], {"ok": True, "version": "p1"})
        self.f.run("call", "voice", path_dirs=[programs], env={"SR_PROVIDER_VOICE": "./voice.py"})
        self.assertEqual(self.f.engine_report()["answer"], {"ok": True, "version": "p1"})

    def test_provider_commands_never_reach_the_container(self):
        self.f.run("call", "voice", env={"SR_PROVIDER_VOICE": "./voice.py", "SR_PROVIDER_EMPTY": "  "})
        argv = self.f.argv()
        provider_env = [e for e in options(argv, "-e") if e.startswith("SR_PROVIDER_")]
        self.assertEqual(len(provider_env), 1)
        self.assertRegex(provider_env[0], r"^SR_PROVIDER_VOICE=sh /\S+/relay VOICE$")
        self.assertNotIn("voice.py", " ".join(argv))

    def test_an_abandoned_call_leaves_no_provider_running(self):
        self.f.run("abandon", "voice", env={"SR_PROVIDER_VOICE": "./voice.py --hang"})
        pid = int(Path(str(self.f.report) + ".pid").read_text())
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline and alive(pid):
            time.sleep(0.05)
        self.assertFalse(alive(pid))
        self.assertEqual([p.name for p in self.f.base.glob("tmp.*")], [])


class StopTests(unittest.TestCase):
    """The container stops however the wrapper or its client ends, and no relay is left behind."""

    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.f = Fixture(tmp.name)

    def assertStopped(self, engine, server=None):
        self.assertTrue(eventually(lambda: not alive(engine)), "the container still runs")
        self.assertIn(options(self.f.argv(), "--name")[0], self.f.kills())
        self.assertTrue(eventually(lambda: not list(self.f.base.glob("tmp.*"))), "the relay folder is left")
        if server is not None:
            self.assertFalse(alive(server))

    def server(self):
        return int(next(self.f.base.glob("tmp.*/server")).read_text())

    def test_sigkill_of_the_wrapper_stops_the_container(self):
        wrapper, engine = self.f.start("hold")
        os.kill(wrapper.pid, signal.SIGKILL)
        wrapper.wait()
        self.assertStopped(engine)

    def test_sigkill_of_the_wrapper_with_providers_stops_the_container_and_the_relay(self):
        wrapper, engine = self.f.start("hold", env={"SR_PROVIDER_VOICE": "./voice.py"})
        server = self.server()
        os.kill(wrapper.pid, signal.SIGKILL)
        wrapper.wait()
        self.assertStopped(engine, server)

    def test_sigkill_of_the_process_group_stops_the_container(self):
        wrapper, engine = self.f.start("hold", env={"SR_PROVIDER_VOICE": "./voice.py"})
        server = self.server()
        os.killpg(wrapper.pid, signal.SIGKILL)
        wrapper.wait()
        self.assertStopped(engine, server)

    def test_sigkill_of_the_client_stops_the_container(self):
        wrapper, engine = self.f.start("hold")
        os.kill(int(Path(str(self.f.report) + ".client").read_text()), signal.SIGKILL)
        self.assertEqual(wrapper.wait(timeout=20), 128 + signal.SIGKILL)
        self.assertStopped(engine)

    def test_sigterm_reaches_the_container_and_its_status_is_the_wrapper_s(self):
        for env in ({}, {"SR_PROVIDER_VOICE": "./voice.py"}):
            with self.subTest(env=env):
                wrapper, engine = self.f.start("hold", env=env)
                os.kill(wrapper.pid, signal.SIGTERM)
                self.assertEqual(wrapper.wait(timeout=20), 128 + signal.SIGTERM)
                self.assertFalse(alive(engine))
                self.assertEqual(list(self.f.base.glob("tmp.*")), [])
                Path(str(self.f.report) + ".engine").unlink()

    def test_a_container_that_exits_is_not_killed_and_leaves_no_watchdog(self):
        done = self.f.run("stdin", input="a document on standard input")
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertEqual(self.f.report.read_text(), "a document on standard input")
        self.assertEqual(self.f.kills(), [])
        name = options(self.f.argv(), "--name")[0]
        self.assertTrue(eventually(lambda: not processes_mentioning(name)), "the watchdog is left")

    def test_each_run_names_its_own_container(self):
        self.f.run("stdin", input="")
        first = options(self.f.argv(), "--name")
        self.f.run("stdin", input="")
        second = options(self.f.argv(), "--name")
        self.assertRegex(first[0], r"^scene-render-\d+-\d+$")
        self.assertEqual(len(first), 1)
        self.assertNotEqual(first, second)


def eventually(condition, seconds=10):
    deadline = time.monotonic() + seconds
    while not condition():
        if time.monotonic() > deadline:
            return False
        time.sleep(0.05)
    return True


def processes_mentioning(text):
    found = []
    for cmdline in Path("/proc").glob("[0-9]*/cmdline"):
        try:
            if text.encode() in cmdline.read_bytes():
                found.append(cmdline.parent.name)
        except OSError:
            pass
    return found


def alive(pid):
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    # a zombie has exited; only its parent has not reaped it yet
    status = Path(f"/proc/{pid}/status")
    return not (status.exists() and re.search(r"^State:\s+Z", status.read_text(), re.M))


class CommandTests(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.f = Fixture(tmp.name)

    def test_without_providers_it_runs_the_image_as_the_caller(self):
        done = self.f.run("exit", "0")
        self.assertEqual(done.returncode, 0, done.stderr)
        argv = self.f.argv()
        self.assertEqual(argv[:4], ["run", "--rm", "-i", "--init"])
        self.assertEqual(len(options(argv, "--name")), 1)
        self.assertEqual(options(argv, "--user"), [f"{os.getuid()}:{os.getgid()}"])
        self.assertEqual(options(argv, "-w"), [str(self.f.project)])
        self.assertEqual(argv[-3:], ["pedroanisio/scene-render:0.9.0", "exit", "0"])
        self.assertNotIn("--gpus", argv)
        self.assertFalse(any(e.startswith(("TMPDIR=", "SR_PROVIDER_")) for e in options(argv, "-e")))
        self.assertIn(f"{self.f.home}/.cache/scene-render/container:/cache", options(argv, "-v"))

    def test_the_container_exit_status_is_the_wrapper_s(self):
        self.assertEqual(self.f.run("exit", "7").returncode, 7)
        self.assertEqual(self.f.run("exit", "7", env={"SR_PROVIDER_VOICE": "./voice.py"}).returncode, 7)

    def test_folders_are_mounted_at_their_own_paths(self):
        other = self.f.base / "other"
        (other / "media").mkdir(parents=True)
        (other / "film.scene.xml").write_text("<scene/>")
        out = self.f.base / "renders"
        out.mkdir()
        extra = self.f.base / "extra"
        extra.mkdir()
        (self.f.project / "sub").mkdir()
        self.f.run("exit", "0", str(other / "film.scene.xml"), f"--output={out}/film.mp4", "sub/x.png", "30000/1001",
                   env={"SR_RESOLVE_ROOT": str(other), "SCENE_RENDER_MOUNTS": f"{extra}:/", "IMAGE": "x"})
        volumes = options(self.f.argv(), "-v")
        store = f"{self.f.home}/.cache/scene-render/resolve"
        for d in (self.f.project, other, out, extra, store):
            self.assertIn(f"{d}:{d}", volumes)
        self.assertNotIn("/:/", volumes)
        self.assertEqual(len([v for v in volumes if v.startswith(f"{self.f.project}")]), 1)  # sub/ is inside it
        self.assertIn(f"SR_RESOLVE_STORE={store}", options(self.f.argv(), "-e"))
        self.assertTrue(Path(store).is_dir())

    def test_a_store_turned_off_stays_off(self):
        self.f.run("exit", "0", env={"SR_RESOLVE_STORE": ""})
        self.assertIn("SR_RESOLVE_STORE=", options(self.f.argv(), "-e"))

    def test_variables_pass_by_name_and_host_programs_stay_out(self):
        self.f.run("exit", "0", env={"SR_GPU_BACKEND": "vulkan", "ELEVENLABS_API_KEY": "secret-key",
                                     "SR_FFMPEG": "/opt/ffmpeg", "SR_PROVIDER_TIMEOUT": "60", "UNRELATED": "1"})
        argv = self.f.argv()
        env = options(argv, "-e")
        self.assertIn("SR_GPU_BACKEND", env)
        self.assertIn("ELEVENLABS_API_KEY", env)
        self.assertIn("SR_PROVIDER_TIMEOUT", env)
        self.assertNotIn("SR_FFMPEG", env)
        self.assertNotIn("UNRELATED", env)
        self.assertNotIn("secret-key", " ".join(argv))

    def test_gpus(self):
        nvidia = self.f.base / "nvidia"
        nvidia.mkdir()
        executable(nvidia / "nvidia-smi", "#!/bin/sh\n")
        toolkit = self.f.base / "toolkit"
        toolkit.mkdir()
        executable(toolkit / "nvidia-container-cli", "#!/bin/sh\n")
        caps = ["--gpus", "all", "-e", "NVIDIA_DRIVER_CAPABILITIES=all"]

        self.f.run("exit", "0", env={"SCENE_RENDER_GPUS": "auto"}, path_dirs=[nvidia, toolkit])
        self.assertTrue(contains(self.f.argv(), caps))

        done = self.f.run("exit", "0", env={"SCENE_RENDER_GPUS": "auto"}, path_dirs=[nvidia])
        self.assertNotIn("--gpus", self.f.argv())
        self.assertIn("NVIDIA Container Toolkit", done.stderr)

        done = self.f.run("exit", "0", env={"SCENE_RENDER_GPUS": "auto"})
        self.assertNotIn("--gpus", self.f.argv())
        self.assertEqual(done.stderr, "")

        self.f.run("exit", "0", env={"SCENE_RENDER_GPUS": "none"}, path_dirs=[nvidia, toolkit])
        self.assertNotIn("--gpus", self.f.argv())

        self.f.run("exit", "0", env={"SCENE_RENDER_GPUS": "device=1"})
        self.assertTrue(contains(self.f.argv(), ["--gpus", "device=1", "-e", "NVIDIA_DRIVER_CAPABILITIES=all"]))

    def test_the_image_can_be_named(self):
        self.f.run("exit", "0", env={"SCENE_RENDER_IMAGE": "scene-render:dev"})
        self.assertEqual(self.f.argv()[-3], "scene-render:dev")


def contains(seq, sub):
    return any(seq[i:i + len(sub)] == sub for i in range(len(seq) - len(sub) + 1))


class VersionTests(unittest.TestCase):
    def test_the_default_image_is_this_version(self):
        version = re.search(r'^version = "([^"]+)"', (ROOT / "Cargo.toml").read_text(), re.M).group(1)
        default = re.search(r"^image=\$\{SCENE_RENDER_IMAGE:-pedroanisio/scene-render:([^}]+)\}$",
                            WRAPPER.read_text(), re.M)
        self.assertIsNotNone(default)
        self.assertEqual(default.group(1), version)


if __name__ == "__main__":
    unittest.main()
