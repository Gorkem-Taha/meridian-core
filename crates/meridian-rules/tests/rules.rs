//! Compile rules over layered data: what each rule refuses, what it lets
//! through, and the ways a layer might try to get round one.

use meridian_ir::{parse_value, Value};
use meridian_resolve::{Data, Hierarchy, KeyRule, MapLoader, Scope, Strategy};
use meridian_rules::*;

fn data(yaml: &str) -> Data {
    match parse_value("literal", yaml.as_bytes())
        .unwrap_or_else(|e| panic!("bad literal {yaml:?}: {e}"))
    {
        Value::Map(m) => m,
        Value::Null => Data::new(),
        other => panic!("{yaml:?} is {}, not a mapping", other.describe()),
    }
}

/// Parses a rules file for a test of one thing. The sets that are on by
/// default are switched off unless the test asks for sets itself, so each
/// test sees only the rule it is about; `defaults` is for the tests of the
/// defaults.
fn rules(yaml: &str) -> Rules {
    if yaml.lines().any(|line| line.starts_with("use:")) {
        defaults(yaml)
    } else {
        defaults(&format!("{yaml}use: []\n"))
    }
}

fn defaults(yaml: &str) -> Rules {
    Rules::parse("rules.yaml", yaml.as_bytes()).unwrap_or_else(|e| panic!("{e}"))
}

fn rules_err(yaml: &str) -> String {
    Rules::parse("rules.yaml", yaml.as_bytes())
        .expect_err("want the rules file refused")
        .to_string()
}

/// The groups every test below shares: who owns which layer.
const GROUPS: &str = r#"rulesVersion: 1
layers:
  - name: organization
    paths: [common.yaml]
  - name: environment
    paths: ["environment/*"]
  - name: team
    paths: ["role/**", "node/**"]
"#;

fn grouped(rest: &str) -> Rules {
    rules(&format!("{GROUPS}{rest}"))
}

/// A hierarchy over exactly the given layers, with `strategies` for some
/// top-level keys and `default` for the rest.
fn hierarchy(
    layers: &[(&str, &str)],
    default: Strategy,
    strategies: &[(&str, Strategy)],
) -> Hierarchy {
    let mut h = Hierarchy::new(layers.iter().map(|(path, _)| *path));
    h.default = KeyRule::new(default);
    for (key, strategy) in strategies {
        h.keys.insert((*key).to_owned(), KeyRule::new(*strategy));
    }
    h
}

fn loader(layers: &[(&str, &str)]) -> MapLoader {
    MapLoader(
        layers
            .iter()
            .map(|(p, y)| ((*p).to_owned(), data(y)))
            .collect(),
    )
}

/// Runs the whole path a compiler takes and returns the refusal's lines, or
/// nothing when the layers pass.
fn check_with(
    rules: &Rules,
    h: &Hierarchy,
    layers: &[(&str, &str)],
    command_line: &[Layer],
) -> Vec<String> {
    match enforce(rules, h, &Scope::new(), &loader(layers), command_line) {
        Ok(_) => Vec::new(),
        Err(Refusal::Violations(v)) => v.iter().map(ToString::to_string).collect(),
        Err(Refusal::Error(e)) => panic!("not a violation: {e}"),
    }
}

/// Deep merge everywhere: the setting DscPipelineRunner's example
/// configuration uses for `variables`.
fn deep(rules: &Rules, layers: &[(&str, &str)]) -> Vec<String> {
    check_with(
        rules,
        &hierarchy(layers, Strategy::DeepMerge, &[]),
        layers,
        &[],
    )
}

/// First-found everywhere: the hierarchy's own default.
fn first_found(rules: &Rules, layers: &[(&str, &str)]) -> Vec<String> {
    check_with(
        rules,
        &hierarchy(layers, Strategy::FirstFound, &[]),
        layers,
        &[],
    )
}

// ---------------------------------------------------------------------------
// no-override: the DscPipelineRunner rule
// ---------------------------------------------------------------------------

#[test]
fn a_more_specific_layer_cannot_change_a_variable() {
    let r = grouped("use: [protect-variables]\n");
    let got = deep(
        &r,
        &[
            (
                "common.yaml",
                "variables: {OrganizationName: Contoso, Region: eu-west-1}",
            ),
            (
                "node/web01.yaml",
                "variables: {OrganizationName: Fabrikam, Owner: web-team}",
            ),
        ],
    );
    assert_eq!(
        got,
        [
            "[meridian/protect-variables] layer \"node/web01.yaml\" (team) changes \
          \"variables.OrganizationName\", which layer \"common.yaml\" (organization) set"
        ]
    );
}

#[test]
fn a_more_specific_layer_cannot_change_a_secret_or_a_credential() {
    let r = grouped("use: [protect-secrets]\n");
    let got = deep(
        &r,
        &[
            (
                "common.yaml",
                "secrets: {db: {from: vault, path: kv/org/db}}\ncredentials: {deploy: {type: ssh, user: deploy}}",
            ),
            (
                "node/web01.yaml",
                "secrets: {db: {path: kv/team/mine}}\ncredentials: {deploy: {user: root}}",
            ),
        ],
    );
    assert_eq!(
        got,
        [
            "[meridian/protect-secrets] layer \"node/web01.yaml\" (team) changes \"credentials.deploy.user\", \
             which layer \"common.yaml\" (organization) set",
            "[meridian/protect-secrets] layer \"node/web01.yaml\" (team) changes \"secrets.db.path\", \
             which layer \"common.yaml\" (organization) set",
        ]
    );
}

#[test]
fn adding_and_restating_are_not_overriding() {
    let r = grouped("use: [protect-variables, protect-secrets]\n");
    let got = deep(
        &r,
        &[
            ("common.yaml", "variables: {Region: eu-west-1, tags: {cost: shared}}"),
            // A new variable, a new nested key, and the same value again.
            (
                "node/web01.yaml",
                "variables: {Owner: web-team, Region: eu-west-1, tags: {app: web}}\nsecrets: {mine: {from: vault}}",
            ),
        ],
    );
    assert_eq!(got, Vec::<String>::new());
}

#[test]
fn keys_outside_the_rule_may_still_be_overridden() {
    let r = grouped("use: [protect-variables]\n");
    let got = deep(
        &r,
        &[
            ("common.yaml", "packages: [curl]\nmotd: hello"),
            ("node/web01.yaml", "motd: goodbye"),
        ],
    );
    assert_eq!(got, Vec::<String>::new());
}

