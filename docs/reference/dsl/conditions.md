# Conditions: `when` and `runtimeWhen`

A resource can carry two conditions. They look alike and are decided at two
different times, by two different parties, in two different languages.

```mermaid
flowchart LR
    subgraph M["Meridian, before compiling"]
        W["when<br/>is this resource in the document at all?"]
    end
    subgraph T["The tool, on the machine, while applying"]
        R["runtimeWhen<br/>should this resource be applied right now?"]
    end
    W -->|kept| C["compiled into the tool's files"] --> R
    W -->|removed| X["not in the output"]
```

| | `when` | `runtimeWhen` |
|---|---|---|
| Decided by | Meridian | the tool |
| Decided when | before anything is compiled | on the machine, at the moment the resource is applied |
| Written in | Meridian's own condition language | the tool's own language |
| Effect when false | the resource is not in the compiled output | the resource is in the output; the tool skips it |
| Can read | the values given with `--data` | whatever the tool can see on the machine |
| Available | on every target | on the targets that can |

## `runtimeWhen`

```yaml
- id: apt_cache
  type: exec
  params:
    command: apt-get update
  runtimeWhen: "ansible_facts['os_family'] == 'Debian'"
```

Text. Meridian does not read it, check it, or translate it. It is copied into
the construct the tool has for a last-moment check, so it must be valid in that
tool's language, and it is the one part of a shared-dialect document that does
not move between tools.

| Target | It becomes | Written in | Example |
|---|---|---|---|
| `ansible` | the task's `when:` | a Jinja2 expression over Ansible facts and variables | `"ansible_facts['os_family'] == 'Debian'"` |
| `salt` | a Jinja `if` around the state, evaluated when the minion renders the file | a Jinja expression over grains and pillar, written without `{{ }}` | `"grains['os_family'] == 'RedHat'"` |
| `chef` | an `only_if` guard | Ruby, with node attributes | `"node['platform_family'] == 'debian'"` |
| `dsc` | a `preCondition` | the DSC runner's function language | `"equals (nodeName) 'web'"` |

Every other target refuses a resource that carries `runtimeWhen`, because its
tool has no such moment:

```
rw.yaml:8:7: puppet: resource "a" [unsupported-runtime-condition]: resource carries runtimeWhen "x", which this target cannot express: a Puppet catalog is compiled before it reaches the node, so there is no apply-time conditional to compile it into
```

| Target | Why not |
|---|---|
| `puppet`, `openvox` | a catalog is compiled before it reaches the node |
| `cloud-init` | it evaluates nothing per resource; the machine applies its user-data once |
| `terraform`, `opentofu`, `pulumi`, `cloudformation`, `bicep`, `arm` | there is no node; the tool talks to an API |
| `packer` | refused; a step's own `only` or `except` param does this job |
| `docker`, `kubernetes`, `helm` | nothing to check a condition against while a project or a set of objects is submitted |
| `podman` | refused; systemd's own `Condition*=` settings under `unit` do this job |

The [capability matrix](../capability-matrix.md) has the `runtimeWhen` column.

**What it does to a plan.** A condition checked at apply time cannot be known
in advance, so a plan shows the resource as the tool's own dry run shows it.

**With `notifies`.** A notified resource's own `runtimeWhen` is not copied onto
the reaction. On Ansible, for example, the handler built for a notified service
does not carry the service task's `when:`.

## `when`

```yaml
- id: debug_log
  type: file
  params: {path: /var/log/app/debug.log}
  when: "data.env != 'prod'"
```

`when` is decided by Meridian itself, before compiling, so that the compiled
output is static: a resource is either in it or not, and what you review is what
will run. No target ever sees the condition, so it works on every target.

### The condition language

A condition reads the values given to the command with `--data` and compares
them. Quote the whole condition in YAML, because `!` and `'` mean something to
YAML at the start of a value.

