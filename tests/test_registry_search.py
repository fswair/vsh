from __future__ import annotations as _annotations

import pytest

from vsh.plans.store import plan_store
from vsh.registry import registry, search
from vsh.registry.search import get_schema


def test_search_empty_query_returns_all_specs_sorted() -> None:
    assert [spec.name for spec in search("   ")] == sorted(registry)


def test_get_schema_unknown_name_raises_key_error() -> None:
    with pytest.raises(KeyError, match="unknown command spec"):
        get_schema("missing")


def test_plan_store_unknown_token_raises_key_error() -> None:
    with pytest.raises(KeyError, match="unknown approval token"):
        plan_store.get_by_token("missing-token")
