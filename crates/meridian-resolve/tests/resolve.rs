use std::collections::BTreeMap;

use meridian_ir::{canonical_map, parse_value, Value};
use meridian_resolve::*;

/// A value written as YAML flow syntax, which reads close to a literal.
fn v(yaml: &str) -> Value {
    parse_value("literal", yaml.as_bytes()).unwrap_or_else(|e| panic!("bad literal {yaml:?}: {e}"))
}

fn data(yaml: &str) -> Data {
    match v(yaml) {
        Value::Map(m) => m,
        other => panic!("{yaml:?} is {}, not a mapping", other.describe()),
    }
}

fn loader(layers: &[(&str, &str)]) -> MapLoader {
    MapLoader(
        layers
            .iter()
            .map(|(path, yaml)| ((*path).to_owned(), data(yaml)))
            .collect(),
    )
}

fn standard_hierarchy() -> Hierarchy {
    Hierarchy::new([
        "common.yaml",
        "environment/%{env}.yaml",
        "role/%{role}.yaml",
        "node/%{node}.yaml",
    ])
}

fn scope(pairs: &[(&str, &str)]) -> Scope {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

fn standard_scope() -> Scope {
    scope(&[("env", "prod"), ("role", "webserver"), ("node", "web01")])
}

fn with_keys(rules: &[(&str, KeyRule)]) -> Hierarchy {
    let mut h = standard_hierarchy();
    h.keys = rules
        .iter()
        .map(|(k, r)| ((*k).to_owned(), r.clone()))
        .collect();
    h
}

fn expand_err(h: &Hierarchy, s: &Scope) -> String {
    h.expand_paths(s).expect_err("want an error").to_string()
}

#[test]
fn expand_paths_interpolates() {
    let paths = standard_hierarchy()
        .expand_paths(&standard_scope())
        .unwrap();
    assert_eq!(
        paths,
        [
            "common.yaml",
            "environment/prod.yaml",
            "role/webserver.yaml",
            "node/web01.yaml"
        ]
    );
}

#[test]
fn expand_paths_rejects_an_undefined_variable() {
    let err = expand_err(&standard_hierarchy(), &scope(&[("env", "prod")]));
    assert_eq!(
        err,
        r#"hierarchy[2] "role/%{role}.yaml": undefined variable %{role}"#
    );
}

#[test]
fn expand_paths_detects_a_circular_reference() {
    let h = Hierarchy::new(["%{a}.yaml"]);
    let err = expand_err(&h, &scope(&[("a", "%{b}"), ("b", "%{a}")]));
    assert!(
        err.contains("circular hierarchy reference: a -> b -> a"),
        "{err}"
    );
    let err = expand_err(&h, &scope(&[("a", "%{a}")]));
    assert!(
        err.contains("circular hierarchy reference: a -> a"),
        "{err}"
    );
}

#[test]
fn expand_paths_bounds_a_chain_that_never_repeats() {
    let mut s = Scope::new();
    for i in 0..40 {
        s.insert(format!("v{i}"), format!("%{{v{}}}", i + 1));
    }
    s.insert("v40".to_owned(), "end".to_owned());
    let err = expand_err(&Hierarchy::new(["%{v0}.yaml"]), &s);
    assert!(
        err.contains("interpolation nested more than 32 levels deep"),
        "{err}"
    );
}

#[test]
fn expand_paths_rejects_a_duplicate_layer() {
    let h = Hierarchy::new(["role/%{role}.yaml", "node/%{node}.yaml"]);
    let err = expand_err(&h, &scope(&[("role", "x"), ("node", "../role/x")]));
    assert!(err.contains("cannot appear twice"), "{err}");
    assert!(
        err.contains(r#"resolves to "role/x.yaml", which hierarchy[0] already resolves to"#),
        "{err}"
    );
}

#[test]
fn expand_paths_rejects_malformed_interpolation() {
    for (path, want) in [
        ("role/%{role.yaml", "unterminated %{ at offset 5"),
        ("role/%{}.yaml", "empty interpolation"),
    ] {
        let err = expand_err(&Hierarchy::new([path]), &scope(&[("role", "x")]));
        assert!(
            err.contains(want),
            "path {path:?}: error = {err}, want it to contain {want:?}"
        );
    }
}

#[test]
fn first_found_takes_the_highest_priority_layer() {
    let l = loader(&[
        (
            "common.yaml",
            "{ntp_server: pool.ntp.org, only_common: kept}",
        ),
        ("environment/prod.yaml", "{ntp_server: prod.ntp.internal}"),
        ("role/webserver.yaml", "{ntp_server: role.ntp.internal}"),
        ("node/web01.yaml", "{ntp_server: web01.ntp.internal}"),
    ]);
    let res = resolve(&standard_hierarchy(), &standard_scope(), &l).unwrap();
    assert_eq!(res.data["ntp_server"], Value::from("web01.ntp.internal"));
    assert_eq!(res.data["only_common"], Value::from("kept"));
}

#[test]
fn precedence_is_node_role_environment_common() {
    let mut l = loader(&[
        ("common.yaml", "{who: common}"),
        ("environment/prod.yaml", "{who: environment}"),
        ("role/webserver.yaml", "{who: role}"),
        ("node/web01.yaml", "{who: node}"),
    ]);
    for (drop, want) in [
        ("", "node"),
        ("node/web01.yaml", "role"),
        ("role/webserver.yaml", "environment"),
        ("environment/prod.yaml", "common"),
    ] {
        l.0.remove(drop);
        let res = resolve(&standard_hierarchy(), &standard_scope(), &l).unwrap();
        assert_eq!(
            res.data["who"],
            Value::from(want),
            "after dropping {drop:?}"
        );
    }
}

#[test]
fn deep_merge_preserves_sibling_keys() {
    let h = with_keys(&[("nginx", KeyRule::new(Strategy::DeepMerge))]);
    let l = loader(&[
        (
            "common.yaml",
            "{nginx: {worker_processes: 2, limits: {nofile: 1024, nproc: 512}, listen: 80}}",
        ),
        ("node/web01.yaml", "{nginx: {limits: {nofile: 65535}}}"),
    ]);
    let res = resolve(&h, &standard_scope(), &l).unwrap();
    assert_eq!(
        res.data["nginx"],
        v("{worker_processes: 2, limits: {nofile: 65535, nproc: 512}, listen: 80}")
    );
}

#[test]
fn deep_merge_replaces_when_either_side_is_not_a_map() {
    let h = with_keys(&[("cfg", KeyRule::new(Strategy::DeepMerge))]);
    let l = loader(&[
        ("common.yaml", "{cfg: {a: {x: 1}, b: [1, 2], c: {keep: 1}}}"),
        ("node/web01.yaml", "{cfg: {a: scalar, b: [3], c: {add: 2}}}"),
    ]);
    let res = resolve(&h, &standard_scope(), &l).unwrap();
    assert_eq!(
        res.data["cfg"],
        v("{a: scalar, b: [3], c: {keep: 1, add: 2}}")
    );
}

#[test]
fn unique_array_merge_of_primitives() {
    let h = with_keys(&[("packages", KeyRule::new(Strategy::UniqueArrayMerge))]);
    let l = loader(&[
        ("common.yaml", "{packages: [curl, vim]}"),
        ("environment/prod.yaml", "{packages: [vim, htop]}"),
        ("node/web01.yaml", "{packages: [curl, nginx]}"),
    ]);
    let res = resolve(&h, &standard_scope(), &l).unwrap();
    assert_eq!(res.data["packages"], v("[curl, vim, htop, nginx]"));
}

#[test]
fn unique_array_merge_of_objects_with_a_dedup_key() {
    let h = with_keys(&[(
        "users",
        KeyRule::new(Strategy::UniqueArrayMerge).with_dedup_key("name"),
    )]);
    let l = loader(&[
        (
            "common.yaml",
            "{users: [{name: deploy, shell: /bin/sh}, {name: ops, shell: /bin/bash}]}",
        ),
        (
            "node/web01.yaml",
            "{users: [{name: deploy, shell: /bin/bash}, {name: app, shell: /bin/false}, plain]}",
        ),
    ]);
    let res = resolve(&h, &standard_scope(), &l).unwrap();
    // The common layer's deploy entry is kept; the node layer's is the duplicate.
    assert_eq!(
        res.data["users"],
        v("[{name: deploy, shell: /bin/sh}, {name: ops, shell: /bin/bash}, {name: app, shell: /bin/false}, plain]")
    );
}

#[test]
fn unique_array_merge_of_objects_without_a_dedup_key() {
    let h = with_keys(&[("users", KeyRule::new(Strategy::UniqueArrayMerge))]);
    let l = loader(&[
        ("common.yaml", "{users: [{name: deploy, shell: /bin/sh}]}"),
        // The first is the same object with its keys in another order.
        (
            "node/web01.yaml",
            "{users: [{shell: /bin/sh, name: deploy}, {name: deploy, shell: /bin/bash}]}",
        ),
    ]);
    let res = resolve(&h, &standard_scope(), &l).unwrap();
    assert_eq!(res.data["users"].as_list().map(<[Value]>::len), Some(2));
}

#[test]
fn unique_array_merge_reports_a_missing_dedup_key() {
    let h = with_keys(&[(
        "users",
        KeyRule::new(Strategy::UniqueArrayMerge).with_dedup_key("name"),
    )]);
    let err = resolve(
        &h,
        &standard_scope(),
        &loader(&[("common.yaml", "{users: [{uid: 1}]}")]),
    )
    .unwrap_err();
    assert_eq!(
        err.to_string(),
        r#"resolve: key "users": unique-array-merge: element {"uid":1} has no dedup key "name""#
    );
}

#[test]
fn unique_array_merge_rejects_a_non_array_layer() {
    let h = with_keys(&[("packages", KeyRule::new(Strategy::UniqueArrayMerge))]);
    let err = resolve(
        &h,
        &standard_scope(),
        &loader(&[("common.yaml", "{packages: curl}")]),
    )
    .unwrap_err();
    assert!(
        err.to_string()
            .contains("requires every layer to provide an array, got a string"),
        "{err}"
    );
}

#[test]
fn the_default_rule_applies_to_keys_without_their_own() {
    let mut h = with_keys(&[
        ("pinned", KeyRule::new(Strategy::FirstFound)),
        ("inherits", KeyRule::default()),
    ]);
    h.default = KeyRule::new(Strategy::DeepMerge);
    let l = loader(&[
        (
            "common.yaml",
            "{pinned: {a: 1}, inherits: {a: 1}, other: {a: 1}}",
        ),
        (
            "node/web01.yaml",
            "{pinned: {b: 2}, inherits: {b: 2}, other: {b: 2}}",
        ),
    ]);
    let res = resolve(&h, &standard_scope(), &l).unwrap();
    assert_eq!(res.data["pinned"], v("{b: 2}"));
    assert_eq!(res.data["inherits"], v("{a: 1, b: 2}"));
    assert_eq!(res.data["other"], v("{a: 1, b: 2}"));
}

#[test]
fn a_missing_layer_is_empty_when_lenient_and_an_error_when_strict() {
    let l = loader(&[("common.yaml", "{who: common}")]);

    let lenient = standard_hierarchy();
    assert_eq!(
        lenient.strictness,
        Strictness::Lenient,
        "lenient is the default"
    );
    let res = resolve(&lenient, &standard_scope(), &l).unwrap();
    assert_eq!(res.data["who"], Value::from("common"));
    assert_eq!(res.layers.iter().filter(|l| !l.found).count(), 3);

    let mut strict = standard_hierarchy();
    strict.strictness = Strictness::Strict;
    let err = resolve(&strict, &standard_scope(), &l).unwrap_err();
    // It names the first missing layer.
    assert_eq!(
        err.to_string(),
        r#"resolve: "environment/prod.yaml": hierarchy layer not found"#
    );
}

#[test]
fn a_hierarchy_with_no_layers_is_rejected() {
    let err = resolve(&Hierarchy::default(), &Scope::new(), &MapLoader::default()).unwrap_err();
    assert_eq!(err.to_string(), "hierarchy: no layers declared");
}

#[test]
fn a_hierarchy_file_parses() {
    let h = Hierarchy::parse(
        "hierarchy.yaml",
        b"hierarchy:\n  - common.yaml\n  - node/%{node}.yaml\ndefault:\n  strategy: deep-merge\nkeys:\n  packages:\n    strategy: unique-array-merge\n  users:\n    strategy: unique-array-merge\n    dedupKey: name\n  plain: {}\nstrictness: strict\n",
    )
    .unwrap();
    assert_eq!(h.paths, ["common.yaml", "node/%{node}.yaml"]);
    assert_eq!(h.default, KeyRule::new(Strategy::DeepMerge));
    assert_eq!(h.keys["packages"], KeyRule::new(Strategy::UniqueArrayMerge));
    assert_eq!(
        h.keys["users"],
        KeyRule::new(Strategy::UniqueArrayMerge).with_dedup_key("name")
    );
    assert_eq!(h.keys["plain"], KeyRule::default());
    assert_eq!(h.strictness, Strictness::Strict);
}

#[test]
fn a_hierarchy_file_with_bad_configuration_is_rejected() {
    for (name, doc, want) in [
        ("no layers", "keys: {}\n", "no layers declared"),
        (
            "bad strictness",
            "hierarchy: [a.yaml]\nstrictness: paranoid\n",
            r#"unknown strictness "paranoid""#,
        ),
        (
            "bad default strategy",
            "hierarchy: [a.yaml]\ndefault: {strategy: smoosh}\n",
            r#"hierarchy: default: unknown merge strategy "smoosh""#,
        ),
        (
            "bad key strategy",
            "hierarchy: [a.yaml]\nkeys: {k: {strategy: smoosh}}\n",
            r#"hierarchy: keys.k: unknown merge strategy "smoosh""#,
        ),
        (
            "unknown field",
            "hierarchy: [a.yaml]\nstrict: true\n",
            r#"unknown field "strict""#,
        ),
        (
            "wrong shape",
            "hierarchy: a.yaml\n",
            "hierarchy: expected a list, found a string",
        ),
        ("malformed", "hierarchy: [a.yaml\n", "hierarchy.yaml:"),
    ] {
        let err = Hierarchy::parse("hierarchy.yaml", doc.as_bytes())
            .expect_err(name)
            .to_string();
        assert!(
            err.contains(want),
            "{name}: error = {err}, want it to contain {want:?}"
        );
    }
}

struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(tag: &str, files: &[(&str, &str)]) -> Self {
        let dir =
            std::env::temp_dir().join(format!("meridian-resolve-{tag}-{}", std::process::id()));
        for (path, content) in files {
            let full = dir.join(path);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, content).unwrap();
        }
        TempDir(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn fs_loader_reads_yaml_and_json() {
    let dir = TempDir::new(
        "reads",
        &[
            (
                "data/common.yaml",
                "ntp_server: pool.ntp.org\npackages: [curl]\n",
            ),
            ("data/role/webserver.yaml", "# nothing yet\n"),
            (
                "data/node/web01.yaml",
                r#"{"ntp_server": "web01.ntp.internal"}"#,
            ),
        ],
    );
    let res = resolve(
        &standard_hierarchy(),
        &standard_scope(),
        &FsLoader::new(dir.0.join("data")),
    )
    .unwrap();
    assert_eq!(res.data["ntp_server"], Value::from("web01.ntp.internal"));
    assert_eq!(res.data["packages"], v("[curl]"));
    let found: Vec<bool> = res.layers.iter().map(|l| l.found).collect();
    assert_eq!(
        found,
        [true, false, true, true],
        "an empty file is a found, empty layer"
    );
}

#[test]
fn fs_loader_reports_a_malformed_layer() {
    let dir = TempDir::new(
        "malformed",
        &[("common.yaml", "a: [1, 2\n"), ("list.yaml", "- a\n")],
    );
    let l = FsLoader::new(&dir.0);
    let err = resolve(&Hierarchy::new(["common.yaml"]), &Scope::new(), &l)
        .unwrap_err()
        .to_string();
    assert!(
        err.starts_with(r#"resolve: "common.yaml": common.yaml:"#),
        "{err}"
    );
    let err = resolve(&Hierarchy::new(["list.yaml"]), &Scope::new(), &l)
        .unwrap_err()
        .to_string();
    assert_eq!(
        err,
        r#"resolve: "list.yaml": a layer must be a mapping, found a list"#
    );
}

/// A layer path is built from scope variables, and a scope variable can come
/// from a node fact. It must not be able to read outside the data directory.
#[test]
fn fs_loader_refuses_a_path_outside_its_root() {
    let dir = TempDir::new(
        "escape",
        &[
            ("secret.yaml", "password: hunter2\n"),
            ("data/common.yaml", "a: 1\n"),
        ],
    );
    let l = FsLoader::new(dir.0.join("data"));
    let absolute = dir.0.join("secret.yaml").to_string_lossy().into_owned();
    for node in ["../secret", absolute.trim_end_matches(".yaml")] {
        let h = Hierarchy::new(["common.yaml", "%{node}.yaml"]);
        let err = resolve(&h, &scope(&[("node", node)]), &l)
            .expect_err(node)
            .to_string();
        assert!(
            err.contains("layer path must stay inside the hierarchy root"),
            "{node}: {err}"
        );
    }
}

fn determinism_fixture() -> (Hierarchy, Scope, MapLoader) {
    let h = with_keys(&[
        ("nginx", KeyRule::new(Strategy::DeepMerge)),
        ("packages", KeyRule::new(Strategy::UniqueArrayMerge)),
        (
            "users",
            KeyRule::new(Strategy::UniqueArrayMerge).with_dedup_key("name"),
        ),
    ]);
    let l = loader(&[
        (
            "common.yaml",
            "{ntp_server: pool.ntp.org, packages: [curl, vim, git], users: [{name: deploy, uid: 1000}, {name: ops, uid: 1001}], nginx: {workers: 2, limits: {nofile: 1024, nproc: 512, stack: 8192}}}",
        ),
        (
            "environment/prod.yaml",
            "{ntp_server: prod.ntp.internal, packages: [htop, vim], nginx: {limits: {nofile: 4096}}}",
        ),
        ("role/webserver.yaml", "{packages: [nginx, curl], nginx: {workers: 4, listen: 80}}"),
        (
            "node/web01.yaml",
            "{ntp_server: web01.ntp.internal, users: [{name: app, uid: 2000}], nginx: {limits: {nofile: 65535}}}",
        ),
    ]);
    (h, standard_scope(), l)
}

fn resolve_canonical() -> String {
    let (h, s, l) = determinism_fixture();
    canonical_map(&resolve(&h, &s, &l).unwrap().data)
}

/// The expected text is the output of the Go implementation's
/// `Resolve(...).Data.Canonical()` over the same fixture.
#[test]
fn the_result_matches_the_go_implementation_byte_for_byte() {
    assert_eq!(
        resolve_canonical(),
        include_str!("golden/determinism.json").trim_end()
    );
}

#[test]
fn resolve_is_deterministic() {
    let want = resolve_canonical();
    for run in 0..100 {
        assert_eq!(resolve_canonical(), want, "run {run} differs");
    }
}

#[test]
fn resolve_is_deterministic_under_concurrency() {
    let want = resolve_canonical();
    let workers: Vec<_> = (0..16)
        .map(|_| std::thread::spawn(resolve_canonical))
        .collect();
    for (i, worker) in workers.into_iter().enumerate() {
        assert_eq!(worker.join().unwrap(), want, "worker {i} differs");
    }
}

#[test]
fn canonical_form_ignores_key_order_and_keeps_array_order() {
    let a = data("{b: 2, a: {y: 2, x: 1}, c: [3, 1, 2]}");
    let b = data("{a: {x: 1, y: 2}, c: [3, 1, 2], b: 2}");
    assert_eq!(canonical_map(&a), canonical_map(&b));
    let c = data("{a: {x: 1, y: 2}, c: [1, 2, 3], b: 2}");
    assert_ne!(
        canonical_map(&a),
        canonical_map(&c),
        "canonical form ignored array order"
    );
}

#[test]
fn layer_provenance_is_ordered_and_sorted() {
    let (h, s, l) = determinism_fixture();
    let res = resolve(&h, &s, &l).unwrap();
    let paths: Vec<&str> = res.layers.iter().map(|l| l.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "common.yaml",
            "environment/prod.yaml",
            "role/webserver.yaml",
            "node/web01.yaml"
        ]
    );
    assert_eq!(
        res.layers[0].keys,
        ["nginx", "ntp_server", "packages", "users"]
    );
    let _: &BTreeMap<String, Value> = &res.data;
}
