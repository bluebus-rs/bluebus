"""Tests of the Python bindings against the mock BlueZ (see conftest.py)."""

import asyncio
import gc
import logging
import pickle

import pytest

import bluebus
from conftest import MockBluez, eventually

SVC_UUID = "0000180f-0000-1000-8000-00805f9b34fb"
CHR_UUID = "00002a19-0000-1000-8000-00805f9b34fb"
DESC_UUID = "00002902-0000-1000-8000-00805f9b34fb"


async def powered(mock):
    session = await bluebus.Session.new()
    adapter = await session.default_adapter()
    await adapter.power_on()
    return session, adapter


async def discovered(mock):
    session, adapter = await powered(mock)
    assert len(await adapter.scan(timeout=0.1)) == 2
    return session, adapter


# ---------------------------------------------------------------------------
# Module, exceptions, data classes
# ---------------------------------------------------------------------------


def test_module_and_exceptions():
    assert bluebus.__version__
    assert bluebus.normalize_uuid("0x2A19") == CHR_UUID
    with pytest.raises(bluebus.InvalidArgument):
        bluebus.normalize_uuid("heart")
    for cls in (bluebus.BluebusTimeout, bluebus.NotFound, bluebus.InvalidArgument, bluebus.DBusError):
        assert issubclass(cls, bluebus.BluebusError)
    assert issubclass(bluebus.BluebusTimeout, TimeoutError)
    assert issubclass(bluebus.NotFound, LookupError)
    assert issubclass(bluebus.InvalidArgument, ValueError)
    e = bluebus.DBusError("boom", "org.bluez.Error.Failed")
    assert e.name == "org.bluez.Error.Failed" and str(e) == "boom"
    e2 = pickle.loads(pickle.dumps(e))
    assert e2.name == e.name and str(e2) == "boom"


def test_builders_are_plain_values():
    c = bluebus.Characteristic("2a19", read=True, notify=True, value=b"\x64")
    assert c.flags == ["read", "notify"]
    assert c.value == b"\x64"
    assert c.descriptor("2901", b"x") is c
    c2 = bluebus.Characteristic("2a06", write=True, flags=["write", "encrypt-write"], value=bytearray(b"ab"))
    assert c2.flags == ["write", "encrypt-write"] and c2.value == b"ab"
    with pytest.raises(bluebus.InvalidArgument):
        bluebus.Characteristic("2a06", write=True, on_write=42)
    s = bluebus.Service("180f").characteristic(c).characteristic(c2)
    assert [x.uuid for x in s.characteristics] == ["2a19", "2a06"]
    assert s.primary and "180f" in repr(s)
    a = bluebus.Advertisement(local_name="x", service_uuids=["180f"], manufacturer_data={0xFFFF: b"\x01"})
    assert a.manufacturer_data == {0xFFFF: b"\x01"} and a.connectable
    assert "local_name='x'" in repr(a)


# ---------------------------------------------------------------------------
# Session and adapters
# ---------------------------------------------------------------------------


async def test_session_and_adapters(mock):
    session = await bluebus.Session.new()
    infos = await session.adapters()
    assert [a.name for a in infos] == ["hci0", "hci1"]
    assert infos[0].address == "00:AA:00:AA:00:AA"
    assert infos[0].alias == "mockhost" and not infos[0].powered
    assert "hci0" in repr(infos[0])

    # Awaiting the constructor works too.
    other = await bluebus.Session()
    assert "Session" in repr(other)
    assert (await other.default_adapter()).name == "hci0"

    for key in ("hci1", "/org/bluez/hci1", "00:bb:00:bb:00:bb"):
        a = await session.adapter(key)
        assert (a.name, a.path, a.address) == ("hci1", "/org/bluez/hci1", "00:BB:00:BB:00:BB")
    assert "hci1" in repr(a)
    with pytest.raises(bluebus.NotFound, match="hci9"):
        await session.adapter("hci9")
    with pytest.raises(LookupError):
        await session.adapter("hci9")

    # A session that was never awaited.
    with pytest.raises(bluebus.InvalidArgument, match="not connected"):
        bluebus.Session().adapters()


