# Jujutsu workflow

- In this colocated repository, use `jj status` and `jj diff` before editing. Check for existing work and use `jj new -m "…"` to start a separate change when needed. A child change inherits its parent's files.
- Keep each change focused on one independently reviewable purpose, including its tests. Preserve existing user edits. Use `jj commit -m "…" <paths>` or `jj split -i` to separate mixed changes.
- Run relevant tests, then inspect `jj diff -r <change>` and `jj log`. Give each completed change a description that explains its purpose.
- Inspect `jj op log` before undoing repository operations. Push only on explicit request.
