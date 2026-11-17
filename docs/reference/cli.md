# The `meridian` command

```
meridian validate <files>...
meridian compile  <files>... [-l SELECTOR] [--data FILE] [--out DIR]
meridian plan     <files>... [-l SELECTOR] [--data FILE]
meridian apply    <files>... [-l SELECTOR] [--data FILE] [--resume [RUN]] [--on-failure stop|rollback]
meridian drift    <files>... [-l SELECTOR] [--data FILE]
meridian rollback <files>... [--run RUN]
meridian status
meridian runs
meridian events   [RUN] [--type TYPE]
meridian report   [RUN] [--format FORMAT] [--out FILE] [--from FILE]
meridian unlock   <ID>
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

A plan runs each tool, so it takes the [state lock](../concepts/run-safety.md#one-run-at-a-time)
and is recorded like an apply.

### `apply`

Compiles and applies each document in order, stopping at the first failure. A
stage's outputs are shown and handed to the stages that asked for them.
Values a tool marks sensitive are shown as `(sensitive)`.

An apply holds the [state lock](../concepts/run-safety.md#one-run-at-a-time)
while it runs, so a second one against the same state is refused, and it is
[recorded](../concepts/run-safety.md#the-record) step by step.

When a stage fails, what its tool had already changed is listed above the
failure, the stages after it are not run, and the command says how to
continue:

```
web (ResourceSet, target ansible on 203.0.113.10)
  ~ update nginx_conf
  FAILED: the play failed (ran `ansible-playbook ...`, exit status 2)
run 2026-11-17T09-14-03Z-0cc7 failed. Once the cause is fixed, `meridian apply --resume` with the same documents continues from the stage that failed
```

`--resume` continues the last apply that failed or was interrupted, and
`--resume <run>` a named one. Stages that run already applied, from documents
that still compile to the same artifact, are left alone; the rest are applied.
[Picking up where a run stopped](../concepts/run-safety.md#picking-up-where-a-run-stopped)
has the rules.

With `--on-failure rollback`, a failed apply instead
[puts back](../concepts/run-safety.md#putting-a-run-back) every stage it
changed, the failed one first and then the ones before it, by applying what
each was last applied from successfully. The command still exits `1`: the
apply failed.

### `drift`

Checks whether what was applied is still what is there, and changes nothing.
Each document is compiled and its tool asked for a preview, and each stage is
reported as `in sync`, `DRIFTED`, `pending` (the document changed since the
last apply, so the differences are changes waiting to be applied), `unknown`
(the tool cannot preview) or `never applied`.
[Finding drift](../concepts/run-safety.md#finding-drift) has the rules.

```
web (ResourceSet, target ansible on 203.0.113.10)
  ~ update sshd_config
  DRIFTED: 1 difference from what was applied
drift: 1 of 2 stages drifted
```

It exits `2` when a stage drifted, so a scheduled job can tell drift from a
clean check (`0`) and from a check that could not be made (`1`).

### `rollback`

Puts back what the last apply changed: each stage that apply handed to a tool
is applied again from the artifact of its last successful apply before it,
last stage first. `--run RUN` names another apply. Give it the files the
apply was given.

A stage that cannot be put back is reported as `NOT ROLLED BACK` with the
reason, and the command exits `1`.
[Putting a run back](../concepts/run-safety.md#putting-a-run-back) lists the
cases, among them a stage that was applied for the first time, which has
nothing to go back to.

### `status`

Shows the current or last run: what was asked, who ran it, how each step
stands, and whether the state is locked and by whom.

```
run 2026-11-17T09-14-03Z-0cc7: apply, failed
  command:  meridian apply site.yaml
  started:  2026-11-17T09:14:03.204Z by mike@build-07
  finished: 2026-11-17T09:15:11.910Z
  network (Infrastructure, target terraform): succeeded, 4 changes
  web (ResourceSet, target ansible on 203.0.113.10): failed (tool-failed), 2 changes
