"""Fixtures: a mock BlueZ (`examples/mock_bluez.rs`) on a private D-Bus bus.

The mock binary is taken from ``$BLUEBUS_MOCK`` or built with
``cargo build --example mock_bluez`` on first use. Each test gets its own
daemon; ``DBUS_SYSTEM_BUS_ADDRESS`` points at it, so ``bluebus.Session``
connects to the mock.
"""

import asyncio
import json
import os
import pathlib
import subprocess

import pytest

ROOT = pathlib.Path(__file__).resolve().parents[2]


def _mock_binary() -> str:
    explicit = os.environ.get("BLUEBUS_MOCK")
    if explicit:
        return explicit
    subprocess.run(
        ["cargo", "build", "--quiet", "--example", "mock_bluez"],
        cwd=ROOT,
        check=True,
    )
    target = pathlib.Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
    return str(target / "debug" / "examples" / "mock_bluez")


@pytest.fixture(scope="session")
def mock_binary():
    return _mock_binary()


class MockBluez:
    HEADSET = "11:22:33:44:55:66"
    ANON = "AA:BB:CC:DD:EE:FF"
    HEADSET_PATH = "/org/bluez/hci0/dev_11_22_33_44_55_66"
    CHR_PATH = HEADSET_PATH + "/service0001/char0002"

    def __init__(self, binary: str):
        self.proc = subprocess.Popen(
            [binary],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            text=True,
            bufsize=1,
        )
        line = self.proc.stdout.readline().strip()
        if not line.startswith("ADDRESS="):
            raise RuntimeError(f"mock_bluez did not start: {line!r}")
        self.address = line[len("ADDRESS="):]

    def cmd(self, line: str) -> str:
        """Sends one command; returns the text after ``ok`` (raises on ``error``)."""
        self.proc.stdin.write(line + "\n")
        self.proc.stdin.flush()
        answer = self.proc.stdout.readline().rstrip("\n")
        if answer == "ok":
            return ""
        if answer.startswith("ok "):
            return answer[3:]
        raise RuntimeError(f"mock: {line!r} → {answer!r}")

    async def acmd(self, line: str) -> str:
        """`cmd` without blocking the event loop (needed when the mock calls
        back into this process, e.g. ``write_local``)."""
        return await asyncio.to_thread(self.cmd, line)

    async def state(self) -> dict:
        return json.loads(await self.acmd("state"))

    def close(self):
        if self.proc.poll() is None:
            try:
                self.proc.stdin.write("quit\n")
                self.proc.stdin.flush()
                self.proc.stdin.close()
            except BrokenPipeError:
                pass
            try:
                self.proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.proc.terminate()
                self.proc.wait(timeout=5)
        self.proc.stdout.close()


@pytest.fixture
def mock(mock_binary, monkeypatch):
    bz = MockBluez(mock_binary)
    monkeypatch.setenv("DBUS_SYSTEM_BUS_ADDRESS", bz.address)
    yield bz
    bz.close()


async def eventually(check, timeout=5.0, interval=0.01):
    """Awaits ``check()`` (sync or async) until it is truthy."""
    deadline = asyncio.get_running_loop().time() + timeout
    while True:
        result = check()
        if asyncio.iscoroutine(result):
            result = await result
        if result:
            return result
        if asyncio.get_running_loop().time() > deadline:
            raise AssertionError("condition not met in time")
        await asyncio.sleep(interval)
