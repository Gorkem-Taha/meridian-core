# Dependencies and notifications

A resource can relate to another in two ways. They look similar and mean
different things, so they are different fields.

## `dependsOn`: order

```yaml
- id: nginx_conf
  type: file
  params: {path: /etc/nginx/nginx.conf, content: "..."}
  dependsOn:
    - resource: nginx_pkg
```

"Do not touch this until that is done." It is about order and nothing else.

Meridian builds a graph from every `dependsOn` in a document, refuses a cycle
with the resources named, and hands each plugin the resources in an order that
satisfies every edge. What the plugin does next depends on how its tool thinks
about order:

| Tool orders by | Targets | What `dependsOn` becomes |
|---|---|---|
| position in the file | `ansible`, `chef` | nothing written; the resources are emitted in order |
| declared relationships | `puppet`, `openvox`, `dsc`, `terraform`, `cloudformation`, `bicep`, `podman` | the tool's own keyword (`require`, `dependsOn`, `depends_on`, `DependsOn`, systemd's `Requires=` and `After=`) |
| its own rules | `docker` | `depends_on` between services; everything else Compose already creates first |
| nothing | `kubernetes` | the order of the manifests, with a warning that `kubectl` does not wait |
| the stages of a first boot | `cloud-init` | nothing written; each resource is placed at the step cloud-init handles it, a file that has to wait is deferred, and a dependency that points backwards through those steps is refused |

Where a reference already implies the order, as `${web_vm.id}` does in
Terraform or `Volume: ${data}:/srv` does in a Podman unit, the plugin does not
write it a second time.

## `notifies`: reaction

```yaml
- id: nginx_conf
  type: file
  params: {path: /etc/nginx/nginx.conf, content: "..."}
  notifies:
    - resource: nginx_svc
      action: restart
```

"When this really changes, do that to the other." It is conditional: on a run
where the file is already right, nothing is restarted. It also orders the
notifier first.

This is where tools differ most, and where Meridian is strict.

**Native** means the tool has its own mechanism. Ansible's handlers, Puppet's
and OpenVox's `notify`, Chef's `notifies`, DscPipelineRunner's `notify`, Compose's
`depends_on` with `restart: true`, and systemd's `PartOf=` for Podman units
([podman](../targets/podman.md#notifications)) are native.

**Synthetic** means the tool has no such mechanism, and Meridian builds the
reaction from something the tool does have. It is real, and it is visible in the
tool's own output, so each one is documented where it happens:

- Kubernetes: a checksum of the notifier in an annotation on the workload's pod
  template. The checksum changes, the template changes, the pods roll.
  ([kubernetes](../targets/kubernetes.md#notifications))
- Docker: a checksum of a config, secret, network or volume in a label on the
  service. ([docker](../targets/docker.md#notifications))
- Terraform: `replace_triggered_by` on the notified resource, which replaces it.
  ([terraform](../targets/terraform.md#notifications))
- cloud-init: the notified action as a command that runs once, after every
  other command of the first boot. A first boot changes everything, so the
  reaction is unconditional. ([cloud-init](../targets/cloud-init.md#notifications))

A synthetic notify always comes with a compile warning that says what it will
look like.

**The action matters.** A notify names an action, and a target accepts only the
actions its tool can really perform on that kind of resource. Puppet can restart
a service on refresh but cannot reload it, so `action: reload` is refused on
Puppet with the reason, not turned into a restart. Kubernetes can roll pods and
nothing else, so only `restart` aimed at a Deployment, StatefulSet or DaemonSet
is accepted. systemd can propagate a restart between Podman units and nothing
else, so only `restart` is accepted there. CloudFormation and Bicep have no way to react to a change at all,
and refuse every notify.

The [capability matrix](../reference/capability-matrix.md) has the summary. The
rule behind it: Meridian either does what the document says or refuses to
compile it. See [the capability model](capability-model.md).
