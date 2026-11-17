# The `meridian` command

```
meridian validate <files>...
meridian compile  <files>... [-l SELECTOR] [--data FILE] [--set KEY=VALUE] [--out DIR]
meridian plan     <files>... [-l SELECTOR] [--data FILE] [--set KEY=VALUE]
meridian apply    <files>... [-l SELECTOR] [--data FILE] [--set KEY=VALUE]
meridian plugins
meridian migrate  <files>... [--to APIVERSION] [--check]
meridian version
meridian fmt      <paths>... [--check] [--keep-order]
meridian lint     <paths>... [--data FILE] [--set KEY=VALUE] [--fix] [--strict] [--json]
meridian schema   [--target TARGET] [-o FILE]
meridian lsp
```

The files given to one command are one workflow. A file may hold several
documents separated by `---`. Infrastructure documents run before ResourceSets
([why](../concepts/two-stage-architecture.md)).

## Commands

### `validate`

Checks the documents and loads no plugin: required fields, unique ids,
dependencies that exist and do not form a cycle, conditions that are written in
the condition language, and hand-offs between documents that name a real
document and output. Errors give the file, line and field. A cycle is reported
with the resources in it:

```
site.yaml:12:7: spec.resources[1].dependsOn: dependency cycle among 2 resources: key -> lock -> key
```