#[test]
fn every_violation_is_listed_and_the_least_specific_setter_is_named() {
    let r = grouped("use: [protect-variables]\n");
    let got = deep(
        &r,
        &[
            ("common.yaml", "variables: {a: 1, b: 1}"),
            ("environment/prod.yaml", "variables: {a: 2}"),
            ("node/web01.yaml", "variables: {a: 2, b: 2}"),
        ],
    );
    assert_eq!(
        got,
        [
            "[meridian/protect-variables] layer \"environment/prod.yaml\" (environment) changes \"variables.a\", \
             which layer \"common.yaml\" (organization) set",
            // The node agrees with the environment layer, and still differs
            // from the organization's value.
            "[meridian/protect-variables] layer \"node/web01.yaml\" (team) changes \"variables.a\", \
             which layer \"common.yaml\" (organization) set",
            "[meridian/protect-variables] layer \"node/web01.yaml\" (team) changes \"variables.b\", \
             which layer \"common.yaml\" (organization) set",
        ]
    );
}

#[test]
fn set_by_and_applies_to_narrow_a_rule() {
    let r = grouped(
        r#"rules:
  - id: org-values-hold
    rule: no-override
    keys: [variables]
    setBy: [organization]
    appliesTo: [team]
"#,
    );
    let got = deep(
        &r,
        &[
            ("common.yaml", "variables: {a: 1, b: 1}"),
            // The environment layer is not constrained, so it may change a.
            ("environment/prod.yaml", "variables: {a: 2, c: 1}"),
            // The team may change c (the environment set it, and the rule
            // protects only the organization's values) but not b.
            ("node/web01.yaml", "variables: {b: 2, c: 2}"),
        ],
    );
    assert_eq!(
        got,
        [
            "[org-values-hold] layer \"node/web01.yaml\" (team) changes \"variables.b\", \
          which layer \"common.yaml\" (organization) set"
        ]
    );
}

#[test]
fn rules_work_without_any_groups() {
    let r = rules("rulesVersion: 1\nuse: [protect-all]\n");
    let got = deep(
        &r,
        &[
            ("common.yaml", "motd: hello"),
            ("node/web01.yaml", "motd: goodbye"),
        ],
    );
    assert_eq!(
        got,
        ["[meridian/protect-all] layer \"node/web01.yaml\" changes \"motd\", which layer \"common.yaml\" set"]
    );
}

#[test]
fn the_author_s_message_is_added() {
    let r = grouped(
        "rules:\n  - {id: region, rule: no-override, keys: [variables.Region], message: \"Ask the platform team.\"}\n",
    );
    let got = deep(
        &r,
        &[
            ("common.yaml", "variables: {Region: a}"),
            ("node/x.yaml", "variables: {Region: b}"),
        ],
    );
    assert_eq!(
        got,
        ["[region] layer \"node/x.yaml\" (team) changes \"variables.Region\", which layer \"common.yaml\" \
          (organization) set. Ask the platform team"]
    );
}

// ---------------------------------------------------------------------------
// The deny list a rules file has out of the box
// ---------------------------------------------------------------------------

#[test]
fn a_rules_file_that_does_not_say_use_gets_the_default_deny_list() {
    let r = defaults("rulesVersion: 1\n");
    let ids: Vec<&str> = r.rules.iter().map(|rule| rule.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "meridian/protect-variables",
            "meridian/protect-secrets",
            "meridian/no-placeholder-secrets"
        ]
    );
    assert_eq!(r.default, DefaultAccess::Allow);

    let got = deep(
        &r,
        &[
            ("common.yaml", "variables: {Region: eu}\nsecrets: {db: {path: kv/org}}\nmotd: hello"),
            ("node/x.yaml", "variables: {Region: us}\nsecrets: {db: {path: kv/mine}, api: {token: changeme}}\nmotd: bye"),
        ],
    );
    assert_eq!(
        got,
        [
            "[meridian/protect-variables] layer \"node/x.yaml\" changes \"variables.Region\", which layer \
             \"common.yaml\" set",
            "[meridian/protect-secrets] layer \"node/x.yaml\" changes \"secrets.db.path\", which layer \
             \"common.yaml\" set",
            "[meridian/no-placeholder-secrets] layer \"node/x.yaml\" sets \"secrets.api.token\" to a value the \
             rule forbids (matches[0])",
        ]
    );
    // It is a deny list: everything it does not forbid is allowed. The
    // unprotected `motd` changed above and nothing was said about it, and a
    // layer may add variables and secrets of its own.
    let got = deep(
        &r,
        &[
            ("common.yaml", "variables: {Region: eu}"),
            (
                "node/x.yaml",
                "variables: {Owner: me}\nsecrets: {api: {from: vault}}\nanything: 1",
            ),
        ],
    );
    assert_eq!(got, Vec::<String>::new());
}

#[test]
fn the_default_deny_list_is_replaced_by_what_use_lists() {
    let layers = [
        ("common.yaml", "variables: {Region: eu}"),
        ("node/x.yaml", "variables: {Region: us}\nsecrets: {a: \"\"}"),
    ];
    // Switched off, explicitly.
    assert_eq!(
        deep(&defaults("rulesVersion: 1\nuse: []\n"), &layers),
        Vec::<String>::new()
    );
    // One set chosen: only that one holds.
    let got = deep(
        &defaults("rulesVersion: 1\nuse: [no-placeholder-secrets]\n"),
        &layers,
    );
    assert_eq!(
        got,
        ["[meridian/no-placeholder-secrets] layer \"node/x.yaml\" sets \"secrets.a\" to a value the rule forbids (values[1])"]
    );
    // The defaults stay on beside a file's own rules and its capabilities.
    let r = defaults("rulesVersion: 1\ndefault: deny\nlayers:\n  - {name: all, paths: [\"**\"], capabilities: {keys: [\"**\"]}}\nrules:\n  - {id: mine, rule: final, keys: [motd]}\n");
    assert_eq!(r.rules.len(), 4);
    assert_eq!(deep(&r, &layers).len(), 2);
}

#[test]
fn empty_and_placeholder_secrets_are_refused() {
    let r = defaults("rulesVersion: 1\nuse: [no-placeholder-secrets]\n");
    for bad in [
        "\"\"",
        "null",
        "~",
        "changeme",
        "ChangeMe",
        "CHANGE-ME",
        "change_me",
        "replace-me",
        "TODO",
        "tbd",
        "FIXME",
        "placeholder",
        "xxx",
        "XXXXXXXX",
        "\"<your token here>\"",
    ] {
        for place in [
            "secrets: {api: {token: V}}",
            "credentials: {deploy: {password: V}}",
            "secrets: {api: V}",
            "secrets: {list: [fine, V]}",
        ] {
            let layer = place.replace('V', bad);
            let got = deep(&r, &[("common.yaml", &layer)]);
            assert_eq!(got.len(), 1, "{layer}: {got:?}");
            assert!(
                got[0].starts_with("[meridian/no-placeholder-secrets]"),
                "{got:?}"
            );
        }
    }
    for fine in [
        "kv/platform/database",
        "vault",
        "s3cr3t",
        "todo-list-service",
        "my-xxx",
        "0",
        "false",
    ] {
        let layer = format!("secrets: {{api: {{from: {fine}}}}}");
        assert_eq!(
            deep(&r, &[("common.yaml", &layer)]),
            Vec::<String>::new(),
            "{fine}"
        );
    }
    // Only secrets and credentials are looked at.
    assert_eq!(
        deep(
            &r,
            &[("common.yaml", "variables: {note: todo}\nmotd: \"\"")]
        ),
        Vec::<String>::new()
    );
    // A secret emptied by a more specific layer is caught there.
    let got = deep(
        &r,
        &[
            ("common.yaml", "secrets: {api: {token: real}}"),
            ("node/x.yaml", "secrets: {api: {token: \"\"}}"),
        ],
    );
    assert_eq!(got.len(), 1, "{got:?}");
    assert!(got[0].contains("layer \"node/x.yaml\""), "{got:?}");
}

