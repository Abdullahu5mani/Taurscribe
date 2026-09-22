#!/usr/bin/env python3
"""Remote meeting participant in a Linux VM (Tart), driven over CDP through SSH.

The Mac under test should only run what a real user runs: their own Chrome in
the meeting (plus BlackHole standing in for their voice). Remote participants
really are on other machines, so the test puts them in a VM: their browser runs
there, their audio reaches the Mac through the meeting itself, and nothing about
them shows up among the Mac's audio processes.

One-time setup (already done on this Mac):
  tart clone ghcr.io/cirruslabs/ubuntu:latest taurscribe-remote
  tart set taurscribe-remote --cpu 2 --memory 4096
  (in the VM) sudo snap install chromium
  ssh-keygen + ssh-copy-id to ~/.taurscribe-harness/vm_key

Chromium binds DevTools to the VM's localhost only, so the harness reaches it
through an SSH tunnel: localhost:<local_port> on the Mac -> VM:9222.
"""

from __future__ import annotations

import json
import subprocess
import time
import urllib.request
from pathlib import Path
from typing import List, Optional

VM_NAME = "taurscribe-remote"
VM_USER = "admin"
VM_KEY = Path.home() / ".taurscribe-harness" / "vm_key"
VM_CDP_PORT = 9222


def _tart(*args: str, timeout: float = 60) -> str:
    res = subprocess.run(["tart", *args], capture_output=True, text=True, timeout=timeout)
    if res.returncode != 0:
        raise RuntimeError(f"tart {' '.join(args)} failed: {res.stderr.strip()}")
    return res.stdout.strip()


def vm_state() -> Optional[str]:
    for line in _tart("list").splitlines():
        cols = line.split()
        if len(cols) >= 2 and cols[0] == "local" and cols[1] == VM_NAME:
            return cols[-1]  # "running" / "stopped"
    return None


def ensure_vm_running(timeout: float = 120) -> str:
    """Boots the VM headless if needed; returns its IP."""
    state = vm_state()
    if state is None:
        raise RuntimeError(f"Tart VM '{VM_NAME}' does not exist (see vm_remote.py setup notes)")
    if state != "running":
        subprocess.Popen(["tart", "run", "--no-graphics", VM_NAME],
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            ip = _tart("ip", VM_NAME, timeout=10)
            if ip and ssh(ip, "true", timeout=10, check=False).returncode == 0:
                return ip
        except Exception:
            pass
        time.sleep(3)
    raise RuntimeError(f"VM '{VM_NAME}' did not come up within {timeout:.0f}s")


def _ssh_base(ip: str) -> List[str]:
    return ["ssh", "-i", str(VM_KEY), "-o", "StrictHostKeyChecking=no", "-o", "UserKnownHostsFile=/dev/null",
            "-o", "LogLevel=ERROR", "-o", "BatchMode=yes", f"{VM_USER}@{ip}"]


def ssh(ip: str, command: str, timeout: float = 60, check: bool = True) -> subprocess.CompletedProcess:
    res = subprocess.run(_ssh_base(ip) + [command], capture_output=True, text=True, timeout=timeout)
    if check and res.returncode != 0:
        raise RuntimeError(f"ssh '{command[:60]}' failed: {res.stderr.strip()}")
    return res


class RemoteChromium:
    """A Chromium in the VM with DevTools tunnelled to localhost:<local_port>.

    profile: directory name under the VM user's home. fresh=True wipes it first
    (anonymous guests); fresh=False keeps it (a signed-in host, e.g. Zoom).
    """

    def __init__(self, local_port: int, remote_port: int = VM_CDP_PORT,
                 profile: str = "taurscribe-remote", fresh: bool = True):
        self.local_port = local_port
        self.remote_port = remote_port
        self.profile = profile
        self.fresh = fresh
        self.ip: Optional[str] = None
        self.tunnel: Optional[subprocess.Popen] = None

    def _kill_pattern(self) -> str:
        # Bracketed first letter: the pattern must not match the shell running it.
        return f"'[u]ser-data-dir=.*/{self.profile}( |$)'"

    def start(self) -> None:
        self.ip = ensure_vm_running()
        reset = f"rm -rf ~/{self.profile}; " if self.fresh else ""
        ssh(self.ip, f"pkill -f {self._kill_pattern()} ; sleep 1; {reset}mkdir -p ~/{self.profile}", check=False)
        ssh(self.ip, (
            "setsid nohup /snap/bin/chromium --headless=new "
            f"--remote-debugging-port={self.remote_port} --remote-allow-origins='*' "
            f"--user-data-dir=$HOME/{self.profile} --no-first-run --no-default-browser-check "
            "--use-fake-ui-for-media-stream --use-fake-device-for-media-stream "
            # The remote participant never plays sound: it is on another machine anyway.
            "--mute-audio --autoplay-policy=no-user-gesture-required --window-size=1280,800 "
            # Headless Chromium says "HeadlessChrome"; meeting sites may refuse it.
            "--user-agent='Mozilla/5.0 (X11; Linux aarch64) AppleWebKit/537.36 (KHTML, like Gecko) "
            "Chrome/153.0.0.0 Safari/537.36' "
            f"about:blank > ~/{self.profile}.log 2>&1 < /dev/null &"
        ))
        self.tunnel = subprocess.Popen(
            _ssh_base(self.ip)[:-1] + ["-N", "-L", f"{self.local_port}:127.0.0.1:{self.remote_port}",
                                       "-o", "ExitOnForwardFailure=yes", f"{VM_USER}@{self.ip}"],
            stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, start_new_session=True)
        deadline = time.time() + 30
        while time.time() < deadline:
            try:
                with urllib.request.urlopen(f"http://127.0.0.1:{self.local_port}/json/version", timeout=2) as r:
                    json.load(r)
                return
            except Exception:
                if self.tunnel.poll() is not None:
                    raise RuntimeError(f"SSH tunnel to the VM exited: {self.tunnel.stderr.read().decode()[:300]}")
                time.sleep(0.5)
        raise RuntimeError("Chromium in the VM did not expose DevTools through the tunnel")

    def stop(self) -> None:
        if self.ip:
            try:
                ssh(self.ip, f"pkill -f {self._kill_pattern()}", timeout=15, check=False)
            except Exception:
                pass
        if self.tunnel and self.tunnel.poll() is None:
            self.tunnel.terminate()
            try:
                self.tunnel.wait(timeout=5)
            except Exception:
                self.tunnel.kill()
        self.tunnel = None
