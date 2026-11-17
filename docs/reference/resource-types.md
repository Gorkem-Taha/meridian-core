# The configuration management resource types

The `ansible`, `puppet` and `chef` targets share six resource types. A resource
written with them compiles for any of the three, within what each tool can do.
Anything outside these tables is refused, never dropped: a param a target does
not take, a state it cannot express, an action it cannot perform.

Other targets do not use these types. Their resources are the tool's own; see
[documents, resources and targets](../concepts/ir-overview.md#two-families-of-types).

## `package`

| | |
|---|---|
| Params | `name` (required) |
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
| Can be notified | Ansible and Chef: `restart`, `reload`, `start`, `stop`. Puppet: `restart` (or `refresh`) only |

Puppet and Chef describe a state a node is left in, and "restarted" is not one,
so they refuse it as a state. To restart a service when something changes, use
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
directory has no `content`.

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
the group and leaves its other members alone.

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
| Params | `command` (required), `chdir`, `creates` (skip when this path exists), `shell` (true to run through a shell). Ansible also takes `removes` |
| States | none |
| Can be notified | `run` |

An `exec` runs on every apply unless `creates` says it is already done, so a
plan for a document with a bare `exec` always shows a change. A notified `exec`
runs in its place in the order, and once more when it is notified.

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
| `runtimeWhen` | a condition checked on the node during the apply, written in the tool's own language | Ansible (a `when:` expression), Chef (the Ruby of an `only_if` guard). Puppet refuses it: a catalog is compiled before it reaches the node |
