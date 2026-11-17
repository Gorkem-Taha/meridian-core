# A policy for `meridian plan --policy` and `meridian apply --policy`.
#
# Meridian asks it about each stage before the stage is applied. Every
# message in `deny` stops the run; every message in `warn` is shown and
# stops nothing. What `input` holds is in docs/reference/policy.md.
package meridian

import rego.v1

# Nothing in production is deleted, or destroyed and created again, by a run.
deny contains msg if {
	input.stage.labels.env == "prod"
	some change in input.plan.changes
	change.action in {"delete", "replace"}
	msg := sprintf("%s: %s would be a %s in prod", [input.stage.name, change.resource, change.action])
}

# A preview the tool could not complete is not applied to production.
deny contains msg if {
	input.stage.labels.env == "prod"
	input.plan.unknown
	msg := sprintf("%s: the tool could not say everything it would change in prod", [input.stage.name])
}

# No file is world-writable.
deny contains msg if {
	some resource in input.resources
	resource.type == "file"
	resource.params.mode in {"0666", "0777"}
	msg := sprintf("%s: %s would be writable by everyone", [input.stage.name, resource.id])
}

# Every stage says which environment it belongs to.
warn contains msg if {
	not input.stage.labels.env
	msg := sprintf("%s has no env label, so no environment's rules apply to it", [input.stage.name])
}
