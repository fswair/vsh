from __future__ import annotations as _annotations

from vsh.schemas import (
    CatCommand,
    CdCommand,
    ChmodCommand,
    CommandExample,
    CommandSpec,
    CopyCommand,
    DuCommand,
    EchoCommand,
    FindCommand,
    GrepCommand,
    HeadCommand,
    LnCommand,
    LsCommand,
    MkdirCommand,
    MoveCommand,
    NlCommand,
    PwdCommand,
    RemoveCommand,
    RgCommand,
    SedCommand,
    SortCommand,
    StatCommand,
    StructuredCommand,
    TailCommand,
    TouchCommand,
    WcCommand,
)

SchemaModel = type[StructuredCommand]


class CommandRegistration:
    def __init__(self, spec: CommandSpec, schema_model: SchemaModel) -> None:
        self.spec = spec
        self.schema_model = schema_model


registrations: dict[str, CommandRegistration] = {
    "vsh_pwd": CommandRegistration(
        spec=CommandSpec(
            name="vsh_pwd",
            summary="Return the current workspace directory.",
            description="Read the session cwd from the current workspace snapshot.",
            tags=["read", "filesystem", "navigation", "discovery", "pwd"],
            schema_model_name="PwdCommand",
            examples=[
                CommandExample(title="Logical cwd", params={"physical": False}),
                CommandExample(title="Physical cwd", params={"physical": True}),
            ],
        ),
        schema_model=PwdCommand,
    ),
    "vsh_cd": CommandRegistration(
        spec=CommandSpec(
            name="vsh_cd",
            summary="Change the simulated current workspace directory.",
            description="Resolve a directory path against the snapshot and predict the cwd after change.",
            tags=["read", "filesystem", "navigation", "cd"],
            schema_model_name="CdCommand",
            examples=[
                CommandExample(title="Enter src", params={"path": "src"}),
                CommandExample(title="Parent directory", params={"path": "..", "physical": True}),
            ],
        ),
        schema_model=CdCommand,
    ),
    "vsh_list": CommandRegistration(
        spec=CommandSpec(
            name="vsh_list",
            summary="List entries in a workspace directory.",
            description="Read directory metadata from the snapshot graph without reading file contents.",
            tags=["read", "filesystem", "discovery", "ls", "list"],
            schema_model_name="LsCommand",
            examples=[
                CommandExample(title="Current directory", params={"path": "."}),
                CommandExample(
                    title="Long listing", params={"path": "src", "all": True, "long": True}
                ),
            ],
        ),
        schema_model=LsCommand,
    ),
    "vsh_cat": CommandRegistration(
        spec=CommandSpec(
            name="vsh_cat",
            summary="Read a file from the workspace.",
            description="Predict file content access without hydrating content in the snapshot.",
            tags=["read", "filesystem", "content", "cat"],
            schema_model_name="CatCommand",
            examples=[CommandExample(title="Read README", params={"path": "README.md"})],
        ),
        schema_model=CatCommand,
    ),
    "vsh_head": CommandRegistration(
        spec=CommandSpec(
            name="vsh_head",
            summary="Read the first lines of a file.",
            description="Predict a bounded content read from the start of a file.",
            tags=["read", "filesystem", "content", "head"],
            schema_model_name="HeadCommand",
            examples=[
                CommandExample(title="First 20 lines", params={"path": "README.md", "lines": 20})
            ],
        ),
        schema_model=HeadCommand,
    ),
    "vsh_tail": CommandRegistration(
        spec=CommandSpec(
            name="vsh_tail",
            summary="Read the last lines of a file.",
            description="Predict a bounded content read from the end of a file.",
            tags=["read", "filesystem", "content", "tail"],
            schema_model_name="TailCommand",
            examples=[
                CommandExample(title="Last 50 lines", params={"path": "logs/app.log", "lines": 50})
            ],
        ),
        schema_model=TailCommand,
    ),
    "vsh_grep": CommandRegistration(
        spec=CommandSpec(
            name="vsh_grep",
            summary="Search file contents with grep-style options.",
            description="Predict content search reads over a file or directory path.",
            tags=["read", "search", "filesystem", "content", "grep"],
            schema_model_name="GrepCommand",
            examples=[
                CommandExample(
                    title="Find TODOs", params={"pattern": "TODO", "path": "src", "recursive": True}
                )
            ],
        ),
        schema_model=GrepCommand,
    ),
    "vsh_rg": CommandRegistration(
        spec=CommandSpec(
            name="vsh_rg",
            summary="Search workspace contents with ripgrep-style options.",
            description="Predict ripgrep-style content search reads over a file or directory path.",
            tags=["read", "search", "filesystem", "content", "rg", "ripgrep"],
            schema_model_name="RgCommand",
            examples=[
                CommandExample(
                    title="Search src", params={"pattern": "StructuredCommand", "path": "src"}
                )
            ],
        ),
        schema_model=RgCommand,
    ),
    "vsh_find": CommandRegistration(
        spec=CommandSpec(
            name="vsh_find",
            summary="Search workspace paths by metadata.",
            description="Predict metadata reads for find-style path discovery.",
            tags=["read", "search", "filesystem", "metadata", "find"],
            schema_model_name="FindCommand",
            examples=[
                CommandExample(
                    title="Python files", params={"path": ".", "name": "*.py", "type": "file"}
                )
            ],
        ),
        schema_model=FindCommand,
    ),
    "vsh_wc": CommandRegistration(
        spec=CommandSpec(
            name="vsh_wc",
            summary="Count file lines, words, bytes, or characters.",
            description="Predict file content reads for wc-style counting.",
            tags=["read", "filesystem", "content", "wc", "count"],
            schema_model_name="WcCommand",
            examples=[
                CommandExample(title="Line count", params={"path": "README.md", "lines": True})
            ],
        ),
        schema_model=WcCommand,
    ),
    "vsh_sort": CommandRegistration(
        spec=CommandSpec(
            name="vsh_sort",
            summary="Sort file lines in simulation.",
            description="Predict file content reads for sort-style line ordering.",
            tags=["read", "filesystem", "content", "sort"],
            schema_model_name="SortCommand",
            examples=[
                CommandExample(title="Sort file", params={"path": "names.txt", "unique": True})
            ],
        ),
        schema_model=SortCommand,
    ),
    "vsh_echo": CommandRegistration(
        spec=CommandSpec(
            name="vsh_echo",
            summary="Emit text or write text to a file.",
            description="Predict stdout text output or shell-redirection-style file writes.",
            tags=["write", "filesystem", "content", "echo"],
            mutates_fs=True,
            schema_model_name="EchoCommand",
            examples=[
                CommandExample(
                    title="Write file", params={"text": "hello", "output_path": "hello.txt"}
                )
            ],
        ),
        schema_model=EchoCommand,
    ),
    "vsh_sed": CommandRegistration(
        spec=CommandSpec(
            name="vsh_sed",
            summary="Read transformed file content with sed.",
            description="Predict sed-style content reads and in-place multi-file edits in simulation.",
            tags=["read", "filesystem", "content", "sed"],
            schema_model_name="SedCommand",
            examples=[
                CommandExample(
                    title="Show first 120 lines", params={"script": "1,120p", "path": "src/app.py"}
                )
            ],
        ),
        schema_model=SedCommand,
    ),
    "vsh_nl": CommandRegistration(
        spec=CommandSpec(
            name="vsh_nl",
            summary="Read a file with line numbers.",
            description="Predict numbered file content reads.",
            tags=["read", "filesystem", "content", "nl", "number-lines"],
            schema_model_name="NlCommand",
            examples=[
                CommandExample(
                    title="Number file", params={"path": "README.md", "number_all": True}
                )
            ],
        ),
        schema_model=NlCommand,
    ),
    "vsh_stat": CommandRegistration(
        spec=CommandSpec(
            name="vsh_stat",
            summary="Inspect path metadata.",
            description="Predict filesystem metadata reads for stat-style inspection.",
            tags=["read", "filesystem", "metadata", "stat"],
            schema_model_name="StatCommand",
            examples=[CommandExample(title="Stat README", params={"path": "README.md"})],
        ),
        schema_model=StatCommand,
    ),
    "vsh_du": CommandRegistration(
        spec=CommandSpec(
            name="vsh_du",
            summary="Estimate path disk usage.",
            description="Predict metadata reads for du-style size inspection.",
            tags=["read", "filesystem", "metadata", "du", "size"],
            schema_model_name="DuCommand",
            examples=[
                CommandExample(
                    title="Summarize src",
                    params={"path": "src", "summarize": True, "human_readable": True},
                )
            ],
        ),
        schema_model=DuCommand,
    ),
    "vsh_chmod": CommandRegistration(
        spec=CommandSpec(
            name="vsh_chmod",
            summary="Change path permissions in simulation.",
            description="Predict permission metadata writes for chmod-style operations.",
            tags=["mutate", "filesystem", "metadata", "permissions", "chmod"],
            mutates_fs=True,
            schema_model_name="ChmodCommand",
            examples=[
                CommandExample(title="Make executable", params={"mode": "+x", "path": "script.sh"})
            ],
        ),
        schema_model=ChmodCommand,
    ),
    "vsh_link": CommandRegistration(
        spec=CommandSpec(
            name="vsh_link",
            summary="Create a hard link or symbolic link in simulation.",
            description="Predict link creation and policy effects before execution.",
            tags=["mutate", "filesystem", "link", "ln", "symlink"],
            mutates_fs=True,
            schema_model_name="LnCommand",
            examples=[
                CommandExample(
                    title="Create symlink",
                    params={"src": "src", "dst": "current-src", "symbolic": True},
                )
            ],
        ),
        schema_model=LnCommand,
    ),
    "vsh_mkdir": CommandRegistration(
        spec=CommandSpec(
            name="vsh_mkdir",
            summary="Create a directory in simulation.",
            description="Predict directory creation in the overlay before real execution exists.",
            tags=["mutate", "filesystem", "mkdir"],
            mutates_fs=True,
            schema_model_name="MkdirCommand",
            examples=[CommandExample(title="Create directory", params={"path": "build"})],
        ),
        schema_model=MkdirCommand,
    ),
    "vsh_touch": CommandRegistration(
        spec=CommandSpec(
            name="vsh_touch",
            summary="Create or update a file timestamp in simulation.",
            description="Predict file touch behavior in the overlay before real execution exists.",
            tags=["mutate", "filesystem", "touch"],
            mutates_fs=True,
            schema_model_name="TouchCommand",
            examples=[CommandExample(title="Touch file", params={"path": "README.md"})],
        ),
        schema_model=TouchCommand,
    ),
    "vsh_move": CommandRegistration(
        spec=CommandSpec(
            name="vsh_move",
            summary="Move a path in simulation.",
            description="Predict a rename operation and its policy effects.",
            tags=["mutate", "filesystem", "mv", "move"],
            mutates_fs=True,
            schema_model_name="MoveCommand",
            examples=[CommandExample(title="Rename file", params={"src": "a.py", "dst": "b.py"})],
        ),
        schema_model=MoveCommand,
    ),
    "vsh_copy": CommandRegistration(
        spec=CommandSpec(
            name="vsh_copy",
            summary="Copy a path in simulation.",
            description="Predict a copy operation and its policy effects.",
            tags=["mutate", "filesystem", "cp", "copy"],
            mutates_fs=True,
            schema_model_name="CopyCommand",
            examples=[CommandExample(title="Copy file", params={"src": "a.py", "dst": "b.py"})],
        ),
        schema_model=CopyCommand,
    ),
    "vsh_remove": CommandRegistration(
        spec=CommandSpec(
            name="vsh_remove",
            summary="Remove a path in simulation.",
            description="Predict deletion before approval and real execution.",
            tags=["mutate", "filesystem", "rm", "remove"],
            mutates_fs=True,
            schema_model_name="RemoveCommand",
            examples=[CommandExample(title="Remove file", params={"path": "tmp.txt"})],
        ),
        schema_model=RemoveCommand,
    ),
}

registry: dict[str, CommandSpec] = {name: item.spec for name, item in registrations.items()}
