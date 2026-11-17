# The plugin template

A small plugin that works, to copy when writing a new one. Do not copy it by
hand. From the repository root:

```
python3 tools/new_plugin.py <name> --category <category>
```

That copies this crate to `plugins/<name>`, renames everything in it and adds
it to the workspace. The guide is
[Writing a plugin](../../docs/contributing/writing-a-plugin.md).

The crate is a workspace member so that the ordinary test run builds it and
runs its tests. A template that no longer compiles against the SDK fails CI
like any other code.
