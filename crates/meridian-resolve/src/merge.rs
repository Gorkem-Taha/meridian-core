use std::collections::HashSet;

use meridian_ir::Value;

/// The recursive merge of `base` and `over`, with `over` winning on conflict.
/// Only two maps merge; anything else is replaced whole.
pub(crate) fn deep_merge(base: Value, over: Value) -> Value {
    match (base, over) {
        (Value::Map(mut base), Value::Map(over)) => {
            for (key, value) in over {
                let merged = match base.remove(&key) {
                    Some(existing) => deep_merge(existing, value),
                    None => value,
                };
                base.insert(key, merged);
            }
            Value::Map(base)
        }
        (_, over) => over,
    }
}

/// Concatenates `values` in priority order and drops duplicates, keeping the
/// first occurrence. `dedup_key`, when set, identifies object elements by that
/// field; without it an object element is identified by its whole canonical
/// value.
pub(crate) fn unique_array_merge(
    values: Vec<Value>,
    dedup_key: &str,
) -> Result<Vec<Value>, String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for value in values {
        let Value::List(items) = value else {
            return Err(format!(
                "unique-array-merge requires every layer to provide an array, got {}",
                value.describe()
            ));
        };
        for item in items {
            if seen.insert(identity(&item, dedup_key)?) {
                out.push(item);
            }
        }
    }
    Ok(out)
}

/// The string identity used to detect duplicates.
fn identity(item: &Value, dedup_key: &str) -> Result<String, String> {
    // A primitive in a list configured with a dedup key is identified by its
    // own value; mixing shapes is legal and should not error.
    let (false, Some(map)) = (dedup_key.is_empty(), item.as_map()) else {
        return Ok(item.canonical());
    };
    match map.get(dedup_key) {
        // Prefixed so a keyed identity can never collide with a whole-value one.
        Some(v) => Ok(format!("key:{}", v.canonical())),
        None => Err(format!(
            "unique-array-merge: element {} has no dedup key {dedup_key:?}",
            item.canonical()
        )),
    }
}
