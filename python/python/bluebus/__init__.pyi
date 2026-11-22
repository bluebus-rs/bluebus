"""Type stubs of bluebus (asyncio BLE for Linux over BlueZ D-Bus)."""

from types import TracebackType
from typing import (
    Awaitable,
    Callable,
    Dict,
    Generator,
    List,
    Literal,
    Mapping,
    Optional,
    Sequence,
    Type,
    Union,
)

from .exceptions import (
    BluebusError as BluebusError,
    BluebusTimeout as BluebusTimeout,
    DBusError as DBusError,
    InvalidArgument as InvalidArgument,
    NotFound as NotFound,
)

__version__: str

BytesLike = Union[bytes, bytearray, memoryview, Sequence[int]]
Transport = Literal["le", "bredr", "auto"]
AgentCapability = Literal[
    "DisplayOnly", "DisplayYesNo", "KeyboardOnly", "NoInputNoOutput", "KeyboardDisplay"
]
WriteCallback = Callable[[bytes], Union[None, Awaitable[None]]]

def normalize_uuid(uuid: str) -> str:
    """Full lower-case form of a 16-, 32- or 128-bit UUID ("2a19" → "00002a19-0000-1000-8000-00805f9b34fb")."""

# ---------------------------------------------------------------------------
# Data classes (read-only snapshots)
# ---------------------------------------------------------------------------

class AdapterInfo:
    @property
    def name(self) -> str: ...
    @property
    def path(self) -> str: ...
    @property
    def address(self) -> str: ...
    @property
    def alias(self) -> str: ...
    @property
    def powered(self) -> bool: ...
    @property
    def discovering(self) -> bool: ...

class DeviceInfo:
    @property
    def address(self) -> str: ...
    @property
    def address_type(self) -> Optional[str]:
        """"public" or "random"."""
    @property
    def name(self) -> Optional[str]: ...
    @property
    def alias(self) -> str:
        """Name, or the address with dashes if the device has none."""
    @property
    def rssi(self) -> Optional[int]: ...
    @property
    def tx_power(self) -> Optional[int]: ...
    @property
    def uuids(self) -> List[str]:
        """Advertised / known service UUIDs (full, lower case)."""
    @property
    def manufacturer_data(self) -> Dict[int, bytes]:
        """Company identifier → payload."""
    @property
    def service_data(self) -> Dict[str, bytes]:
        """Service UUID → payload."""
    @property
    def connected(self) -> bool: ...
    @property
    def paired(self) -> bool: ...
    @property
    def trusted(self) -> bool: ...
    @property
    def services_resolved(self) -> bool: ...

class DescriptorInfo:
    @property
    def uuid(self) -> str: ...
    @property
    def path(self) -> str: ...
    @property
    def flags(self) -> List[str]: ...

class CharacteristicInfo:
    @property
    def uuid(self) -> str: ...
    @property
    def path(self) -> str: ...
    @property
    def flags(self) -> List[str]:
        """BlueZ flags: "read", "write", "write-without-response", "notify", …"""
    @property
    def descriptors(self) -> List[DescriptorInfo]: ...

class ServiceInfo:
    @property
    def uuid(self) -> str: ...
    @property
    def path(self) -> str: ...
    @property
    def characteristics(self) -> List[CharacteristicInfo]: ...

class ScanEvent:
    @property
    def kind(self) -> Literal["found", "updated"]: ...
    @property
    def device(self) -> DeviceInfo: ...

# ---------------------------------------------------------------------------
# Streams
# ---------------------------------------------------------------------------

class ScanStream:
    """Async iterator of ScanEvents; the scan runs until it is closed."""

    def __aiter__(self) -> "ScanStream": ...
    def __anext__(self) -> Awaitable[ScanEvent]: ...
    def __await__(self) -> Generator[None, None, "ScanStream"]:
        """Starts the scan now and returns the stream."""
    def start(self) -> Awaitable["ScanStream"]: ...
    def __aenter__(self) -> Awaitable["ScanStream"]: ...
    def __aexit__(
        self,
        exc_type: Optional[Type[BaseException]],
        exc: Optional[BaseException],
        tb: Optional[TracebackType],
    ) -> Awaitable[None]: ...
    def aclose(self) -> Awaitable[None]:
        """Stops the discovery and ends the iteration (also interrupts a pending __anext__)."""
    @property
    def closed(self) -> bool: ...

