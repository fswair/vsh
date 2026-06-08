from __future__ import annotations as _annotations

from copy import deepcopy
from typing import Any

JsonSchema = dict[str, Any]

__all__ = ("inline_json_schema",)


def inline_json_schema(schema: JsonSchema) -> JsonSchema:
    """Expand local ``#/$defs`` references for JSON-Schema-limited model providers.

    Gemini rejects tool responses that still contain ``$defs`` / ``$ref`` entries.
    """
    root = deepcopy(schema)
    defs: dict[str, Any] = root.pop("$defs", {})
    resolving: set[str] = set()

    def resolve(node: Any) -> Any:
        if isinstance(node, dict):
            ref = node.get("$ref")
            if ref is not None and set(node) == {"$ref"}:
                return resolve_ref(str(ref))
            return {
                key: resolve(value) for key, value in node.items() if key not in {"$defs", "$ref"}
            }
        if isinstance(node, list):
            return [resolve(item) for item in node]
        return node

    def resolve_ref(ref: str) -> Any:
        if not ref.startswith("#/$defs/"):
            return {"$ref": ref}
        name = ref.removeprefix("#/$defs/")
        if name not in defs:
            return {"$ref": ref}
        if name in resolving:
            return {"$ref": ref}
        resolving.add(name)
        try:
            return resolve(deepcopy(defs[name]))
        finally:
            resolving.discard(name)

    return resolve(root)
