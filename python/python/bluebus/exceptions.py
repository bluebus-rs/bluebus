"""Exceptions raised by bluebus.

Every error is a :class:`BluebusError`. Timeouts are also
:class:`TimeoutError` (so ``except TimeoutError`` and ``asyncio.wait_for``
style handling work), missing objects are also :class:`LookupError` and bad
arguments are also :class:`ValueError`.
"""

from typing import Optional

__all__ = ["BluebusError", "BluebusTimeout", "NotFound", "InvalidArgument", "DBusError"]


class BluebusError(Exception):
    """Base class of every bluebus error."""


class BluebusTimeout(BluebusError, TimeoutError):
    """An operation did not complete in time (scan, connect, power, …)."""


class NotFound(BluebusError, LookupError):
    """An adapter, device, characteristic or descriptor does not exist."""


class InvalidArgument(BluebusError, ValueError):
    """An argument is malformed (UUID, MAC address, timeout, …)."""


class DBusError(BluebusError):
    """A D-Bus or BlueZ error.

    ``name`` is the D-Bus error name, for example
    ``"org.bluez.Error.NotReady"`` (``None`` for transport failures).
    """

    name: Optional[str]

    def __init__(self, message: str, name: Optional[str] = None) -> None:
        super().__init__(message)
        self.name = name

    def __reduce__(self):  # keep `name` when pickled
        return (type(self), (str(self), self.name))
