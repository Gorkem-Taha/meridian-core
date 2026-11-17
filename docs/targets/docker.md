# Target: `docker`

Compiles a document into a Docker Compose file and brings it up with
`docker compose`. An Infrastructure document runs on the local Docker daemon,
or the one its `project` names. A ResourceSet runs on its target host's daemon.

## What it does and does not support

| | |
|---|---|
| Resource types | the things a Compose file declares: `service`, `network`, `volume`, `config`, `secret`, plus `project` for where it runs |
| `dependsOn` | Between two services, Compose's own `depends_on`, waiting for the other service to be healthy when it declares a healthcheck. Anything else Compose already creates before it starts a service |
| `notifies` | The action `restart`, aimed at a service. Native from a service, synthetic from anything else. See [Notifications](#notifications) |
| `runtimeWhen` | No. Refused |
| `state: absent` | No. Remove the resource from the document; bringing the project up removes services the document no longer declares |
| Plan | Yes: the file is compared with what Docker is running. See [Running](#running) |
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
| `directory` | an absolute path that Compose resolves the file's relative paths against. See [Relative paths](#relative-paths) |
| `profiles` | a list of Compose profiles to bring up with the project |
| `wait` | whether an apply waits for every service to be running, or healthy where it declares a healthcheck. `true` unless set |
| `wait_timeout` | how many seconds to wait before the apply fails; Compose's own limit when left out |

A document that names a target host runs on that host's daemon over SSH.

## What is checked at compile time

Compose would refuse each of these itself, but only when the project is planned
or brought up, and in terms of the compiled file. Meridian refuses them when the
document compiles, against the line you wrote.

| The document | Is refused with |
|---|---|
| uses an attribute Compose does not know, such as `enviroment` | `unsupported-param`, naming the attribute that was probably meant. An extension field, which starts with `x-`, is yours to name |
| has a service that uses a network, named volume, config or secret it does not declare | `invalid-reference`. The network `default` needs no declaration, and neither does a bind mount or an anonymous volume |
| has a `depends_on` naming something that is not a service | `invalid-reference` |
| waits for `service_healthy` on a service with no healthcheck | `invalid-param`, because Compose would wait for ever |
| has services that wait for each other in a circle | `dependency-cycle`. A `restart` notify counts, because the notified service starts after the one that notifies it |

```yaml
- id: db
  type: service
  params:
    image: postgres:17
    volumes: ["dbdata:/var/lib/postgresql/data"]   # the document declares db_data
```

```
stack.yaml:11:7: docker: resource "db" [invalid-reference]: uses the volume "dbdata", which the document does not declare. Add a resource of type "volume" with that id
```

## Dependencies

`dependsOn` between two services writes `depends_on` in Compose's long form. The
condition is chosen from the service being waited for:

| The other service | Condition written |
|---|---|
| declares a `healthcheck` | `service_healthy` |
| declares none, or disables it | `service_started` |

```yaml
- id: db
  type: service
  params:
    image: postgres:17
    healthcheck:
      test: [CMD-SHELL, pg_isready -U postgres]
      interval: 5s

- id: app
  type: service
  params:
    image: ghcr.io/example/app:1.4.2
  dependsOn:
    - resource: db
```

```yaml
app:
  image: ghcr.io/example/app:1.4.2
  depends_on:
    db:
      condition: service_healthy
```

For anything else, write `depends_on` in the service's `params` as Compose
documents it. What you write is kept, and a `dependsOn` on the same service adds
nothing to it. This is how a service waits for a job that runs once:

```yaml
- id: migrate
  type: service
  params:
    image: ghcr.io/example/app:1.4.2
    command: [migrate]
    restart: "no"

- id: app
  type: service
  params:
    image: ghcr.io/example/app:1.4.2
    depends_on:
      migrate:
        condition: service_completed_successfully
```

## Relative paths

Compose resolves a relative host path against the project directory, which is
the directory Meridian keeps the compiled file in, under its state directory.
Nothing you keep beside the document is there. A build context, an `env_file`, a
bind mount, or the `file` of a config or secret that is a relative path compiles
with a warning for each:

```
docker: web: the build context "./web" is a relative path, which Compose resolves against the directory Meridian keeps the compiled file in and not the document's. Write an absolute path, or name the directory with "directory" on a resource of type "project"
```

Write absolute paths, or say where the relative ones start:

```yaml
- id: target
  type: project
  params:
    directory: /srv/shop

- id: web
  type: service
  params:
    build: ./web                      # /srv/shop/web
    env_file: web.env                 # /srv/shop/web.env
    volumes: ["./public:/usr/share/nginx/html:ro"]
```

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

## Examples

[`examples/docker/monitoring.yaml`](../../examples/docker/monitoring.yaml) is a
ResourceSet: Prometheus, a node exporter and Grafana on the host the document
names, with Grafana waiting for Prometheus to be healthy. It compiles to
[`monitoring.compose_yaml`](../../plugins/docker/testdata/monitoring.compose_yaml).

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

A plan changes nothing. It does not parse what `docker compose up --dry-run`
prints. It asks for data and compares it the way Compose itself does: Compose
stamps every container it creates with a hash of its service's configuration,
and recreates the container when the hash of the file's service differs.

| A plan runs | To learn |
|---|---|
| `docker compose config --format json` | what the file asks for, as Compose reads it. A file Compose rejects fails here, with what Compose said |
| `docker compose config --hash '*'` | the configuration hash of each service in the file |
| `docker ps --all`, `docker network ls`, `docker volume ls`, filtered to the project | what exists now, and the hash each container was created from |

| The plan reports | When |
|---|---|
| `create` | a service has no container, or a network or volume of the file does not exist |
| `replace` | a service's configuration hash differs from its container's, so the container is recreated. What was in the container, outside its volumes, is lost |
| `update` | a container is not running and would be started, or the number of containers differs from the service's `scale` or `deploy.replicas` |
| `delete` | containers belong to a service the document no longer declares, or a service is scaled to none |

A plan with no changes means the project is as the document describes. The one
exception is a service built from source (`build`): whether building again would
give a different image cannot be known without building it, so the plan says it
cannot be sure.

An apply makes the same comparison, so it reports what the plan would have, and
then runs `docker compose up --detach --remove-orphans`, with:

| Flag | When |
|---|---|
| `--wait` | unless the project sets `wait: false`. The apply succeeds once every service is running, or healthy where it declares a healthcheck, and fails if one does not get there |
| `--wait-timeout <seconds>` | when the project sets `wait_timeout` |
| `--build` | when a service is built from source, so a change to the source is applied and not only a change to the file |
| `--project-directory`, `--profile` | when the project sets `directory` or `profiles`; these go on every call |

Set `wait: false` for a project whose services are meant to exit, such as a job
that runs once with nothing depending on it: Compose's wait treats a service
that has stopped as one that failed to stay up.

The outputs are then read with `docker compose port`, `docker compose ps` and
`docker inspect`.

| Variable | Meaning |
|---|---|
| `MERIDIAN_DOCKER` | the program to run, when it is not `docker` on `PATH` |
| `MERIDIAN_DOCKER_ARGS` | arguments added to `docker compose up`: `--pull always`, `--force-recreate` |

## How it is tested

Beside the compile and runner tests, the plugin is driven as a real process over
the plugin protocol, and end to end against a real Docker daemon: a project is
brought up, its outputs are read and the page it serves is fetched, a service is
changed and another removed, and a container stopped behind Compose's back is
found by the next plan. Those tests run wherever `docker compose` and a daemon
are available, which includes CI, and are skipped with a note where they are
not. `MERIDIAN_REAL_DOCKER=1` makes a missing Docker a failure, and
`MERIDIAN_REAL_DOCKER=0` skips them.
