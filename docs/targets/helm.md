# Helm with Kustomize

<img src="../branding/logos/meridian-helm.svg" width="72" alt="Meridian Helm plugin logo">

The `helm` target compiles a Meridian document of Kubernetes objects into a Helm chart
and installs it as a release. A document may also carry a Kustomize overlay, which is
applied to what the chart renders before Helm installs it.

| | |
|---|---|
| Target name | `helm` |
| Category | `container-orchestration` |
| Operations | compile, plan, apply, outputs |
| Document kind | `Infrastructure` |
| Tools it runs | `helm`; `kustomize` when the document has an overlay; `kubectl` when an output reads from the cluster |

## What this target does and does not support

| Feature | Support |
|---|---|
| Kubernetes objects of any kind, including custom resources | Yes. The type is the object's `apiVersion` and kind. |
| Chart values, and reading them from objects | Yes. `${values.path}` becomes `{{ .Values.path }}`. |
| Kustomize overlay on the rendered chart | Yes, with inline patches. An overlay cannot name other files. |
| Outputs read from live objects | Yes, with `kubectl`, after an apply. |
| `notifies` with action `restart` | Emulated. See [Restarting a workload](#restarting-a-workload-when-something-changes). |
| `notifies` with any other action | No. Refused at compile time. |
| `dependsOn` | Only where Helm's own install order already agrees. See [Ordering](#ordering). |
| `runtimeWhen` | No. Refused at compile time. |
| `state: absent` | No. Remove the resource; Helm deletes what the chart no longer declares. |
| Chart dependencies (subcharts), hooks written by hand, `lookup` | No. The chart holds only what the document declares. |
| Drift detection | No. A plan compares with what Helm last installed, not with the live cluster. |

Nothing in this list degrades silently. A document that needs something the target
lacks fails at compile time with the rule named in
[Compile errors](#compile-errors).

## A first document

This is [`examples/helm/web-app.yaml`](../../examples/helm/web-app.yaml), shortened.

```yaml
apiVersion: meridian/v1
kind: Infrastructure
metadata:
  name: web-app
spec:
  target: helm
  resources:
    - id: values
      type: values
      params:
        replicas: 2
        image:
          repository: nginx
          tag: "1.27"

    - id: release
      type: release
      params:
        namespace: web
        createNamespace: true
        wait: true

    - id: web_config
      type: v1/ConfigMap
      params:
        metadata:
          name: ${release.name}-config
        data:
          index.html: "<h1>Hello</h1>"
      notifies:
        - resource: web
          action: restart

    - id: web
      type: apps/v1/Deployment
      params:
        metadata:
          name: ${release.name}
        spec:
          replicas: ${values.replicas}
          selector:
            matchLabels: {app: web}
          template:
            metadata:
              labels: {app: web}
            spec:
              containers:
                - name: nginx
                  image: ${values.image.repository}:${values.image.tag}
                  volumeMounts:
                    - name: content
                      mountPath: /usr/share/nginx/html
              volumes:
                - name: content
                  configMap:
                    name: ${web_config}

    - id: web_svc
      type: v1/Service
      params:
        metadata:
          name: ${release.name}
        spec:
          type: LoadBalancer
          selector: {app: web}
          ports:
            - port: 80

  outputs:
    - name: web_address
      value: ${web_svc.status.loadBalancer.ingress[0].ip}
```

```console
$ meridian compile web-app.yaml     # write the chart, change nothing
$ meridian plan web-app.yaml        # what an apply would create, update and delete
$ meridian apply web-app.yaml       # install or upgrade the release, report the outputs
```

## Resource types

### Kubernetes objects

A resource whose `type` is an `apiVersion` and a kind joined by a slash is a Kubernetes
object: `v1/Service`, `apps/v1/Deployment`, `networking.k8s.io/v1/Ingress`,
`cert-manager.io/v1/Certificate`. Its `params` are the rest of the manifest, exactly as
Kubernetes documents it: `metadata`, `spec`, `data` and so on. Do not write `apiVersion`
or `kind` in `params`; the type already says them.

An object with no `metadata.name` is named after its id, with underscores turned into
hyphens: the resource `web_config` is the ConfigMap `web-config`.

Each object becomes one template of the chart, `templates/<id>.yaml`.

This is the same spelling the `kubernetes` target uses, so the objects of a document
move between the two targets unchanged.

### `values`

The chart's default values, written to `values.yaml`. Anything a YAML document can hold
is allowed. Values are literal: they cannot refer to anything.

```yaml
- id: values
  type: values
  params:
    replicas: 2
    image: {repository: nginx, tag: "1.27"}
```

Quote a value that looks like a number but is not one, such as a version. `tag: 1.20` is
the number 1.2 to YAML, to Meridian and to Helm alike.

### `chart`

The chart's own metadata. Every param is optional.

| Param | Default | Meaning |
|---|---|---|
| `name` | the document's `metadata.name` | The chart's name: lowercase letters, digits and hyphens. |
| `version` | `0.1.0` | A semantic version, in quotes if it could be read as a number. |
| `appVersion` | none | The version of what the chart deploys. |
| `description` | "Compiled by Meridian from the document …" | One line about the chart. |

### `release`

How the chart is installed. Every param is optional.

| Param | Default | Meaning |
|---|---|---|
| `name` | the document's `metadata.name` | The release name: lowercase letters, digits and hyphens, at most 53 characters. |
| `namespace` | `default` | The namespace the release is installed into. |
| `createNamespace` | `false` | Create that namespace if it is not there (`--create-namespace`). |
| `wait` | `false` | Wait until the release's workloads are ready (`--wait`). |
| `atomic` | `false` | Roll back a failed install or upgrade (`--atomic`). |
| `timeout` | Helm's own | How long to wait, as a duration: `90s`, `5m`, `1h30m`. |

### `kustomization`

A Kustomize overlay. See [Kustomize overlays](#kustomize-overlays).

A document has at most one resource of each of these four types.

## References

A reference is written `${id}` or `${id.path}`, where `id` is the id of a resource of the
same document. `$${` writes a literal `${`.

| Written | Refers to | In the chart |
|---|---|---|
| `${values.image.tag}` | A value, by path. `values` is the id of the `values` resource. | `{{ .Values.image.tag }}` |
| `${values.hosts[0]}`, `${values["my-key"]}` | A list item, or a key that is not a plain word. | `{{ index … }}` |
| `${release.name}`, `${release.namespace}` | The release. `release` is the id of the `release` resource. | `{{ .Release.Name }}`, `{{ .Release.Namespace }}` |
| `${chart.name}`, `${chart.version}`, `${chart.appVersion}` | The chart. | `{{ .Chart.Name }}` and so on |
| `${web_config}` or `${web_config.metadata.name}` | The name of another object. | That object's name, as its own manifest writes it |
| `${web_config.metadata.namespace}` | The namespace another object lands in. | Its own, or `{{ .Release.Namespace }}` |

A reference to a value that does not exist is a compile error, not an empty string at
install time.

A reference that is the whole of a value keeps the value's type, so
`replicas: ${values.replicas}` is a number and `resources: ${values.resources}` is a
whole mapping. A reference inside longer text makes text:
`image: ${values.image.repository}:${values.image.tag}`.

A template cannot read what only the cluster knows, such as an object's `status`. Read
that in an [output](#outputs).

### Text is never a template

Whatever you write in `params` reaches the cluster as you wrote it. A `{{` in a
ConfigMap, a Go template for another program, a Prometheus rule: none of it is
interpreted by Helm, and there is nothing to escape.

```yaml
data:
  alert.tmpl: "{{ .Labels.instance }} is down"   # arrives exactly like this
```

To have Helm fill something in, use a reference.

## Restarting a workload when something changes

Helm has no handlers: nothing in a chart runs because another object changed. Chart
authors get the effect by hand, with a checksum of the ConfigMap on the pod template, and
that is what `notifies` compiles to.

```yaml
- id: web_config
  type: v1/ConfigMap
  params: {data: {mode: fast}}
  notifies:
    - resource: web
      action: restart
```

The Deployment `web` gets this on its pod template:

```yaml
annotations:
  checksum/web_config: {{ include (print $.Template.BasePath "/web_config.yaml") . | sha256sum | quote }}
```

When the ConfigMap's rendered manifest changes, the annotation changes, and Kubernetes
rolls the workload's pods. Because this is an emulation, the compile reports a warning
that says so, and a plan shows the workload as updated whenever the object it watches
is.

Only the action `restart` is accepted, and only a Deployment, a StatefulSet or a
DaemonSet can be notified. Two workloads cannot notify each other.

## Ordering

Helm sends every object of a release to the cluster in one pass, sorted by kind:
namespaces, then quotas, service accounts, Secrets, ConfigMaps, storage, RBAC, Services,
workloads, and finally Ingresses and anything it does not know. Nothing in a chart can
change that order.

So `dependsOn` is accepted where that order already honours it, and nothing is written
into the chart. A Deployment that depends on a ConfigMap is fine. A dependency Helm would
not honour is refused with `unsupported-dependency`, for example one Deployment that
depends on another. Kubernetes retries until what an object needs exists, so such a
dependency can usually just be removed. To wait for the whole release to be ready, set
`wait: true` on the release.

## Kustomize overlays

A resource of type `kustomization` is a Kustomize overlay on top of the chart. Its params
are the fields of a `kustomization.yaml`. This is
[`examples/helm/web-app-kustomize.yaml`](../../examples/helm/web-app-kustomize.yaml),
shortened:

```yaml
- id: overlay
  type: kustomization
  params:
    namePrefix: prod-
    labels:
      - pairs: {environment: production}
    images:
      - name: nginx
        newName: registry.example.com/mirror/nginx
    replicas:
      - name: ${web}
        count: 4
    patches:
      - target: {kind: Deployment, name: "${web}"}
        patch:
          - op: add
            path: /spec/template/spec/containers/0/resources
            value: {limits: {memory: 256Mi}}
```

A patch is written as the YAML it is. Meridian turns it into the string Kustomize reads.
A patch already written as a string is passed through.

### How the two tools are combined

1. Helm renders the chart (`helm template`).
2. Kustomize applies the overlay to what Helm rendered (`kustomize build`).
3. Helm installs the patched objects as the release.

The result is one Helm release, with `helm history`, `helm rollback` and `helm
uninstall` working as usual. The release holds the objects as the overlay left them, so
`helm get manifest` shows what is really in the cluster.

This is done without Helm's `--post-renderer` flag, whose form differs between Helm 3
and Helm 4, so the same document works on both.

### What an overlay cannot do

An overlay is applied to what the chart renders and to nothing else. It cannot name
other files, so these fields are refused: `resources`, `bases`, `components`, `crds`,
`configurations`, `generators`, `transformers`, `validators`, `helmCharts`,
`helmGlobals` and `openapi`, and a patch's `path`. Declare the objects as resources of
the document and write patches inline.

In an overlay a reference is replaced by its value when the document is compiled, since
Kustomize has no values of its own: `${web}` is the Deployment's name as the chart
renders it, before the overlay renames it.

Meridian follows an overlay's `namePrefix`, `nameSuffix` and `namespace`, so plans,
outputs and reports still name the resource you wrote. An object that a patch renames in
some other way is reported under the tool's own name, such as `Deployment/other-name`.

## Outputs

An output is a value later stages can use. It may hold:

- anything a template can read: `${release.name}`, `${values.port}`, `${web_svc}`;
- any path into a live object, read with `kubectl` after the apply:
  `${web_svc.status.loadBalancer.ingress[0].ip}`,
  `${web.status.readyReplicas}`,
  `${web.metadata.annotations["example.com/owner"]}`.

```yaml
outputs:
  - name: web_url
    value: http://${web_svc.status.loadBalancer.ingress[0].ip}/
  - name: ready
    value: ${web.status.readyReplicas}
```

An output that is one reference keeps the type the cluster gave it; `ready` above is a
number. Anything read from a Secret is marked sensitive and masked wherever Meridian
shows it. The value is as the cluster stores it, so a Secret's `data` is still base64.

A value the cluster fills in later, such as a load balancer's address, is only there
once the release is ready. Set `wait: true` on the release so that Helm waits for it.

## What a compile writes

```
chart/Chart.yaml
chart/values.yaml
chart/templates/<id>.yaml      one per object
kustomize/kustomization.yaml   only with an overlay
release.json                   for Meridian's runner; Helm never reads it
```

`chart/` is an ordinary Helm chart. `helm lint chart` passes on it and `helm install`
installs it, with or without Meridian. `release.json` holds the release's settings, the
object each resource became, and how to read each output.

## What plan and apply run

| Step | Command |
|---|---|
| Render | `helm template <release> chart --namespace <ns>` |
| Overlay, if any | `kustomize build kustomize` |
| Read the release | `helm status <release> --output json`, `helm get manifest <release>` |
| Apply | `helm upgrade --install <release> chart --namespace <ns>` plus the release's settings |
| Outputs | `kubectl get <kind> <name> --namespace <ns> --output json` |

**Plan** compares what would be installed with the manifest of the release Helm last
installed, and reports each object as created, updated or deleted. It changes nothing.
It does not see changes made to the cluster behind Helm's back.

**Apply** does nothing when the release is deployed and already matches, so an unchanged
document does not add a revision to the release's history. A release left in a failed
state is upgraded again even when its objects match; a plan reports that as "may
change".

## Configuration

Set these in the environment of the machine that runs Meridian.

| Variable | Default | Meaning |
|---|---|---|
| `MERIDIAN_HELM` | `helm` | The Helm program. |
| `MERIDIAN_HELM_ARGS` | none | Arguments added to every `helm` call: `--kube-context prod`. |
| `MERIDIAN_KUSTOMIZE` | `kustomize build` | The command that builds an overlay. `kubectl kustomize` uses the copy built into kubectl. |
| `MERIDIAN_KUBECTL` | `kubectl` | The program that reads outputs. |
| `MERIDIAN_KUBECTL_ARGS` | none | Arguments added to every `kubectl` call: `--context prod`. |

Helm and kubectl find the cluster the way they always do: the current context of your
kubeconfig, or `KUBECONFIG`. Point both at the same cluster.

## Compile errors

| Rule | Meaning | What to do |
|---|---|---|
| `unsupported-resource-type` | The type is not `apiVersion/Kind` or a settings type. | Write `apps/v1/Deployment`, not `Deployment`. |
| `invalid-reference` | A reference names nothing, reads a value that does not exist, or reads live state inside a template. | Fix the name or the path. Read live state in an output. |
| `invalid-param` | A setting has the wrong type or form, or `apiVersion` or `kind` is among the params. | The message names the param. |
| `unsupported-param` | A settings resource has a param the target does not know, or an overlay names a file. | The message lists what is accepted. |
| `invalid-name` | An id cannot name a template, or a name cannot be a Kubernetes name. | Use letters, digits, `_` and `-` in ids; set `metadata.name`. |
| `invalid-release-name`, `invalid-chart-name` | The document's name cannot be used as it is. | Add a `release` or `chart` resource with a `name`. |
| `duplicate-settings` | Two resources of one settings type. | Keep one. |
| `duplicate-object` | Two resources are the same object. | Give one a different `metadata.name`. |
| `duplicate-output` | Two outputs share a name. | Rename one. |
| `empty-chart` | The document has no Kubernetes object. | Add one. |
| `unsupported-dependency` | A `dependsOn` Helm's install order does not honour, or one on a settings resource. | Remove it; see [Ordering](#ordering). |
| `unsupported-action` | A notification other than a restart of a workload. | See [Restarting a workload](#restarting-a-workload-when-something-changes). |
| `unsupported-state`, `unsupported-runtime-condition` | `state: absent` or `runtimeWhen`. | Remove the resource or the condition. |

## Run errors

| Message | Meaning |
|---|---|
| `could not run helm: it is not installed…` | The tool is not on the `PATH`. Install it or set `MERIDIAN_HELM`. The same goes for `kustomize` and `kubectl`. |
| `helm could not render the chart` | Helm rejected the chart. The output that follows is Helm's own. |
| `kustomize could not apply the overlay` | Kustomize rejected the overlay, most often a patch whose target matches nothing. |
| `helm could not install the release…` | The install or upgrade failed. `helm status` and `helm history` show the state it was left in. |
| `output "x": the Service web has no status.loadBalancer.ingress yet` | The cluster has not filled the value in. Set `wait: true` on the release. |
| `nothing has been applied for this document yet, so it has no outputs` | Outputs were asked for before the first apply. |

## How it is tested

- Unit tests for the template writer, reference paths, install order and manifest
  comparison.
- Golden files for both examples in
  [`plugins/helm/testdata`](../../plugins/helm/testdata).
- Runner tests against stand-ins for `helm`, `kustomize` and `kubectl`, which run
  anywhere.
- Tests against the real `helm` and `kustomize`, and against a real cluster created with
  `kind`, in the `helm` workflow: plan, install, outputs, upgrade, delete, and an overlay
  installed and read back in sync. Run them yourself with:

```console
$ cargo test -p meridian-target-helm --test real -- --ignored
$ cargo test -p meridian-target-helm --test cluster -- --ignored
```
