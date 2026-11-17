# Worked workflows

A workflow is several documents given to one `meridian` command. This page walks
through six of them, each a file under
[`examples/workflow/`](../../examples/workflow). Between them they use every
target Meridian ships, and each shows a different way of fitting tools together.

Every file on this page is compiled by a test with the real plugins, in each of
the ways the page describes, so what you read here is what the command does.

| Workflow | Tools, in the order they run | Shows |
|---|---|---|
| [Three-tier application](#three-tier-application) | OpenTofu, Salt, Ansible, Docker | several machines from one provisioning stage; labels to run part of a workflow |
| [Cluster platform](#cluster-platform) | Terraform, Helm, Kubernetes | three Infrastructure documents in a fixed order |
| [Golden image](#golden-image) | Packer, CloudFormation, cloud-init, Puppet | build once, launch, bootstrap, then keep converged |
| [Windows web server](#windows-web-server) | ARM, Bicep, DSC | two deployment scopes, then configuration over WinRM |
| [Edge site](#edge-site) | Pulumi, Chef, OpenVox, Podman | two machines with different roles; containers under systemd |
| [Environments](#environments) | Terraform, Ansible, Kubernetes | one workflow for every environment, decided by `--data` |

If you have not yet run a workflow of two documents, start with
[a two-stage workflow](first-two-stage.md). The rules every workflow follows are
in [the language reference](../reference/dsl/index.md#workflows-several-documents);
the three that matter most here:

- Every Infrastructure document runs before any ResourceSet. Within a kind,
  documents run in the order they are given.
- A ResourceSet names its machine with `targetHost`. When that is an output of an
  Infrastructure document, the ResourceSet is **deferred** until the output
  exists: `compile` and `plan` say so, and compile nothing for it against a guess.
- When a stage fails, the stages after it do not run. Nothing is rolled back.

## Three-tier application

[`examples/workflow/three-tier.yaml`](../../examples/workflow/three-tier.yaml)

```mermaid
flowchart LR
    N["network<br/>OpenTofu<br/>VPC, two machines"]
    N -->|db_address| D["database<br/>Salt<br/>PostgreSQL"]
    N -->|web_address| W["web_host<br/>Ansible<br/>Docker, deploy user"]
    N -->|web_address| S["shop<br/>Docker Compose<br/>proxy and application"]
    W -. "listed first, so it runs first" .-> S
```

One Infrastructure document creates two machines and promises three outputs.
Three ResourceSets each take a host from one of them. No address is written
anywhere in the file.

```
$ meridian compile examples/workflow/three-tier.yaml --out out
network (Infrastructure, target opentofu)
  compiled 1 file into out/network
database (ResourceSet, target salt)
  deferred: needs the output "db_address" of Infrastructure "network", which has not been applied yet
web_host (ResourceSet, target ansible)
  deferred: needs the output "web_address" of Infrastructure "network", which has not been applied yet
shop (ResourceSet, target docker)
  deferred: needs the output "web_address" of Infrastructure "network", which has not been applied yet
```

`meridian apply` runs OpenTofu, reads the outputs, and then compiles and runs
each ResourceSet against the address it was waiting for.

Things to notice in the file:

- **Two ResourceSets, one machine.** `web_host` (Ansible) and `shop` (Docker)
  both read `web_address`. Ansible installs Docker and Compose then runs the
  containers. `shop` is listed after `web_host`, which is the whole of what
  orders them: ResourceSets run in the order given.
- **The same six types on two tools.** `database` compiles for Salt and
  `web_host` for Ansible. Both use `package`, `file`, `service`, `user`,
  `group` and `exec`, and either could be pointed at the other tool by changing
  `target`.
- **A reference inside a file's content.** `data_directory = '${data_dir.path}'`
  is settled when the document is compiled: the path is written once.
- **Labels.** Every document carries `stage` and, where it applies, `tier`:

  ```
  meridian apply examples/workflow/three-tier.yaml -l tier=db
  meridian apply examples/workflow/three-tier.yaml -l 'stage!=provision'
  ```

  The first runs only the database. The second leaves OpenTofu alone and
  configures the machines it created the last time it was applied; a selected
  ResourceSet finds its host from that document's last apply.

## Cluster platform

[`examples/workflow/cluster-platform.yaml`](../../examples/workflow/cluster-platform.yaml)

```mermaid
flowchart LR
    C["cluster<br/>Terraform<br/>network, IAM, EKS"] --> P["platform<br/>Helm<br/>quota, shared config"] --> A["shop<br/>Kubernetes<br/>ConfigMap, Deployment, Service"]
```

All three documents are `kind: Infrastructure`, so they run in the order the
file gives them: the cluster, then what every team's workloads share, then one
application. Nothing here is deferred, and `meridian compile` writes all three.

- **`dependsOn` where the tool cannot see the order.** The cluster's IAM role
  must be allowed to manage a cluster before the cluster is created. No param
  of the cluster mentions the policy attachment, so the document says it.
- **A chart built from the document.** In `platform`, `values`, `chart` and
  `release` are settings, and `${values.cpuLimit}` becomes a read of
  `.Values.cpuLimit` in the chart's templates, so the value can still be
  overridden when the chart is installed.
- **A notify the tool does not have.** Kubernetes does not roll a Deployment
  when a ConfigMap it mounts changes. `notifies` on `shop_config` makes it, and
  the compile says how:

  ```
  warning: kubernetes: shop_app: restarts by rolling its pods whenever the content of "shop_config" changes; ...
  ```

The second and third documents reach the cluster through a kubeconfig context.
An output of one Infrastructure document is not handed to another: outputs go
from an Infrastructure document to a ResourceSet's `targetHost`.

## Golden image

[`examples/workflow/golden-image.yaml`](../../examples/workflow/golden-image.yaml)

```mermaid
flowchart LR
    I["web-image<br/>Packer<br/>build the image"] --> F["fleet<br/>CloudFormation<br/>launch a machine"]
    F -->|address| B["first_boot<br/>cloud-init<br/>users, keys, motd"]
    F -->|address| P["baseline<br/>Puppet<br/>site, service"]
```

Four tools, each doing the part it is best at. Packer bakes what is slow to
install into an image. CloudFormation launches from it. cloud-init does what
must happen once, on the first boot. Puppet keeps the configuration converged
on every run after that.

- **Two configuration tools, two jobs.** `first_boot` and `baseline` are both
  ResourceSets on the same machine, and both use the shared types. cloud-init
  takes params the others do not (`ssh_authorized_keys` on a `user`), and
  refuses what a first boot cannot do, such as removing a package. The
  [conformance matrix](../reference/conformance-matrix.md) has every difference.
- **An exec that runs once.** `creates:` makes `ssh-keygen -A` a no-op once the
  host key exists, so a plan does not show it as a change every time.
- **What is not connected.** The image id Packer reports is an output, and the
  CloudFormation document's `ImageId` is written by hand. An output cannot yet
  fill a param of another Infrastructure document; the example says so where
  the id is written.

## Windows web server

[`examples/workflow/windows-web.yaml`](../../examples/workflow/windows-web.yaml)

```mermaid
flowchart LR
    L["landing-zone<br/>ARM template<br/>resource groups"] --> V["web-vm<br/>Bicep<br/>network, address"]
    V -->|address| D["iis<br/>DSC<br/>features, site, service"]
```

- **Two documents for two scopes.** A template is deployed to one scope. The
  resource groups are created at subscription scope by the ARM document; the
  network inside one of them is a separate deployment, written in Bicep. Both
  compile to what Azure Resource Manager deploys.
- **The tool's own expressions pass through.** `${resourceGroup().location}` and
  `${uniqueString(resourceGroup().id)}` are Bicep's functions, not Meridian's.
  A `${...}` that is not a reference to a resource of the document is handed to
  the tool as written.
- **DSC resources are DSC's own.** `PSDscResources/WindowsFeature` is the
  module and the resource, and `Ensure: Present` is the resource's own
  property. The `runner` resource is settings: which engine, how the host is
  reached, and the name of a credential the runner resolves itself. No secret
  is in the file.

## Edge site

[`examples/workflow/edge-fleet.yaml`](../../examples/workflow/edge-fleet.yaml)

```mermaid
flowchart LR
    E["edge-site<br/>Pulumi<br/>gateway and worker"]
    E -->|gateway_address| G["gateway<br/>Chef<br/>HAProxy"]
    E -->|worker_address| W["worker<br/>OpenVox<br/>Podman, apps user"]
    E -->|worker_address| A["edge-apps<br/>Podman<br/>two containers as units"]
    W -.-> A
```

- **Roles as labels.** `role=gateway` and `role=worker` select the documents
  for one machine: `meridian apply examples/workflow/edge-fleet.yaml -l role=worker`
  runs OpenVox and then Podman, and leaves the gateway alone.
- **A reload, not a restart.** The HAProxy configuration notifies its service
  with `action: reload`. Chef can do that. Had this document targeted Puppet it
  would have been refused, because Puppet refreshes a service by restarting it.
- **Containers as systemd units.** The Podman document compiles to Quadlet
  files (`api.container`, `edge-net.network`, ...). `dependsOn` becomes the
  unit's own ordering, and `${collector.name}` is the name the container is
  reachable by.

## Environments

[`examples/workflow/environments.yaml`](../../examples/workflow/environments.yaml),
with [`environments.data.yaml`](../../examples/workflow/environments.data.yaml) and
[`environments.prod.data.yaml`](../../examples/workflow/environments.prod.data.yaml)

```mermaid
flowchart TD
    D["--data environments.data.yaml<br/>--data environments.prod.data.yaml"] --> M
    subgraph M["Meridian decides every when, once"]
        direction LR
        T["machines<br/>Terraform"]
        A["site<br/>Ansible"]
        K["jobs<br/>Kubernetes"]
    end
    M --> O["compiled files with no condition in them"]
```

One file describes development and production. What differs between them is
decided by [`when`](../reference/dsl/conditions.md#when), from data given on the
command line:

```
meridian compile examples/workflow/environments.yaml \
    --data examples/workflow/environments.data.yaml

meridian compile examples/workflow/environments.yaml \
    --data examples/workflow/environments.data.yaml \
    --data examples/workflow/environments.prod.data.yaml
```

The first is development. The second layers production over it: a key in the
later file replaces the same key in the earlier one, whole.

| Resource | Document | Condition | Development | Production |
|---|---|---|---|---|
| `bastion` | `machines` (Terraform) | `data.env != 'prod'` | kept | removed |
| `web_ip` | `machines` (Terraform) | `data.env == 'prod'` | removed | kept |
| `audit_bucket` | `machines` (Terraform) | `defined(data.audit) && data.audit.enabled` | removed | kept |
| `debug_log` | `site` (Ansible) | `data.env != 'prod'` | kept | removed |
| `tls_conf` | `site` (Ansible) | `data.features.tls && defined(data.certificate)` | removed | kept |
| `budget` | `jobs` (Kubernetes) | `data.replicas == 3 \|\| data.high_availability` | removed | kept |
| `load_test` | `jobs` (Kubernetes) | `data.env != 'prod' && (data.load_test == true)` | kept | removed |

What the example is there to show:

- **A condition means the same on every target.** `data.env != 'prod'` is
  written in a Terraform document, an Ansible one and a Kubernetes one. Meridian
  decides it itself, before compiling, so no tool sees it and no tool can read it
  differently. A removed resource is not in the compiled files at all.
- **`ifMissing: skip`.** `ready` depends on `bastion`, which production removes.
  Without `ifMissing: skip` on that entry the production compile is refused,
  because a resource depending on something that is not there is far more often
  a mistake than an intention.
- **Guarding an optional key.** `defined(data.audit) && data.audit.enabled`
  reads `audit.enabled` only when `audit` is set. Development's data has no
  `audit` key and still compiles.
- **No data, no answer.** With no `--data`, the compile is refused:
  `data.env is not set (give it with --data, or test for it with defined(data.env))`.
  A condition is never taken as false because its key is missing.
- **`runtimeWhen` is the other kind.** `apt_cache` in the Ansible document
  carries `runtimeWhen`, which Meridian does not decide: it is copied into the
  task's `when:` and Ansible checks it on the machine.

## Using these as a starting point

Copy a file, change the names, and check it as you go:

```
meridian validate my-workflow.yaml          # the shape of the language, no plugins
meridian compile  my-workflow.yaml --out out # what each tool will be given
meridian plan     my-workflow.yaml          # what each tool would change
```

`validate` needs no plugin and no tool. `compile` needs the plugins. `plan` and
`apply` need the tools themselves; [install](install.md) lists what each target
uses.

A workflow that takes data keeps it beside it: `site.yaml` with
`site.data.yaml`, and one `site.<name>.data.yaml` for each variant. That is the
convention the examples test follows, compiling the workflow once with the first
file and once more for each variant layered over it.
