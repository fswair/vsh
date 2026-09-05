"""Native VSH filesystem capability for Pydantic AI."""

from __future__ import annotations

import asyncio
import base64
from collections.abc import Mapping
from dataclasses import dataclass
from os import PathLike, fspath
from typing import TypeAlias

try:
    from pydantic_ai.capabilities import Capability
    from pydantic_ai.toolsets import FunctionToolset
except ImportError as error:  # pragma: no cover - exercised in an environment without the extra
    raise ImportError(
        "VshCapability requires the 'pydantic-ai' extra: install 'vsh-python[pydantic-ai]'"
    ) from error

from ._judge import CommitJudge, JudgeReport
from ._native import HookScope, Receipt, RunMode, RunRequest, Runtime
from .hooks import HookedRuntime, HookHandler

_ACTIONABLE_STATES = frozenset(("auto_approved", "pending_approval"))
JsonValue: TypeAlias = None | bool | int | float | str | list[object] | dict[str, object]


@dataclass(frozen=True, slots=True)
class VshToolResult:
    """Compact transaction result returned to the calling agent."""

    transaction: str
    state: str
    result: JsonValue
    changed_paths: int
    hook_verdict: str | None = None
    feedback: str | None = None

    @property
    def requires_review(self) -> bool:
        """Return whether host changes are still waiting for approval."""

        return self.state == "pending_approval"


class VshCapability(Capability[object]):
    """Open and own a native VSH runtime as a Pydantic AI capability."""

    def __init__(
        self,
        workspace: str | PathLike[str],
        *,
        data_directory: str | PathLike[str] | None = None,
        policy: str = "balanced",
        worker_path: str | PathLike[str] | None = None,
        hook_handler: HookHandler | None = None,
        hook_scope: HookScope = HookScope.REVIEW_REQUIRED,
        hook_id: str = "vsh.pydantic-ai",
        review_content_bytes: int = 0,
        id: str = "vsh",
        defer_loading: bool = False,
    ) -> None:
        if hook_handler is None:
            if review_content_bytes:
                raise ValueError("review_content_bytes requires hook_handler")
            self.runtime: Runtime | HookedRuntime = Runtime.open(
                workspace,
                data_directory=data_directory,
                policy=policy,
                worker_path=worker_path,
            )
        else:
            self.runtime = HookedRuntime.open(
                workspace,
                hook_handler=hook_handler,
                hook_scope=hook_scope,
                hook_id=hook_id,
                review_content_bytes=review_content_bytes,
                data_directory=data_directory,
                policy=policy,
                worker_path=worker_path,
            )
        toolset = FunctionToolset[object](id=id, sequential=True)
        toolset.tool_plain(self.vsh_read)
        toolset.tool_plain(self.vsh_write)
        toolset.tool_plain(self.vsh_list)
        toolset.tool_plain(self.vsh_mkdir)
        toolset.tool_plain(self.vsh_remove)
        toolset.tool_plain(self.vsh_move)
        toolset.tool_plain(self.vsh_copy)
        toolset.tool_plain(self.vsh_glob)
        toolset.tool_plain(self.vsh_search)
        toolset.tool_plain(self.vsh_patch)
        toolset.tool_plain(self.vsh_run)
        super().__init__(
            id=id,
            description=(
                "Capability-rooted filesystem tools that simulate changes in VSH before "
                "policy-controlled commit."
            ),
            defer_loading=defer_loading,
            instructions=_INSTRUCTIONS,
            toolsets=[toolset],
        )

    async def vsh_read(self, path: str) -> VshToolResult:
        """Read a UTF-8 file from the current VSH snapshot."""

        return await self._call("vsh_read", path, intent=f"read {path}")

    async def vsh_write(self, path: str, data: str, append: bool = False) -> VshToolResult:
        """Write UTF-8 text in VSH; commit only when policy and hooks allow it."""

        return await self._call(
            "vsh_write",
            path,
            data,
            append=append,
            intent=f"{'append to' if append else 'write'} {path}",
        )

    async def vsh_list(self, path: str = "/workspace") -> VshToolResult:
        """List one directory from the current VSH snapshot."""

        return await self._call("vsh_list", path, intent=f"list {path}")

    async def vsh_mkdir(
        self,
        path: str,
        parents: bool = True,
        exist_ok: bool = True,
    ) -> VshToolResult:
        """Create a directory in VSH under policy-controlled commit."""

        return await self._call(
            "vsh_mkdir",
            path,
            parents=parents,
            exist_ok=exist_ok,
            intent=f"create directory {path}",
        )

    async def vsh_remove(
        self,
        path: str,
        recursive: bool = False,
        missing_ok: bool = False,
    ) -> VshToolResult:
        """Remove a path in VSH under policy-controlled commit."""

        return await self._call(
            "vsh_remove",
            path,
            recursive=recursive,
            missing_ok=missing_ok,
            intent=f"remove {path}",
        )

    async def vsh_move(self, source: str, destination: str) -> VshToolResult:
        """Move a path inside the VSH snapshot."""

        return await self._call(
            "vsh_move",
            source,
            destination,
            intent=f"move {source} to {destination}",
        )

    async def vsh_copy(
        self,
        source: str,
        destination: str,
        recursive: bool = False,
        overwrite: bool = False,
    ) -> VshToolResult:
        """Copy a file or directory tree inside the VSH snapshot."""

        return await self._call(
            "vsh_copy",
            source,
            destination,
            recursive=recursive,
            overwrite=overwrite,
            intent=f"copy {source} to {destination}",
        )

    async def vsh_glob(
        self,
        pattern: str,
        path: str = "/workspace",
        max_results: int = 1_000,
    ) -> VshToolResult:
        """Match paths in VSH with bounded glob results."""

        return await self._call(
            "vsh_glob",
            pattern,
            path=path,
            max_results=max_results,
            intent=f"glob {pattern} under {path}",
        )

    async def vsh_search(
        self,
        query: str,
        path: str = "/workspace",
        case_sensitive: bool = True,
        max_results: int = 100,
    ) -> VshToolResult:
        """Search UTF-8 files in VSH with bounded structured results."""

        return await self._call(
            "vsh_search",
            query,
            path=path,
            case_sensitive=case_sensitive,
            max_results=max_results,
            intent=f"search for text under {path}",
        )

    async def vsh_patch(
        self,
        path: str,
        old: str,
        new: str,
        count: int = 1,
    ) -> VshToolResult:
        """Replace exact UTF-8 text in one VSH file."""

        return await self._call(
            "vsh_patch",
            path,
            old,
            new,
            count=count,
            intent=f"replace exact text in {path}",
        )

    async def vsh_run(self, code: str, intent: str) -> VshToolResult:
        """Run atomic Monty code with vsh_* calls; use Python arguments, never JSON objects."""

        return await self._execute(code, intent)

    async def _call(
        self,
        function: str,
        *args: object,
        intent: str,
        **kwargs: object,
    ) -> VshToolResult:
        positional = [repr(value) for value in args]
        keywords = [f"{name}={value!r}" for name, value in kwargs.items()]
        code = f"result = {function}({', '.join((*positional, *keywords))})\nresult"
        return await self._execute(code, intent)

    async def _execute(self, code: str, intent: str) -> VshToolResult:
        request = RunRequest(code, intent=intent, mode=RunMode.AUTO)
        hook_verdict: str | None = None
        feedback: str | None = None
        if isinstance(self.runtime, HookedRuntime):
            receipt = await asyncio.to_thread(self.runtime.preview, request)
            if receipt.state in _ACTIONABLE_STATES:
                resolution = await self.runtime.acommit(receipt.transaction)
                receipt = resolution.receipt
                if resolution.hook is not None:
                    hook_verdict = resolution.hook.verdict
                    if hook_verdict in {"review", "reject"}:
                        feedback = resolution.hook.reason
        else:
            receipt = await asyncio.to_thread(self.runtime.run, request)
        return _tool_result(receipt, hook_verdict=hook_verdict, feedback=feedback)


