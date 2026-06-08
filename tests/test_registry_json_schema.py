from __future__ import annotations as _annotations

from typing import Any, cast

from vsh.registry import get_schema
from vsh.registry.json_schema import inline_json_schema
from vsh.schemas import LsCommand


def test_inline_json_schema_expands_local_defs() -> None:
    schema = LsCommand.model_json_schema(by_alias=False)

    inlined = inline_json_schema(schema)

    assert "$defs" not in inlined
    assert "$ref" not in _collect_refs(inlined)
    side_effects = inlined["properties"]["side_effects"]
    assert side_effects["items"]["properties"]["kind"]["enum"] == [
        "read",
        "write",
        "delete",
        "move",
        "create",
        "list",
        "mutate",
        "search",
        "copy",
    ]


def test_inline_json_schema_preserves_external_refs() -> None:
    schema = {"$ref": "#/properties/name"}

    assert inline_json_schema(schema) == {"$ref": "#/properties/name"}


def test_inline_json_schema_preserves_unknown_local_refs() -> None:
    schema = {
        "$defs": {"Known": {"type": "string"}},
        "items": {"$ref": "#/$defs/Missing"},
    }

    assert inline_json_schema(schema) == {"items": {"$ref": "#/$defs/Missing"}}


def test_inline_json_schema_preserves_circular_local_refs() -> None:
    schema = {
        "$defs": {
            "Node": {
                "properties": {
                    "child": {"$ref": "#/$defs/Node"},
                },
                "type": "object",
            }
        },
        "$ref": "#/$defs/Node",
    }

    inlined = inline_json_schema(schema)

    assert inlined == {
        "properties": {
            "child": {"$ref": "#/$defs/Node"},
        },
        "type": "object",
    }


def test_get_schema_returns_inlined_json_schema() -> None:
    schema = get_schema("vsh_list")

    assert "$defs" not in schema
    assert "$ref" not in _collect_refs(schema)
    assert schema["title"] == "LsCommand"


def _collect_refs(node: object) -> list[str]:
    refs: list[str] = []
    if isinstance(node, dict):
        mapping = cast(dict[str, Any], node)
        ref = mapping.get("$ref")
        if ref is not None:
            refs.append(str(ref))
        for value in node.values():
            refs.extend(_collect_refs(value))
    elif isinstance(node, list):
        for item in node:
            refs.extend(_collect_refs(item))
    return refs
