# The configuration management flavour

Configuration management tools log in to a machine, or run on it, and converge
packages, files and services. This page is how the
[Meridian language](index.md) is written for them.

| | |
|---|---|
| Category, as `meridian plugins` prints it | `configuration-management` |
| Kind of document | `ResourceSet` |
| The machine | `spec.targetHost`, required |
| Outputs | none: a ResourceSet has no `outputs` |

## Targets

| Target | Types | Page |
|---|---|---|
| `ansible` | the six shared types | [ansible](../../targets/ansible.md) |
| `puppet` | the six shared types | [puppet](../../targets/puppet.md) |
| `chef` | the six shared types | [chef](../../targets/chef.md) |
| `dsc` | DSC's own resources | [dsc](../../targets/dsc.md) |

There are two dialects in this segment, and the table shows which target speaks
which.

## The shared dialect: six types

`ansible`, `puppet` and `chef` take the same six resource types. A resource
written with them compiles for any of the three, so changing `spec.target` is
all it takes to move a document from one tool to another, within what each tool
can do. This is the one place where the language is "write once".

| `type` | Manages | `state` | Required params |
|---|---|---|---|
| `package` | an installed package | `present` (default), `absent`, `latest` | `name` |
| `service` | a running service | `running` (default), `started`, `stopped` | `name` |
| `file` | a file or directory | `present` (default) or `file`, `directory`, `absent` | `path` |
| `user` | a user account | `present` (default), `absent` | `name` |
| `group` | a group | `present` (default), `absent` | `name` |
| `exec` | a command to run | none | `command` |

Every param, every state and which actions each type can be notified with are
in [the resource types reference](../resource-types.md).

```yaml
apiVersion: meridian/v1
kind: ResourceSet
metadata:
  name: web
spec:
  target: ansible
  targetHost:
    from: inline
    host: web01.example.com
  resources:
    - id: nginx_pkg
      type: package
      params:
        name: nginx

    - id: nginx_conf
      type: file
      params:
        path: /etc/nginx/nginx.conf
        content: "worker_processes 4;\n"
        mode: "0644"
      dependsOn:
        - resource: nginx_pkg
      notifies:
        - resource: nginx_svc
          action: restart

    - id: nginx_svc
      type: service
      state: running
      params:
        name: nginx
        enabled: true
      dependsOn:
        - resource: nginx_pkg
```

This document is
[`examples/flavours/configuration-management.yaml`](../../../examples/flavours/configuration-management.yaml),
and it compiles for all three tools. A longer one for each tool is under
[`examples/ansible/`](../../../examples/ansible),
[`examples/puppet/`](../../../examples/puppet) and
[`examples/chef/`](../../../examples/chef).

Anything outside the six types and their params is refused when the document is
compiled: an unknown type, a param the type does not have, a state the tool
cannot express. Nothing is dropped.

In this dialect a reference is settled when the document is compiled, because
there is nothing to read from a package or a file at apply time: `${id.param}`
is the value the document gives that param on that resource. Write `$${` for a
literal `${`, as in a shell script held in a file's `content`. See
[references](references.md#configuration-management-the-shared-types).

## The native dialect: DSC

`dsc` does not take the six types. DSC's vocabulary is its resources, and there
are too many, with too many properties each, to hide behind six words.

| | |
|---|---|
| `type` | the DSC resource, `Module/Resource`: `PSDscResources/Service`, `Microsoft.Windows/Registry` |
| `params` | the resource's properties, exactly as the resource documents them |
| `state` | not used, and refused. A DSC resource carries its desired state in its own properties (`Ensure`, `_exist`) |
| Settings resource | one optional resource of type `runner`: the engine (DSC v2 or v3), and how the host is reached |
| References | `${id.Property}` reads a property another resource reported; the reader is ordered after it |

See [the `dsc` target](../../targets/dsc.md) and
[`examples/dsc/web-server.yaml`](../../../examples/dsc/web-server.yaml).

A document written in one dialect does not compile for a target of the other.

## Order

`dependsOn` works on every target in the segment. What it turns into depends on
how the tool thinks about order:

| Target | `dependsOn` becomes |
|---|---|
| `ansible`, `chef` | the order of the tasks or recipe. These tools run top to bottom, so resources with no dependency between them still run one after another, and the compile says which |
| `puppet` | `require` |
| `dsc` | the runner's `dependsOn` |

## `notifies`

Every target in the segment has its own mechanism for reacting to a change, so
a notify is **native** everywhere: an Ansible handler, Puppet's `notify`, Chef's
`notifies`, the DSC runner's `notify`.

| Target | Actions accepted |
|---|---|
| `ansible` | `restart`, `reload`, `start`, `stop` on a service; `run` on an exec |
| `chef` | `restart`, `reload`, `start`, `stop` on a service; `run` on an exec |
| `puppet` | `restart` (or `refresh`) on a service; `run` on an exec. `reload`, `start` and `stop` are refused: Puppet's refresh restarts a service and takes no argument |
| `dsc` | `refresh`: the notified resource's `Set` runs even when its `Test` says there is nothing to do |

A notified `exec` runs in its place in the order, and once more when it is
notified.

## `runtimeWhen`

A condition the tool checks on the machine at the moment it applies the
resource. Meridian does not read it. It is written in the tool's own language,
so it is the one part of a shared-dialect document that does not move between
tools.

| Target | `runtimeWhen` is | Example |
|---|---|---|
| `ansible` | a task's `when:` expression | `"ansible_facts['os_family'] == 'Debian'"` |
| `chef` | the Ruby of an `only_if` guard | `"node['platform_family'] == 'debian'"` |
| `dsc` | a `preCondition` in the runner's function language | `"equals (nodeName) 'web'"` |
| `puppet` | refused. A catalog is compiled before it reaches the node, so nothing is left to evaluate at apply time | |

## Plan and apply

`meridian plan` uses each tool's own dry run (`--check --diff`, `--noop`,
`--why-run`, the DSC runner's `Test` mode). Where a tool has none for the way
the host is reached, the plan is reported as unknown and not as empty. Each
target's page lists the exact commands under "Running".

## What this flavour does not have

- Outputs. Nothing a ResourceSet does is handed to a later stage.
- A way to create the machine. That is an Infrastructure document, joined to
  this one through `targetHost` with `from: infrastructure`
  ([two-stage workflow](../../quickstart/first-two-stage.md)).
- `when`, the condition decided before compiling. It is
  [not available yet](index.md#when-and-runtimewhen).