async def test_power(mock):
    session = await bluebus.Session.new()
    adapter = await session.default_adapter()
    assert not await adapter.is_powered()
    await adapter.power_on()
    assert await adapter.is_powered()
    assert (await adapter.info()).powered
    await adapter.power_on()  # already on
    await adapter.power_off()
    assert not await adapter.is_powered()


async def test_dbus_error_names(mock):
    session = await bluebus.Session.new()
    adapter = await session.default_adapter()
    # Scanning with the radio off: org.bluez.Error.NotReady.
    with pytest.raises(bluebus.DBusError) as info:
        await adapter.scan(timeout=0.1)
    assert info.value.name == "org.bluez.Error.NotReady"
    assert isinstance(info.value, bluebus.BluebusError)


async def test_session_connect_error(monkeypatch):
    monkeypatch.setenv("DBUS_SYSTEM_BUS_ADDRESS", "unix:path=/nonexistent/bluebus-test-bus")
    with pytest.raises(bluebus.DBusError) as info:
        await bluebus.Session.new()
    assert info.value.name is None


# ---------------------------------------------------------------------------
# Scanning
# ---------------------------------------------------------------------------


async def test_scan_and_filters(mock):
    _, adapter = await powered(mock)
    found = await adapter.scan(timeout=0.1)
    assert [d.address for d in found] == [MockBluez.HEADSET, MockBluez.ANON]
    headset = found[0]
    assert headset.name == "Headset" and headset.alias == "Headset"
    assert headset.address_type == "public"
    assert headset.rssi == -48 and headset.tx_power == 4
    assert SVC_UUID in headset.uuids
    assert headset.manufacturer_data == {0x004C: b"\x02\x15"}
    assert headset.service_data == {SVC_UUID: b"\x57"}
    assert not (headset.connected or headset.paired or headset.trusted or headset.services_resolved)
    assert found[1].name is None
    r = repr(headset)
    assert r.startswith("DeviceInfo(address='11:22:33:44:55:66', name='Headset', rssi=-48")
    assert "0x004c: b'\\x02\\x15'" in r
    assert headset == found[0] and headset != found[1]
    with pytest.raises(AttributeError):
        headset.name = "x"
    assert not (await adapter.info()).discovering

    for kwargs in (
        dict(name="Headset"),
        dict(name_prefix="Head"),
        dict(address="11:22:33:44:55:66"),
        dict(service_uuids=["180d", "180f"], min_rssi=-50),
        dict(transport="le", service_uuids=["0x180F"], min_rssi=-60, duplicate_data=True),
    ):
        devices = await adapter.scan(timeout=0.05, **kwargs)
        assert [d.address for d in devices] == [MockBluez.HEADSET], kwargs
    assert len(await adapter.scan(timeout=0.05, service_uuids=["180f"])) == 2
    assert await adapter.scan(timeout=0.05, service_uuids=["180d"]) == []
    assert await adapter.scan(timeout=0.05, name="Nobody") == []

    for kwargs in (
        dict(service_uuids=["heart"]),
        dict(address="nope"),
        dict(transport="radio"),
        dict(name="a", name_prefix="b"),
        dict(timeout=-1.0),
        dict(timeout=float("nan")),
    ):
        with pytest.raises(bluebus.InvalidArgument):
            await adapter.scan(**{"timeout": 0.0, **kwargs})


