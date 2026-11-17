# The configuration management resource types

The `ansible`, `puppet`, `chef` and `salt` targets share six resource types. A
resource written with them compiles for any of the four, within what each tool
can do.
Anything outside these tables is refused, never dropped: a param a target does
not take, a state it cannot express, an action it cannot perform.

Other targets do not use these types. Their resources are the tool's own; see
[documents, resources and targets](../concepts/ir-overview.md#two-families-of-types).

## `package`

| | |
|---|---|
| Params | `name` (required). Salt also takes `version`, with `present` only |
| States | `present` (default), `absent`, `latest` |
| Can be notified | no |

```yaml
- id: nginx_pkg
  type: package
  params:
    name: nginx
```

## `service`

| | |
|---|---|
| Params | `name` (required), `enabled` (true or false: start at boot) |
| States | `running` (default), `started` (the same), `stopped`. Ansible also takes `restarted` and `reloaded` |
| Can be notified | Ansible, Chef and Salt: `restart`, `reload`, `start`, `stop`. Puppet: `restart` (or `refresh`) only |

Puppet, Chef and Salt describe a state a node is left in, and "restarted" is not
one, so they refuse it as a state. Salt also refuses `restart` and `reload`
aimed at a service whose state is `stopped`. To restart a service when something changes, use
`notifies`.

```yaml
- id: nginx_svc
  type: service
  state: running
  params:
    name: nginx
    enabled: true
```

## `file`

| | |
|---|---|
| Params | `path` (required), `content`, `owner`, `group`, `mode` |
| States | `present` (default) or `file`, `directory`, `absent` |
| Can be notified | no |

Write `mode` as a quoted string (`"0644"`), so it is not read as a number. A
directory has no `content`. On Salt a `file` with no `content` creates the file
if it is missing, a directory is created with its parents, and the attributes of
an absent file are left out with a warning.

```yaml
- id: app_conf
  type: file
  params:
    path: /etc/app/app.conf
    content: "workers = 8\n"
    owner: root
    mode: "0640"
```

## `user`

| | |
|---|---|
| Params | `name` (required), `uid`, `home`, `shell`, `system`, `groups` |
| States | `present` (default), `absent` |
| Can be notified | no |

`groups` are supplementary groups. On Chef, whose `user` resource has a primary
group only, each one becomes a separate membership block that adds the user to
the group and leaves its other members alone. On Salt the list is complete: the
user is removed from any group not in it.

## `group`

| | |
|---|---|
| Params | `name` (required), `gid`, `system` |
| States | `present` (default), `absent` |
| Can be notified | no |

## `exec`

The escape hatch: run a command. It has no state, because there is nothing to
converge on.

| | |
|---|---|
| Params | `command` (required), `chdir`, `creates` (skip when this path exists), `shell` (true to run through a shell). Ansible and Salt also take `removes` (run only when this path exists) |
| States | none |
| Can be notified | `run` |

An `exec` runs on every apply unless `creates` says it is already done, so a
plan for a document with a bare `exec` always shows a change. A notified `exec`
runs in its place in the order, and once more when it is notified.

Salt always runs a command through a shell. So that a document means the same
on every target, Salt refuses a command that holds shell syntax unless the
resource says `shell: true`; see [the Salt page](../targets/salt.md#exec).

```yaml
- id: warm_cache
  type: exec
  params:
    command: /usr/local/bin/warm-cache --all
    chdir: /srv/app
    creates: /var/cache/app/warm
```

## What every resource can carry

| Field | Meaning | Where |
|---|---|---|
| `dependsOn` | order | every target ([how](../concepts/dependencies.md)) |
| `notifies` | react to a change | where the tool can ([matrix](capability-matrix.md)) |
| `runtimeWhen` | a condition checked on the node during the apply, written in the tool's own language | Ansible (a `when:` expression), Chef (the Ruby of an `only_if` guard), Salt (a Jinja expression over grains and pillar). Puppet refuses it: a catalog is compiled before it reaches the node |
