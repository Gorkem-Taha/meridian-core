//! A document's outputs, which Packer does not have.
//!
//! An output is text with references to what a source produced:
//! `${ubuntu.id}`. Compiling checks every reference and keeps the text; after
//! a build the runner fills it in from the manifest Packer wrote. Both halves
//! are here so they cannot disagree about what can be read.

use std::collections::BTreeMap;

use meridian_sdk::expr::{self, Part};
use meridian_sdk::Value;

/// What an output can read of a source, and what each is.
pub(crate) const ATTRIBUTES: &[(&str, &str)] = &[
    (
        "id",
        "the image's id, without the region an Amazon builder puts before it",
    ),
    (
        "artifact_id",
        "the artifact's id exactly as Packer reports it",
    ),
    (
        "builder_type",
        "the builder that made it, such as amazon-ebs",
    ),
    (
        "build_time",
        "when the build finished, in seconds since 1970",
    ),
    (
        "files",
        "the files of the artifact, each with its name and size",
    ),
    ("packer_run_uuid", "the id of the packer run that made it"),
];

fn attribute_names() -> String {
    ATTRIBUTES
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join(", ")
}

/// One reference of an output, split into the source and what is read of it.
fn reference<'a>(
    expression: &'a str,
    sources: &BTreeMap<String, String>,
) -> Result<(&'a str, &'a str), String> {
    let (name, rest) = expr::head(expression);
    if !sources.contains_key(name) {
        return Err(format!(
            "${{{expression}}} does not refer to a source of this document. An output is filled \
             in after the build, from what each source produced, so it reads \
             ${{<source id>.<attribute>}} (attributes: {})",
            attribute_names()
        ));
    }
    let attribute = rest.strip_prefix('.').unwrap_or("");
    if !ATTRIBUTES.iter().any(|(known, _)| *known == attribute) {
        return Err(format!(
            "${{{expression}}} reads {rest:?} of the source {name:?}, which a build does not \
             report (attributes: {})",
            attribute_names()
        ));
    }
    Ok((name, attribute))
}

/// Checks an output's value at compile time and returns it in a form that
/// [`fill`] reads back to the same thing.
pub(crate) fn check(value: &str, sources: &BTreeMap<String, String>) -> Result<String, String> {
    let mut out = String::with_capacity(value.len());
    for part in expr::parse(value)? {
        match part {
            Part::Text(literal) => out.push_str(&literal.replace("${", "$${")),
            Part::Expr(expression) => {
                let (name, attribute) = reference(&expression, sources)?;
                out.push_str(&format!("${{{name}.{attribute}}}"));
            }
        }
    }
    Ok(out)
}

/// One build a manifest records: what one source produced in one run.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Built {
    /// The source's name, which is the resource's id.
    pub name: String,
    pub fields: BTreeMap<String, Value>,
}

impl Built {
    /// One of [`ATTRIBUTES`].
    fn attribute(&self, attribute: &str) -> Value {
        if attribute == "id" {
            return Value::String(image_id(
                self.fields
                    .get("artifact_id")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
            ));
        }
        self.fields.get(attribute).cloned().unwrap_or(Value::Null)
    }
}

/// The id a later stage can use.
///
/// The Amazon builders report `us-east-1:ami-0abc`, and one such pair per
/// region when an image is copied. An image copied to several regions has no
/// single id, so that artifact id is left as it is.
pub(crate) fn image_id(artifact_id: &str) -> String {
    match artifact_id.split_once(':') {
        Some((region, id)) if !artifact_id.contains(',') && is_region(region) => id.to_owned(),
        _ => artifact_id.to_owned(),
    }
}

/// `us-east-1`, `ap-southeast-2`, `us-gov-west-1`: words, then a number.
fn is_region(text: &str) -> bool {
    let words: Vec<&str> = text.split('-').collect();
    words.len() >= 3
        && words[..words.len() - 1]
            .iter()
            .all(|w| !w.is_empty() && w.chars().all(|c| c.is_ascii_lowercase()))
        && words[words.len() - 1].chars().all(|c| c.is_ascii_digit())
        && !words[words.len() - 1].is_empty()
}

