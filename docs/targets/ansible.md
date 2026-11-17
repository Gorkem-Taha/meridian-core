# Target: `ansible`

Compiles a ResourceSet into a playbook and an inventory, and runs them with
`ansible-playbook`.

## What it does and does not support

| | |
|---|---|
| Resource types | `package`, `file`, `service`, `user`, `group`, `exec` ([reference](../reference/resource-types.md)) |
| `dependsOn` | Yes, as task order. Ansible runs a play top to bottom, so independent resources still run in sequence; a warning says which |
| `notifies` | Native: handlers. Actions `restart`, `reload`, `start`, `stop` on a service, `run` on an exec |
| `runtimeWhen` | Yes: a task's `when:` |
| Plan | Yes: `--check --diff`. A task Ansible skips in check mode cannot be previewed |
| Outputs | No |

## What a document compiles to

`playbook.yml` (one play, one task a resource, named by the resource's id) and
`inventory.ini` (the document's host). Modules are the `ansible.builtin` ones:
`package`, `service`, `copy` for a file with content and `file` otherwise,
`user`, `group`, `command`, and `shell` for an exec with `shell: true`.

A notify becomes a handler named after the action and the resource, such as
`reload nginx_svc`. Handlers run once, at the end of the play, only when a task
that notifies them changed something.

`runtimeWhen` is copied into the task's `when:` as written, so it is an Ansible
expression:

```yaml
runtimeWhen: "ansible_facts['os_family'] == 'Debian'"
```

## Example

[`examples/ansible/web-server.yaml`](../../examples/ansible/web-server.yaml), walked
through in [your first ResourceSet](../quickstart/first-resourceset.md).

## Running

| | |
|---|---|
| Plan | `ansible-playbook --inventory inventory.ini --check --diff playbook.yml` |
| Apply | `ansible-playbook --inventory inventory.ini playbook.yml` |
| `MERIDIAN_ANSIBLE_PLAYBOOK` | the program to run, when `ansible-playbook` is not on `PATH` |
| `MERIDIAN_ANSIBLE_ARGS` | arguments added to every run: `--user deploy --private-key ~/.ssh/deploy`, or `--connection local` |

Changes are read from Ansible's own report (`changed:` per task and handler). A
failed task or an unreachable host fails the stage, with the end of Ansible's
output.
