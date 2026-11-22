"""Runs ``on_write`` callbacks of local characteristics.

Called by the native module from its callback thread (with the GIL held).
The callback is scheduled on the asyncio loop that started the peripheral,
so it runs on the event loop thread; a coroutine result is scheduled as a
task. Exceptions are logged to the ``"bluebus"`` logger and never reach
BlueZ or the Rust runtime.
"""

import asyncio
import inspect
import logging

logger = logging.getLogger("bluebus")

# Keeps tasks created for coroutine callbacks alive until they finish.
_tasks = set()


def _log_task(task):
    _tasks.discard(task)
    if task.cancelled():
        return
    exc = task.exception()
    if exc is not None:
        logger.error("on_write callback raised", exc_info=exc)


def _run(loop, callback, value):
    try:
        result = callback(value)
        if inspect.isawaitable(result):
            if loop is not None and not loop.is_closed():
                task = asyncio.ensure_future(result, loop=loop)
                _tasks.add(task)
                task.add_done_callback(_log_task)
            else:
                asyncio.run(_await(result))
    except Exception:
        logger.exception("on_write callback raised")


async def _await(awaitable):
    await awaitable


def dispatch(loop, callback, value):
    if loop is not None and not loop.is_closed():
        try:
            loop.call_soon_threadsafe(_run, loop, callback, value)
            return
        except RuntimeError:  # loop closed meanwhile
            pass
    _run(None, callback, value)
