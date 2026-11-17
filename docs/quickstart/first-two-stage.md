# A two-stage workflow

Most real work has two halves: something creates a machine, and something else
configures it. The second half needs to know what the first produced, usually an
address. In Meridian that hand-off is declared, not scripted.

## The documents

[`examples/workflow/vm-then-web.yaml`](../../examples/workflow/vm-then-web.yaml)
holds two documents separated by `---`. They could as well be two files passed
to the same command.

The first is an **Infrastructure** document. Terraform creates an instance, and
the document promises one output:

```yaml
apiVersion: meridian/v1
kind: Infrastructure
metadata:
  name: machine
spec:
  target: terraform
  resources:
    - id: aws
      type: provider
      params:
        region: eu-west-1

    - id: web
      type: aws_instance
      params:
        ami: ami-0abcdef1234567890
        instance_type: t3.micro
        tags:
          Name: web

  outputs:
    - name: address
      value: ${web.public_ip}
```

The second is a **ResourceSet**. Its host is not written down. It is whatever
`address` turns out to be:

```yaml
apiVersion: meridian/v1
kind: ResourceSet
metadata:
  name: site
spec:
  target: ansible
  targetHost:
    from: infrastructure
    ref: machine
    output: address
  resources:
    - id: nginx_pkg
      type: package
      params:
        name: nginx

    - id: nginx_svc
      type: service
      state: running
      params:
        name: nginx
        enabled: true
      dependsOn:
        - resource: nginx_pkg
```

## What Meridian does with them

Infrastructure documents always run before ResourceSets, whatever order the
files are given in.

```
$ meridian compile examples/workflow/vm-then-web.yaml --out build
machine (Infrastructure, target terraform)
  compiled 1 file into build/machine
site (ResourceSet, target ansible)
  deferred: needs the output "address" of Infrastructure "machine", which has not been applied yet
```

The second stage is **deferred**, and that is the honest answer. Its playbook
names a host, and the host does not exist yet. Meridian will not compile a
playbook for a placeholder. `meridian plan` says the same until the machine is
there.

```
$ meridian apply examples/workflow/vm-then-web.yaml
```

applies the first stage, reads `address` from Terraform, compiles the second
stage for that address, and applies it. If the first stage fails, the second
does not run. Once the infrastructure exists, `plan` covers both stages, because
the output is known.

Outputs and each tool's working files are kept under `.meridian` (change it with
`--state-dir` or `MERIDIAN_STATE_DIR`), so a later run can read what an earlier
one produced. A value a tool marks sensitive is shown as `(sensitive)` wherever
Meridian prints.

## Which targets can hand something on

A stage can only promise outputs if its tool can report values after an apply.
The `outputs` column of the [capability matrix](../reference/capability-matrix.md)
says which do: Terraform, OpenTofu, CloudFormation, Bicep, Docker and Kubernetes. The
configuration management targets configure a machine and report nothing.

More on why the two kinds are kept apart:
[the two-stage architecture](../concepts/two-stage-architecture.md).