// ---------------------------------------------------------------------------
// Adversarial: ways to change a protected value without naming it
// ---------------------------------------------------------------------------

#[test]
fn adversarial_replacing_the_parent_with_a_scalar() {
    let r = grouped("use: [protect-variables]\n");
    for attempt in [
        "variables: null",
        "variables: wiped",
        "variables: []",
        "variables: 0",
    ] {
        let got = deep(
            &r,
            &[
                ("common.yaml", "variables: {db: {host: a, port: 5432}}"),
                ("node/x.yaml", attempt),
            ],
        );
        assert_eq!(
            got,
            [
                "[meridian/protect-variables] layer \"node/x.yaml\" (team) sets \"variables\", replacing \
                 \"variables.db.host\", which layer \"common.yaml\" (organization) set",
                "[meridian/protect-variables] layer \"node/x.yaml\" (team) sets \"variables\", replacing \
                 \"variables.db.port\", which layer \"common.yaml\" (organization) set",
            ],
            "{attempt}"
        );
    }
}

#[test]
fn adversarial_replacing_a_nested_mapping_with_a_scalar_or_null() {
    let r = grouped("rules:\n  - {id: db-host, rule: no-override, keys: [variables.db.host]}\n");
    for attempt in [
        "variables: {db: null}",
        "variables: {db: gone}",
        "variables: {db: {host: null}}",
    ] {
        let got = deep(
            &r,
            &[
                ("common.yaml", "variables: {db: {host: a, port: 1}}"),
                ("node/x.yaml", attempt),
            ],
        );
        assert_eq!(got.len(), 1, "{attempt}: {got:?}");
        assert!(
            got[0].contains("\"variables.db.host\""),
            "{attempt}: {got:?}"
        );
    }
    // The unprotected sibling may change.
    let got = deep(
        &r,
        &[
            ("common.yaml", "variables: {db: {host: a, port: 1}}"),
            ("node/x.yaml", "variables: {db: {port: 2}}"),
        ],
    );
    assert_eq!(got, Vec::<String>::new());
}

#[test]
fn adversarial_first_found_drops_what_the_layer_leaves_out() {
    // Under first-found the most specific layer's `variables` is the whole
    // answer. A layer that sets one new variable has removed all the others.
    let r = grouped("use: [protect-variables]\n");
    let got = first_found(
        &r,
        &[
            ("common.yaml", "variables: {Region: eu, Tier: gold}"),
            ("node/x.yaml", "variables: {Owner: me}"),
        ],
    );
    assert_eq!(
        got,
        [
            "[meridian/protect-variables] layer \"node/x.yaml\" (team) sets \"variables\", which is merged by \
             replacing it whole (first-found), and so drops \"variables.Region\", which layer \"common.yaml\" \
             (organization) set",
            "[meridian/protect-variables] layer \"node/x.yaml\" (team) sets \"variables\", which is merged by \
             replacing it whole (first-found), and so drops \"variables.Tier\", which layer \"common.yaml\" \
             (organization) set",
        ]
    );
    // Repeating every protected value and adding to them is allowed.
    let got = first_found(
        &r,
        &[
            ("common.yaml", "variables: {Region: eu, Tier: gold}"),
            (
                "node/x.yaml",
                "variables: {Owner: me, Region: eu, Tier: gold}",
            ),
        ],
    );
    assert_eq!(got, Vec::<String>::new());
}

#[test]
fn adversarial_first_found_nested_drop_and_change() {
    let r = grouped("use: [protect-variables]\n");
    let got = first_found(
        &r,
        &[
            (
                "common.yaml",
                "variables: {db: {host: a, port: 1}, Region: eu}",
            ),
            ("node/x.yaml", "variables: {db: {host: b}, Region: eu}"),
        ],
    );
    assert_eq!(got.len(), 2, "{got:?}");
    assert!(got[0].contains("changes \"variables.db.host\""), "{got:?}");
    assert!(got[1].contains("drops \"variables.db.port\""), "{got:?}");
}

#[test]
fn adversarial_a_list_is_replaced_whole_under_deep_merge() {
    let r = grouped("use: [protect-variables]\n");
    let got = deep(
        &r,
        &[
            ("common.yaml", "variables: {admins: [alice, bob]}"),
            ("node/x.yaml", "variables: {admins: [alice, bob, mallory]}"),
        ],
    );
    assert_eq!(
        got,
        ["[meridian/protect-variables] layer \"node/x.yaml\" (team) changes \"variables.admins\", \
          which layer \"common.yaml\" (organization) set"]
    );
}

#[test]
fn appended_lists_lose_nothing_so_they_are_not_an_override() {
    let r = grouped("rules:\n  - {id: pkgs, rule: no-override, keys: [packages]}\n");
    let layers = [
        ("common.yaml", "packages: [curl]"),
        ("node/x.yaml", "packages: [nginx]"),
    ];
    let h = hierarchy(
        &layers,
        Strategy::FirstFound,
        &[("packages", Strategy::UniqueArrayMerge)],
    );
    assert_eq!(check_with(&r, &h, &layers, &[]), Vec::<String>::new());
    // The same layers under first-found do lose the organization's list.
    assert_eq!(first_found(&r, &layers).len(), 1);
}

#[test]
fn adversarial_a_changed_type_is_a_change() {
    let r = grouped("use: [protect-variables]\n");
    for (was, now) in [
        ("port: 5432", "port: \"5432\""),
        ("on: true", "on: \"true\""),
        ("n: 1", "n: 1.5"),
    ] {
        let got = deep(
            &r,
            &[
                ("common.yaml", &format!("variables: {{{was}}}")),
                ("node/x.yaml", &format!("variables: {{{now}}}")),
            ],
        );
        assert_eq!(got.len(), 1, "{was} -> {now}: {got:?}");
    }
}

