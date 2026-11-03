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
COMMANDS = ["bash", "sh", "python3", "cat", "dirname", "id", "mkdir", "mkfifo", "mktemp", "rm", "timeout"]

FAKE_DOCKER = r'''#!/usr/bin/env python3
import json, os, sys
args = sys.argv[1:]
log = os.environ["FAKE_DOCKER_LOG"]
with open(log, "w") as f:
    json.dump(args, f)
assert args[0] == "run", args
env = {"PATH": os.environ["PATH"], "FAKE_ENGINE_REPORT": os.environ["FAKE_ENGINE_REPORT"]}
cwd = None
i = 1
valued = {"--user", "-w", "-v", "--gpus", "-e"}
while args[i].startswith("-"):
    opt = args[i]
    if opt in valued:
        value = args[i + 1]
        i += 2
        if opt == "-w":
            cwd = value
        elif opt == "-e":
            name, eq, val = value.partition("=")
            env[name] = val if eq else os.environ.get(name, "")
    else:
        i += 1
image, command = args[i], args[i + 1:]
if command and command[0] == "exit":
    sys.exit(int(command[1]))
os.chdir(cwd)
os.execve(os.environ["FAKE_ENGINE"], ["scene-render", *command], env)
'''

FAKE_ENGINE = r'''#!/usr/bin/env python3
import json, os, signal, subprocess, sys, tempfile, time
mode, name = sys.argv[1], sys.argv[2]
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

    def run(self, *args, env=None, path_dirs=()):
        full = {"PATH": os.pathsep.join([*map(str, path_dirs), str(self.bin), str(self.commands)]),
                "HOME": str(self.home), "FAKE_DOCKER_LOG": str(self.log), "FAKE_ENGINE": str(self.engine),
                "FAKE_ENGINE_REPORT": str(self.report), "HOST_ONLY": "host", "TMPDIR": str(self.base),
                "SCENE_RENDER_GPUS": "none", **(env or {})}
        done = subprocess.run([str(WRAPPER), *args], cwd=self.project, env=full, capture_output=True, text=True,
                              timeout=60)
        return done

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
