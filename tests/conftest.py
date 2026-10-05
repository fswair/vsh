"""Release cyclic agent/toolset fixtures between tests on low-FD-limit hosts."""

import gc
from collections.abc import Iterator

import pytest


@pytest.fixture(autouse=True)
def collect_test_owned_cycles() -> Iterator[None]:
    yield
    # Pydantic AI toolsets retain bound capability methods, creating Python
    # cycles. Native runtimes release descriptors when the cycle is collected.
    # Production integrations reuse their capabilities; this suite creates many.
    gc.collect()