#[test]
fn adversarial_the_command_line_is_the_most_specific_layer() {
    let r = grouped("use: [protect-variables]\n");
    let layers = [("common.yaml", "variables: {Region: eu}")];
    let h = hierarchy(&layers, Strategy::DeepMerge, &[]);
    let from_cli = [Layer::command_line(
        "--data override.yaml",
        data("variables: {Region: us}"),
    )];
    assert_eq!(
        check_with(&r, &h, &layers, &from_cli),
        [
            "[meridian/protect-variables] layer \"--data override.yaml\" ((command line)) changes \
          \"variables.Region\", which layer \"common.yaml\" (organization) set"
        ]
    );
    // A command-line value replaces the key it names whole, whatever the
    // hierarchy's strategy for it, so leaving a variable out drops it.
    let layers2 = [("common.yaml", "variables: {Region: eu, Tier: gold}")];
    let partial = [Layer::command_line(
        "--data p.yaml",
        data("variables: {Region: eu}"),
    )];
    assert_eq!(
        check_with(&r, &hierarchy(&layers2, Strategy::DeepMerge, &[]), &layers2, &partial),
        ["[meridian/protect-variables] layer \"--data p.yaml\" ((command line)) sets \"variables\", which a \
          command-line value replaces whole, and so drops \"variables.Tier\", which layer \"common.yaml\" \
          (organization) set"]
    );
    // A command-line file is never mistaken for a hierarchy layer, even when
    // it is named like the organization's own.
    let spoof = [Layer::command_line(
        "common.yaml",
        data("variables: {Region: us}"),
    )];
    assert_eq!(check_with(&r, &h, &layers, &spoof).len(), 1);
}

#[test]
fn adversarial_keys_that_look_like_paths_are_not_paths() {
    // A key with a dot in it is one key. It is not `variables` -> `Region`,
    // nothing reads it as that, and the refusal quotes it so nobody is misled.
    let r = grouped("use: [protect-all]\n");
    let got = deep(
        &r,
        &[
            (
                "common.yaml",
                "variables: {Region: eu}\n\"variables.Region\": x",
            ),
            ("node/x.yaml", "\"variables.Region\": y"),
        ],
    );
    assert_eq!(
        got,
        ["[meridian/protect-all] layer \"node/x.yaml\" (team) changes \"\\\"variables.Region\\\"\", \
          which layer \"common.yaml\" (organization) set"]
    );
    // And a rule can name such a key by quoting it.
    let r =
        grouped("rules:\n  - {id: dotted, rule: no-override, keys: ['\"variables.Region\"']}\n");
    assert_eq!(
        deep(
            &r,
            &[
                ("common.yaml", "\"variables.Region\": x"),
                ("node/x.yaml", "\"variables.Region\": y")
            ]
        )
        .len(),
        1
    );
}

#[test]
fn adversarial_yaml_anchors_and_merge_keys_are_seen_as_what_they_expand_to() {
    let r = grouped("use: [protect-variables]\n");
    let got = deep(
        &r,
        &[
            ("common.yaml", "variables: {Region: eu}"),
            ("node/x.yaml", "mine: &m {Region: us}\nvariables: *m"),
        ],
    );
    assert_eq!(got.len(), 1, "{got:?}");
    assert!(got[0].contains("\"variables.Region\""));
}

#[test]
fn adversarial_an_unclaimed_layer_may_set_nothing() {
    let r = grouped("use: [protect-variables]\n");
    let got = deep(
        &r,
        &[
            ("common.yaml", "variables: {a: 1}"),
            ("extra/sneaky.yaml", "anything: 1"),
        ],
    );
    assert_eq!(
        got,
        ["[meridian/unclaimed-layer] layer \"extra/sneaky.yaml\" is not claimed by any group under `layers`; \
          a layer nobody owns may set nothing"]
    );
}

#[test]
fn adversarial_a_layer_cannot_claim_a_group_by_what_it_says() {
    // Ownership is where the file is. Nothing inside a layer changes it.
    let r = grouped("rules:\n  - {id: secrets-are-org, rule: restrict, keys: [secrets], allow: [organization]}\n");
    let got = deep(
        &r,
        &[(
            "node/x.yaml",
            "group: organization\nlayers: [{name: organization}]\nsecrets: {db: {from: vault}}",
        )],
    );
    assert_eq!(got.len(), 1, "{got:?}");
    assert!(got[0]
        .starts_with("[secrets-are-org] layer \"node/x.yaml\" (team) sets \"secrets.db.from\""));
}

#[test]
fn a_refusal_never_carries_a_value() {
    let r = grouped(
        r#"use: [protect-all]
rules:
  - {id: f, rule: final, keys: [secrets]}
  - {id: r, rule: restrict, keys: [secrets], allow: [organization]}
  - {id: v, rule: forbidden-value, keys: ["**"], matches: ["hunter2*"]}
"#,
    );
    let got = first_found(
        &r,
        &[
            (
                "common.yaml",
                "secrets: {db: {password: s3cr3t-original}}\nvariables: {k: s3cr3t-original}",
            ),
            (
                "node/x.yaml",
                "secrets: {db: {password: hunter2-replacement}}\nvariables: hunter2-replacement",
            ),
        ],
    );
    assert!(got.len() >= 5, "{got:?}");
    let all = got.join("\n");
    assert!(!all.contains("s3cr3t") && !all.contains("hunter2"), "{all}");
}

// ---------------------------------------------------------------------------
// The DscPipelineRunner configuration from the reference
// ---------------------------------------------------------------------------

/// The hierarchy and rules `docs/reference/compile-rules.md` gives for a
/// DscPipelineRunner configuration, over that repository's own example
/// layers.
#[test]
fn the_dscpipelinerunner_shape_from_the_reference() {
    let h = Hierarchy::parse(
        "hierarchy.yaml",
        b"hierarchy:\n  - OrganizationPolicies/Organization.yaml\n  - ProjectPolicies/Project.yaml\n  - Projects/%{Project}.yaml\nkeys:\n  variables: {strategy: deep-merge}\n",
    )
    .unwrap();
    let r = rules(
        r#"rulesVersion: 1
layers:
  - name: platform
    paths: ["OrganizationPolicies/**", "ProjectPolicies/**"]
  - name: teams
    paths: ["Projects/**"]
rules:
  - id: platform-variables-hold
    rule: no-override
    keys: [variables]
    setBy: [platform]
    appliesTo: [teams]
  - id: secrets-are-the-platform-s
    rule: restrict
    keys: [secrets]
    allow: [platform]
  - id: variables-merge-deep
    rule: merge-strategy
    keys: [variables]
    allow: [deep-merge]
"#,
    );
    let scope: Scope = [("Project".to_owned(), "Magenta".to_owned())].into();
    let organization = "parameters: {}\nvariables: {OrganizationName: Contoso, Project_Service_GitRepositories: enabled}\nresources: []";
    let project = "variables: {ProjectName: Magenta}\nresources: [{name: Project, type: AzureDevOpsDscNative/AzDoProject}]";
    let run = |node: &str| {
        let layers = [
            ("OrganizationPolicies/Organization.yaml", organization),
            ("ProjectPolicies/Project.yaml", project),
            ("Projects/Magenta.yaml", node),
        ];
        match enforce(&r, &h, &scope, &loader(&layers), &[]) {
            Ok(_) => Vec::new(),
            Err(Refusal::Violations(v)) => v.iter().map(ToString::to_string).collect(),
            Err(Refusal::Error(e)) => panic!("{e}"),
        }
    };
    // The node file from DscPipelineRunner's own example: a variable of its own.
    assert_eq!(
        run("variables: {ProjectDescription: The Magenta project.}"),
        Vec::<String>::new()
    );
    assert_eq!(
        run("variables: {OrganizationName: Fabrikam}\nsecrets: {pat: {from: env}}"),
        [
            "[platform-variables-hold] layer \"Projects/Magenta.yaml\" (teams) changes \"variables.OrganizationName\", \
             which layer \"OrganizationPolicies/Organization.yaml\" (platform) set",
            "[secrets-are-the-platform-s] layer \"Projects/Magenta.yaml\" (teams) sets \"secrets.pat.from\", \
             which only platform may set",
        ]
    );
}

