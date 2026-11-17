use meridian_ast::{Edge, EdgeKind, Error, Resource, ResourceGraph};
use meridian_ir::{
    parse_value, Bundle, Dependency, Document, Infrastructure, InfrastructureSpec, Metadata,
    Notification, Output, Position, ResourceSet, ResourceSetSpec, TargetHost, Value,
    API_VERSION_V1, TARGET_HOST_FROM_INFRASTRUCTURE,
};

type IrResource = meridian_ir::Resource;

/// `"id:type"` with optional `dependsOn` ids and `(resource, action)` notifies.
fn r(spec: &str, depends_on: &[&str], notifies: &[(&str, &str)]) -> IrResource {
    let (id, ty) = spec.split_once(':').unwrap();
    IrResource {
        id: id.to_owned(),
        r#type: ty.to_owned(),
        depends_on: depends_on
            .iter()
            .map(|d| Dependency {
                resource: (*d).to_owned(),
                ..Dependency::default()
            })
            .collect(),
        notifies: notifies
            .iter()
            .map(|(res, action)| Notification {
                resource: (*res).to_owned(),
                action: (*action).to_owned(),
                ..Notification::default()
            })
            .collect(),
        ..IrResource::default()
    }
}

fn with_params(mut res: IrResource, yaml: &str) -> IrResource {
    match parse_value("params", yaml.as_bytes()).unwrap() {
        Value::Map(m) => res.params = m,
        _ => panic!("params must be a mapping"),
    }
    res
}

fn resource_set(mut resources: Vec<IrResource>) -> ResourceSet {
    for (i, res) in resources.iter_mut().enumerate() {
        res.index = i;
    }
    ResourceSet {
        api_version: API_VERSION_V1.to_owned(),
        metadata: Metadata {
            name: "web".to_owned(),
            ..Metadata::default()
        },
        spec: ResourceSetSpec {
            target: "ansible".to_owned(),
            resources,
            ..ResourceSetSpec::default()
        },
        pos: Position::default(),
    }
}

fn build(resources: Vec<IrResource>) -> ResourceGraph {
    ResourceGraph::build(&resource_set(resources), "web01").unwrap_or_else(|e| panic!("build: {e}"))
}

fn build_err(resources: Vec<IrResource>) -> String {
    ResourceGraph::build(&resource_set(resources), "web01")
        .expect_err("want an error")
        .to_string()
}

#[test]
fn build_orders_resources_topologically() {
    let g = build(vec![
        r("svc:service", &["pkg"], &[]),
        r("pkg:package", &[], &[]),
    ]);
    assert_eq!(g.order(), ["pkg", "svc"]);
    // The declaration index survives the reordering.
    assert_eq!(g.resource("pkg").map(|r| r.index), Some(1));
}

#[test]
fn build_rejects_an_unevaluated_when() {
    let mut res = r("pkg:package", &[], &[]);
    res.when = "data.install".to_owned();
    res.pos = Position {
        file: "site.yaml".to_owned(),
        doc_idx: 0,
        line: 7,
        column: 7,
    };
    assert_eq!(
        build_err(vec![res]),
        "site.yaml:7:7: resource \"pkg\" still carries an unevaluated `when` expression; \
         the condition stage must prune before an AST is built"
    );
}

#[test]
fn build_keeps_runtime_when() {
    let mut res = r("pkg:package", &[], &[]);
    res.runtime_when = "ansible_os_family == 'Debian'".to_owned();
    let g = build(vec![res]);
    assert_eq!(
        g.resource("pkg").unwrap().runtime_when,
        "ansible_os_family == 'Debian'"
    );
}

#[test]
fn build_requires_a_host() {
    let err =
        ResourceGraph::build(&resource_set(vec![r("pkg:package", &[], &[])]), "").unwrap_err();
    assert_eq!(
        err.to_string(),
        r#"document "web" has no resolved target host"#
    );
}