not locked
```

A run that was killed is shown as `interrupted`.

### `runs`

Lists every recorded run, oldest first: its id, whether it was a plan or an
apply, how it ended, and which run it resumed.

```
2026-11-17T09-14-03Z-0cc7  apply    failed     2 steps, 1 failed
2026-11-17T09-21-40Z-1f42  apply    succeeded  2 steps, resumes 2026-11-17T09-14-03Z-0cc7
```

### `events`

Prints a run's [events](state.md#events), one JSON object per line; the last
run's when no run is named. `--type` keeps only events of that type, takes a
family as `'step.*'`, and may be repeated.

```
meridian events --type step.failed --type 'resource.*'
```

### `report`

Writes a [report](reporting.md) of a recorded run, the last one when no run
is named: `--format` is `json`, `junit`, `markdown` (the default) or `html`,
and `--out` names a file to write instead of standard output. `--from FILE`
reports on a file of events, as `--events json` wrote them, and not on the
state; `-` reads standard input.

```
meridian report --format junit --out meridian.xml
meridian apply site.yaml --events json | meridian report --from - --format html --out run.html
```

### `unlock`

Removes the state lock a run left behind when it was killed. It takes the
lock's id, as `status` and the refused run show it, and refuses any other, so
the lock removed is the one you looked at. Make sure the run that held it is
really gone first: removing the lock of a run that is still going lets a
second one collide with it.

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

## Options

| Option | Environment | Default | Meaning |
|---|---|---|---|
| `--plugin-dir DIR` | `MERIDIAN_PLUGIN_DIR` (separated by `:`) | the directory `meridian` is in | where to look for plugins; may be repeated |
| `--state-dir DIR` | `MERIDIAN_STATE_DIR` | `.meridian` | each tool's working files, and with the local backend Meridian's own record |
| `--backend NAME` | `MERIDIAN_BACKEND` | `local` | where Meridian's own record (the lock, the run-state, the runs) is kept: `local`, or a [backend program](state.md#writing-a-backend) |
| `--backend-config KEY=VALUE` | | | a setting for the backend; may be repeated |
| `--lock-timeout DURATION` | | `0s` | `plan`, `apply`, `drift`, `rollback`: how long to wait for another run to release the lock, as in `30s`, `5m`, `1h`. By default a locked state is refused at once |
| `--resume [RUN]` | | | `apply` only: continue the last apply, or the run named |
| `--on-failure ACTION` | | `stop` | `apply` only: `stop` leaves what was applied in place; `rollback` puts back every stage the run changed |
| `--run RUN` | | the last apply | `rollback` only: the apply to put back |
| `--events json` | | | `compile`, `plan`, `apply`, `drift`, `rollback`: write every [event](state.md#events) to standard output as it happens, one JSON object per line. The report for people moves to standard error |
| `--report FORMAT=FILE` | | | `compile`, `plan`, `apply`, `drift`, `rollback`: write a [report](reporting.md#--report) of the run when it ends, as `json`, `junit`, `markdown` or `html`; may be repeated |
| `--sink NAME` | `MERIDIAN_SINKS` (separated by `,`) | | the same commands: send every event, as it happens, to a [sink](reporting.md#sinks); may be repeated |
| `--sink-config SINK.KEY=VALUE` | | | a setting for a sink; may be repeated. `on` chooses the event types and `required=true` fails the run when the sink fails |
| `--sink-timeout DURATION` | | `30s` | how long a sink has to finish sending after the run's last event |
| `--keep-runs N` | | `50` | `plan`, `apply`, `drift`, `rollback`: how many runs' records to keep |
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
not compile, a tool failed, a stage could not be rolled back, or the run was
refused because the state is locked or there was nothing to resume or roll
back, or a [sink](reporting.md#sinks) marked `required` failed. `2` for a mistake in the command line itself, and from `drift` when a
stage has drifted.

## Plugins and their tools

A plugin is a separate program named `meridian-target-<name>`. Each one runs its
tool as you would by hand, with the credentials and configuration the tool
already has. Every plugin reads a few environment variables that name a
different binary or add arguments to every call; they are listed on each
[target's page](../targets/ansible.md#running).