class NotificationStream:
    """Async iterator of notification values of one characteristic."""

    def __aiter__(self) -> "NotificationStream": ...
    def __anext__(self) -> Awaitable[bytes]: ...
    def __await__(self) -> Generator[None, None, "NotificationStream"]:
        """Subscribes now and returns the stream."""
    def start(self) -> Awaitable["NotificationStream"]: ...
    def __aenter__(self) -> Awaitable["NotificationStream"]: ...
    def __aexit__(
        self,
        exc_type: Optional[Type[BaseException]],
        exc: Optional[BaseException],
        tb: Optional[TracebackType],
    ) -> Awaitable[None]: ...
    def aclose(self) -> Awaitable[None]:
        """Unsubscribes and ends the iteration."""
    @property
    def uuid(self) -> str: ...
    @property
    def closed(self) -> bool: ...

# ---------------------------------------------------------------------------
# Central
# ---------------------------------------------------------------------------

class Session:
    """The connection to BlueZ on the system bus (honours DBUS_SYSTEM_BUS_ADDRESS)."""

    def __init__(self) -> None:
        """Must be awaited: ``session = await bluebus.Session()``."""
    def __await__(self) -> Generator[None, None, "Session"]: ...
    @staticmethod
    def new() -> Awaitable["Session"]: ...
    def adapters(self) -> Awaitable[List[AdapterInfo]]: ...
    def adapter(self, name: Optional[str] = None) -> Awaitable["Adapter"]:
        """By name ("hci1"), object path or MAC address; None: the default adapter."""
    def default_adapter(self) -> Awaitable["Adapter"]: ...
    def set_agent(
        self,
        mode: Optional[Literal["auto", "passkey"]] = None,
        *,
        passkey: Optional[int] = None,
        pin_code: Optional[str] = None,
        capability: Optional[AgentCapability] = None,
    ) -> Awaitable[None]:
        """Pairing agent: "auto" accepts every confirmation; passkey=N answers passkey requests."""
    def remove_agent(self) -> Awaitable[None]: ...

class Adapter:
    @property
    def name(self) -> str: ...
    @property
    def address(self) -> str: ...
    @property
    def path(self) -> str: ...
    def info(self) -> Awaitable[AdapterInfo]: ...
    def is_powered(self) -> Awaitable[bool]: ...
    def power_on(self) -> Awaitable[None]: ...
    def power_off(self) -> Awaitable[None]: ...
    def scan(
        self,
        timeout: float = 5.0,
        *,
        transport: Optional[Transport] = None,
        name: Optional[str] = None,
        name_prefix: Optional[str] = None,
        address: Optional[str] = None,
        service_uuids: Optional[Sequence[str]] = None,
        min_rssi: Optional[int] = None,
        duplicate_data: Optional[bool] = None,
    ) -> Awaitable[List[DeviceInfo]]: ...
    def scan_stream(
        self,
        *,
        transport: Optional[Transport] = None,
        name: Optional[str] = None,
        name_prefix: Optional[str] = None,
        address: Optional[str] = None,
        service_uuids: Optional[Sequence[str]] = None,
        min_rssi: Optional[int] = None,
        duplicate_data: Optional[bool] = None,
    ) -> ScanStream: ...
    def find_device(
        self,
        *,
        name: Optional[str] = None,
        name_prefix: Optional[str] = None,
        address: Optional[str] = None,
        service_uuids: Optional[Sequence[str]] = None,
        min_rssi: Optional[int] = None,
        transport: Optional[Transport] = None,
        timeout: float = 10.0,
    ) -> Awaitable["Device"]:
        """Raises BluebusTimeout if nothing matches within timeout seconds."""
    def device(self, address: str) -> "Device": ...
    def devices(self) -> Awaitable[List[DeviceInfo]]: ...
    def remove_device(self, address: str) -> Awaitable[None]: ...
    def peripheral(self) -> "PeripheralBuilder": ...

class Connection:
    """Returned by Device.connected(): connects on entry, disconnects on exit."""

    def __aenter__(self) -> Awaitable["Device"]: ...
    def __aexit__(
        self,
        exc_type: Optional[Type[BaseException]],
        exc: Optional[BaseException],
        tb: Optional[TracebackType],
    ) -> Awaitable[bool]: ...

