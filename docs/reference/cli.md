# The `meridian` command

```
meridian validate <files>...
meridian compile  <files>... [-l SELECTOR] [--data FILE] [--out DIR]
meridian plan     <files>... [-l SELECTOR] [--data FILE]
meridian apply    <files>... [-l SELECTOR] [--data FILE]
meridian plugins
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
where its binary is, and what checking its signature found.

```
dsc
  category:  configuration-management
  can:       compile, plan, apply
  resources: <Module>/<Resource>, runner
  versions:  2, 3
  sdk:       0.1.0 (protocol 2)
  binary:    /usr/local/bin/meridian-target-dsc
  signature: verified, signed by key 780001b5de1d7691
```

The category decides which kind of document the plugin is handed, and the
versions decide which `targetVersion` a document may ask it for.

### `signing`

Makes signing keys, signs plugin binaries and checks their signatures. It is
for people who ship a plugin, or who want to check one without starting it;
[Plugin signatures](plugin-signatures.md) is the guide.

```
meridian signing keygen --secret acme.key --public acme.pub
meridian signing sign --key acme.key meridian-target-acme
meridian signing verify --trusted-key acme.pub meridian-target-acme
```

| Command | Does |
|---|---|
| `signing keygen --secret FILE --public FILE` | makes a new key pair. Neither file may exist already |
| `signing sign --key FILE BINARY...` | writes `<BINARY>.sig` beside each binary |
| `signing verify [--trusted-key FILE]... BINARY...` | prints what each binary's signature amounts to; exits non-zero if any is not verified |

## Options

| Option | Environment | Default | Meaning |
|---|---|---|---|
| `--plugin-dir DIR` | `MERIDIAN_PLUGIN_DIR` (separated by `:`, or `;` on Windows) | the directory `meridian` is really in, following a link to it | where to look for plugins; may be repeated |
| `--plugin-signatures POLICY` | `MERIDIAN_PLUGIN_SIGNATURES` | `require` in a released `meridian`, `off` in one built from source | what to do about [plugin signatures](plugin-signatures.md): `off` does not look, `warn` starts an unsigned or wrongly signed plugin and says so, `require` starts only plugins signed by a trusted key |
| `--trusted-key FILE` | `MERIDIAN_TRUSTED_KEYS` (separated by `:`, or `;` on Windows) | the key a released `meridian` was built with | a public key whose signatures on plugins are trusted, or a directory of `*.pub` files; may be repeated |
| `--state-dir DIR` | `MERIDIAN_STATE_DIR` | `.meridian` | each tool's working files, and the outputs of applied stages |
| `--out DIR` | | | `compile` only: where to write the compiled files |
| `-l SELECTOR`, `--selector SELECTOR` | | every document | `compile`, `plan`, `apply`: run only the documents whose `metadata.labels` match, as in `tier=web,env!=prod`. May be repeated; every requirement must hold. [Labels and selectors](dsl/index.md#labels-and-selectors) |
| `--data FILE` | | no data | `compile`, `plan`, `apply`: a YAML or JSON mapping of values that [`when`](dsl/index.md#when-and-runtimewhen) conditions read as `data.<key>`. May be repeated; a later file's key replaces an earlier file's |

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

## Exit status

`0` when everything asked for succeeded. `1` when a document was invalid, did
not compile, or a tool failed. `2` for a mistake in the command line itself.

## Plugins and their tools

A plugin is a separate program named `meridian-target-<name>`. Each one runs its
tool as you would by hand, with the credentials and configuration the tool
already has. Every plugin reads a few environment variables that name a
different binary or add arguments to every call; they are listed on each
[target's page](../targets/ansible.md#running).
