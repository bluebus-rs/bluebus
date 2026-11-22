"""End-to-end test of the Python bindings between two adapters driven by the
real BlueZ daemon: one acts as a peripheral (GATT server + advertisement),
the other as a central (scan, connect, GATT read/write/notify, pairing).
Mirrors ``examples/ble_e2e.rs``.

Intended for two virtual controllers created by BlueZ' ``btvirt``::

    docker run -d --rm --name bluebus-vhci --privileged bluebus-btvirt -l2
    python examples/e2e.py hci1 hci2   # peripheral central
    docker stop bluebus-vhci
"""

import asyncio
import sys

import bluebus

SVC = "b1eb05e0-0000-4000-8000-00000000e2e0"
CHR = "b1eb05e0-0000-4000-8000-00000000e2e1"
NAME = "bluebus-py-e2e"

checks = 0


def ok(what, detail=""):
    global checks
    checks += 1
    print(f"  ok  {what:<60} {detail}")


async def until(what, secs, check):
    """Awaits ``check()`` every 50 ms until it is truthy."""
    async def loop():
        while not await check():
            await asyncio.sleep(0.05)

    try:
        await asyncio.wait_for(loop(), secs)
    except asyncio.TimeoutError:
        raise RuntimeError(f"timeout: {what}") from None


