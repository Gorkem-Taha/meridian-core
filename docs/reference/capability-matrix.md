# Capability matrix

What each target can express, as each plugin itself declares it. This page is
generated: `crates/meridian-cli/tests/capability_matrix.rs` writes it from the
plugins and fails when it is out of date, so it cannot drift from the code.

A cell that says **no** is not a missing feature waiting to be papered over.
A document that asks a target for something it cannot do is refused when it is
compiled, with the reason. Each target's own page says how it does what it can.

- **notifies** is *native* when the tool has its own way to react to a change,
  and *synthetic* when Meridian builds the reaction out of something else the
  tool has. A synthetic notify shows up in the tool's own output, and the
  target's page says what it looks like.
- **runtimeWhen** is a condition the tool checks on the node at the moment it
  applies a resource.
- **plan** asks the tool what an apply would change, without changing it.
- **outputs** are values a later stage of a workflow can read.

| Target | Category | Resource types | notifies | runtimeWhen | plan | apply | outputs |
|---|---|---|---|---|---|---|---|
| [ansible](../targets/ansible.md) | configuration-management | `exec`, `file`, `group`, `package`, `service`, `user` | native | yes | yes | yes | no |
| [bicep](../targets/bicep.md) | infrastructure-as-code | `‹Namespace›/‹type›@‹api-version›`, `deployment` | no | no | yes | yes | yes |
| [chef](../targets/chef.md) | configuration-management | `exec`, `file`, `group`, `package`, `service`, `user` | native | yes | yes | yes | no |
| [cloudformation](../targets/cloudformation.md) | infrastructure-as-code | `‹Vendor›::‹Service›::‹Type›`, `stack` | no | no | yes | yes | yes |
| [docker](../targets/docker.md) | containerization | `service`, `network`, `volume`, `config`, `secret`, `project` | native and synthetic | no | yes | yes | yes |
| [dsc](../targets/dsc.md) | configuration-management | `‹Module›/‹Resource›`, `runner` | native | yes | yes | yes | no |
| [kubernetes](../targets/kubernetes.md) | container-orchestration | `‹apiVersion›/‹Kind›`, `cluster` | synthetic | no | yes | yes | yes |
| [openvox](../targets/openvox.md) | configuration-management | `exec`, `file`, `group`, `package`, `service`, `user` | native | no | yes | yes | no |
| [puppet](../targets/puppet.md) | configuration-management | `exec`, `file`, `group`, `package`, `service`, `user` | native | no | yes | yes | no |
| [terraform](../targets/terraform.md) | infrastructure-as-code | `‹provider›_‹type›`, `backend`, `data.‹type›`, `module`, `provider` | synthetic | no | yes | yes | yes |
