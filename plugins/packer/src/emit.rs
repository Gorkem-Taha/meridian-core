//! Stage four: a template becomes its files.

use std::collections::BTreeMap;

use meridian_sdk::Value;

use crate::{Block, Kind, Template};

/// The template, in Packer's JSON syntax. Packer reads every `*.pkr.json`
/// file of its working directory.
pub const CONFIG_PATH: &str = "build.pkr.json";

/// What the runner needs to know about the build and Packer does not: the
/// sources it builds and the outputs the document promises. Packer ignores
/// the file, since its name does not end in `.pkr.json`.
pub const BUILD_PATH: &str = "meridian-build.json";

/// Where Packer's `manifest` post-processor records what each source
/// produced. It is not part of the artifact; Packer writes it beside the
/// template when a build succeeds.
pub const MANIFEST_PATH: &str = "meridian-manifest.json";

fn map(entries: impl IntoIterator<Item = (String, Value)>) -> Value {
    Value::Map(entries.into_iter().collect())
}

fn text(s: &str) -> Value {
    Value::String(s.to_owned())
}

/// Inserts `value` at `outer.key.name`, creating the levels as needed.
fn nest(outer: &mut BTreeMap<String, Value>, key: &str, name: &str, value: Value) {
    let level = outer
        .entry(key.to_owned())
        .or_insert_with(|| Value::Map(BTreeMap::new()));
    if let Value::Map(level) = level {
        level.insert(name.to_owned(), value);
    }
}

/// A block with one label, written so that a list of them keeps its order:
/// `{"shell": {...}}`.
fn labelled(block: &Block) -> Value {
    map([(block.r#type.clone(), Value::Map(block.body.clone()))])
}

/// Renders the template. This stage performs no mapping and no ordering. Keys
/// are sorted at every level, and steps keep the order they were given, so
/// the same document always renders to the same bytes.
pub(crate) fn serialise(template: &Template) -> BTreeMap<String, String> {
    let mut root: BTreeMap<String, Value> = BTreeMap::new();
    let mut plugins: BTreeMap<String, Value> = BTreeMap::new();
    let mut variables: BTreeMap<String, Value> = BTreeMap::new();
    let mut locals: BTreeMap<String, Value> = BTreeMap::new();
    let mut data: BTreeMap<String, Value> = BTreeMap::new();
    let mut sources: BTreeMap<String, Value> = BTreeMap::new();

    let mut build_sources = Vec::new();
    let mut provisioners = Vec::new();
    let mut cleanup = None;
    let mut described_sources = Vec::new();

    for block in &template.blocks {
        let body = Value::Map(block.body.clone());
        match block.kind {
            Kind::Plugin => {
                plugins.insert(block.name.clone(), body);
            }
            Kind::Variable => {
                variables.insert(block.name.clone(), body);
            }
            Kind::Local => {
                locals.insert(
                    block.name.clone(),
                    block.body.get("value").cloned().unwrap_or(Value::Null),
                );
            }
            Kind::Data => nest(&mut data, &block.r#type, &block.name, body),
            Kind::Source => {
                nest(&mut sources, &block.r#type, &block.name, body);
                build_sources.push(text(&format!("source.{}", block.source_name())));
                described_sources.push(map([
                    ("id".to_owned(), text(&block.name)),
                    ("type".to_owned(), text(&block.r#type)),
                ]));
            }
            Kind::Provisioner => provisioners.push(labelled(block)),
            Kind::ErrorCleanup => cleanup = Some(labelled(block)),
            // Written from the sequences below.
            Kind::PostProcessor => {}
        }
    }

    // Each sequence is a `post-processors` block. The last one is Meridian's
    // own: a sequence of its own, so that it records what each source built
    // and not what some other post-processor made of it.
    let mut sequences: Vec<Value> = template
        .chains
        .iter()
        .map(|chain| {
            map([(
                "post-processor".to_owned(),
                Value::List(
                    chain
                        .iter()
                        .map(|&i| labelled(&template.blocks[i]))
                        .collect(),
                ),
            )])
        })
        .collect();
    sequences.push(map([(
        "post-processor".to_owned(),
        Value::List(vec![map([(
            "manifest".to_owned(),
            map([
                ("output".to_owned(), text(MANIFEST_PATH)),
                ("strip_path".to_owned(), Value::Bool(true)),
            ]),
        )])]),
    )]));

    let mut build: BTreeMap<String, Value> = BTreeMap::new();
    build.insert("name".to_owned(), text(&template.name));
    build.insert("sources".to_owned(), Value::List(build_sources));
    if !provisioners.is_empty() {
        build.insert("provisioner".to_owned(), Value::List(provisioners));
    }
    if let Some(cleanup) = cleanup {
        build.insert("error-cleanup-provisioner".to_owned(), cleanup);
    }
    build.insert("post-processors".to_owned(), Value::List(sequences));

    if !plugins.is_empty() {
        root.insert(
            "packer".to_owned(),
            map([("required_plugins".to_owned(), Value::Map(plugins))]),
        );
    }
    if !variables.is_empty() {
        root.insert("variable".to_owned(), Value::Map(variables));
    }
    if !locals.is_empty() {
        root.insert("locals".to_owned(), Value::Map(locals));
    }
    if !data.is_empty() {
        root.insert("data".to_owned(), Value::Map(data));
    }
    root.insert("source".to_owned(), Value::Map(sources));
    root.insert("build".to_owned(), Value::Map(build));

    let described = map([
        ("build".to_owned(), text(&template.name)),
        ("sources".to_owned(), Value::List(described_sources)),
        (
            "outputs".to_owned(),
            map(template
                .outputs
                .iter()
                .map(|o| (o.name.clone(), text(&o.template)))),
        ),
    ]);

    BTreeMap::from([
        (CONFIG_PATH.to_owned(), Value::Map(root).pretty()),
        (BUILD_PATH.to_owned(), described.pretty()),
    ])
}
