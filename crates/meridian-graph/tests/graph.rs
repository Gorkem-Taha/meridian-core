use meridian_graph::{EdgeKind, Error, Graph};
use meridian_ir::{parse, validate, Dependency, Notification, Position, Resource};

/// A resource with `dependsOn` edges, in the shape the parser produces.
fn res(id: &str, depends_on: &[&str]) -> Resource {
    Resource {
        id: id.to_owned(),
        r#type: "package".to_owned(),
        depends_on: depends_on
            .iter()
            .map(|d| Dependency {
                resource: (*d).to_owned(),
                ..Dependency::default()
            })
            .collect(),
        ..Resource::default()
    }
}

fn notifier(id: &str, notifies: &str) -> Resource {
    Resource {
        id: id.to_owned(),
        r#type: "service".to_owned(),
        notifies: vec![Notification {
            resource: notifies.to_owned(),
            action: "restart".to_owned(),
            ..Notification::default()
        }],
        ..Resource::default()
    }
}

fn build(resources: &[Resource]) -> Graph {
    Graph::build(resources).unwrap_or_else(|e| panic!("build: {e}"))
}

/// `"a:b,c"` is resource `a` depending on `b` and `c`.
fn graph(spec: &[&str]) -> Graph {
    let resources: Vec<Resource> = spec
        .iter()
        .map(|s| match s.split_once(':') {
            Some((id, deps)) => res(id, &deps.split(',').collect::<Vec<_>>()),
            None => res(s, &[]),
        })
        .collect();
    build(&resources)
}

#[test]
fn topo_sort_respects_dependencies() {
    let g = graph(&[
        "nginx_conf:nginx_pkg",
        "nginx_svc:nginx_pkg,nginx_conf",
        "nginx_pkg",
    ]);
    assert_eq!(
        g.topo_sort().unwrap(),
        ["nginx_pkg", "nginx_conf", "nginx_svc"]
    );
}

#[test]
fn topo_sort_tiebreaks_by_declaration_index() {
    // Ten resources with no edges at all: only the declaration index can
    // decide their order. The ids run counter to it on purpose.
    let ids: Vec<String> = (0..10).map(|i| format!("r{:02}", 9 - i)).collect();
    let resources: Vec<Resource> = ids.iter().map(|id| res(id, &[])).collect();
    let g = build(&resources);
    for run in 0..50 {
        assert_eq!(
            g.topo_sort().unwrap(),
            ids,
            "run {run}: declaration order, not id order"
        );
    }
}

/// The orders and cycle messages below are what the Go implementation
/// produced for the same graphs.
#[test]
fn topo_sort_matches_the_go_implementation() {
    let g = graph(&["e:d", "d", "c:a", "b", "a:b", "f:e,c"]);
    assert_eq!(g.topo_sort().unwrap(), ["d", "e", "b", "a", "c", "f"]);
    let nodes: Vec<&str> = g
        .topo_sort_nodes()
        .unwrap()
        .iter()
        .map(|n| n.id.as_str())
        .collect();
    assert_eq!(nodes, g.topo_sort().unwrap());

    for (spec, want) in [
        (
            &["a:c", "b:a", "c:b", "unrelated"][..],
            "dependency cycle among 3 resources: a -> b -> c -> a",
        ),
        (
            &["a:c", "b:a", "c:b", "x:y", "y:x"],
            "dependency cycle among 3 resources: a -> b -> c -> a",
        ),
        // Resources that only lead into a cycle, or hang off it, are left out.
        (
            &["p", "q:p", "a:b,q", "b:c", "c:d,a", "d:b"],
            "dependency cycle among 3 resources: a -> c -> b -> a",
        ),
        (
            &["a:b", "b:a"],
            "dependency cycle among 2 resources: a -> b -> a",
        ),
        (
            &["z", "m:n,z", "n:o", "o:m", "k:m"],
            "dependency cycle among 3 resources: m -> o -> n -> m",
        ),
    ] {
        for run in 0..20 {
            let err = graph(spec).topo_sort().expect_err("a cycle");
            assert_eq!(err.to_string(), want, "{spec:?} run {run}");
            assert_eq!(
                err.cycle.first(),
                err.cycle.last(),
                "the cycle closes where it opened"
            );
        }
    }
}

#[test]
fn a_very_long_chain_sorts_and_a_very_long_cycle_is_traced() {
    let n = 200_000;
    let mut resources: Vec<Resource> = (0..n)
        .map(|i| {
            if i == 0 {
                res("r0", &[])
            } else {
                res(&format!("r{i}"), &[&format!("r{}", i - 1)])
            }
        })
        .collect();
    assert_eq!(build(&resources).topo_sort().unwrap().len(), n);
    // Close the chain into one cycle through every resource.
    resources[0] = res("r0", &[&format!("r{}", n - 1)]);
    let err = build(&resources).topo_sort().unwrap_err();
    assert_eq!(err.cycle.len(), n + 1);
}