// ---------------------------------------------------------------------------
// Capabilities: DscPipelineRunner's Policy.yml, for keys
// ---------------------------------------------------------------------------

/// The shape of DscPipelineRunner's `Example Policy Configuration/Policy.yml`:
/// a default of deny, a platform group that may do anything, and a teams
/// group that may do what it lists.
const POLICY: &str = r#"rulesVersion: 1
default: deny
layers:
  - name: platform
    paths: ["Platform/**"]
    capabilities:
      keys: ["**"]
  - name: teams
    paths: ["Projects/**"]
    capabilities:
      keys: [variables.Owner, packages]
"#;

#[test]
fn a_group_may_set_only_the_keys_it_is_allowed() {
    let got = deep(
        &rules(POLICY),
        &[
            ("Platform/Baseline.yaml", "variables: {Region: eu}\nsecrets: {db: {from: vault}}\npackages: [curl]"),
            (
                "Projects/Present/WebTeam.yaml",
                "variables: {Owner: web, Region: us}\npackages: [nginx]\nsecrets: {db: {from: env}}",
            ),
        ],
    );
    assert_eq!(
        got,
        [
            "[meridian/capabilities] layer \"Projects/Present/WebTeam.yaml\" (teams) sets \"secrets.db.from\", \
             which teams is not allowed to set: it may set only variables.Owner, packages",
            "[meridian/capabilities] layer \"Projects/Present/WebTeam.yaml\" (teams) sets \"variables.Region\", \
             which teams is not allowed to set: it may set only variables.Owner, packages",
        ]
    );
    // What the team is allowed passes, and the platform may set anything.
    let got = deep(
        &rules(POLICY),
        &[
            (
                "Platform/Baseline.yaml",
                "variables: {Region: eu}\nsecrets: {db: {from: vault}}",
            ),
            (
                "Projects/Present/WebTeam.yaml",
                "variables: {Owner: web}\npackages: [nginx]",
            ),
        ],
    );
    assert_eq!(got, Vec::<String>::new());
}

#[test]
fn a_capability_a_group_does_not_list_follows_the_default() {
    let unlisted = "layers:\n  - {name: platform, paths: [\"Platform/**\"], capabilities: {keys: [\"**\"]}}\n  - {name: teams, paths: [\"Projects/**\"]}\n";
    let layers = [
        ("Platform/Baseline.yaml", "a: 1"),
        ("Projects/x.yaml", "b: 1"),
    ];
    assert_eq!(
        deep(&rules(&format!("rulesVersion: 1\ndefault: deny\n{unlisted}")), &layers),
        ["[meridian/capabilities] layer \"Projects/x.yaml\" (teams) sets \"b\", which teams is not allowed \
          to set: it lists no `capabilities.keys`, and the default is deny"]
    );
    for allow in ["default: allow\n", ""] {
        assert_eq!(
            deep(
                &rules(&format!("rulesVersion: 1\n{allow}{unlisted}")),
                &layers
            ),
            Vec::<String>::new()
        );
    }
    // An empty list is not the same as no list: it allows nothing, whatever
    // the default.
    let none = rules(
        "rulesVersion: 1\nlayers:\n  - {name: teams, paths: [\"**\"], capabilities: {keys: []}}\n",
    );
    assert_eq!(
        deep(&none, &[("Projects/x.yaml", "b: 1")]),
        ["[meridian/capabilities] layer \"Projects/x.yaml\" (teams) sets \"b\", which teams is not allowed \
          to set: it may set no keys"]
    );
}

#[test]
fn adversarial_capabilities_cannot_be_reached_round() {
    let r = rules(POLICY);
    // Writing above an allowed key is not writing the allowed key.
    for attempt in [
        "variables: null",
        "variables: {}",
        "variables: [Owner]",
        "variables: {Owner: {x: 1}, owner: me}",
    ] {
        let got = deep(&r, &[("Projects/x.yaml", attempt)]);
        assert_eq!(got.len(), 1, "{attempt}: {got:?}");
        assert!(
            got[0].starts_with("[meridian/capabilities]"),
            "{attempt}: {got:?}"
        );
    }
    // A file outside every group may do nothing, whatever the default says.
    let open = rules(&POLICY.replace("default: deny", "default: allow"));
    let got = deep(&open, &[("Elsewhere/x.yaml", "packages: [nginx]")]);
    assert_eq!(got.len(), 1, "{got:?}");
    assert!(got[0].starts_with("[meridian/unclaimed-layer]"), "{got:?}");
    // Nothing a layer says about itself changes its group or what it may do.
    let got = deep(
        &r,
        &[(
            "Projects/x.yaml",
            "layer: platform\ncapabilities: {keys: [\"**\"]}\ndefault: allow",
        )],
    );
    assert_eq!(got.len(), 3, "{got:?}");
    // The narrower group listed first owns the file, as in Policy.yml.
    let first = rules(&POLICY.replace(
        "paths: [\"Platform/**\"]",
        "paths: [\"Platform/**\", \"Projects/Trusted/**\"]",
    ));
    assert_eq!(
        deep(&first, &[("Projects/Trusted/x.yaml", "secrets: {a: 1}")]),
        Vec::<String>::new()
    );
}

#[test]
fn capabilities_and_rules_are_both_held() {
    // Allowed to set the key, and still not allowed to change the platform's value.
    let r = rules(&format!("{POLICY}use: [protect-all]\n"));
    let got = deep(
        &r,
        &[
            (
                "Platform/Baseline.yaml",
                "packages: [curl]\nvariables: {Owner: platform}",
            ),
            ("Projects/x.yaml", "variables: {Owner: web}"),
        ],
    );
    assert_eq!(
        got,
        ["[meridian/protect-all] layer \"Projects/x.yaml\" (teams) changes \"variables.Owner\", which layer \
          \"Platform/Baseline.yaml\" (platform) set"]
    );
}

// ---------------------------------------------------------------------------
// final
// ---------------------------------------------------------------------------

