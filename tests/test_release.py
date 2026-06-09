from __future__ import annotations as _annotations

import vsh
from vsh import get_schema, registry
from vsh.registry import registrations


def test_public_version_matches_package_metadata() -> None:
    assert vsh.__version__ == "0.3.0"


def test_every_registered_command_exposes_schema() -> None:
    assert set(registry) == set(registrations)

    for name in sorted(registry):
        schema = get_schema(name)
        assert schema["title"] == registry[name].schema_model_name
        assert schema["type"] == "object"


def test_registry_surface_matches_current_release() -> None:
    assert len(registry) == 28