class Device:
    @property
    def address(self) -> str: ...
    @property
    def path(self) -> str: ...
    @property
    def adapter_name(self) -> str: ...
    def info(self) -> Awaitable[DeviceInfo]: ...
    def connect(self, timeout: float = 20.0) -> Awaitable[None]:
        """Connects and waits until the GATT services are resolved."""
    def disconnect(self) -> Awaitable[None]: ...
    def connected(self, timeout: float = 20.0) -> Connection: ...
    def is_connected(self) -> Awaitable[bool]: ...
    def pair(self) -> Awaitable[None]: ...
    def trust(self, trusted: bool = True) -> Awaitable[None]: ...
    def rssi(self) -> Awaitable[Optional[int]]: ...
    def battery_level(self) -> Awaitable[int]: ...
    def services(self) -> Awaitable[List[ServiceInfo]]: ...
    def read(self, uuid: str) -> Awaitable[bytes]: ...
    def write(self, uuid: str, data: BytesLike, *, response: bool = True) -> Awaitable[None]: ...
    def read_descriptor(self, characteristic: str, descriptor: str) -> Awaitable[bytes]: ...
    def write_descriptor(self, characteristic: str, descriptor: str, data: BytesLike) -> Awaitable[None]: ...
    def notifications(self, uuid: str) -> NotificationStream: ...

# ---------------------------------------------------------------------------
# Peripheral
# ---------------------------------------------------------------------------

class Characteristic:
    def __init__(
        self,
        uuid: str,
        *,
        read: bool = False,
        write: bool = False,
        write_without_response: bool = False,
        notify: bool = False,
        indicate: bool = False,
        flags: Optional[Sequence[str]] = None,
        value: Optional[BytesLike] = None,
        on_write: Optional[WriteCallback] = None,
        descriptors: Optional[Mapping[str, BytesLike]] = None,
    ) -> None:
        """on_write runs on the event loop that started the peripheral (coroutines become tasks; exceptions are logged)."""
    @property
    def uuid(self) -> str: ...
    @property
    def flags(self) -> List[str]: ...
    @property
    def value(self) -> bytes: ...
    def descriptor(self, uuid: str, value: BytesLike) -> "Characteristic": ...

class Service:
    def __init__(
        self,
        uuid: str,
        characteristics: Optional[Sequence[Characteristic]] = None,
        *,
        primary: bool = True,
    ) -> None: ...
    @property
    def uuid(self) -> str: ...
    @property
    def primary(self) -> bool: ...
    @property
    def characteristics(self) -> List[Characteristic]: ...
    def characteristic(self, characteristic: Characteristic) -> "Service": ...

class Advertisement:
    def __init__(
        self,
        *,
        local_name: Optional[str] = None,
        service_uuids: Optional[Sequence[str]] = None,
        manufacturer_data: Optional[Mapping[int, BytesLike]] = None,
        service_data: Optional[Mapping[str, BytesLike]] = None,
        discoverable: bool = True,
        connectable: bool = True,
    ) -> None: ...
    @property
    def local_name(self) -> Optional[str]: ...
    @property
    def service_uuids(self) -> List[str]: ...
    @property
    def manufacturer_data(self) -> Dict[int, bytes]: ...
    @property
    def service_data(self) -> Dict[str, bytes]: ...
    @property
    def discoverable(self) -> bool: ...
    @property
    def connectable(self) -> bool: ...

class PeripheralBuilder:
    def service(self, service: Service) -> "PeripheralBuilder": ...
    def advertise(self, advertisement: Advertisement) -> "PeripheralBuilder": ...
    def start(self) -> Awaitable["Peripheral"]: ...

class Peripheral:
    def notify(self, uuid: str, value: BytesLike) -> Awaitable[None]: ...
    def set_value(self, uuid: str, value: BytesLike) -> Awaitable[None]: ...
    def value(self, uuid: str) -> Awaitable[bytes]: ...
    def is_notifying(self, uuid: str) -> Awaitable[bool]: ...
    @property
    def running(self) -> bool: ...
    def stop(self) -> Awaitable[None]: ...
    def __aenter__(self) -> Awaitable["Peripheral"]: ...
    def __aexit__(
        self,
        exc_type: Optional[Type[BaseException]],
        exc: Optional[BaseException],
        tb: Optional[TracebackType],
    ) -> Awaitable[bool]: ...
