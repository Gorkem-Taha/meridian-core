# Target: `docker`

<img src="../branding/logos/meridian-docker.svg" width="72" alt="Meridian Docker plugin logo">

Compiles an Infrastructure document into a Compose file and brings it up with
`docker compose`.

## What it does and does not support

| | |
|---|---|
| Resource types | the things a Compose file declares: `service`, `network`, `volume`, `config`, `secret`, plus `project` for where it runs |
| `dependsOn` | Between two services, Compose's own `depends_on`. Anything else Compose already creates before it starts a service |
| `notifies` | The action `restart`, aimed at a service. Native from a service, synthetic from anything else. See [Notifications](#notifications) |
| `runtimeWhen` | No. Refused |
| `state: absent` | No. Remove the resource from the document; bringing the project up removes services the document no longer declares |
| Plan | Yes: `docker compose --dry-run up` |
| Outputs | Yes: a published port, or a path into `docker inspect` |

## What a document looks like

A resource's `type` is the section of the Compose file it belongs to and its
`params` are that element's fields, exactly as the Compose specification
documents them. The resource's id is the element's name in the file. A service
needs an `image` or a `build`.

One resource may have the type `project`. It is not part of the Compose file:

| Param | Meaning |
|---|---|
| `name` | the Compose project name; the document's name when left out |
| `host` | the Docker daemon to use. A bare host such as `deploy@web01` is reached over SSH; an address such as `tcp://10.0.0.5:2376` is used as written |
| `context` | a Docker context to use instead of a host |

A document that names a target host runs on that host's daemon over SSH.

## References

Inside `params`, `${id}` is the name of another element of the document, checked
at compile time. Nothing else can be read there, because nothing else is known
until the project is up.

A `$` in a param is written to the Compose file as `$$`, so Compose does not
take it for a variable of its own. Values belong in the document, not in the
shell environment of whoever runs it.

## Outputs

| You write | You get |
|---|---|
| `${proxy.port.80}` | the host port that the service's port 80 is published on |
| `${db.State.Status}`, `${db.NetworkSettings.Networks.web_backend.IPAddress}` | that path in `docker inspect` of the service's container |

An output with nothing behind it, such as a port the service does not publish,
is an error and not an empty value.

## Notifications

A service notifying a service is Compose's own mechanism: the notified service
gets `depends_on` with `restart: true`, and Compose restarts it after the other
is updated.

A config, secret, network or volume notifying a service is synthetic. Compose
does not restart a service because a config it mounts changed. Meridian writes a
checksum of the notifier into a label on the service,
`meridian.dev/restart-on-<id>`. When the notifier's content changes the label
changes, and Compose recreates the container. The compile warns that this is how
it works, and `docker inspect` will show the label.

## Example

[`examples/docker/web.yaml`](../../examples/docker/web.yaml) declares a database,
an application and a proxy on one network, and compiles to
[`web.compose_yaml`](../../plugins/docker/testdata/web.compose_yaml). The proxy:

```yaml
- id: site
  type: config
  params:
    content: |
      server { ... }
  notifies:
    - resource: proxy
      action: restart

- id: proxy
  type: service
  params:
    image: nginx:1.27
    ports:
      - "80"
    configs:
      - source: ${site}
        target: /etc/nginx/conf.d/default.conf
    networks:
      - ${backend}
  dependsOn:
    - resource: app
```

```yaml
proxy:
  image: nginx:1.27
  depends_on:
    app:
      condition: service_started
  configs:
    - source: site
      target: /etc/nginx/conf.d/default.conf
  labels:
    meridian.dev/restart-on-site: 185ed24724f46635
  networks:
    - backend
  ports:
    - "80"
```

## Running

| | |
|---|---|
| Plan | `docker compose --project-name <name> --file compose.yaml --dry-run up --detach --remove-orphans` |
| Apply | the same without `--dry-run`, then the outputs are read with `docker compose port`, `docker compose ps` and `docker inspect` |
| `MERIDIAN_DOCKER` | the program to run, when it is not `docker` on `PATH` |
| `MERIDIAN_DOCKER_ARGS` | arguments added to `docker compose up`: `--wait`, `--build`, `--pull always` |

The compiled Compose file for the example passes real `docker compose config`.
The runner has been tested against a stand-in for `docker`; it has not yet been
run against a real daemon.