It does not know what a target can express, so it cannot say whether the
target takes the document's kind, its `targetVersion` or its resource types;
`compile` does. [Which check happens when](dsl/index.md#which-check-happens-when)
has the full list.

### `compile`

Compiles each document into its target's own files and changes nothing. With
`--out DIR` the files are written to `DIR/<document name>/`. Without it, the
command only reports whether each document compiles, with any warnings.

A ResourceSet whose host is an output that has not been produced yet is reported
as deferred.

Before a document reaches its plugin, Meridian settles what is its own to
settle, and refuses the document if it cannot:

- the document's kind is one the target's category takes
  ([the pairing](dsl/index.md#the-document));
- the target compiles for the document's
  [`targetVersion`](dsl/index.md#targetversion), if it sets one;
- every [`when`](dsl/index.md#when-and-runtimewhen) is decided from the `--data`
  files, and the resources whose condition is false are left out.

`plan` and `apply` compile first, so the same holds for them.

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
types it takes, the versions of its tool it compiles for (when it declares any),
and where its binary is.

```
dsc
  category:  configuration-management
  can:       compile, plan, apply
  resources: <Module>/<Resource>, runner
  versions:  2, 3
  sdk:       0.1.0 (protocol 2)
  binary:    /usr/local/bin/meridian-target-dsc
```

The category decides which kind of document the plugin is handed, and the
versions decide which `targetVersion` a document may ask it for.

`sdk` is the SDK release the plugin was built against, and `protocol` is the
protocol version `meridian` and the plugin agreed on when it started. A plugin
whose protocol or SDK this `meridian` cannot work with is not listed: the
command fails and says which side is behind.
[Compatibility](compatibility.md) has the rules.

### `migrate`

Rewrites documents from an older language version (`apiVersion`) to a newer
one, in place.

```
meridian migrate site.yaml modules/*.yaml
meridian migrate site.yaml --to meridian/v1
meridian migrate site.yaml --check
```

- `--to` names the version to move to. Without it, the newest version this
  build supports.
- `--check` changes nothing and exits `1` if any file would change. Use it in
  CI to keep an estate on the current version.

Each file is reported on its own line:

```
site.yaml: already at meridian/v1
```

A file is rewritten as text, so comments, key order and layout are kept. Only
an `apiVersion` at the top level of a document is changed; an `apiVersion`
inside a resource's `params`, such as a Kubernetes object's, is left alone. A
file is rewritten whole or not at all.

`migrate` never guesses. Where a change between two versions needs a decision
only you can make, it lists those decisions for that file, leaves the file as
it was, and exits `1`. It never moves a document to an older version.

`meridian/v1` is the only language version so far, so today the command
reports that every Meridian document is already current. It is here now so
that the release that introduces `meridian/v2` ships with its migration.

### `version`

Shows every version this build carries. One number cannot answer "which
version of Meridian": the core, the language, the plugin SDK and the plugin
protocol each move on their own.

```
meridian 0.1.0
  language         meridian/v1  supported
  plugin sdk       0.1.0
  plugin protocol  2
```

A language version is `supported`, `deprecated since meridian <release>` or
`removed in meridian <release>`. `plugin protocol` is the range this build
speaks, such as `2` or `2 to 3`. `meridian --version` prints the first line
only. [Compatibility](compatibility.md) explains each.

### `fmt`

Rewrites documents in the one standard layout: two-space indentation, the
language's own field order, no trailing spaces. It changes how a document is
laid out and never what it says, and it checks that before writing. `--check`
changes nothing, lists the files that are not formatted and exits `1` if there
are any. `--keep-order` leaves fields in the order they were written. `-` reads
standard input and writes standard output.

A path may be a directory, which is searched for Meridian documents. Full
description: [The formatter](../tooling/formatter.md).

### `lint`

Reports everything `validate` and `compile` would refuse, for every document
under the paths given, and then things that compile and are probably mistakes.
Each document is compiled by its target's plugin in memory; nothing is written
and no tool is run.

```console
$ meridian lint stacks/
stacks/web.yaml:16:15: warning: mode 0644 is read as a number, not as permissions; write it as a quoted string [unquoted-mode]
stacks/web.yaml:35:21: error: spec.resources[3].dependsOn[0].resource: resource "warm" depends on unknown resource "wbe" [invalid-document]
1 error, 1 warning, 0 notes
```

`--fix` applies the fixes that are safe. `--strict` fails on warnings as well
as errors. `--json` prints one JSON object per finding. `--data` and `--set`
are as for `compile`; without them every resource is checked as if its `when`
held. Full
description: [The linter](../tooling/linter.md); the rules:
[Lint rules](lint.md).

### `schema`

Prints a JSON Schema (draft 7) of the document language and of the resource
types of every plugin found, for editors that validate YAML against a schema.
`--target` writes one target's rules only; `-o` writes to a file. Full
description: [The JSON Schema](../tooling/json-schema.md).

### `lsp`

Runs the language server on standard input and output. An editor starts it;
there is nothing to see when it is run by hand. Full description:
[The language server](../tooling/language-server.md).

## Options

| Option | Environment | Default | Meaning |
|---|---|---|---|
| `--plugin-dir DIR` | `MERIDIAN_PLUGIN_DIR` (separated by `:`) | the directory `meridian` is in | where to look for plugins; may be repeated |
| `--state-dir DIR` | `MERIDIAN_STATE_DIR` | `.meridian` | each tool's working files, and the outputs of applied stages |
| `--out DIR` | | | `compile` only: where to write the compiled files |
| `-l SELECTOR`, `--selector SELECTOR` | | every document | `compile`, `plan`, `apply`: run only the documents whose `metadata.labels` match, as in `tier=web,env!=prod`. May be repeated; every requirement must hold. [Labels and selectors](dsl/index.md#labels-and-selectors) |
| `--data FILE` | | no data | `compile`, `plan`, `apply`: a YAML or JSON mapping of values that [`when`](dsl/index.md#when-and-runtimewhen) conditions read as `data.<key>`. May be repeated; a later file's key replaces an earlier file's |
| `--set KEY=VALUE` | | | `compile`, `plan`, `apply`: one value for `when` conditions, applied after every `--data` file, so it wins. `KEY` may be dotted to set one key inside a mapping (`region.name=eu-west-1`). The value is typed as YAML types it. May be repeated. See [giving the data](dsl/conditions.md#giving-the-data) |

### Running part of a workflow

```
meridian apply site.yaml -l tier=web
```

Only the documents labelled `tier: web` run. The others are still loaded and
checked, and a selected document can still take its host from an unselected
one's last apply. A selector that matches no document is an error.

### Deciding conditions

```
meridian plan site.yaml --data common.yaml --data prod.yaml
```

A resource with `when: "data.env != 'prod'"` is in the plan or not, depending on
what the data files say `env` is. A condition that reads a key no file sets is
an error; nothing is guessed.

### A deprecated language version

When a run reads documents written in a deprecated `apiVersion`, it prints one
warning for the whole run, however many documents there are, and carries on:

```
warning: 14 documents, the first being "web", use apiVersion "meridian/v1", which is deprecated since meridian 2.0.0 and will stop being read; run `meridian migrate --to meridian/v2` to move to meridian/v2
```

No version is deprecated today.

## Exit status

`0` when everything asked for succeeded. `1` when a document was invalid, did
not compile, or a tool failed; when `fmt --check` found a file that is not
formatted; and when `lint` found an error, or a warning with `--strict`. `2`
for a mistake in the command line itself.

## Plugins and their tools

A plugin is a separate program named `meridian-target-<name>`. Each one runs its
tool as you would by hand, with the credentials and configuration the tool
already has. Every plugin reads a few environment variables that name a
different binary or add arguments to every call; they are listed on each
[target's page](../targets/ansible.md#running).
