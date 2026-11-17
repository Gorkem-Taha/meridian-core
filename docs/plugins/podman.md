# Podman plugin

<img src="../branding/logos/meridian-podman.svg" width="72" alt="Meridian Podman plugin logo">

| | |
|---|---|
| Target name | `podman` |
| Category | containerization |
| Compiles to | [Quadlet](https://docs.podman.io/en/latest/markdown/podman-systemd.unit.5.html) unit files, one per resource |
| Runs with | `systemctl` and `podman`, on this machine or over `ssh` |
| Operations | plan, apply, outputs |
| Needs on the host | Linux with systemd, and Podman 4.4 or later (5.0 for pods, 5.2 for builds) |

Podman has no daemon and no `apply` command. Its declarative form is Quadlet: you write a
small unit file for a container, and systemd turns it into a service that runs `podman`.
The containers then start at boot and restart on failure with nothing else running.

This plugin compiles a Meridian document into those unit files, installs them, and has
systemd start them. A plan is an exact comparison of the unit files on the host with the
ones your document compiles to.

## A first document

```yaml
apiVersion: meridian/v1
kind: Infrastructure
metadata:
  name: hello
spec:
  target: podman
  resources:
    - id: site
      type: volume

    - id: web
      type: container
      params:
        Image: docker.io/library/nginx:1.27
        PublishPort: "8080:80"
        Volume: ${site}:/usr/share/nginx/html:ro

  outputs:
    - name: url
      value: http://localhost:8080/
```

```
meridian compile hello.yaml --out build   # write site.volume and web.container; change nothing
meridian plan hello.yaml                  # what would be installed, changed, started or removed
meridian apply hello.yaml                 # install the units and start them
```

`meridian compile` writes this `web.container`:

```ini
# Written by Meridian from the document "hello". The next apply overwrites any change made here.

[Container]
Image=docker.io/library/nginx:1.27
PublishPort=8080:80
Volume=site.volume:/usr/share/nginx/html:ro

[Install]
WantedBy=default.target
```

## Resource types

Each resource becomes one unit file named after its `id`.

| `type` | Unit file | systemd service | Name in Podman |
|---|---|---|---|
| `container` | `<id>.container` | `<id>.service` | `systemd-<id>`, or `ContainerName` |
| `pod` | `<id>.pod` | `<id>-pod.service` | `systemd-<id>`, or `PodName` |
| `kube` | `<id>.kube` | `<id>.service` | set by the YAML it plays |
| `volume` | `<id>.volume` | `<id>-volume.service` | `systemd-<id>`, or `VolumeName` |
| `network` | `<id>.network` | `<id>-network.service` | `systemd-<id>`, or `NetworkName` |
| `image` | `<id>.image` | `<id>-image.service` | the `Image` it pulls |
| `build` | `<id>.build` | `<id>-build.service` | its `ImageTag` |

An `id` starts with a letter or a digit and holds only letters, digits, `_` and `-`,
because it is a file name and a systemd unit name.

## Params

`params` are Quadlet's own keys, spelled exactly as
[Podman documents them](https://docs.podman.io/en/latest/markdown/podman-systemd.unit.5.html):
`Image`, `PublishPort`, `Volume`, `Environment`, `Network`, `Pod`, `Exec`, `AutoUpdate`
and the rest. Meridian does not put a smaller vocabulary in front of them, so anything
Quadlet can do, a document can say.

How a value is written:

| In the document | In the unit file |
|---|---|
| `PublishPort: "8080:80"` | `PublishPort=8080:80` |
| `ReadOnly: true` | `ReadOnly=true` |
| `PublishPort: ["8080:80", "8443:443"]` | one `PublishPort=` line per item |
| `Environment: {TZ: UTC, NOTE: two words}` | `Environment=TZ=UTC` and `Environment="NOTE=two words"` |

The other sections of a unit file are four lower-case params, each a map:

```yaml
    - id: web
      type: container
      params:
        Image: docker.io/library/nginx:1.27
        unit:
          Description: The web front end
          After: network-online.target
        service:
          Restart: always
          TimeoutStartSec: 300
        install:
          WantedBy: multi-user.target
        quadlet:
          DefaultDependencies: false
```

Containers, pods and kube units get `[Install] WantedBy=default.target` unless the
document has its own `install`, so what an apply starts is started again after a reboot.
Write `install: {}` for a unit that should only run when something starts it.

### Literal values and systemd's expansions

systemd expands `%` specifiers (`%H` is the host name) and `${VAR}` in what Quadlet hands
it. Meridian keeps the two cases apart:

- A value written as a **string or a list** is systemd's to expand. `HostName: web-%H`
  works as systemd documents it.
- The values of a **map** mean exactly what they say. `Environment: {PASSWORD: "50%off$now"}`
  reaches the container as `50%off$now`; Meridian writes the escapes systemd needs.

`${...}` in a document is a Meridian reference (below). To hand `${HOME}` to systemd as
written, escape it as `$${HOME}`.

### Mistakes caught at compile time

- A key in the wrong capitalisation, such as `image`, is refused with the right spelling.
  Quadlet reads keys exactly, and would otherwise reject the unit where only the journal
  sees it.
- A key this plugin does not know is kept and warned about, since a newer Podman may have
  added it. If the host's Podman does not know it either, the plan refuses the unit.
- A container without `Image` or `Rootfs`, a `kube` without `Yaml`, an `image` without
  `Image`, and a `build` without `ImageTag` and a `File` are refused.
- A value with a line break, an empty value, and a resource type that is not in the table
  above are refused.

## Wiring resources together

| Reference | Becomes | Use it for |
|---|---|---|
| `${data}` | the unit file, `data.volume` | `Volume`, `Network`, `Pod`, `Image`: how Quadlet links units |
| `${data.name}` | the name Podman knows it by, `systemd-data` | environment variables, anything outside Quadlet |
| `${data.service}` | the systemd service, `data-volume.service` | the `unit` section |
| `${data.inspect.Mountpoint}` | a field of `podman volume inspect`, read after an apply | outputs only |

Quadlet orders a unit after every unit file it names, so `Volume: ${data}:/srv` needs no
`dependsOn`. An order the params do not show is written out:

```yaml
    - id: web
      type: container
      params:
        Image: docker.io/library/nginx:1.27
      dependsOn:
        - resource: db        # becomes Requires=db.service and After=db.service
```

`notifies` with the action `restart` becomes `PartOf=`: when the notifying unit changes
and is restarted, systemd restarts the notified one with it. `restart` is the only action
systemd can carry, so any other is refused.

```yaml
    - id: proxy-config
      type: volume
      notifies:
        - resource: proxy
          action: restart     # proxy.container gets PartOf=proxy-config-volume.service
```

## Outputs

An Infrastructure document's outputs can be plain text, the names above, or values read
from Podman once the thing exists:

```yaml
  outputs:
    - name: web_service
      value: ${web.service}
    - name: data_directory
      value: ${data.inspect.Mountpoint}
    - name: started
      value: ${web.inspect.State.StartedAt}
```

What follows `.inspect.` is a path of field names from `podman container inspect`,
`podman volume inspect` and so on. A `kube` unit cannot be inspected.

## What plan and apply do

`meridian plan`:

1. Compares each compiled unit file with the one on the host: new, changed or the same.
2. Asks systemd whether each service is running.
3. Has the host's own Quadlet read the compiled units, in a temporary directory. A unit it
   refuses, or a kind of unit the host's Podman is too old for, fails the plan with what
   Quadlet said. Nothing has been touched at that point.
4. Lists unit files a previous apply installed that the document no longer has.

It changes nothing and needs no more rights than you have, even for system units.

| Plan line | Meaning |
|---|---|
| `create web: install web.container and start web.service` | the unit is not on the host |
| `update web: web.container changed; restart web.service` | the file differs |
| `update web: web.service is inactive; start it` | the file matches but the service is not running |
| `delete web: stop web.service and remove web.container` | the document no longer has the resource |

`meridian apply` makes the same checks, then stops and removes what was deleted, writes
what is new or changed, runs `systemctl daemon-reload`, restarts the changed units and
starts all of them. If a unit does not start, the apply fails and shows the end of that
unit's journal.

Only units Meridian installed are ever removed. Other unit files in the same directory are
left alone.

## Where it runs

- An **Infrastructure** document runs on the machine running Meridian.
- A **ResourceSet** runs on its `targetHost`, reached over `ssh`. The host needs Podman and
  nothing else. The host can come from an earlier stage, so one workflow can create a
  machine with Terraform and then run containers on it:

```yaml
apiVersion: meridian/v1
kind: ResourceSet
metadata:
  name: edge-proxy
spec:
  target: podman
  targetHost:
    from: infrastructure
    ref: edge-vm
    output: public_ip
  resources:
    - id: proxy
      type: container
      params:
        Image: docker.io/library/caddy:2.9
        PublishPort: ["80:80", "443:443"]
```

## Settings

The runner is configured through the environment of the `meridian` command.

| Variable | Default | Meaning |
|---|---|---|
| `MERIDIAN_PODMAN_SCOPE` | `user` | `user` installs rootless units in `~/.config/containers/systemd` and uses `systemctl --user`. `system` installs them in `/etc/containers/systemd`, using `sudo -n` for the changes when you are not root. |
| `MERIDIAN_PODMAN_SSH_ARGS` | none | Extra arguments for every `ssh`, such as `-l deploy -i ~/.ssh/deploy`. |
| `MERIDIAN_PODMAN_SSH` | `ssh` | The `ssh` program. |
| `MERIDIAN_PODMAN` | `podman` | The `podman` program on the host. |
| `MERIDIAN_PODMAN_SYSTEMCTL` | `systemctl` | The `systemctl` program on the host. |
| `MERIDIAN_PODMAN_QUADLET` | found automatically | Quadlet's generator on the host, when it is not where distributions install it. |
| `MERIDIAN_PODMAN_UNIT_DIR` | by scope | The directory unit files are installed in. |

For rootless units to keep running after you log out, enable lingering for the user once:
`loginctl enable-linger <user>`.

## What this target cannot express

Meridian refuses these at compile time instead of emitting something that does less.

| In the document | Why it is refused | What to do |
|---|---|---|
| `state: absent` | A unit that is not in the document is removed by the next apply. | Remove the resource. |
| `runtimeWhen` | A unit's start conditions are systemd's fixed `Condition*=` tests, not an expression. | Use `unit: {ConditionPathExists: ...}` or another `Condition*=`. |
| `notifies` with an action other than `restart` | systemd can only propagate a restart. | Use `restart`. |
| `${x.inspect...}` in a param | It is only known after the thing exists; a unit file is written first. | Use it in an output. |

## Things to know

- **Volumes and networks are created once.** Quadlet creates them if they are missing and
  does not alter one that exists. Changing a volume's or a network's params rewrites the
  unit; to apply the change to the real thing, remove it with `podman volume rm` or
  `podman network rm` first.
- **Removing a volume from the document removes its unit, not its data.** Delete the data
  with `podman volume rm` when you mean to.
- **A container that exits is started again by the next apply**, because its service is
  no longer active. For a job that is meant to finish, set
  `service: {Type: oneshot, RemainAfterExit: yes}`.
- **Pods need Podman 5.0, builds 5.2, image units 4.8.** On an older host the plan says so
  by name.

## Examples

- [`examples/podman/web-app.yaml`](../../examples/podman/web-app.yaml): a web server and a
  database on a private network with a volume, and three outputs.
- [`examples/podman/pod.yaml`](../../examples/podman/pod.yaml): two containers in one pod,
  with a pre-pulled image.
- [`examples/podman/edge-host.yaml`](../../examples/podman/edge-host.yaml): a ResourceSet
  for another machine, with a bind-mounted volume that restarts the container when it
  changes.

## Tests

```
cargo test -p meridian-target-podman
```

runs the compiler against golden files, and the runner against stand-ins for `podman`,
`systemctl`, Quadlet and `ssh`, so it needs nothing installed. Two further levels use the
real tools:

```
MERIDIAN_TEST_PODMAN=quadlet cargo test -p meridian-target-podman --test real
MERIDIAN_TEST_PODMAN=system  cargo test -p meridian-target-podman --test real
```

`quadlet` has the machine's own Quadlet read every example and installs nothing. `system`
also installs, starts, changes and removes real units in `/etc/containers/systemd`; it
needs systemd and root or passwordless `sudo`, and CI runs it on every pull request.
