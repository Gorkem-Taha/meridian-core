//! The only place the protocol's shapes and this crate's shapes meet.
//!
//! Keeping the conversion in one module means a protocol change has exactly
//! one place to break, and neither an emitter nor a host ever handles a
//! generated type.

use meridian_value::{canonical_map, decode_map};

use crate::pb;
use crate::types::*;

fn to_i32(n: usize) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

fn from_i32(n: i32) -> usize {
    usize::try_from(n).unwrap_or(0)
}

fn position_to_wire(p: &Position) -> pb::Position {
    pb::Position {
        file: p.file.clone(),
        doc_idx: to_i32(p.doc_idx),
        line: to_i32(p.line),
        column: to_i32(p.column),
    }
}

fn position_from_wire(p: Option<pb::Position>) -> Position {
    p.map(|p| Position {
        file: p.file,
        doc_idx: from_i32(p.doc_idx),
        line: from_i32(p.line),
        column: from_i32(p.column),
    })
    .unwrap_or_default()
}

/// Converts a resource tree into its protocol form.
pub fn graph_to_wire(g: &ResourceGraph) -> pb::ResourceGraph {
    pb::ResourceGraph {
        name: g.name.clone(),
        host: g.host.clone(),
        target: g.target.clone(),
        resources: g
            .resources
            .iter()
            .map(|r| pb::Resource {
                id: r.id.clone(),
                r#type: r.r#type.clone(),
                state: r.state.clone(),
                params_json: canonical_map(&r.params).into_bytes(),
                runtime_when: r.runtime_when.clone(),
                index: to_i32(r.index),
                pos: Some(position_to_wire(&r.pos)),
            })
            .collect(),
        edges: g
            .edges
            .iter()
            .map(|e| pb::Edge {
                from: e.from.clone(),
                to: e.to.clone(),
                kind: match e.kind {
                    EdgeKind::HardOrder => pb::EdgeKind::HardOrder,
                    EdgeKind::Notify => pb::EdgeKind::Notify,
                } as i32,
                action: e.action.clone(),
                pos: Some(position_to_wire(&e.pos)),
            })
            .collect(),
    }
}

/// Converts a protocol tree back into this crate's own types.
///
/// An edge of unspecified kind is refused, not defaulted: guessing between an
/// ordering constraint and a deferred trigger would change what the document
/// means.
pub fn graph_from_wire(g: pb::ResourceGraph) -> Result<ResourceGraph, String> {
    let mut out = ResourceGraph {
        name: g.name,
        host: g.host,
        target: g.target,
        ..ResourceGraph::default()
    };
    for r in g.resources {
        let params = decode_map(&r.params_json).map_err(|e| format!("resource {:?}: {e}", r.id))?;
        out.resources.push(Resource {
            id: r.id,
            r#type: r.r#type,
            state: r.state,
            params,
            runtime_when: r.runtime_when,
            index: from_i32(r.index),
            pos: position_from_wire(r.pos),
        });
    }
    for e in g.edges {
        let kind = match pb::EdgeKind::try_from(e.kind) {
            Ok(pb::EdgeKind::HardOrder) => EdgeKind::HardOrder,
            Ok(pb::EdgeKind::Notify) => EdgeKind::Notify,
            _ => {
                return Err(format!(
                    "edge {} -> {}: edge kind {} is not a kind this protocol version defines",
                    e.from, e.to, e.kind
                ))
            }
        };
        out.edges.push(Edge {
            from: e.from,
            to: e.to,
            kind,
            action: e.action,
            pos: position_from_wire(e.pos),
        });
    }
    Ok(out)
}

pub fn capabilities_to_wire(c: Capabilities) -> pb::Capabilities {
    pb::Capabilities {
        native_notify: c.native_notify,
        synthetic_notify: c.synthetic_notify,
        runtime_condition: c.runtime_condition,
    }
}

pub fn capabilities_from_wire(c: Option<pb::Capabilities>) -> Capabilities {
    c.map(|c| Capabilities {
        native_notify: c.native_notify,
        synthetic_notify: c.synthetic_notify,
        runtime_condition: c.runtime_condition,
    })
    .unwrap_or_default()
}

/// Builds the protocol request carrying a tree and its resolved data.
pub fn emit_request(g: &ResourceGraph, data: &Data) -> pb::EmitRequest {
    pb::EmitRequest {
        graph: Some(graph_to_wire(g)),
        data_json: canonical_map(data).into_bytes(),
    }
}

/// Reads a protocol request back into a tree and its resolved data.
pub fn emit_request_from_wire(req: pb::EmitRequest) -> Result<(ResourceGraph, Data), String> {
    let graph = graph_from_wire(req.graph.ok_or("received no resource graph")?)
        .map_err(|e| format!("reading the resource graph: {e}"))?;
    let data = decode_map(&req.data_json).map_err(|e| format!("reading the resolved data: {e}"))?;
    Ok((graph, data))
}

/// Builds the protocol response for the outcome of a compile.
pub fn emit_response(outcome: Result<Emitted, CompileError>) -> pb::EmitResponse {
    match outcome {
        Ok(emitted) => pb::EmitResponse {
            artifact: Some(pb::Artifact {
                files: emitted.artifact.files.into_iter().collect(),
            }),
            warnings: emitted
                .warnings
                .into_iter()
                .map(|w| pb::Warning {
                    target: w.target,
                    resource: w.resource,
                    msg: w.msg,
                })
                .collect(),
            error: None,
        },
        Err(e) => pb::EmitResponse {
            artifact: None,
            warnings: Vec::new(),
            error: Some(pb::CompileError {
                resource: e.resource,
                rule: e.rule,
                msg: e.msg,
                pos: Some(position_to_wire(&e.pos)),
            }),
        },
    }
}

/// Converts a protocol response back into this crate's values. A refusal
/// carried in the response becomes a [`CompileError`] stamped with the target
/// that raised it.
pub fn emit_response_from_wire(
    resp: pb::EmitResponse,
    target: &str,
) -> Result<Emitted, CompileError> {
    if let Some(e) = resp.error {
        return Err(CompileError {
            target: target.to_owned(),
            resource: e.resource,
            rule: e.rule,
            msg: e.msg,
            pos: position_from_wire(e.pos),
        });
    }
    Ok(Emitted {
        artifact: Artifact {
            files: resp
                .artifact
                .map(|a| a.files.into_iter().collect())
                .unwrap_or_default(),
        },
        warnings: resp
            .warnings
            .into_iter()
            .map(|w| Warning {
                target: w.target,
                resource: w.resource,
                msg: w.msg,
            })
            .collect(),
    })
}