async def test_scan_stream(mock):
    _, adapter = await powered(mock)

    # BlueZ-side filter keys while the stream runs.
    async with adapter.scan_stream(transport="le", service_uuids=["180f"], min_rssi=-60) as stream:
        state = await mock.state()
        assert sorted(state["discovery_filter"]) == ["RSSI", "Transport", "UUIDs"]
        assert state["discovery_filter"]["Transport"] == "le"
        event = await asyncio.wait_for(stream.__anext__(), 5)
        assert event.kind == "found" and event.device.address == MockBluez.HEADSET
        assert "ScanEvent(kind='found'" in repr(event)
    assert stream.closed
    assert (await mock.state())["discovery_filter"] == {}
    assert not (await adapter.info()).discovering
    with pytest.raises(StopAsyncIteration):
        await stream.__anext__()

    # Found, then Updated on a new RSSI; aclose stops the discovery.
    stream = await adapter.scan_stream(name="Headset")
    assert (await adapter.info()).discovering
    first = await asyncio.wait_for(stream.__anext__(), 5)
    assert first.kind == "found"
    await mock.acmd(f"rssi {MockBluez.HEADSET_PATH} -30")
    updated = await asyncio.wait_for(stream.__anext__(), 5)
    assert updated.kind == "updated" and updated.device.rssi == -30
    await stream.aclose()
    await stream.aclose()  # idempotent
    assert not (await adapter.info()).discovering

    # async for + break: the dropped iterator stops the scan in the background.
    seen = []
    async for event in adapter.scan_stream():
        seen.append(event.device.address)
        if len(seen) == 2:
            break
    assert sorted(seen) == [MockBluez.HEADSET, MockBluez.ANON]
    gc.collect()

    async def not_discovering():
        return not (await adapter.info()).discovering

    await eventually(not_discovering)

    # aclose interrupts a pending __anext__.
    stream = adapter.scan_stream(name="Nobody")
    pending = asyncio.ensure_future(stream.__anext__())
    await asyncio.sleep(0.2)
    assert not pending.done()
    await stream.aclose()
    with pytest.raises(StopAsyncIteration):
        await asyncio.wait_for(pending, 5)
    assert not (await adapter.info()).discovering

    # Errors surface when the stream is opened.
    with pytest.raises(bluebus.InvalidArgument):
        adapter.scan_stream(transport="nope")
    with pytest.raises(bluebus.InvalidArgument):
        await adapter.scan_stream(service_uuids=["heart"])


async def test_find_device(mock):
    _, adapter = await powered(mock)
    device = await adapter.find_device(name="Headset", timeout=5)
    assert device.address == MockBluez.HEADSET
    assert device.path == MockBluez.HEADSET_PATH
    assert device.adapter_name == "hci0"
    assert "11:22:33:44:55:66" in repr(device)
    assert (await device.info()).name == "Headset"
    assert (await adapter.find_device(name_prefix="Head", timeout=5)).address == MockBluez.HEADSET
    with pytest.raises(bluebus.BluebusTimeout):
        await adapter.find_device(name="Nobody", timeout=0.2)
    with pytest.raises(TimeoutError):
        await adapter.find_device(name="Nobody", timeout=0.1)
    assert not (await adapter.info()).discovering

    # Cancelling the awaitable cancels the scan.
    with pytest.raises(asyncio.TimeoutError):
        await asyncio.wait_for(adapter.find_device(name="Nobody", timeout=30), 0.2)

    async def not_discovering():
        return not (await adapter.info()).discovering

    await eventually(not_discovering)

    known = await adapter.devices()
    assert [d.address for d in known] == [MockBluez.HEADSET, MockBluez.ANON]
    await adapter.remove_device(MockBluez.ANON)
    assert [d.address for d in await adapter.devices()] == [MockBluez.HEADSET]
    with pytest.raises(bluebus.NotFound):
        await adapter.remove_device(MockBluez.ANON)
    with pytest.raises(bluebus.InvalidArgument):
        adapter.device("not-a-mac")
    with pytest.raises(bluebus.NotFound):
        await adapter.device("01:02:03:04:05:07").info()


# ---------------------------------------------------------------------------
# GATT client
# ---------------------------------------------------------------------------


