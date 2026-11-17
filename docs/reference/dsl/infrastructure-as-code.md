# The infrastructure as code flavour

Infrastructure as code tools create things through an API, keep track of what
they created, and can report facts about the result. This page is how the
[Meridian language](index.md) is written for them.

| | |
|---|---|
| Category, as `meridian plugins` prints it | `infrastructure-as-code` |
| Kind of document | `Infrastructure`. A `ResourceSet` is refused: these tools create things, and are not pointed at one machine |
| `type` and `params` | the tool's own, passed through |
| Outputs | yes |

## Targets

| Target | `type` is | Example | Settings resource | Page |
|---|---|---|---|---|
| `terraform` | the Terraform type | `aws_instance` | `provider`, `backend` | [terraform](../../targets/terraform.md) |
| `cloudformation` | the CloudFormation type | `AWS::EC2::Instance` | `stack` (optional) | [cloudformation](../../targets/cloudformation.md) |
| `bicep` | the Azure type and API version | `Microsoft.Network/publicIPAddresses@2023-09-01` | `deployment` (required) | [bicep](../../targets/bicep.md) |
| `opentofu` | the OpenTofu type | `aws_instance` | `provider`, `backend`, `encryption` | [opentofu](../../targets/opentofu.md) |
| `pulumi` | the Pulumi type token | `aws:ec2:Instance` | `stack`, `config` | [pulumi](../../targets/pulumi.md) |
| `arm` | the Azure type, with the API version in the `apiVersion` param | `Microsoft.Network/publicIPAddresses` | `deployment`, `parameter` | [arm](../../targets/arm.md) |
| `packer` | the part of a build and its Packer type | `source.amazon-ebs`, `provisioner.shell` | `variable`, `local`, `plugin` | [packer](../../targets/packer.md) |

## Native passthrough

A Terraform provider has thousands of resource types and Azure has every
service. Meridian does not put a smaller vocabulary in front of them. A
resource's `type` is the tool's own type, and its `params` are that type's
arguments exactly as the tool's documentation gives them, in the tool's own
spelling (`instance_type` for Terraform, `InstanceType` for CloudFormation).

So a document in this flavour is written for one tool and does not move to
another by changing `spec.target`. What it shares with every other Meridian
document is everything around the resource: the document format, ids,
`dependsOn`, references checked at compile time, outputs, and one way to plan
and apply.

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

    - id: web_sg
      type: aws_security_group
      params:
        name: web

    - id: web
      type: aws_instance
      params:
        ami: ami-0abcdef1234567890
        instance_type: t3.micro
        vpc_security_group_ids: ["${web_sg.id}"]

  outputs:
    - name: address
      value: ${web.public_ip}
```

This document is
[`examples/flavours/infrastructure-as-code.yaml`](../../../examples/flavours/infrastructure-as-code.yaml).
A longer one for each tool:
[`examples/terraform/web-vm.yaml`](../../../examples/terraform/web-vm.yaml),
[`examples/cloudformation/web-vm.yaml`](../../../examples/cloudformation/web-vm.yaml),
[`examples/bicep/web-vm.yaml`](../../../examples/bicep/web-vm.yaml).

## The settings resource

Most targets in this segment take a resource that is not something to create.
It says where and how the rest is deployed: a Terraform `provider` and
`backend`, a CloudFormation `stack` (name, region, capabilities), a Bicep
`deployment` (resource group, subscription). It is written as a resource so that
a document stays one list, and it has a fixed type name. The target's page lists
its params and says whether it is required.

## `state`

Left out. These tools have one idea of state: what the document declares exists,
and what it no longer declares is removed (or, on a Bicep resource group
deployment, left alone). `state: absent` is refused on every target in the
segment, with the instruction to remove the resource from the document.

## References

This is the flavour where `${...}` does the most.

| You write | Terraform | CloudFormation | Bicep |
|---|---|---|---|
| `${web}` | the resource's address | `Ref` | the symbolic name |
| `${web.attribute}` | an attribute | `Fn::GetAtt` | a property |
| a reference inside longer text | interpolation | `Fn::Sub` | interpolation |
| something that is not a path | Terraform's expression language | pseudo parameters such as `${AWS::Region}` | Bicep's expression language, such as `${resourceGroup().location}` |

The first name of a path must be a resource of the document, or the document
does not compile. What can be read after it is whatever the tool exposes for
that type.

## Order

`dependsOn` becomes the tool's own keyword (`depends_on`, `DependsOn`,
`dependsOn`). These tools also derive order from references, so where a
reference already puts one resource after another, the dependency is not written
a second time.

## `notifies`

These tools have no handlers. What each can be made to do instead:

| Target | Notify |
|---|---|
| `terraform` | synthetic: the action `replace` only, aimed at a managed resource. It becomes `replace_triggered_by`, so the notified resource is destroyed and created again. The compile warns |
| `cloudformation` | refused. There is no way to react to a change in another resource |
| `bicep` | refused, for the same reason |

## `runtimeWhen`

Refused on every target in the segment. There is no node, and no moment on it,
to check a condition.

## Outputs

An output names a value the tool reports once the document is applied:

```yaml
outputs:
  - name: address
    value: ${web.public_ip}
```

It becomes the tool's own output (`output`, `Outputs`, `output`), is read back
after the apply, and is kept for the stages that ask for it. Today that is a
ResourceSet's `targetHost`:

```yaml
targetHost:
  from: infrastructure
  ref: machine
  output: address
```

A value the tool marks sensitive is reported as `(sensitive)` and never printed.
Bicep reports every output as text.

## Plan and apply

`meridian plan` is the tool's own preview: a Terraform plan, a CloudFormation
change set that is then discarded, an Azure `what-if`. Where the tool cannot
analyse a resource, the plan is reported as unknown. Each tool keeps its own
state and uses its own credentials; Meridian stores neither.
