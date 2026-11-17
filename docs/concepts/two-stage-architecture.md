# The two-stage architecture

Meridian has two kinds of document, `Infrastructure` and `ResourceSet`, and runs
them as two stages. This page says why they are not one.

## They answer different questions

An Infrastructure document answers "what should exist?" Its tools (Terraform,
CloudFormation, Bicep, Docker, Kubernetes, Helm) create things through an API, keep
track of what they created, and can tell you facts about the result: an address,
an id, a URL.

A ResourceSet answers "how should this machine be set up?" Its tools (Ansible,
Puppet, Chef, DSC) log in to a machine, or run on it, and converge packages,
files and services. They need the machine to exist first, and they need to know
where it is.

A single graph over both would have to pretend that "create a VM" and "install
nginx" are the same sort of step. They are not: one of them does not have a
place to run until the other has finished.

## The hand-off is one declared value

An Infrastructure document promises **outputs**:

```yaml
outputs:
  - name: address
    value: ${web.public_ip}
```

A ResourceSet names the output it needs as its host:

```yaml
targetHost:
  from: infrastructure
  ref: machine          # the Infrastructure document's name
  output: address
```

That is the whole interface between the stages. `meridian validate` checks that
the document and the output exist. The value itself is only known after the
first stage has been applied, and Meridian is strict about that:

- Until then, the ResourceSet is **deferred**. It is not compiled against a
  guess.
- When the first stage is applied, its tool reports the outputs, Meridian keeps
  them in the state directory, and the ResourceSet is compiled for the real
  value and applied.
- If the first stage fails, later stages do not run.

A ResourceSet for a machine that is already there says so directly, and needs no
first stage:

```yaml
targetHost:
  from: inline
  host: web01.example.com
```

## Order within a workflow

Every Infrastructure document in a workflow runs before any ResourceSet,
whatever order the files are given in. A workflow is simply the documents passed
to one `meridian` command, in one file separated by `---` or in several files.

## What this does not do

Meridian does not hold a lock across stages, and it does not roll back a first
stage because a second failed. Each tool keeps its own state and its own idea of
drift: Terraform notices drift when it plans, Ansible and Chef only when they
run, Puppet and DSC agents on their own schedule. Meridian reports what each
tool says and does not paper over the difference.

A worked example: [a two-stage workflow](../quickstart/first-two-stage.md).