async def test_connect_read_write_services(mock):
    _, adapter = await discovered(mock)
    device = adapter.device(MockBluez.HEADSET.lower())
    with pytest.raises(bluebus.NotFound):
        await device.read("2a19")  # not connected: no services

    async with device.connected(timeout=5) as d:
        assert d is device
        assert await device.is_connected()
        info = await device.info()
        assert info.connected and info.services_resolved

        services = await device.services()
        assert len(services) == 1
        svc = services[0]
        assert svc.uuid == SVC_UUID and svc.path == MockBluez.HEADSET_PATH + "/service0001"
        chr_ = svc.characteristics[0]
        assert chr_.uuid == CHR_UUID and chr_.path == MockBluez.CHR_PATH
        assert chr_.flags == ["read", "write", "notify"]
        assert chr_.descriptors[0].uuid == DESC_UUID
        assert chr_.descriptors[0].flags == ["read", "write"]
        assert "ServiceInfo(uuid='0000180f" in repr(svc)

        for uuid in ("2a19", "0x2A19", CHR_UUID, CHR_UUID.upper()):
            value = await device.read(uuid)
            assert value == b"\x57" and isinstance(value, bytes)
        assert await device.battery_level() == 87
        assert await device.rssi() == -48

        await device.write("2a19", b"\x01\x02")
        await device.write("2a19", b"x", response=False)
        await device.write("2a19", bytearray(b"ba"))
        await device.write("2a19", memoryview(b"mv"))
        await device.write("2a19", [1, 2, 3])
        writes = (await mock.state())["remote_writes"]
        assert [w["type"] for w in writes] == ["request", "command", "request", "request", "request"]
        assert writes[0]["path"] == MockBluez.CHR_PATH and writes[0]["value"] == "0102"
        assert [w["value"] for w in writes[2:]] == ["6261", "6d76", "010203"]
        assert await device.read("2a19") == b"\x01\x02\x03"
        with pytest.raises(TypeError):
            await device.write("2a19", "text")

        assert await device.read_descriptor("2a19", "2902") == b"\x00\x00"
        await device.write_descriptor("0x2a19", "0x2902", b"\x01\x00")
        assert await device.read_descriptor("2a19", "2902") == b"\x01\x00"

        # Concurrent requests.
        values = await asyncio.gather(*(device.read("2a19") for _ in range(20)))
        assert values == [b"\x01\x02\x03"] * 20

        with pytest.raises(bluebus.NotFound):
            await device.read("2a00")
        with pytest.raises(bluebus.NotFound):
            await device.read_descriptor("2a19", "2901")
        with pytest.raises(bluebus.InvalidArgument):
            await device.read("not a uuid")
    assert not await device.is_connected()

    await device.connect()
    await device.trust()
    assert (await device.info()).trusted
    await device.trust(False)
    await device.disconnect()
    assert not await device.is_connected()

    # The block's exception propagates (and the device is disconnected).
    with pytest.raises(RuntimeError, match="inside"):
        async with device.connected(timeout=5):
            raise RuntimeError("inside")
    assert not await device.is_connected()

    with pytest.raises(bluebus.InvalidArgument):
        device.connected(timeout=-1)


async def test_notifications(mock):
    _, adapter = await discovered(mock)
    device = await adapter.find_device(address=MockBluez.HEADSET, timeout=5)
    await device.connect(timeout=5)

    async with device.notifications("2a19") as values:
        assert values.uuid == "2a19" and not values.closed
        # A second subscriber shares the BlueZ subscription.
        second = await device.notifications("0x2A19")
        for v in ("01", "02", "03"):
            await mock.acmd(f"notify {MockBluez.CHR_PATH} {v}")
        got = [await asyncio.wait_for(values.__anext__(), 5) for _ in range(3)]
        assert got == [b"\x01", b"\x02", b"\x03"]
        got = [await asyncio.wait_for(second.__anext__(), 5) for _ in range(3)]
        assert got == [b"\x01", b"\x02", b"\x03"]
        await second.aclose()
        # Writes of the mock are notified too (Value changes).
        await device.write("2a19", b"\x09")
        assert await asyncio.wait_for(values.__anext__(), 5) == b"\x09"
    assert values.closed
    with pytest.raises(StopAsyncIteration):
        await values.__anext__()

    # async for, ended by aclose() from another task.
    stream = device.notifications("2a19")
    received = []

    async def consume():
        async for value in stream:
            received.append(value)

    task = asyncio.ensure_future(consume())
    await asyncio.sleep(0.2)  # first __anext__ subscribes
    for i in range(5):
        await mock.acmd(f"notify {MockBluez.CHR_PATH} {i:02x}")
    await eventually(lambda: len(received) == 5)
    await stream.aclose()
    await asyncio.wait_for(task, 5)
    assert received == [bytes([i]) for i in range(5)]
    assert "NotificationStream uuid='2a19' closed=True" in repr(stream)

    with pytest.raises(bluebus.NotFound):
        await device.notifications("2a37")
    await device.disconnect()