| You write | Meaning |
|---|---|
| `data.env`, `data.region.name` | a value from the data files: a key, then keys inside mappings |
| `true`, `false`, `null` | themselves |
| `3`, `-1`, `1.5` | numbers. `3` and `3.0` are equal |
| `'prod'`, `"prod"` | text. `\'` and `\"` put a quote inside |
| `a == b`, `a != b` | whether two values are the same |
| `!a` | not |
| `a && b`, `a \|\| b` | and, or. `&&` binds tighter than `\|\|`; the right side is only read when the left leaves the answer open |
| `( ... )` | grouping |
| `defined(data.key)` | whether the data sets that key |

| Rule | Checked by | Message |
|---|---|---|
| The condition must be written in this language | `validate` | `spec.resources[0].when: unexpected ">" (a condition is built from ...)` |
| `facts.*` is not available: nothing gathers facts from a node yet | `validate` | `facts are not available: ... a condition can only read data.<key>` |
| A key the condition reads must be set | `compile` | `data.env is not set (give it with --data, or test for it with defined(data.env))` |
| The condition must come out `true` or `false` | `compile` | `data.env is a string, not true or false; compare it with == or !=` |

There is no `<`, `>`, arithmetic or function other than `defined`. A condition
that needs more belongs in the data: work the answer out where the data is
produced, and give it as a boolean. Guard an optional key with
`defined(data.x) && data.x == 'y'`.

### Giving the data

```
meridian apply site.yaml --data common.yaml --data prod.yaml
```

A data file is YAML or JSON holding one mapping. `--data` may be repeated; a
key in a later file replaces the same key from an earlier file, whole. With no
`--data` there is no data, and a condition that reads a key is refused, not
taken as false.

### `ifMissing`

A resource whose `when` is false is not in the output, and neither are its own
`dependsOn` and `notifies`. An entry of *another* resource that names it has
lost its other end, and `ifMissing` on that entry says what happens:

```yaml
- id: legacy_dir
  type: file
  state: directory
  params: {path: /opt/legacy}
  when: data.legacy

- id: app_conf
  type: file
  params: {path: /etc/app.conf, content: "x"}
  dependsOn:
    - resource: legacy_dir
      ifMissing: skip
```

```mermaid
flowchart LR
    A["app_conf dependsOn legacy_dir"] --> Q{"legacy_dir kept?"}
    Q -->|yes| K["the entry stands"]
    Q -->|"no, ifMissing: error (default)"| E["the document is refused"]
    Q -->|"no, ifMissing: skip"| S["the entry is dropped;<br/>app_conf carries on"]
```

| Value | When the named resource was removed |
|---|---|
| `error` (the default) | refused: `resource "app_conf" depends on "legacy_dir", which its when removed from this run; set ifMissing: skip on this entry if the resource should carry on without it` |
| `skip` | the entry is dropped and the resource carries on without it |

It is the same on `dependsOn` and on `notifies`. The default is `error` because
a resource depending on something that is not there is far more often a mistake
than an intention. `ifMissing` does nothing to an entry whose resource is
present, and never excuses an entry naming a resource the document does not
declare.

Also:

- An output or a `${...}` reference that reads a removed resource is refused by
  the target, as a reference to a resource that is not there.
- A dependency cycle is refused whatever the conditions say.
- If every resource is removed the document compiles to nothing, and applying
  it tells the tool that nothing should exist.

| Command | A document that uses `when` |
|---|---|
| `validate` | the condition's syntax is checked |
| `compile`, `plan`, `apply` | the condition is decided from `--data`, and the resource is kept or removed |

## Choosing

| The condition depends on | Use |
|---|---|
| something only known on the machine at that moment (a file exists, a fact, a node attribute) | `runtimeWhen`, on `ansible`, `chef`, `salt` or `dsc` |
| which environment or kind of machine the document is for, as far as you can say it in a data file | `when`, on any target |
| which documents of a workflow to run at all | not a condition: [labels and a selector](index.md#labels-and-selectors) |
| whether another resource changed | not a condition: [`notifies`](notifies.md) |

## A worked example

[`examples/workflow/environments.yaml`](../../../examples/workflow/environments.yaml)
is one workflow for development and production: a Terraform document, an
Ansible one and a Kubernetes one, with the same conditions in each, two data
files, `ifMissing: skip`, a guarded optional key and one `runtimeWhen`.
[Worked workflows](../../quickstart/workflows.md#environments) goes through it,
with what each condition keeps in each environment.