async def run(p_hci, c_hci):
    # Two independent applications (two D-Bus connections).
    p_session = await bluebus.Session.new()
    c_session = await bluebus.Session()
    periph = await p_session.adapter(p_hci)
    central = await c_session.adapter(c_hci)
    p_addr, c_addr = periph.address, central.address
    print(f"peripheral {periph.name} ({p_addr})  central {central.name} ({c_addr})")

    print("\n[setup]")
    await periph.power_on()
    await central.power_on()
    ok("Adapter.power_on (both)")
    # Forget the peer from a previous run.
    for adapter, addr in ((central, p_addr), (periph, c_addr)):
        try:
            await adapter.remove_device(addr)
        except bluebus.NotFound:
            pass

    writes = []

    async def on_write_async(value):  # coroutine callbacks become tasks
        writes.append(("async", value))

    peripheral = await (
        periph.peripheral()
        .service(
            bluebus.Service(SVC).characteristic(
                bluebus.Characteristic(
                    CHR,
                    read=True,
                    write=True,
                    write_without_response=True,
                    notify=True,
                    value=b"hello",
                    on_write=lambda v: writes.append(("sync", v)),
                ).descriptor("2901", b"e2e")
            )
        )
        .service(
            bluebus.Service("180f").characteristic(
                bluebus.Characteristic("2a19", read=True, write=True, notify=True, value=bytes([77]), on_write=on_write_async)
            )
        )
        .advertise(
            bluebus.Advertisement(
                local_name=NAME,
                service_uuids=[SVC],
                manufacturer_data={0xFFFF: b"\xe2\xe0"},
            )
        )
        .start()
    )
    ok("peripheral: PeripheralBuilder.start (GATT + advertisement)")

    print("\n[discovery]")
    found = None
    async with central.scan_stream(transport="le", service_uuids=[SVC]) as scan:
        async def first():
            async for event in scan:
                if event.kind == "found" and event.device.address == p_addr:
                    return event.device

        found = await asyncio.wait_for(first(), 30)
    ok("central: scan_stream → found", f"{found.address} alias={found.alias!r} rssi={found.rssi}")
    if found.manufacturer_data.get(0xFFFF) != b"\xe2\xe0":
        raise RuntimeError(f"manufacturer data {found.manufacturer_data}")
    ok("central: advertised manufacturer_data", "ffff → e2e0")
    if SVC not in found.uuids:
        raise RuntimeError(f"service UUID not advertised: {found.uuids}")
    ok("central: advertised service uuids", SVC)

    device = await central.find_device(address=p_addr, transport="le", timeout=30)
    info = await device.info()
    ok("central: find_device(address)", f"name={info.name!r} rssi={info.rssi}")
    named = await central.scan(timeout=3.0, transport="le", name=NAME)
    ok("central: scan(name filter, 3 s)", f"{len(named)} device(s)")

    print("\n[connection + GATT client]")
    async with device.connected(timeout=30):
        ok("central: Device.connected (+ ServicesResolved)", await device.is_connected())
        services = await device.services()
        chr_ = next(
            (c for s in services for c in s.characteristics if c.uuid == CHR), None
        )
        if chr_ is None:
            raise RuntimeError("characteristic not found on the remote")
        ok(
            "central: services()",
            f"{len(services)} services, CHR flags {chr_.flags}, {len(chr_.descriptors)} descriptor(s)",
        )
        v = await device.read(CHR)
        if v != b"hello":
            raise RuntimeError(f"read {v!r}")
        ok("central: read", v.decode())

        await device.write(CHR, b"world", response=True)

        def arrived(expected):
            async def check():
                return await peripheral.value(CHR) == expected
            return check

        await until("write arrives at the peripheral", 10, arrived(b"world"))
        await until("on_write called", 10, _async(lambda: ("sync", b"world") in writes))
        ok("central: write(response=True) → peripheral value + on_write", "world")
        await device.write(CHR, b"cmd", response=False)
        await until("write without response arrives", 10, arrived(b"cmd"))
        ok("central: write(response=False) → peripheral value", "cmd")
        await device.write("2a19", b"\x05")
        await until("coroutine on_write called", 10, _async(lambda: ("async", b"\x05") in writes))
        ok("central: write → coroutine on_write", "05")
        d = await device.read_descriptor(CHR, "2901")
        ok("central: read_descriptor(0x2901)", d.decode())
        ok("central: battery_level", f"{await device.battery_level()}%")

        async with device.notifications(CHR) as notes:
            await until("peripheral sees the subscription", 10, lambda: peripheral.is_notifying(CHR))
            ok("central: notifications → peripheral is_notifying")
            for value in (b"n1", b"n2", b"n3"):
                await peripheral.notify(CHR, value)
                got = await asyncio.wait_for(notes.__anext__(), 10)
                if got != value:
                    raise RuntimeError(f"notification {got!r}, expected {value!r}")
            ok("peripheral notify → central stream (3 in order)", "n1 n2 n3")
        await until(
            "peripheral sees the unsubscription",
            10,
            _negate(lambda: peripheral.is_notifying(CHR)),
        )
        ok("central: NotificationStream.aclose → is_notifying=False")
        ok("central: rssi", await device.rssi())

        print("\n[pairing]")
        await c_session.set_agent("auto")
        ok('Session.set_agent("auto")')
        try:
            await asyncio.wait_for(device.pair(), 30)
        finally:
            await c_session.remove_agent()
        info = await device.info()
        if not info.paired:
            raise RuntimeError("paired=False after pair()")
        ok("central: pair → paired", info.paired)
        await device.trust(True)
        ok("central: trust(True)", (await device.info()).trusted)

        print("\n[teardown]")
    await until("disconnected", 10, _negate(device.is_connected))
    ok("central: disconnect (end of async with)")
    await central.remove_device(p_addr)
    if any(d.address == p_addr for d in await central.devices()):
        raise RuntimeError("device still known after remove_device")
    ok("central: remove_device")
    await peripheral.stop()
    ok("peripheral: stop (unregister GATT + advertisement)")

    # Several peripherals one after another in the same process.
    for i in range(2):
        async with await (
            periph.peripheral()
            .service(bluebus.Service(SVC).characteristic(bluebus.Characteristic(CHR, read=True)))
            .advertise(bluebus.Advertisement(local_name=f"{NAME}-{i}"))
            .start()
        ):
            pass
    ok("peripheral: start/stop twice more (async with)")

    # Errors.
    try:
        await central.find_device(name="nobody-has-this-name", timeout=1.0)
        raise RuntimeError("find_device should time out")
    except bluebus.BluebusTimeout:
        ok("central: find_device timeout → BluebusTimeout")
    try:
        await device.read(CHR)
        raise RuntimeError("read after disconnect should fail")
    except bluebus.BluebusError as e:
        ok("central: read while disconnected → BluebusError", type(e).__name__)
    try:
        await central.remove_device(p_addr)
        raise RuntimeError("remove_device twice should fail")
    except bluebus.NotFound:
        ok("central: remove_device twice → NotFound")
    try:
        await periph.remove_device(c_addr)
    except bluebus.NotFound:
        pass
    return checks


def _async(fn):
    async def check():
        return fn()
    return check


def _negate(check):
    async def negated():
        return not await check()
    return negated


def main():
    p_hci = sys.argv[1] if len(sys.argv) > 1 else "hci1"
    c_hci = sys.argv[2] if len(sys.argv) > 2 else "hci2"
    try:
        n = asyncio.run(run(p_hci, c_hci))
    except Exception as e:  # noqa: BLE001
        print(f"\nFAILED: {type(e).__name__}: {e}")
        sys.exit(1)
    print(f"\nALL {n} END-TO-END CHECKS PASSED")


if __name__ == "__main__":
    main()
