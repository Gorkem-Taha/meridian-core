//! The writer against the library whose output it reproduces.
//!
//! `corpus.json` is a few hundred strings and values chosen to hit every
//! decision the writer makes. `corpus.expected` is what gopkg.in/yaml.v3
//! wrote for each of them, in several positions, produced by a throwaway Go
//! program that is not kept. Regenerate both together or not at all.

use meridian_yaml::{write, Yaml};
use serde_json::Value;

fn convert(v: &Value) -> Yaml {
    match v {
        Value::Null => Yaml::Null,
        Value::Bool(b) => Yaml::Bool(*b),
        Value::Number(n) => match (n.as_i64(), n.as_f64()) {
            (Some(i), _) => Yaml::Int(i),
            // The Go side reads a number too large for a signed integer as a
            // float, and so does the corpus.
            (None, Some(f)) => Yaml::Float(f),
            (None, None) => unreachable!("a JSON number is one or the other"),
        },
        Value::String(s) => Yaml::Str(s.clone()),
        Value::Array(items) => Yaml::Seq(items.iter().map(convert).collect()),
        Value::Object(map) => {
            // Sorted by key, as the Go targets write their mappings.
            let mut pairs: Vec<(String, Yaml)> =
                map.iter().map(|(k, v)| (k.clone(), convert(v))).collect();
            pairs.sort_by(|a, b| a.0.cmp(&b.0));
            Yaml::Map(pairs)
        }
    }
}

fn map(pairs: Vec<(&str, Yaml)>) -> Yaml {
    Yaml::Map(pairs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
}

#[test]
fn the_writer_matches_the_reference_library_over_the_corpus() {
    let corpus: Value = serde_json::from_str(include_str!("corpus.json")).unwrap();
    let expected: Vec<&str> = include_str!("corpus.expected").split("\u{1e}\n").collect();
    let mut expected = expected.into_iter();
    let mut failures = Vec::new();
    let mut checked = 0;
    let mut check = |what: String, doc: Yaml| {
        let want = expected
            .next()
            .expect("the expected file has an entry per case");
        if reference_loses_data(&what) {
            return;
        }
        let got = write(&doc);
        checked += 1;
        if got != want {
            failures.push(format!("{what}\n--- want ---\n{want}--- got ---\n{got}"));
        }
    };

    for s in corpus["strings"].as_array().unwrap() {
        let s = s.as_str().unwrap();
        let value = Yaml::str(s);
        check(
            format!("{s:?} as a value and a list item"),
            Yaml::Seq(vec![map(vec![
                ("v", value.clone()),
                ("l", Yaml::Seq(vec![value.clone()])),
            ])]),
        );
        check(format!("{s:?} as a key"), map(vec![(s, Yaml::str("x"))]));
        check(format!("{s:?} at the root"), value);
    }
    for v in corpus["values"].as_array().unwrap() {
        let value = convert(v);
        check(format!("{v} at the root"), value.clone());
        check(
            format!("{v} nested"),
            Yaml::Seq(vec![map(vec![("v", value)])]),
        );
    }

    assert_eq!(
        expected.next(),
        Some(""),
        "the expected file has entries left over"
    );
    assert!(
        failures.is_empty(),
        "{} of {checked} cases differ:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

/// The cases where the reference library writes something that does not read
/// back as the value it was given: a string that starts with a line break
/// loses it, a line or paragraph separator becomes a run of spaces, and `<<`
/// is tagged as a merge key. The writer is right about these instead of
/// compatible, and the next test pins what it writes.
fn reference_loses_data(what: &str) -> bool {
    what.starts_with("\"\\n")
        || what.contains("\\u{2028}")
        || what.contains("\\u{2029}")
        || what.starts_with("\"<<\"")
        || what.contains("\\nstarts with newline")
}

#[test]
fn where_the_reference_loses_data_the_writer_keeps_it() {
    let item = |s: &str| write(&Yaml::Seq(vec![map(vec![("v", Yaml::str(s))])]));
    // A leading line break is kept, with the indentation indicator a block
    // needs when its first line cannot show how far it is indented.
    assert_eq!(item("\nlead"), "- v: |2-\n\n    lead\n");
    assert_eq!(item("\n"), "- v: |2+\n\n");
    assert_eq!(item("\n\n"), "- v: |2+\n\n\n");
    // Separators that some parsers treat as line breaks are escaped.
    assert_eq!(item("ls\u{2028}x"), "- v: \"ls\\Lx\"\n");
    assert_eq!(item("ps\u{2029}x"), "- v: \"ps\\Px\"\n");
    // A string that looks like a merge key is quoted, so it stays a string.
    assert_eq!(item("<<"), "- v: \"<<\"\n");
    assert_eq!(write(&map(vec![("<<", Yaml::Int(1))])), "\"<<\": 1\n");
}

#[test]
fn a_document_reads_the_way_it_was_laid_out() {
    let doc = Yaml::Seq(vec![map(vec![
        ("name", Yaml::str("web")),
        ("hosts", Yaml::str("web01.example.com")),
        (
            "tasks",
            Yaml::Seq(vec![map(vec![
                ("name", Yaml::str("nginx_conf")),
                (
                    "ansible.builtin.copy",
                    map(vec![
                        ("content", Yaml::str("worker_processes 4;\n")),
                        ("mode", Yaml::str("0644")),
                        ("enabled", Yaml::Bool(true)),
                        ("port", Yaml::Int(8080)),
                    ]),
                ),
                ("notify", Yaml::Seq(vec![Yaml::str("restart nginx_svc")])),
            ])]),
        ),
    ])]);
    assert_eq!(
        write(&doc),
        "- name: web
  hosts: web01.example.com
  tasks:
    - name: nginx_conf
      ansible.builtin.copy:
        content: |
          worker_processes 4;
        mode: \"0644\"
        enabled: true
        port: 8080
      notify:
        - restart nginx_svc
"
    );
}
