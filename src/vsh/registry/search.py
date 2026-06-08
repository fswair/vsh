from __future__ import annotations as _annotations

from typing import Any

from vsh.schemas import CommandSpec

from .json_schema import inline_json_schema
from .specs import registrations, registry


def search(query: str) -> list[CommandSpec]:
    normalized_query = query.casefold().strip()
    if not normalized_query:
        return [registry[name] for name in sorted(registry)]

    matches: list[CommandSpec] = []
    for spec in registry.values():
        searchable = _searchable_text(spec)
        if normalized_query in searchable:
            matches.append(spec)
    return sorted(matches, key=lambda item: item.name)


def search_names(query: str) -> list[str]:
    return [spec.name for spec in search(query)]


def get_schema(name: str) -> dict[str, Any]:
    try:
        registration = registrations[name]
    except KeyError as exc:
        raise KeyError(f"unknown command spec: {name}") from exc
    schema = registration.schema_model.model_json_schema(by_alias=False)
    return inline_json_schema(schema)


def _searchable_text(spec: CommandSpec) -> str:
    parts: list[str] = [
        spec.name,
        spec.summary,
        spec.description,
        spec.schema_model_name,
        *spec.tags,
    ]
    return " ".join(parts).casefold()