#[test]
fn self_reference_is_rejected_at_build_time() {
    let err = Graph::build(&[res("a", &["a"])]).expect_err("not a cycle error at sort time");
    assert_eq!(err.to_string(), r#"resource "a" depends on itself"#);
    let err = Graph::build(&[notifier("a", "a")]).unwrap_err();
    assert_eq!(err.to_string(), r#"resource "a" notifies itself"#);
}

#[test]
fn build_rejects_unknown_and_duplicate_ids() {
    for (name, resources, want) in [
        (
            "unknown dependsOn",
            vec![res("a", &["ghost"])],
            r#"resource "a" depends on unknown resource "ghost""#,
        ),
        (
            "duplicate id",
            vec![res("a", &[]), res("a", &[])],
            r#"duplicate resource id "a""#,
        ),
        (
            "empty id",
            vec![res("", &[])],
            "resource at declaration index 0 has no id",
        ),
        (
            "unknown notifies",
            vec![notifier("a", "ghost")],
            r#"resource "a" notifies unknown resource "ghost""#,
        ),
    ] {
        let err = Graph::build(&resources).expect_err(name);
        assert_eq!(err.to_string(), want, "{name}");
    }
}

/// The case the design plan calls out: `dependsOn` and `notifies` between one
/// pair of resources are two different relations and must not collapse into
/// one edge.
#[test]
fn edge_kinds_are_distinguished_between_the_same_pair() {
    let mut conf = notifier("nginx_conf", "nginx_svc");
    conf.depends_on = vec![Dependency {
        resource: "nginx_svc".to_owned(),
        ..Dependency::default()
    }];
    let g = build(&[res("nginx_svc", &[]), conf]);

    assert_eq!(g.edges().len(), 2);
    let hard: Vec<_> = g.edges_of_kind(EdgeKind::HardOrder).collect();
    assert_eq!(hard.len(), 1);
    assert_eq!(
        (hard[0].from.as_str(), hard[0].to.as_str()),
        ("nginx_svc", "nginx_conf")
    );
    let notify: Vec<_> = g.edges_of_kind(EdgeKind::Notify).collect();
    assert_eq!(notify.len(), 1);
    assert_eq!(
        (notify[0].from.as_str(), notify[0].to.as_str()),
        ("nginx_conf", "nginx_svc")
    );
    assert_eq!(notify[0].action, "restart");

    let kinds = |from, to| {
        g.edges_between(from, to)
            .map(|e| e.kind)
            .collect::<Vec<_>>()
    };
    assert_eq!(kinds("nginx_conf", "nginx_svc"), [EdgeKind::Notify]);
    assert_eq!(kinds("nginx_svc", "nginx_conf"), [EdgeKind::HardOrder]);
}

/// `notifies` is a deferred trigger, so a notification loop must still sort.
#[test]
fn notify_edges_do_not_constrain_order() {
    let g = build(&[notifier("a", "b"), notifier("b", "a")]);
    assert_eq!(g.topo_sort().unwrap(), ["a", "b"]);
}

#[test]
fn edge_kind_names() {
    assert_eq!(EdgeKind::HardOrder.to_string(), "dependsOn");
    assert_eq!(EdgeKind::Notify.to_string(), "notifies");
}

#[test]
fn dependencies_and_dependents_are_in_declaration_order() {
    let g = graph(&["a", "b", "c:b,a,b", "d:a"]);
    assert_eq!(g.dependencies("c"), ["a", "b"]);
    assert_eq!(g.dependents("a"), ["c", "d"]);
    assert!(g.dependencies("a").is_empty());
    assert!(g.dependents("nope").is_empty());
}

#[test]
fn node_lookup() {
    let g = graph(&["a", "b"]);
    assert_eq!(g.node("b").map(|n| n.index), Some(1));
    assert!(g.node("zzz").is_none());
    assert_eq!(g.nodes().len(), 2);
}

#[test]
fn a_graph_built_from_a_document_keeps_source_positions() {
    let docs = parse(
        "rs.yaml",
        b"apiVersion: meridian/v1\nkind: ResourceSet\nmetadata: {name: web}\nspec:\n  target: ansible\n  resources:\n    - {id: nginx_pkg, type: package}\n    - id: nginx_conf\n      type: file\n      dependsOn: [{resource: nginx_pkg}]\n      notifies: [{resource: nginx_svc, on: changed, action: restart}]\n    - id: nginx_svc\n      type: service\n      dependsOn: [{resource: nginx_pkg}]\n",
    )
    .unwrap();
    validate(&docs[0]).unwrap();
    let g = Graph::from_document(&docs[0]).unwrap();
    assert_eq!(
        g.topo_sort().unwrap(),
        ["nginx_pkg", "nginx_conf", "nginx_svc"]
    );
    // Positions survive from the parser through graph construction, so a later
    // stage can point at the source line that declared an edge.
    let notify: Vec<_> = g.edges_of_kind(EdgeKind::Notify).collect();
    assert_eq!(notify.len(), 1);
    assert_eq!((notify[0].pos.line, notify[0].pos.column), (11, 18));
}

#[test]
fn a_cycle_error_points_at_the_resource_that_opens_it() {
    let docs = parse(
        "rs.yaml",
        b"apiVersion: meridian/v1\nkind: ResourceSet\nmetadata: {name: web}\nspec:\n  target: ansible\n  resources:\n    - {id: a, type: t, dependsOn: [{resource: b}]}\n    - {id: b, type: t, dependsOn: [{resource: a}]}\n",
    )
    .unwrap();
    let err = Graph::from_document(&docs[0])
        .unwrap()
        .topo_sort()
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "rs.yaml:7:7: dependency cycle among 2 resources: a -> b -> a"
    );
}

#[test]
fn error_formatting() {
    let e = Error {
        msg: "boom".to_owned(),
        pos: Position {
            file: "a.yaml".to_owned(),
            doc_idx: 0,
            line: 3,
            column: 5,
        },
        cycle: vec!["a".into(), "b".into(), "a".into()],
    };
    assert_eq!(e.to_string(), "a.yaml:3:5: boom: a -> b -> a");
    assert_eq!(
        Error {
            msg: "plain".to_owned(),
            ..Error::default()
        }
        .to_string(),
        "plain"
    );
}
