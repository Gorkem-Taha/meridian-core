# Your first ResourceSet

A ResourceSet describes how one machine that already exists should be
configured. This page takes one from a document to a running web server.

## The document

[`examples/ansible/web-server.yaml`](../../examples/ansible/web-server.yaml):

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
        path: /etc/nginx/conf.d/site.conf
        content: |
          server {
            listen 80;
            root /srv/www;
          }
        owner: root
        mode: "0644"
      dependsOn:
        - resource: nginx_pkg
      notifies:
        - resource: nginx_svc
          action: reload

    - id: nginx_svc
      type: service
      state: running
      params:
        name: nginx
        enabled: true
      dependsOn:
        - resource: nginx_pkg
```

Three resources: a package, a file and a service. The file comes after the
package (`dependsOn`), and when the file really changes the service is reloaded
(`notifies`). `target` says which tool does the work and `targetHost` says on
which machine.

## Check it

```
$ meridian validate examples/ansible/web-server.yaml
1 document valid
```

`validate` loads no plugin. It checks the document's own shape: required fields,
ids that exist, dependencies that do not form a cycle (a cycle is reported with
the resources in it). An error names the file,
the line and the field.

## Compile it

```
$ meridian compile examples/ansible/web-server.yaml --out build
web (ResourceSet, target ansible on web01.example.com)
  warning: ansible: resources "nginx_conf", "nginx_svc" are independent but will run in sequence; this target flattens the dependency graph and cannot express parallelism
  compiled 2 files into build/web
```

Nothing has been changed anywhere. `build/web/playbook.yml` is an ordinary
playbook that you could run by hand:

```yaml
---
- name: web
  hosts: web01.example.com
  tasks:
    - name: nginx_pkg
      ansible.builtin.package:
        name: nginx
        state: present
    - name: nginx_conf
      ansible.builtin.copy:
        content: |
          server {
            listen 80;
            root /srv/www;
          }
        dest: /etc/nginx/conf.d/site.conf
        mode: "0644"
        owner: root
      notify:
        - reload nginx_svc
    - name: nginx_svc
      ansible.builtin.service:
        enabled: true
        name: nginx
        state: started
  handlers:
    - name: reload nginx_svc
      ansible.builtin.service:
        enabled: true
        name: nginx
        state: reloaded
```

The warning is Meridian telling you something true about the tool: the document
says the file and the service are independent of each other, and Ansible will
run them one after the other anyway. Warnings never hide a loss of meaning. When
a tool cannot do what a document asks, the compile fails instead.

## Plan it

```
$ meridian plan examples/ansible/web-server.yaml
```

runs the tool's own preview (`ansible-playbook --check --diff`) and reports what
an apply would change, resource by resource, using the ids you wrote. How
Ansible connects is Ansible's business; pass it what it needs through
`MERIDIAN_ANSIBLE_ARGS`:

```
$ MERIDIAN_ANSIBLE_ARGS="--user deploy --private-key ~/.ssh/deploy" \
    meridian plan examples/ansible/web-server.yaml
```

## Apply it

```
$ meridian apply examples/ansible/web-server.yaml
```

Apply it a second time and nothing changes: the tools Meridian drives converge
on the state the document describes.

## The same document, another tool

Change `target: ansible` to `target: chef` and compile again. The resources are
the same; the output is now a cookbook, and the reload is Chef's own `notifies
:reload`.

Change it to `target: puppet` and the compile fails:

```
web (ResourceSet, target puppet on web01.example.com)
  FAILED: web-server.yaml:37:11: puppet: resource "nginx_conf" [unsupported-action]: notifies resource "nginx_svc" with action "reload", which this target cannot express: Puppet's refresh restarts a service; it takes no argument and cannot be told to reload instead. Emitting a restart here would do more than the document asked for. ...
```

Puppet has no way to reload a service in response to a change, only to restart
it. Meridian will not quietly turn your reload into a restart. You decide:
change the action to `restart` for this target
([`examples/puppet/web-server.yaml`](../../examples/puppet/web-server.yaml)), or
keep the tool that can do what you asked. The
[capability matrix](../reference/capability-matrix.md) shows these differences
for every target before you run into them.

Next: [a two-stage workflow](first-two-stage.md), where one tool creates the
machine and another configures it.