#[test]
fn a_final_key_cannot_be_set_again_even_to_the_same_value() {
    let r = grouped("rules:\n  - {id: region-is-final, rule: final, keys: [variables.Region]}\n");
    let got = deep(
        &r,
        &[
            ("common.yaml", "variables: {Region: eu, Tier: gold}"),
            ("node/x.yaml", "variables: {Region: eu, Tier: silver}"),
        ],
    );
    assert_eq!(
        got,
        ["[region-is-final] layer \"node/x.yaml\" (team) sets \"variables.Region\", which is final once set, \
          and layer \"common.yaml\" (organization) set it"]
    );
}

#[test]
fn a_final_key_may_be_set_by_whichever_layer_sets_it_first() {
    let r = grouped("rules:\n  - {id: f, rule: final, keys: [variables.Owner]}\n");
    assert_eq!(
        deep(
            &r,
            &[
                ("common.yaml", "variables: {a: 1}"),
                ("node/x.yaml", "variables: {Owner: me}")
            ]
        ),
        Vec::<String>::new()
    );
    let got = deep(
        &r,
        &[
            ("environment/p.yaml", "variables: {Owner: env}"),
            ("role/w.yaml", "variables: {Owner: role}"),
            ("node/x.yaml", "variables: {Owner: role}"),
        ],
    );
    assert_eq!(got.len(), 2, "{got:?}");
    assert!(
        got.iter()
            .all(|v| v.contains("layer \"environment/p.yaml\" (environment) set it")),
        "{got:?}"
    );
}

#[test]
fn a_final_list_cannot_be_appended_to() {
    let r = grouped("rules:\n  - {id: f, rule: final, keys: [admins]}\n");
    let layers = [
        ("common.yaml", "admins: [alice]"),
        ("node/x.yaml", "admins: [mallory]"),
    ];
    let h = hierarchy(&layers, Strategy::UniqueArrayMerge, &[]);
    assert_eq!(check_with(&r, &h, &layers, &[]).len(), 1);
}

// ---------------------------------------------------------------------------
// restrict
// ---------------------------------------------------------------------------

#[test]
fn only_the_allowed_groups_may_set_a_restricted_key() {
    let r = grouped("rules:\n  - {id: secrets-are-org, rule: restrict, keys: [secrets, \"**.password\"], allow: [organization]}\n");
    let got = deep(
        &r,
        &[
            (
                "common.yaml",
                "secrets: {db: {from: vault}}\ndb: {password: x}",
            ),
            ("environment/prod.yaml", "cache: {password: y}"),
            ("node/x.yaml", "secrets: {mine: {from: env}}\nmotd: hi"),
        ],
    );
    assert_eq!(
        got,
        [
            "[secrets-are-org] layer \"environment/prod.yaml\" (environment) sets \"cache.password\", \
             which only organization may set",
            "[secrets-are-org] layer \"node/x.yaml\" (team) sets \"secrets.mine.from\", \
             which only organization may set",
        ]
    );
}

#[test]
fn an_empty_allow_list_forbids_the_key_to_everyone() {
    let r = grouped("rules:\n  - {id: no-debug, rule: restrict, keys: [debug], allow: []}\n");
    assert_eq!(
        deep(&r, &[("common.yaml", "debug: true")]),
        ["[no-debug] layer \"common.yaml\" (organization) sets \"debug\", which no layer may set"]
    );
}

#[test]
fn adversarial_restricted_keys_cannot_be_wiped_from_above() {
    // The team never writes a key under `secrets.db`; it writes `secrets`.
    let r = grouped(
        "rules:\n  - {id: s, rule: restrict, keys: [secrets.db.password], allow: [organization]}\n",
    );
    for attempt in [
        "secrets: null",
        "secrets: {db: nothing}",
        "secrets: {db: {}}",
    ] {
        let got = first_found(
            &r,
            &[
                ("common.yaml", "secrets: {db: {password: {from: vault}}}"),
                ("node/x.yaml", attempt),
            ],
        );
        assert_eq!(got.len(), 1, "{attempt}: {got:?}");
        assert!(
            got[0].contains("replacing \"secrets.db.password.from\", which layer \"common.yaml\" (organization) set"),
            "{attempt}: {got:?}"
        );
    }
    // With nothing there to replace, a value above the key is just a value.
    let got = first_found(
        &r,
        &[
            ("common.yaml", "motd: hi"),
            ("node/x.yaml", "secrets: {other: 1}"),
        ],
    );
    assert_eq!(got, Vec::<String>::new());
}

#[test]
fn the_command_line_is_its_own_group_for_restrict() {
    let r =
        grouped("rules:\n  - {id: s, rule: restrict, keys: [secrets], allow: [organization]}\n");
    let layers = [("common.yaml", "secrets: {a: 1}")];
    let h = hierarchy(&layers, Strategy::DeepMerge, &[]);
    let cli = [Layer::command_line(
        "--data x.yaml",
        data("secrets: {b: 2}"),
    )];
    assert_eq!(check_with(&r, &h, &layers, &cli).len(), 1);
    // A rules file may let the command line in by name.
    let r = grouped("rules:\n  - {id: s, rule: restrict, keys: [secrets], allow: [organization, \"(command line)\"]}\n");
    assert_eq!(check_with(&r, &h, &layers, &cli), Vec::<String>::new());
}

// ---------------------------------------------------------------------------
// required
// ---------------------------------------------------------------------------

#[test]
fn a_key_required_in_the_result() {
    let r = grouped(
        "rules:\n  - {id: owner, rule: required, keys: [variables.Owner, variables.CostCentre]}\n",
    );
    let got = deep(
        &r,
        &[
            ("common.yaml", "variables: {CostCentre: 42}"),
            ("node/x.yaml", "variables: {Owner: null}"),
        ],
    );
    assert_eq!(
        got,
        ["[owner] no layer sets \"variables.Owner\", which must be set"]
    );
    assert_eq!(
        deep(
            &r,
            &[
                ("common.yaml", "variables: {CostCentre: 42}"),
                ("node/x.yaml", "variables: {Owner: me}")
            ]
        ),
        Vec::<String>::new()
    );
}

#[test]
fn a_key_required_of_every_layer_of_a_group() {
    let r = grouped(
        "rules:\n  - {id: owner, rule: required, keys: [variables.Owner], appliesTo: [team]}\n",
    );
    let got = deep(
        &r,
        &[
            ("common.yaml", "variables: {Owner: platform}"),
            ("role/web.yaml", "variables: {Owner: web}"),
            ("node/x.yaml", "variables: {Other: 1}"),
        ],
    );
    assert_eq!(
        got,
        ["[owner] layer \"node/x.yaml\" (team) does not set \"variables.Owner\", which every layer of team must set"]
    );
}

#[test]
fn adversarial_required_is_not_met_by_a_value_the_merge_throws_away() {
    // Under first-found the node's `variables` replaces the organization's,
    // so the organization's Owner never reaches the result.
    let r = grouped("rules:\n  - {id: owner, rule: required, keys: [variables.Owner]}\n");
    let got = first_found(
        &r,
        &[
            ("common.yaml", "variables: {Owner: platform}"),
            ("node/x.yaml", "variables: {a: 1}"),
        ],
    );
    assert_eq!(
        got,
        ["[owner] no layer sets \"variables.Owner\", which must be set"]
    );
}