# ---------------------------------------------------------------------------
# Pairing agent
# ---------------------------------------------------------------------------


async def test_agents_and_pairing(mock):
    session, adapter = await discovered(mock)
    device = adapter.device(MockBluez.HEADSET)
    await session.set_agent("auto")
    state = await mock.state()
    assert [a["capability"] for a in state["agents"]] == ["NoInputNoOutput"]
    assert state["default_agent"]
    await device.pair()
    assert (await device.info()).paired

    await session.set_agent(passkey=123456)
    state = await mock.state()
    assert [a["capability"] for a in state["agents"]] == ["KeyboardDisplay"]
    await session.set_agent(passkey=1, pin_code="0000", capability="display_yes_no")
    assert [a["capability"] for a in (await mock.state())["agents"]] == ["DisplayYesNo"]
    await session.remove_agent()
    state = await mock.state()
    assert state["agents"] == [] and not state["default_agent"]
    await session.remove_agent()

    for args, kwargs in (
        (("bogus",), {}),
        (("auto",), {"passkey": 1}),
        (("passkey",), {}),
        ((), {"passkey": 1_000_000}),
        ((), {"passkey": 1, "capability": "telepathy"}),
    ):
        with pytest.raises(bluebus.InvalidArgument):
            session.set_agent(*args, **kwargs)


# ---------------------------------------------------------------------------
# Peripheral
# ---------------------------------------------------------------------------


async def test_peripheral(mock, caplog):
    _, adapter = await powered(mock)
    writes = []
    async_writes = []
    threads = []
    got_write = asyncio.Event()

    def on_write(value):
        import threading

        threads.append(threading.current_thread().name)
        writes.append(value)
        got_write.set()

    async def on_write_async(value):
        await asyncio.sleep(0)
        async_writes.append(value)

    def broken(value):
        raise ValueError("callback failure")

    builder = (
        adapter.peripheral()
        .service(
            bluebus.Service("180f").characteristic(
                bluebus.Characteristic("2a19", read=True, notify=True, value=b"\x64", on_write=on_write)
            )
        )
        .service(
            bluebus.Service("b1eb0000-0000-4000-8000-000000000001")
            .characteristic(
                bluebus.Characteristic(
                    "b1eb0000-0000-4000-8000-000000000002",
                    read=True,
                    write=True,
                    write_without_response=True,
                    on_write=on_write_async,
                ).descriptor("2901", b"desc")
            )
            .characteristic(
                bluebus.Characteristic("b1eb0000-0000-4000-8000-000000000003", write=True, on_write=broken)
            )
        )
        .advertise(
            bluebus.Advertisement(
                local_name="my-sensor",
                service_uuids=["180f"],
                manufacturer_data={0xFFFF: b"\xe2\xe0"},
            )
        )
    )
    assert "services=2" in repr(builder)
    peripheral = await builder.start()
    assert peripheral.running and "running=True" in repr(peripheral)

    state = await mock.state()
    assert len(state["gatt_apps"]) == 1
    objects = state["gatt_apps"][0]["objects"]
    chr_uuids = sorted(
        o["interfaces"]["org.bluez.GattCharacteristic1"]["UUID"]
        for o in objects
        if "org.bluez.GattCharacteristic1" in o["interfaces"]
    )
    assert chr_uuids == [
        CHR_UUID,
        "b1eb0000-0000-4000-8000-000000000002",
        "b1eb0000-0000-4000-8000-000000000003",
    ]
    (adv,) = state["advertisements"]
    assert adv["properties"]["LocalName"] == "my-sensor"
    assert adv["properties"]["ServiceUUIDs"] == [SVC_UUID]
    assert adv["properties"]["Type"] == "peripheral"

    # A remote central reads and writes.
    assert await mock.acmd("read_local 2a19") == "64"
    await mock.acmd("write_local 2a19 0102")
    await asyncio.wait_for(got_write.wait(), 5)
    assert writes == [b"\x01\x02"]
    assert threads == ["MainThread"], "callbacks run on the event loop thread"
    assert await peripheral.value("2a19") == b"\x01\x02"

    await mock.acmd("write_local b1eb0000-0000-4000-8000-000000000002 aa")
    await eventually(lambda: async_writes == [b"\xaa"])

    with caplog.at_level(logging.ERROR, logger="bluebus"):
        await mock.acmd("write_local b1eb0000-0000-4000-8000-000000000003 ff")
        await eventually(lambda: "callback failure" in caplog.text)
    # Still serving after the failing callback.
    await mock.acmd("write_local 2a19 03")
    await eventually(lambda: writes == [b"\x01\x02", b"\x03"])

    # Many writes keep their order.
    got_write.clear()
    for i in range(20):
        await mock.acmd(f"write_local 2a19 {i:02x}")
    await eventually(lambda: len(writes) == 22)
    assert writes[2:] == [bytes([i]) for i in range(20)]

    await peripheral.set_value("2a19", b"\x50")
    assert await mock.acmd("read_local 2a19") == "50"
    await peripheral.notify("0x2A19", b"\x63")
    assert await peripheral.value("2a19") == b"\x63"
    assert not await peripheral.is_notifying("2a19")
    with pytest.raises(bluebus.NotFound):
        await peripheral.notify("2a37", b"\x00")

    await peripheral.stop()
    assert not peripheral.running
    state = await mock.state()
    assert state["gatt_apps"] == [] and state["advertisements"] == []
    with pytest.raises(bluebus.InvalidArgument):
        await peripheral.notify("2a19", b"\x00")
    await peripheral.stop()  # idempotent


