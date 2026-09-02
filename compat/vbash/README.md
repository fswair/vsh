# vbash

`vbash` is a compatibility installer for VSH. It contains no Python modules and
depends on the exact matching `vsh-python` release. Existing code continues to use
`import vsh`; new dependency declarations should use `vsh-python` directly.
