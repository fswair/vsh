"""One disposable transaction across Monty functions, bounded Bash and search."""

from pathlib import Path
from tempfile import TemporaryDirectory

from vsh import BashConfig, Runtime

with TemporaryDirectory(prefix="vsh-mixed-example-") as temporary:
    workspace = Path(temporary)
    runtime = Runtime.open(workspace, bash=BashConfig())
    preview = runtime.preview("""
vsh_write('/workspace/input.txt', 'approved\\n')
shell = vsh_bash('cat input.txt > report.txt; cat report.txt')
assert shell['exit_code'] == 0
assert shell['stdout'] == b'approved\\n'
matches = vsh_search('approved', path='/workspace/report.txt')
assert len(matches) == 1
{'files': 2, 'matches': len(matches)}
""")
    assert not (workspace / "input.txt").exists()
    assert not (workspace / "report.txt").exists()
    committed = runtime.commit(preview.transaction, 0)
    assert committed.state == "committed"
    assert (workspace / "report.txt").read_text() == "approved\n"
    print(committed.state, committed.result)