/// Fills an output's text in from what was built.
///
/// A value that is nothing but one reference keeps the type of what it
/// reads, so a time stays a number. Anything else is text.
pub(crate) fn fill(template: &str, built: &[Built]) -> Result<Value, String> {
    let parts = expr::parse(template)?;
    let read = |expression: &str| -> Result<Value, String> {
        let (name, rest) = expr::head(expression);
        let attribute = rest.strip_prefix('.').unwrap_or("");
        let source = built.iter().find(|b| b.name == name).ok_or_else(|| {
            format!("the last build did not report anything for the source {name:?}")
        })?;
        Ok(source.attribute(attribute))
    };
    if let [Part::Expr(expression)] = parts.as_slice() {
        return read(expression);
    }
    let mut out = String::new();
    for part in &parts {
        match part {
            Part::Text(literal) => out.push_str(literal),
            Part::Expr(expression) => match read(expression)? {
                Value::String(s) => out.push_str(&s),
                Value::Null => {}
                other => out.push_str(&other.canonical()),
            },
        }
    }
    Ok(Value::String(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sources() -> BTreeMap<String, String> {
        BTreeMap::from([("ubuntu".to_owned(), "amazon-ebs".to_owned())])
    }

    fn built(artifact_id: &str) -> Vec<Built> {
        vec![Built {
            name: "ubuntu".into(),
            fields: BTreeMap::from([
                ("artifact_id".to_owned(), Value::String(artifact_id.into())),
                ("build_time".to_owned(), Value::Int(1_794_000_000)),
            ]),
        }]
    }

    #[test]
    fn an_image_id_loses_only_a_single_region() {
        for (artifact, want) in [
            ("us-east-1:ami-0abc", "ami-0abc"),
            ("us-gov-west-1:ami-0abc", "ami-0abc"),
            (
                "us-east-1:ami-1,eu-west-1:ami-2",
                "us-east-1:ami-1,eu-west-1:ami-2",
            ),
            ("sha256:9f86d081", "sha256:9f86d081"),
            (
                "registry.example.com:5000/app",
                "registry.example.com:5000/app",
            ),
            ("Null", "Null"),
            ("", ""),
        ] {
            assert_eq!(image_id(artifact), want, "{artifact}");
        }
    }

    #[test]
    fn checking_keeps_references_and_literals_apart() {
        assert_eq!(check("${ ubuntu.id }", &sources()).unwrap(), "${ubuntu.id}");
        assert_eq!(
            check("cost $${5} for ${ubuntu.artifact_id}", &sources()).unwrap(),
            "cost $${5} for ${ubuntu.artifact_id}"
        );
        assert!(check("${debian.id}", &sources())
            .unwrap_err()
            .contains("does not refer to a source"));
        assert!(check("${ubuntu.public_ip}", &sources())
            .unwrap_err()
            .contains("which a build does not report"));
        assert!(check("${ubuntu}", &sources()).is_err());
        assert!(check("${upper(ubuntu.id)}", &sources()).is_err());
    }

    #[test]
    fn filling_keeps_the_type_of_a_lone_reference() {
        let built = built("us-east-1:ami-0abc");
        assert_eq!(
            fill("${ubuntu.id}", &built).unwrap(),
            Value::String("ami-0abc".into())
        );
        assert_eq!(
            fill("${ubuntu.build_time}", &built).unwrap(),
            Value::Int(1_794_000_000)
        );
        assert_eq!(
            fill(
                "$${x} ${ubuntu.artifact_id} at ${ubuntu.build_time}",
                &built
            )
            .unwrap(),
            Value::String("${x} us-east-1:ami-0abc at 1794000000".into())
        );
        assert_eq!(fill("${ubuntu.files}", &built).unwrap(), Value::Null);
        assert!(fill("${debian.id}", &built)
            .unwrap_err()
            .contains("did not report anything"));
    }
}
