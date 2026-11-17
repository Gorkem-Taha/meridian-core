# The `meridian` command

```
meridian validate <files>...
meridian compile  <files>... [--out DIR]
meridian plan     <files>...
meridian apply    <files>...
meridian plugins
```

The files given to one command are one workflow. A file may hold several
documents separated by `---`. Infrastructure documents run before ResourceSets
([why](../concepts/two-stage-architecture.md)).

## Commands

### `validate`

Checks the documents and loads no plugin: required fields, unique ids,
dependencies that exist, and hand-offs between documents that name a real
document and output. Errors give the file, line and field. It does not know what
a target can express, and it does not look for dependency cycles; `compile`
does both. [Which check happens when](dsl/index.md#which-check-happens-when) has
the full list.

### `compile`

Compiles each document into its target's own files and changes nothing. With
`--out DIR` the files are written to `DIR/<document name>/`. Without it, the
command only reports whether each document compiles, with any warnings.

A ResourceSet whose host is an output that has not been produced yet is reported
as deferred.

### `plan`

Compiles each document and asks its tool what an apply would change, using the
tool's own preview. Changes are listed by the ids the document uses:

```
  + create ...
  ~ update ...
  - delete ...
  ! replace ...
```

When a tool cannot say what would change, the plan is reported as unknown
instead of empty.

### `apply`

Compiles and applies each document in order, stopping at the first failure. A
stage's outputs are shown, kept, and handed to the stages that asked for them.
Values a tool marks sensitive are shown as `(sensitive)`.

### `plugins`

Lists every plugin Meridian can find: its category, what it can do, the resource
types it takes, and where its binary is.

## Options

| Option | Environment | Default | Meaning |
|---|---|---|---|
| `--plugin-dir DIR` | `MERIDIAN_PLUGIN_DIR` (separated by `:`) | the directory `meridian` is in | where to look for plugins; may be repeated |
| `--state-dir DIR` | `MERIDIAN_STATE_DIR` | `.meridian` | each tool's working files, and the outputs of applied stages |
| `--out DIR` | | | `compile` only: where to write the compiled files |

## Exit status

`0` when everything asked for succeeded. `1` when a document was invalid, did
not compile, or a tool failed. `2` for a mistake in the command line itself.

## Plugins and their tools

A plugin is a separate program named `meridian-target-<name>`. Each one runs its
tool as you would by hand, with the credentials and configuration the tool
already has. Every plugin reads a few environment variables that name a
different binary or add arguments to every call; they are listed on each
[target's page](../targets/ansible.md#running).
