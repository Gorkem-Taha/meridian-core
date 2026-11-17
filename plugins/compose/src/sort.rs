//! Stage two: Compose's ordering semantics.

use meridian_sdk::{CompileError, EdgeKind, ResourceGraph};

use crate::{refuse_at, Dependency, Kind, Project};

/// The one notify action Compose can perform.
const RESTART: &str = "restart";

/// Turns dependency and notify edges into `depends_on`.
///
/// Compose orders only services. It creates every network, volume, config
/// and secret before it starts any of them, so a service that depends on one
/// needs nothing written, and one of those cannot wait for anything.
///
/// A notify with the action `restart` between two services becomes
/// `restart: true` on the dependency: Compose restarts the notified service
/// when it updates the one that notified it. That also starts the notified
/// service second, which is the only order in which the restart could matter.
pub(crate) fn apply_ordering(
    graph: &ResourceGraph,
    project: &mut Project,
) -> Result<(), CompileError> {
    for edge in &graph.edges {
        let kind = |id: &str| project.entry(id).map(|e| e.kind);
        let (Some(from), Some(to)) = (kind(&edge.from), kind(&edge.to)) else {
            return Err(refuse_at(
                &edge.to,
                &edge.pos,
                "unsupported-dependency",
                "a dependency must be between two elements of the Compose file; the project's \
                 own settings are not one"
                    .to_owned(),
            ));
        };
        let restart = match edge.kind {
            EdgeKind::HardOrder => false,
            EdgeKind::Notify => {
                if edge.action != RESTART || from != Kind::Service || to != Kind::Service {
                    return Err(refuse_at(
                        &edge.from,
                        &edge.pos,
                        "unsupported-action",
                        format!(
                            "notifies {} {:?} with action {:?}, which this target cannot \
                             express: the one thing Compose does because something changed is \
                             restart a service when a service it depends on is updated \
                             (accepted: restart, from a service to a service)",
                            to.as_str(),
                            edge.to,
                            edge.action
                        ),
                    ));
                }
                true
            }
        };
        if to != Kind::Service {
            return Err(refuse_at(
                &edge.to,
                &edge.pos,
                "unsupported-dependency",
                format!(
                    "a {} cannot depend on {:?}: Compose creates every network, volume, config \
                     and secret before it starts a service, and orders nothing among them",
                    to.as_str(),
                    edge.from
                ),
            ));
        }
        if from != Kind::Service {
            continue;
        }
        let depends_on = &mut project
            .entries
            .iter_mut()
            .find(|e| e.id == edge.to)
            .expect("the entry was found above")
            .depends_on;
        match depends_on.iter_mut().find(|d| d.service == edge.from) {
            Some(written) => written.restart |= restart,
            None => depends_on.push(Dependency {
                service: edge.from.clone(),
                condition: String::new(),
                restart,
                required: None,
            }),
        }
    }

    // A dependency with no condition waits for the other service to be
    // healthy when it can say so, and to have started when it cannot.
    let healthy: Vec<String> = project
        .of_kind(Kind::Service)
        .filter(|e| e.has_healthcheck())
        .map(|e| e.id.clone())
        .collect();
    for entry in &mut project.entries {
        for dependency in &mut entry.depends_on {
            if dependency.condition.is_empty() {
                dependency.condition = if healthy.contains(&dependency.service) {
                    "service_healthy"
                } else {
                    "service_started"
                }
                .to_owned();
            }
        }
        entry.depends_on.sort_by(|a, b| a.service.cmp(&b.service));
    }
    Ok(())
}
