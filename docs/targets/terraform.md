# Target: `terraform`

<img src="../branding/logos/meridian-terraform.svg" width="72" alt="Meridian Terraform plugin logo">

Compiles an Infrastructure document into a Terraform configuration and runs it
with `terraform`. OpenTofu's `tofu` takes the same commands.

## What it does and does not support

| | |
|---|---|
| Resource types | Terraform's own: any `<provider>_<type>`, plus `data.<type>`, `module`, `provider` and `backend` |
| `dependsOn` | Yes, as `depends_on`, unless a reference already implies the order |
| `notifies` | Synthetic, and blunt: the action `replace` only. See [Notifications](#notifications) |
| `runtimeWhen` | No. Refused |
| `state: absent` | No. Terraform removes what the configuration no longer declares, so remove the resource from the document |
| Plan | Yes: Terraform's own plan |
| Outputs | Yes |

## What a document looks like

A resource's `type` is the Terraform type and its `params` are that type's
arguments, exactly as the provider documents them.

| `type` | Becomes |
|---|---|
| `aws_instance`, `azurerm_resource_group`, any `<provider>_<type>` | a `resource` block named by the resource's id |
| `data.<type>` | a `data` block |
| `module` | a `module` block |
| `provider` | a `provider` block; `source` and `version` go to `required_providers` |
| `backend` | the `backend` block, whose type is the resource's id |
| `variable` | a `variable` block named by the resource's id. Its params are Terraform's: `type`, `default`, `description`, `sensitive`, `nullable`, `validation` |

### Variables, and values from other documents

A `variable` is an input. Another resource reads it by its id, `${user_data}`,
which is written as `${var.user_data}`. A variable has its value before
Terraform creates anything, so nothing can `dependsOn` one.

```yaml
spec:
  target: terraform
  imports:
    - name: user_data
      from: artifact
      ref: bootstrap
      file: user-data
      sensitive: true
  resources:
    - id: user_data
      type: variable
      params: {type: string, sensitive: true}
    - id: web
      type: aws_instance
      params:
        ami: ami-0abcdef1234567890
        instance_type: t3.micro
        user_data: ${user_data}
```

A variable gets its value from `TF_VAR_<id>` in the environment `meridian` runs
in, as it would with `terraform` itself. A value the document
[imports](../reference/dsl/outputs-and-hand-offs.md#imports) from another
document arrives the same way, under the import's name, so the import and the
variable share a name. A string is passed as it is; a number, a boolean, a list
or a map is passed as JSON. The value is never written into `main.tf.json`.
[`examples/workflow/bootstrap-then-machine.yaml`](../../examples/workflow/bootstrap-then-machine.yaml)
creates a machine with the user-data a cloud-init document compiles to.

## References

`${web_vm.public_ip}` in a param or an output refers to another resource of the
document by its id, and is rewritten to Terraform's own address for it
(`aws_instance.web_vm.public_ip`, or `data.aws_ami.ubuntu.id` for a data
source). A reference to a resource that is not in the document is a compile
error, with the line.

## Why the output is JSON

The configuration is written in Terraform's JSON syntax (`main.tf.json`), not
HCL. In HCL a nested block and a map-valued argument are written differently,
and only the provider's schema says which a param is. In the JSON syntax
Terraform decides that itself, so a document compiles correctly with no schema.
Terraform reads the file like any other configuration.

## Notifications

Terraform has no handlers. The one reaction it can be given is
`replace_triggered_by`: when the notifier changes, the notified resource is
destroyed and created again. So this target accepts a notify with the action
`replace`, aimed at a managed resource, and nothing else. The compile warns that
this is how the notify works, and a `terraform plan` will show the notified
resource as "must be replaced". That is the notify, not a bug.

```yaml
- id: launch_config
  type: aws_launch_template
  params: {...}
  notifies:
    - resource: web_vm
      action: replace
```

## Example

[`examples/terraform/web-vm.yaml`](../../examples/terraform/web-vm.yaml) declares a
provider, a data source, a security group, an instance and an address, and two
outputs. Its compiled configuration is checked in as
[`web-vm.main_tf_json`](../../plugins/terraform/testdata/web-vm.main_tf_json). The
instance, for example:

```yaml
- id: web_vm
  type: aws_instance
  params:
    ami: ${ubuntu.id}
    instance_type: t3.small
    vpc_security_group_ids: ["${web_sg.id}"]
```

```json
"aws_instance": {
  "web_vm": {
    "ami": "${data.aws_ami.ubuntu.id}",
    "instance_type": "t3.small",
    "vpc_security_group_ids": ["${aws_security_group.web_sg.id}"]
  }
}
```

## Running

| | |
|---|---|
| Plan | `terraform init`, then `terraform plan -out=meridian.tfplan`, then `terraform show -json` to read it |
| Apply | the same plan, then `terraform apply meridian.tfplan`, then `terraform output -json` |
| `MERIDIAN_TERRAFORM` | the program to run, when it is not `terraform` on `PATH`; set it to `tofu` for OpenTofu |

What is applied is exactly the plan that was shown. Values the document
[imports](#variables-and-values-from-other-documents) reach the configuration
as `TF_VAR_<name>` input variables.
Terraform's state stays where the configuration's `backend` puts it; with no
backend it is a local file in the document's directory under the state
directory, so keep that directory.

An output Terraform marks sensitive is reported as sensitive and never printed.

The runner is tested against the real `terraform` command (1.13) on every pull
request that touches it, and the same tests run again with `tofu`: a plan that
creates nothing, an apply checked on disk, a second plan that is in sync, an
update, a replacement, outputs including a sensitive one, and a configuration
Terraform refuses. They use `terraform_data` and a local state file, so a
provider that talks to a cloud has not been run this way. See
[Real-tool tests](../contributing/real-tool-tests.md).