// ---------------------------------------------------------------------------
// forbidden-value
// ---------------------------------------------------------------------------

#[test]
fn forbidden_values_exact_and_by_pattern() {
    let r = grouped(
        r#"rules:
  - id: no-latest
    rule: forbidden-value
    keys: ["**.image", "**.tag"]
    values: [latest]
    matches: ["*:latest"]
  - id: no-plain-http
    rule: forbidden-value
    keys: ["**"]
    matches: ["http://*"]
    appliesTo: [team]
"#,
    );
    let got = deep(
        &r,
        &[
            ("common.yaml", "registry: http://internal.example\napp: {image: \"nginx:1.27\"}"),
            ("node/x.yaml", "app: {tag: latest, sidecars: [{image: \"envoy:latest\"}, {image: \"x:1\"}]}\nurls: [https://a, http://b]"),
        ],
    );
    assert_eq!(
        got,
        [
            "[no-latest] layer \"node/x.yaml\" (team) sets \"app.sidecars[0].image\" to a value the rule forbids (matches[0])",
            "[no-latest] layer \"node/x.yaml\" (team) sets \"app.tag\" to a value the rule forbids (values[0])",
            "[no-plain-http] layer \"node/x.yaml\" (team) sets \"urls[1]\" to a value the rule forbids (matches[0])",
        ]
    );
}

#[test]
fn forbidden_values_compare_by_type_and_case_unless_told_otherwise() {
    let strict = grouped("rules:\n  - {id: v, rule: forbidden-value, keys: [x], values: [\"0\", yes], matches: [\"ROOT\"]}\n");
    assert_eq!(
        deep(&strict, &[("common.yaml", "x: [0, \"Yes\", root]")]),
        Vec::<String>::new()
    );
    assert_eq!(deep(&strict, &[("common.yaml", "x: [\"0\"]")]).len(), 1);
    // Numbers and booleans are matched by a pattern as they are written.
    let loose = grouped("rules:\n  - {id: v, rule: forbidden-value, keys: [x], values: [yes], matches: [\"ROOT\", \"0\", \"true\"], ignoreCase: true}\n");
    assert_eq!(
        deep(
            &loose,
            &[("common.yaml", "x: [0, \"YES\", root, true, null]")]
        )
        .len(),
        4
    );
}

// ---------------------------------------------------------------------------
// merge-strategy
// ---------------------------------------------------------------------------

#[test]
fn a_key_may_be_held_to_a_merge_strategy() {
    let r = grouped("rules:\n  - {id: m, rule: merge-strategy, keys: [variables, packages], allow: [deep-merge, unique-array-merge]}\n");
    let layers = [("common.yaml", "variables: {a: 1}")];
    let h = hierarchy(
        &layers,
        Strategy::FirstFound,
        &[("packages", Strategy::UniqueArrayMerge)],
    );
    assert_eq!(
        check_with(&r, &h, &layers, &[]),
        ["[m] \"variables\" is merged with first-found, and may only be merged with deep-merge or unique-array-merge"]
    );
    let h = hierarchy(&layers, Strategy::DeepMerge, &[]);
    assert_eq!(check_with(&r, &h, &layers, &[]), Vec::<String>::new());
}

#[test]
fn an_unknown_strategy_in_a_rule_is_an_error_not_a_rule_nothing_can_meet() {
    let r =
        grouped("rules:\n  - {id: m, rule: merge-strategy, keys: [variables], allow: [deep]}\n");
    let layers = [("common.yaml", "variables: {a: 1}")];
    let h = hierarchy(&layers, Strategy::DeepMerge, &[]);
    match enforce(&r, &h, &Scope::new(), &loader(&layers), &[]) {
        Err(Refusal::Error(e)) => assert_eq!(
            e.to_string(),
            "rule \"m\": allow: unknown merge strategy \"deep\" (expected one of: first-found, deep-merge, unique-array-merge)"
        ),
        other => panic!("{other:?}"),
    }
}

// ---------------------------------------------------------------------------
// The whole refusal, and what passes through
// ---------------------------------------------------------------------------

#[test]
fn the_refusal_lists_everything_and_says_nothing_was_compiled() {
    let r = grouped("use: [protect-variables]\nrules:\n  - {id: owner, rule: required, keys: [variables.Owner]}\n");
    let layers = [
        ("common.yaml", "variables: {Region: eu}"),
        ("node/x.yaml", "variables: {Region: us}"),
    ];
    let h = hierarchy(&layers, Strategy::DeepMerge, &[]);
    let refusal = enforce(&r, &h, &Scope::new(), &loader(&layers), &[]).unwrap_err();
    assert_eq!(
        refusal.to_string(),
        "compile rules: 2 violations; nothing was compiled\n  \
         - [meridian/protect-variables] layer \"node/x.yaml\" (team) changes \"variables.Region\", which layer \
         \"common.yaml\" (organization) set\n  \
         - [owner] no layer sets \"variables.Owner\", which must be set"
    );
    let Refusal::Violations(v) = refusal else {
        panic!()
    };
    assert_eq!(v[0].rule, "meridian/protect-variables");
    assert_eq!(v[0].check, "no-override");
    assert_eq!(v[0].layer.as_deref(), Some("node/x.yaml"));
    assert_eq!(v[0].group.as_deref(), Some("team"));
    assert_eq!(v[0].key, "variables.Region");
    assert_eq!((v[1].layer.as_deref(), v[1].check), (None, "required"));
}

#[test]
fn layers_that_pass_are_handed_over_merged() {
    let r = grouped("use: [protect-variables]\n");
    let layers = [
        ("common.yaml", "variables: {Region: eu}"),
        ("node/x.yaml", "variables: {Owner: me}"),
    ];
    let mut h = hierarchy(&layers, Strategy::DeepMerge, &[]);
    h.paths.insert(1, "environment/%{env}.yaml".to_owned());
    let scope: Scope = [("env".to_owned(), "prod".to_owned())].into();
    let cli = [Layer::command_line("--data x.yaml", data("extra: 1"))];
    let loaded = enforce(&r, &h, &scope, &loader(&layers), &cli).unwrap();
    assert_eq!(
        loaded.resolved.data,
        data("variables: {Region: eu, Owner: me}\nextra: 1")
    );
    // The missing environment layer is not a layer the rules see.
    let paths: Vec<&str> = loaded.layers.iter().map(|l| l.path.as_str()).collect();
    assert_eq!(paths, ["common.yaml", "node/x.yaml", "--data x.yaml"]);
}

