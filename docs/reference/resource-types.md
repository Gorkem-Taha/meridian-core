# The configuration management resource types

The `ansible`, `puppet`, `openvox`, `chef` and `cloud-init` targets share six
resource types. A resource written with them compiles for any of the five, within
what each tool can do. OpenVox reads the language Puppet reads, so wherever this
page says Puppet, the same holds for `openvox`.
Anything outside these tables is refused, never dropped: a param a target does
not take, a state it cannot express, an action it cannot perform.

How the rest of a configuration management document is written is in
[the configuration management flavour](dsl/configuration-management.md).

Other targets do not use these types. Their resources are the tool's own; see
[documents, resources and targets](../concepts/ir-overview.md#two-families-of-types).

## `package`

| | |
|---|---|
| Params | `name` (required). cloud-init also takes `version`, to pin one |
| States | `present` (default), `absent`, `latest`. cloud-init: `present` only, because a first boot installs and cannot remove or track |
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
| States | `running` (default), `started` (the same), `stopped`. Ansible and cloud-init also take `restarted` and `reloaded` |
| Can be notified | Ansible, Chef and cloud-init: `restart`, `reload`, `start`, `stop`. Puppet: `restart` (or `refresh`) only |

Puppet and Chef describe a state a node is left in, and "restarted" is not one,
so they refuse it as a state. To restart a service when something changes, use
`notifies`.

cloud-init has no module for services. A service compiles to `systemctl`
commands that run at the end of the first boot, with a warning that the image
must use systemd.

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
| Params | `path` (required), `content`, `owner`, `group`, `mode`. cloud-init also takes `encoding` and `append` |
| States | `present` (default) or `file`, `directory`, `absent` |
| Can be notified | no |

Write `mode` as a quoted string (`"0644"`), so it is not read as a number. A
directory has no `content`.

On cloud-init a file in state `present` must have `content`, because cloud-init
writes whole files, and the `directory` and `absent` states compile to commands
(`install -d`, `rm -rf`).

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
| Params | `name` (required), `uid`, `home`, `shell`, `system`, `groups`. cloud-init also takes `gecos`, `primary_group`, `lock_passwd`, `sudo` and `ssh_authorized_keys` |
| States | `present` (default), `absent`. cloud-init: `present` only |
| Can be notified | no |

`groups` are supplementary groups. On Chef, whose `user` resource has a primary
group only, each one becomes a separate membership block that adds the user to
the group and leaves its other members alone.

## `group`

| | |
|---|---|
| Params | `name` (required), `gid`, `system`. cloud-init takes `name` and `members` only: it cannot choose a group's id |
| States | `present` (default), `absent`. cloud-init: `present` only |
| Can be notified | no |

## `exec`

The escape hatch: run a command. It has no state, because there is nothing to
converge on.

| | |
|---|---|
| Params | `command` (required), `chdir`, `creates` (skip when this path exists), `shell` (true to run through a shell). Ansible and cloud-init also take `removes` |
| States | none |
| Can be notified | `run` |

An `exec` runs on every apply unless `creates` says it is already done, so a
plan for a document with a bare `exec` always shows a change. A notified `exec`
runs in its place in the order, and once more when it is notified. On
cloud-init every `exec` runs once, on the first boot, and stops the commands
after it if it fails.

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
| `runtimeWhen` | a condition checked on the node during the apply, written in the tool's own language | Ansible (a `when:` expression), Chef (the Ruby of an `only_if` guard). Puppet refuses it: a catalog is compiled before it reaches the node. cloud-init refuses it: it evaluates nothing per resource |
