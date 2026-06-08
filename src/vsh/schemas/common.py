from __future__ import annotations as _annotations

import shlex
from typing import Any, ClassVar, Literal

from pydantic import BaseModel, ConfigDict, Field

CommandKind = Literal[
    "read", "write", "delete", "move", "create", "list", "mutate", "search", "copy"
]
SingleArgStyle = Literal["concatenate", "individual"]


class SideEffect(BaseModel):
    """Declared side-effect category used by simulation and policy."""

    model_config = ConfigDict(extra="forbid")

    kind: CommandKind = Field(description="High-level side-effect category emitted by the command.")
    risks: list[str] = Field(
        default_factory=list,
        description="Short risk notes surfaced to agents before execution.",
    )


class StructuredCommand(BaseModel):
    """Base model for all structured commands handled by vsh."""

    model_config = ConfigDict(extra="forbid", populate_by_name=True)

    kind: CommandKind = Field(
        default="read",
        description="High-level command category used by the simulator.",
    )
    side_effects: list[SideEffect] = Field(
        default_factory=list,
        description="Declared side effects used by policy and approval flows.",
    )
    raw_command: str | None = Field(
        default=None,
        description=(
            "Optional raw shell command supplied by the caller. "
            "When present, it must match the canonical shell preview before execution is eligible."
        ),
    )

    _command_alias: ClassVar[str]
    _single_arg_type: ClassVar[SingleArgStyle | None] = None
    _boolean_flag_aliases: ClassVar[dict[str, str]] = {}
    _flag_order: ClassVar[tuple[str, ...]] = ()
    _value_flag_aliases: ClassVar[dict[str, str]] = {}
    _value_flag_order: ClassVar[tuple[str, ...]] = ()
    _positional_fields: ClassVar[tuple[str, ...]] = ()

    def to_shell(self) -> str:
        return render_shell(self)

    def raw_matches_shell_preview(self, shell_preview: str | None = None) -> bool | None:
        if self.raw_command is None:
            return None
        preview = self.to_shell() if shell_preview is None else shell_preview
        return self.raw_command.strip() == preview.strip()

    def __repr__(self) -> str:
        return self.to_shell()


class CommandExample(BaseModel):
    """Example input shown in command discovery surfaces."""

    model_config = ConfigDict(extra="forbid")

    title: str = Field(description="Short label for the example.")
    params: dict[str, Any] = Field(description="Parameter payload for the example.")


class CommandSpec(BaseModel):
    """Discovery metadata describing a command exposed by the registry."""

    model_config = ConfigDict(extra="forbid")

    name: str = Field(description="Stable registry name for the command.")
    summary: str = Field(description="Short one-line summary shown in search results.")
    description: str = Field(description="Longer explanation of the command behavior.")
    tags: list[str] = Field(default_factory=list, description="Search tags used for discovery.")
    mutates_fs: bool = Field(
        default=False, description="Whether the command mutates the filesystem."
    )
    supports_execute: bool = Field(
        default=True, description="Whether real execution is implemented."
    )
    schema_model_name: str = Field(description="Pydantic schema model backing this command.")
    examples: list[CommandExample] = Field(
        default_factory=list,
        description="Example parameter payloads for agents.",
    )


def quote_shell_token(value: object) -> str:
    """Quote a shell token for canonical preview rendering."""
    return shlex.quote(str(value))


def render_shell(command: StructuredCommand) -> str:
    flag_tokens: list[str] = []
    concatenated_flags: list[str] = []

    for field_name in command._flag_order:
        alias = command._boolean_flag_aliases.get(field_name)
        if alias is None:
            continue
        if not getattr(command, field_name):
            continue
        if command._single_arg_type == "concatenate" and len(alias) == 1:
            concatenated_flags.append(alias)
        else:
            flag_tokens.append(f"-{alias}" if len(alias) == 1 else f"--{alias}")

    if concatenated_flags:
        flag_tokens.insert(0, f"-{''.join(concatenated_flags)}")

    for field_name in command._value_flag_order:
        alias = command._value_flag_aliases.get(field_name)
        if alias is None:
            continue
        value = getattr(command, field_name)
        if value in (None, ""):
            continue
        flag_tokens.append(f"-{alias}" if len(alias) == 1 else f"--{alias}")
        flag_tokens.append(quote_shell_token(value))

    positional_tokens = [
        quote_shell_token(getattr(command, field_name))
        for field_name in command._positional_fields
        if getattr(command, field_name) not in (None, "")
    ]

    tokens = [command._command_alias, *flag_tokens, *positional_tokens]
    return " ".join(tokens)
