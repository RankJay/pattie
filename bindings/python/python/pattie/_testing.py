"""The SDK's test backend, for this package's own tests: Devices without the
driver. It exists only in the project's test builds (uv's dev build), never in
the published wheels."""

from pattie._pattie import TestBackend, sdk_error

__all__ = ["TestBackend", "sdk_error"]