#[test]
fn evaluation_is_deterministic() {
    let r = grouped("use: [protect-all]\n");
    let layers = [
        ("common.yaml", "z: 1\na: {c: 1, b: 1}\nm: 1"),
        ("node/x.yaml", "m: 2\na: {b: 2, c: 2}\nz: 2"),
    ];
    let first = deep(&r, &layers);
    assert_eq!(first.len(), 4);
    for _ in 0..5 {
        assert_eq!(deep(&r, &layers), first);
    }
    let keys: Vec<&str> = first.iter().map(|v| v.split('"').nth(3).unwrap()).collect();
    assert_eq!(keys, ["a.b", "a.c", "m", "z"]);
}

// ---------------------------------------------------------------------------
// The rules file is read strictly
// ---------------------------------------------------------------------------

#[test]
fn a_rules_file_that_does_not_mean_something_is_refused() {
    for (yaml, want) in [
        ("", "rules.yaml: rules file: expected a mapping, found null"),
        ("rules: []", "rules.yaml: rulesVersion: required (write `rulesVersion: 1`)"),
        ("rulesVersion: 2", "rules.yaml: rulesVersion: this Meridian reads version 1, found 2"),
        ("rulesVersion: 1\ndefault: Deny", "rules.yaml: default: expected \"allow\" or \"deny\", found \"Deny\""),
        ("rulesVersion: 1\ndefault: deny", "rules.yaml: default: deny needs groups under `layers`: with none, no layer could set anything"),
        ("rulesVersion: 1\nlayers: [{name: a, paths: [x], capabilities: {targets: [ssh]}}]", "rules.yaml: layers[0].capabilities: unknown field \"targets\""),
        ("rulesVersion: 1\nlayers: [{name: a, paths: [x], capabilities: {keys: [\"a*\"]}}]", "rules.yaml: layers[0].capabilities.keys: the key pattern \"a*\" mixes `*` with other characters in the segment \"a*\"; a segment is a key, `*` or `**` (quote a key that contains a star)"),
        ("rulesVersion: 1\nrule: []", "rules.yaml: rules file: unknown field \"rule\""),
        ("rulesVersion: 1\nuse: [protect-everything]", "rules.yaml: use[0]: there is no shipped rule set \"protect-everything\" (there are: protect-variables, protect-secrets, no-placeholder-secrets, protect-all)"),
        ("rulesVersion: 1\nuse: [protect-all, protect-all]", "rules.yaml: use[1]: \"protect-all\" is listed twice"),
        ("rulesVersion: 1\nlayers: [{name: a, paths: [x]}, {name: a, paths: [y]}]", "rules.yaml: layers[1].name: the group \"a\" is declared twice"),
        ("rulesVersion: 1\nlayers: [{name: a}]", "rules.yaml: layers[0].paths: the group \"a\" claims no layers"),
        ("rulesVersion: 1\nlayers: [{name: \"(command line)\", paths: [x]}]", "rules.yaml: layers[0].name: \"(command line)\" is not a usable group name (names in brackets are Meridian's own)"),
        ("rulesVersion: 1\nlayers: [{name: a, paths: [\"../x\"]}]", "rules.yaml: layers[0].paths: the layer path pattern \"../x\" contains `..`; patterns are matched against cleaned paths"),
        ("rulesVersion: 1\nrules: [{rule: final, keys: [a]}]", "rules.yaml: rules[0].id: required"),
        ("rulesVersion: 1\nrules: [{id: meridian/x, rule: final, keys: [a]}]", "rules.yaml: rules[0].id: \"meridian/x\" is not a usable id (ids starting `meridian/` are the shipped rules')"),
        ("rulesVersion: 1\nrules: [{id: a, rule: final, keys: [a]}, {id: a, rule: final, keys: [b]}]", "rules.yaml: rules[1].id: the id \"a\" is used twice"),
        ("rulesVersion: 1\nrules: [{id: a, rule: locked, keys: [a]}]", "rules.yaml: rules[0].rule: unknown rule \"locked\" (expected no-override, final, restrict, required, forbidden-value or merge-strategy)"),
        ("rulesVersion: 1\nrules: [{id: a, rule: final}]", "rules.yaml: rules[0].keys: required, and cannot be empty"),
        ("rulesVersion: 1\nrules: [{id: a, rule: final, keys: [\"a..b\"]}]", "rules.yaml: rules[0].keys: the key pattern \"a..b\" has an empty segment"),
        ("rulesVersion: 1\nrules: [{id: a, rule: final, keys: [a], allow: [x]}]", "rules.yaml: rules[0]: a final rule does not take the field \"allow\""),
        ("rulesVersion: 1\nrules: [{id: a, rule: required, keys: [a], setBy: [x]}]", "rules.yaml: rules[0]: a required rule does not take the field \"setBy\""),
        ("rulesVersion: 1\nrules: [{id: a, rule: final, keys: [a], appliesTo: [team]}]", "rules.yaml: rules[0].appliesTo: there is no group \"team\" under `layers` (the file declares none)"),
        ("rulesVersion: 1\nlayers: [{name: org, paths: [x]}]\nrules: [{id: a, rule: restrict, keys: [a], allow: [team]}]", "rules.yaml: rules[0].allow: there is no group \"team\" under `layers` (there are: org)"),
        ("rulesVersion: 1\nrules: [{id: a, rule: restrict, keys: [a]}]", "rules.yaml: rules[0].allow: required: the groups that may set the keys"),
        ("rulesVersion: 1\nrules: [{id: a, rule: required, keys: [\"a.*\"]}]", "rules.yaml: rules[0].keys: a required rule names exact keys, and a.* has a wildcard"),
        ("rulesVersion: 1\nrules: [{id: a, rule: forbidden-value, keys: [a]}]", "rules.yaml: rules[0]: a forbidden-value rule needs `values` or `matches`"),
        ("rulesVersion: 1\nrules: [{id: a, rule: forbidden-value, keys: [a], values: [[1]]}]", "rules.yaml: rules[0].values: a forbidden value is a scalar, and [1] is a list"),
        ("rulesVersion: 1\nrules: [{id: a, rule: merge-strategy, keys: [a.b], allow: [deep-merge]}]", "rules.yaml: rules[0].keys: merge strategies belong to top-level keys, so a merge-strategy rule names them exactly, and a.b is not one"),
        ("rulesVersion: 1\nrules: [{id: a, rule: merge-strategy, keys: [a]}]", "rules.yaml: rules[0].allow: required: the strategies the keys may merge with"),
    ] {
        assert_eq!(rules_err(yaml), want, "{yaml}");
    }
}

#[test]
fn the_first_group_whose_pattern_matches_owns_a_layer() {
    let r = rules("rulesVersion: 1\nlayers:\n  - {name: special, paths: [node/db*.yaml]}\n  - {name: team, paths: [\"node/**\"]}\n");
    assert_eq!(r.group_of("node/db01.yaml"), "special");
    assert_eq!(r.group_of("node/web01.yaml"), "team");
    assert_eq!(r.group_of("elsewhere.yaml"), UNCLAIMED);
}
