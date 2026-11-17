# The `meridian` command

```
meridian validate <files>...
meridian compile  <files>... [-l SELECTOR] [--data FILE] [--set KEY=VALUE] [--out DIR]
meridian plan     <files>... [-l SELECTOR] [--data FILE] [--set KEY=VALUE]
meridian apply    <files>... [-l SELECTOR] [--data FILE] [--set KEY=VALUE]
meridian plugins
meridian migrate  <files>... [--to APIVERSION] [--check]
meridian version
meridian module   install [--locked] | update [NAME]... | verify | list | pack [--out FILE] | push REPOSITORY
meridian rules check --hierarchy FILE [--rules FILE] [--require-rules] [--data FILE] [--set KEY=VALUE]
```

The files given to one command are one workflow. A file may hold several
documents separated by `---`. Infrastructure documents run before ResourceSets
([why](../concepts/two-stage-architecture.md)).

A file can also be one inside a [module](modules.md) the project has pinned,
written `@<module>:<path>`:

```
meridian plan site.yaml @acme/nginx:documents/proxy.yaml --data @acme/nginx:data/defaults.yaml
```

This works wherever a document or a `--data` file is given. The module must be
in the project's `meridian.lock` and already fetched: `validate`, `compile`,
`plan` and `apply` never fetch anything, and they check each module against
its pinned hash before reading it.

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
  lockfile format  1
```

A language version is `supported`, `deprecated since meridian <release>` or
`removed in meridian <release>`. `plugin protocol` is the range this build
speaks, such as `2` or `2 to 3`. `lockfile format` is the format of
[`meridian.lock`](modules.md#the-lockfile) this build writes. `meridian --version`
prints the first line only. [Compatibility](compatibility.md) explains each.

### `rules check`

Loads the layers of a hierarchy and checks them against its
[compile rules](compile-rules.md): policies such as "a more specific layer may
not change a variable or a secret a less specific layer set". It compiles
nothing and loads no plugin.

```
$ meridian rules check --hierarchy config/hierarchy.yaml --set env=prod --set node=web01
error: compile rules: 1 violation; nothing was compiled
  - [meridian/protect-variables] layer "node/web01.yaml" (team) changes "variables.Region", which layer "common.yaml" (organization) set
```

The rules file is `rules.yaml` beside the hierarchy file, or the file given
with `--rules`. Every violation is listed, each naming the rule, the layer,
its group and the key, and never a value. With no rules file nothing is
checked, unless `--require-rules` is given, which makes that an error.
`--data FILE` and `--set KEY=VALUE` mean what they mean on `compile`: values
merged over the hierarchy, `--set` last. Each is checked as a layer of the
group `(command line)`, and a layer path's `%{KEY}` reads the `--set` value. [The reference](compile-rules.md#running-the-check) has the options
and the output in full.

## Options

| Option | Environment | Default | Meaning |
|---|---|---|---|
| `--plugin-dir DIR` | `MERIDIAN_PLUGIN_DIR` (separated by `:`) | the directory `meridian` is in | where to look for plugins; may be repeated |
| `--state-dir DIR` | `MERIDIAN_STATE_DIR` | `.meridian` | each tool's working files, the outputs of applied stages, and fetched [modules](modules.md#where-modules-are-kept) |
| `--out DIR` | | | `compile` only: where to write the compiled files |
| `-l SELECTOR`, `--selector SELECTOR` | | every document | `compile`, `plan`, `apply`: run only the documents whose `metadata.labels` match, as in `tier=web,env!=prod`. May be repeated; every requirement must hold. [Labels and selectors](dsl/index.md#labels-and-selectors) |
| `--data FILE` | | no data | `compile`, `plan`, `apply`: a YAML or JSON mapping of values that [`when`](dsl/index.md#when-and-runtimewhen) conditions and [interpolations](dsl/interpolation.md) read as `data.<key>`. May be repeated; a later file's key replaces an earlier file's |
| `--set KEY=VALUE` | | | `compile`, `plan`, `apply`: one value for `when` conditions and interpolations, applied after every `--data` file, so it wins. `KEY` may be dotted to set one key inside a mapping (`region.name=eu-west-1`). The value is typed as YAML types it. May be repeated. See [giving the data](dsl/conditions.md#giving-the-data) |

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

The same values are what an [interpolation](dsl/interpolation.md) reads:
`${{ data.env }}` inside a value is replaced before the document is compiled.

Every decision is printed under its stage, and a document its own `spec.when`
leaves out is reported as skipped. Neither is a failure:

```
site (ResourceSet, target ansible on shop-dev.example.com)
  when: 2 removed
    resource "site_cert": cond.needs_tls is false
    "site_conf" depends on "site_cert": cond.needs_tls is false
  compiled 2 files into build/site
monitoring (Infrastructure, target kubernetes)
  skipped: its when (data.monitoring.enabled ?? false) is false
```

A label can be interpolated too, and `-l` matches the value it comes to, so
the data is read before the selector is applied.

### `module`

Fetches, pins and checks the [modules](modules.md) that the `meridian.yaml` in
the current directory requires.

| Command | Does |
|---|---|
| `meridian module install` | Resolves the requirements, fetches what is missing, writes `meridian.lock`. With `--locked`, fails if the lockfile would change |
| `meridian module update [NAME]...` | Moves the named modules, or all of them, to the newest version that still fits |
| `meridian module verify` | Checks every pinned module against its hash; exits `1` if any differs |
| `meridian module list` | Lists the pinned modules, their versions and sources |
| `meridian module pack [--out FILE]` | Writes the module in the current directory as the archive a registry stores |
| `meridian module push REPOSITORY [--insecure]` | Publishes the module in the current directory to an OCI registry under its version. A version that is already published is never replaced with other content |

Only `install`, `update` and `push` use the network. Each command, its output and its
errors are described in [Modules](modules.md#commands).

### A deprecated language version

When a run reads documents written in a deprecated `apiVersion`, it prints one
warning for the whole run, however many documents there are, and carries on:

```
warning: 14 documents, the first being "web", use apiVersion "meridian/v1", which is deprecated since meridian 2.0.0 and will stop being read; run `meridian migrate --to meridian/v2` to move to meridian/v2
```

No version is deprecated today.

## Exit status

`0` when everything asked for succeeded. `1` when a document was invalid, did
not compile, or a tool failed. `2` for a mistake in the command line itself.

## Plugins and their tools

A plugin is a separate program named `meridian-target-<name>`. Each one runs its
tool as you would by hand, with the credentials and configuration the tool
already has. Every plugin reads a few environment variables that name a
different binary or add arguments to every call; they are listed on each
[target's page](../targets/ansible.md#running).
