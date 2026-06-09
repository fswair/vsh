from __future__ import annotations as _annotations

__all__ = ("normalize_agent_tool_name",)


def normalize_agent_tool_name(tool_name: str) -> str:
    """Strip provider-specific prefixes from vsh tool names.

    Some models (notably Gemini via OpenRouter) emit names like
    ``default_api:vsh_search`` instead of ``vsh_search``.
    """
    if tool_name.startswith("vsh_"):
        return tool_name
    if ":" not in tool_name:
        return tool_name
    candidate = tool_name.rsplit(":", 1)[-1]
    if candidate.startswith("vsh_"):
        return candidate
    return tool_name
