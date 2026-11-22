"""bluebus: asyncio Bluetooth Low Energy for Linux (BlueZ over D-Bus).

>>> import asyncio, bluebus
>>> async def main():
...     session = await bluebus.Session.new()
...     adapter = await session.default_adapter()
...     await adapter.power_on()
...     for d in await adapter.scan(timeout=5.0):
...         print(d.address, d.name, d.rssi)
>>> asyncio.run(main())  # doctest: +SKIP
"""

from .exceptions import BluebusError, BluebusTimeout, DBusError, InvalidArgument, NotFound
from ._bluebus import (
    Adapter,
    AdapterInfo,
    Advertisement,
    Characteristic,
    CharacteristicInfo,
    Connection,
    DescriptorInfo,
    Device,
    DeviceInfo,
    NotificationStream,
    Peripheral,
    PeripheralBuilder,
    ScanEvent,
    ScanStream,
    Service,
    ServiceInfo,
    Session,
    __version__,
    normalize_uuid,
)

__all__ = [
    "Adapter",
    "AdapterInfo",
    "Advertisement",
    "BluebusError",
    "BluebusTimeout",
    "Characteristic",
    "CharacteristicInfo",
    "Connection",
    "DBusError",
    "DescriptorInfo",
    "Device",
    "DeviceInfo",
    "InvalidArgument",
    "NotFound",
    "NotificationStream",
    "Peripheral",
    "PeripheralBuilder",
    "ScanEvent",
    "ScanStream",
    "Service",
    "ServiceInfo",
    "Session",
    "__version__",
    "normalize_uuid",
]