def _tool_result(
    receipt: Receipt,
    *,
    hook_verdict: str | None,
    feedback: str | None,
) -> VshToolResult:
    return VshToolResult(
        transaction=receipt.transaction,
        state=receipt.state,
        result=_json_value(receipt.result) if receipt.state == "committed" else None,
        changed_paths=receipt.changed_paths,
        hook_verdict=hook_verdict,
        feedback=feedback,
    )


def _json_value(value: object) -> JsonValue:
    if value is None or isinstance(value, bool | int | float | str):
        return value
    if isinstance(value, bytes):
        return {"encoding": "base64", "data": base64.b64encode(value).decode("ascii")}
    if isinstance(value, PathLike):
        return fspath(value)
    if isinstance(value, Mapping):
        return {str(key): _json_value(item) for key, item in value.items()}
    if isinstance(value, list | tuple):
        return [_json_value(item) for item in value]
    raise TypeError(f"VSH returned a value that cannot be sent to Pydantic AI: {type(value)!r}")


_INSTRUCTIONS = """\
Use the vsh_* tools for workspace filesystem work. Every call runs against an isolated
virtual snapshot before VSH policy decides whether exact changes may reach the host.
Use vsh_run for dependent multi-step work that should be evaluated and committed as one
transaction. Code passed to vsh_run can call only vsh_read, vsh_write, vsh_list, vsh_mkdir,
vsh_remove, vsh_move, vsh_copy, vsh_glob, vsh_search, and vsh_patch; functions such as
read_file or write_file do not exist. Write ordinary Python calls, for example
vsh_patch('/workspace/app.toml', 'old', 'new', count=1); never pass a JSON object as the
single positional argument. A result with state='pending_approval' has not changed host
files; report its transaction and feedback to the user instead of claiming completion.
Treat intent as context, not proof that the resulting changes are safe.
"""

__all__ = ("CommitJudge", "JudgeReport", "VshCapability", "VshToolResult")