#[test]
fn build_propagates_graph_failures_and_cycles() {
    let err = build_err(vec![r("a:package", &["b"], &[])]);
    assert_eq!(err, r#"resource "a" depends on unknown resource "b""#);
    let err = build_err(vec![
        r("a:package", &["b"], &[]),
        r("b:package", &["a"], &[]),
    ]);
    assert_eq!(err, "dependency cycle among 2 resources: a -> b -> a");
}

#[test]
fn build_keeps_both_edge_kinds() {
    let g = build(vec![
        r("conf:file", &["pkg"], &[("svc", "restart")]),
        r("pkg:package", &[], &[]),
        r("svc:service", &[], &[]),
    ]);
    let hard: Vec<_> = g.edges_of_kind(EdgeKind::HardOrder).collect();
    assert_eq!(hard.len(), 1);
    assert_eq!(
        (hard[0].from.as_str(), hard[0].to.as_str()),
        ("pkg", "conf")
    );
    let notify: Vec<_> = g.edges_of_kind(EdgeKind::Notify).collect();
    assert_eq!(notify.len(), 1);
    assert_eq!(
        (
            notify[0].from.as_str(),
            notify[0].to.as_str(),
            notify[0].action.as_str()
        ),
        ("conf", "svc", "restart")
    );
    assert_eq!(g.edges().len(), 2);
}

#[test]
fn notify_edges_do_not_constrain_order() {
    let g = build(vec![
        r("conf:file", &[], &[("svc", "restart")]),
        r("svc:service", &[], &[]),
    ]);
    assert_eq!(g.order(), ["conf", "svc"]);
}

#[test]
fn lookup_of_an_unknown_id_finds_nothing() {
    let g = build(vec![r("pkg:package", &[], &[])]);
    assert!(g.resource("nope").is_none());
    assert!(g.edges().is_empty());
}

#[test]
fn edge_kind_names() {
    assert_eq!(EdgeKind::HardOrder.to_string(), "dependsOn");
    assert_eq!(EdgeKind::Notify.to_string(), "notifies");
}

fn canonical_fixture() -> Vec<IrResource> {
    vec![
        with_params(
            r("conf:file", &["pkg"], &[("svc", "restart")]),
            r#"{path: /etc/n.conf, mode: "0644", owner: root}"#,
        ),
        with_params(r("pkg:package", &[], &[]), "{name: nginx}"),
        with_params(
            IrResource {
                state: "running".to_owned(),
                ..r("svc:service", &[], &[])
            },
            "{name: nginx}",
        ),
    ]
}

/// The expected text is the Go implementation's `Canonical()` for the same
/// document.
#[test]
fn canonical_matches_the_go_implementation_byte_for_byte() {
    assert_eq!(
        build(canonical_fixture()).canonical(),
        include_str!("golden/canonical.txt")
    );
}

#[test]
fn canonical_is_stable_across_rebuilds_and_threads() {
    let want = build(canonical_fixture()).canonical();
    for run in 0..100 {
        assert_eq!(
            build(canonical_fixture()).canonical(),
            want,
            "run {run} differed"
        );
    }
    let workers: Vec<_> = (0..16)
        .map(|_| std::thread::spawn(|| build(canonical_fixture()).canonical()))
        .collect();
    for (i, worker) in workers.into_iter().enumerate() {
        assert_eq!(worker.join().unwrap(), want, "thread {i} differed");
    }
}

#[test]
fn build_from_bundle_resolves_an_infrastructure_output() {
    let infra = Infrastructure {
        api_version: API_VERSION_V1.to_owned(),
        metadata: Metadata {
            name: "net".to_owned(),
            ..Metadata::default()
        },
        spec: InfrastructureSpec {
            target: "terraform".to_owned(),
            resources: vec![r("vm:vm", &[], &[])],
            outputs: vec![Output {
                name: "ip".to_owned(),
                value: "10.0.0.7".to_owned(),
                ..Output::default()
            }],
            ..InfrastructureSpec::default()
        },
        pos: Position::default(),
    };
    let mut rs = resource_set(vec![r("pkg:package", &[], &[])]);
    rs.spec.target_host = Some(TargetHost {
        from: TARGET_HOST_FROM_INFRASTRUCTURE.to_owned(),
        r#ref: "net".to_owned(),
        output: "ip".to_owned(),
        ..TargetHost::default()
    });
    let bundle = Bundle::new([Document::from(infra), Document::from(rs.clone())]);
    let g = ResourceGraph::build_from_bundle(&bundle, &rs).unwrap();
    assert_eq!(
        g.host, "10.0.0.7",
        "the host is the resolved output literal"
    );

    rs.spec.target_host.as_mut().unwrap().r#ref = "missing".to_owned();
    let err = ResourceGraph::build_from_bundle(&Bundle::new([Document::from(rs.clone())]), &rs)
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        r#"<input>: spec.targetHost.ref: no Infrastructure document named "missing" is loaded"#
    );
}

