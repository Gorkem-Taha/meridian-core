# The containerization flavour

Containerization tools run containers, and what they need around them, on one
host. This page is how the [Meridian language](index.md) is written for them.

| | |
|---|---|
| Category, as `meridian plugins` prints it | `containerization` |
| Kind of document | `Infrastructure`. A `ResourceSet` is also taken: the project then runs on the daemon of the host its `targetHost` names |
| `type` and `params` | the tool's own, passed through |
| Outputs | yes |

## Targets

| Target | `type` is | Settings resource | Page |
|---|---|---|---|
| `docker` | an element of a Compose file: `service`, `network`, `volume`, `config`, `secret` | `project` (optional) | [docker](../../targets/docker.md) |
| `podman` | a kind of Quadlet unit: `container`, `pod`, `kube`, `volume`, `network`, `image`, `build` | none; the runner is set through the environment | [podman](../../targets/podman.md) |

## Two tools, one shape

The two targets differ in what runs the containers, and the flavour is the same
for both: a resource is one element, its `params` are that element's own fields
in the tool's own spelling, and elements find each other by `id`.

| | `docker` | `podman` |
|---|---|---|
| Compiles to | one Compose file | one Quadlet unit file per resource |
| Run by | `docker compose`, talking to a daemon | systemd, with no daemon |
| Params are spelt | as the Compose specification has them: `image`, `ports` | as Quadlet has them: `Image`, `PublishPort` |
| `dependsOn` | `depends_on` between services | `Requires=` and `After=` |
| `notifies` | `restart`, native or synthetic | `restart`, native: systemd's `PartOf=` |
| Another machine | the `project` resource names a daemon | a ResourceSet's `targetHost`, over `ssh` |

The rest of this page uses Docker for its examples. Podman's own page has the
same sections for Quadlet.

## A document is a project

A document in this flavour is one project: a set of containers with their
networks, volumes and configuration, brought up and taken down together. The
document's name is the project's name unless the settings resource gives
another.

A resource's `type` is the kind of element and its `params` are that element's
fields, exactly as the tool's own format documents them. The resource's `id` is
the element's name in the compiled file, so it is also how containers find each
other.

```yaml
apiVersion: meridian/v1
kind: Infrastructure
metadata:
  name: web
spec:
  target: docker
  resources:
    - id: backend
      type: network

    - id: db_data
      type: volume

    - id: db
      type: service
      params:
        image: postgres:16
        networks: ["${backend}"]
        volumes: ["${db_data}:/var/lib/postgresql/data"]

    - id: app
      type: service
      params:
        image: ghcr.io/example/app:1.4
        networks: ["${backend}"]
        ports: ["8080:8080"]
      dependsOn:
        - resource: db

  outputs:
    - name: app_port
      value: ${app.port.8080}
```

This document is
[`examples/flavours/containerization.yaml`](../../../examples/flavours/containerization.yaml).
A longer one: [`examples/docker/web.yaml`](../../../examples/docker/web.yaml).

## The settings resource

One optional resource of type `project` says where the project runs: its name,
and the daemon to use (a host reached over SSH, an address, or a Docker
context). It is not part of the compiled file.

## `state`

Left out. Bringing a project up removes what the document no longer declares, so
`state: absent` is refused with the instruction to remove the resource.

## References

Inside `params`, `${id}` is the name of another element of the document, and
nothing more can be read there: nothing else is known until the project is up.
The reference is checked when the document is compiled.

In an output, a reference reads the running project:

| You write | You get |
|---|---|
| `${app.port.8080}` | the host port the service's port 8080 is published on |
| `${db.State.Status}` | that path in the inspection of the service's container |

A `$` written in a param is escaped in the compiled file, so the tool does not
take it for a variable of its own. Values belong in the document, not in the
environment of whoever runs it.

## Order

Between two services, `dependsOn` becomes the tool's own `depends_on`. Networks,
volumes, configs and secrets are created before any service starts whatever the
document says, so a dependency on one of those adds nothing.

## `notifies`

One action, `restart`, aimed at a service.

| Notifier | How |
|---|---|
| a service | native: `depends_on` with `restart: true` |
| a config, secret, network or volume | synthetic: a checksum of the notifier in a label on the service, `meridian.dev/restart-on-<id>`. The content changes, the label changes, the container is recreated. The compile warns |

## `runtimeWhen`

Refused. There is nothing to check a condition against while a project comes up.

## Plan and apply

`meridian plan` is the tool's own dry run, and `meridian apply` brings the
project up and then reads the outputs. The exact commands are on the target's
page.

## How this differs from container orchestration

A containerization document describes one host and one project, and the tool
does the work itself, in order, while Meridian waits. A
[container orchestration](container-orchestration.md) document hands objects to
a cluster, which converges them on its own schedule.
