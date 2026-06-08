from __future__ import annotations as _annotations

from typing import ClassVar

from pydantic import Field, model_validator

from .common import CommandKind, SideEffect, StructuredCommand, quote_shell_token


class SedCommand(StructuredCommand):
    """Read or edit file content through a sed script."""

    _command_alias: ClassVar[str] = "sed"
    _boolean_flag_aliases: ClassVar[dict[str, str]] = {"quiet": "n", "in_place": "i"}
    _flag_order: ClassVar[tuple[str, ...]] = ("quiet",)

    kind: CommandKind = Field(
        default="read",
        description="Command category for transformed file reads or writes.",
    )
    side_effects: list[SideEffect] = Field(
        default_factory=lambda: [SideEffect(kind="read", risks=["reads file contents"])],
        description="Declared side effects for this command.",
    )
    script: str = Field(description="Sed expression to apply, for example 1,120p.")
    path: str | None = Field(default=None, description="Single file path to read or edit.")
    paths: list[str] = Field(
        default_factory=list, description="One or more file paths to read or edit."
    )
    quiet: bool = Field(default=True, description="Suppress automatic printing.")
    in_place: bool = Field(
        default=False, description="Edit files in place instead of printing transformed output."
    )
    backup_suffix: str | None = Field(
        default=None, description="Optional backup suffix for in-place edits."
    )

    @model_validator(mode="after")
    def normalize_paths_and_effects(self) -> SedCommand:
        if self.path is not None and self.path not in self.paths:
            self.paths.insert(0, self.path)
        if not self.paths:
            raise ValueError("at least one sed path is required")
        self.path = self.paths[0]
        if self.in_place:
            self.kind = "write"
            self.side_effects = [SideEffect(kind="write", risks=["edits file contents in place"])]
        return self

    def to_shell(self) -> str:
        tokens = [self._command_alias]
        if self.in_place:
            tokens.append("-i")
            if self.backup_suffix is not None:
                tokens.append(quote_shell_token(self.backup_suffix))
        elif self.quiet:
            tokens.append("-n")
        tokens.append(quote_shell_token(self.script))
        tokens.extend(quote_shell_token(path) for path in self.paths)
        return " ".join(tokens)