async def test_peripheral_context_and_errors(mock):
    _, adapter = await powered(mock)
    async with await (
        adapter.peripheral()
        .advertise(bluebus.Advertisement(local_name="beacon", connectable=False))
        .start()
    ) as peripheral:
        (adv,) = (await mock.state())["advertisements"]
        assert adv["properties"]["Type"] == "broadcast"
    assert not peripheral.running
    assert (await mock.state())["advertisements"] == []

    with pytest.raises(bluebus.InvalidArgument):
        await adapter.peripheral().start()
    with pytest.raises(bluebus.InvalidArgument, match="no flags"):
        await adapter.peripheral().service(bluebus.Service("180f").characteristic(bluebus.Characteristic("2a19"))).start()
    with pytest.raises(bluebus.InvalidArgument):
        await adapter.peripheral().service(bluebus.Service("nope")).start()
    state = await mock.state()
    assert state["gatt_apps"] == [] and state["advertisements"] == []

    # Peripherals one after another and side by side.
    def make(name):
        return (
            adapter.peripheral()
            .service(bluebus.Service("180f").characteristic(bluebus.Characteristic("2a19", read=True)))
            .advertise(bluebus.Advertisement(local_name=name))
            .start()
        )

    a, b = await asyncio.gather(make("a"), make("b"))
    state = await mock.state()
    assert len(state["gatt_apps"]) == 2 and len(state["advertisements"]) == 2
    await a.stop()
    await b.stop()
    for i in range(3):
        p = await make(f"p{i}")
        await p.stop()
    assert (await mock.state())["gatt_apps"] == []


async def test_peripheral_released_when_collected(mock):
    _, adapter = await powered(mock)
    peripheral = await (
        adapter.peripheral()
        .service(bluebus.Service("180f").characteristic(bluebus.Characteristic("2a19", read=True)))
        .start()
    )
    assert len((await mock.state())["gatt_apps"]) == 1
    del peripheral
    gc.collect()

    async def released():
        return (await mock.state())["gatt_apps"] == []

    await eventually(released)