#[test]
fn new_assembles_a_tree_verbatim() {
    let res = |id: &str, index| Resource {
        id: id.to_owned(),
        index,
        ..Resource::default()
    };
    let g = ResourceGraph::new(
        "web",
        "web01",
        "ansible",
        // The third repeats an id and is dropped.
        vec![res("svc", 0), res("pkg", 1), res("svc", 2)],
        vec![
            Edge::new("pkg", "svc", EdgeKind::HardOrder, ""),
            Edge::new("conf", "svc", EdgeKind::Notify, "restart"),
        ],
    );
    assert_eq!(g.order(), ["svc", "pkg"], "the caller's own order");
    assert_eq!(g.resource("svc").map(|r| r.index), Some(0));
    assert_eq!(
        (g.name.as_str(), g.host.as_str(), g.target.as_str()),
        ("web", "web01", "ansible")
    );
    assert_eq!(g.edges_of_kind(EdgeKind::HardOrder).count(), 1);
    assert_eq!(g.edges_of_kind(EdgeKind::Notify).count(), 1);
}

#[test]
fn edges_are_ordered_deterministically() {
    let edges = vec![
        Edge::new("b", "a", EdgeKind::Notify, "restart"),
        Edge::new("a", "z", EdgeKind::HardOrder, ""),
        Edge::new("b", "a", EdgeKind::Notify, "reload"),
        Edge::new("a", "b", EdgeKind::HardOrder, ""),
        Edge::new("b", "a", EdgeKind::HardOrder, ""),
    ];
    let reversed: Vec<Edge> = edges.iter().rev().cloned().collect();
    let first = ResourceGraph::new("n", "h", "t", Vec::new(), edges);
    let second = ResourceGraph::new("n", "h", "t", Vec::new(), reversed);
    assert_eq!(first.edges(), second.edges());

    let listed: Vec<String> = first
        .edges()
        .iter()
        .map(|e| format!("{}>{} {} {}", e.from, e.to, e.kind, e.action))
        .collect();
    assert_eq!(
        listed,
        [
            "a>b dependsOn ",
            "a>z dependsOn ",
            "b>a dependsOn ",
            "b>a notifies reload",
            "b>a notifies restart"
        ]
    );
}

#[test]
fn error_formatting() {
    assert_eq!(
        Error {
            msg: "wrapped".to_owned(),
            ..Error::default()
        }
        .to_string(),
        "wrapped"
    );
    let located = Error {
        msg: "wrapped".to_owned(),
        pos: Position {
            file: "site.yaml".to_owned(),
            doc_idx: 0,
            line: 3,
            column: 0,
        },
    };
    assert_eq!(located.to_string(), "site.yaml:3: wrapped");
}

#[test]
fn an_infrastructure_document_builds_a_tree_with_its_outputs_and_no_host() {
    let docs = meridian_ir::parse(
        "infra.yaml",
        br#"apiVersion: meridian/v1
kind: Infrastructure
metadata:
  name: web-vm
spec:
  target: terraform
  resources:
    - id: web_vm
      type: virtual_machine
      params:
        size: small
      dependsOn:
        - resource: web_net
    - id: web_net
      type: network
  outputs:
    - name: web_vm_ip
      value: "${web_vm.public_ip}"
"#,
    )
    .unwrap();
    let Document::Infrastructure(doc) = &docs[0] else {
        panic!("expected an Infrastructure document");
    };
    let g = ResourceGraph::build_infrastructure(doc).unwrap();
    assert_eq!(
        (g.name.as_str(), g.target.as_str(), g.host.as_str()),
        ("web-vm", "terraform", "")
    );
    assert_eq!(g.order(), ["web_net", "web_vm"]);
    assert_eq!(g.outputs.len(), 1);
    assert_eq!(g.outputs[0].name, "web_vm_ip");
    assert_eq!(g.outputs[0].value, "${web_vm.public_ip}");
    assert_eq!(g.outputs[0].pos.line, 17);
}
