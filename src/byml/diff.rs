//! Structural, path-keyed diff of two BYML value trees.
//!
//! Walks both trees in lockstep, matching hash entries by key and array
//! entries by index, and reports leaf/subtree differences as JSON-pointer-ish
//! paths (`/SystemData/EffectList/3`). Useful for comparing two versions of a
//! game-data file (e.g. `GameDataList.Product.110` vs `.140`).

use serde::Serialize;

use super::Byml;

/// A value present on only one side (an addition or removal). The `value` is a
/// short summary of the subtree rooted there.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DiffEntry {
    pub path: String,
    pub value: String,
}

/// A leaf (or type) that changed between the two trees.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChangedEntry {
    pub path: String,
    pub old: String,
    pub new: String,
}

/// The result of [`diff_byml`].
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct BymlDiff {
    /// Paths present in `new` but not `old`.
    pub added: Vec<DiffEntry>,
    /// Paths present in `old` but not `new`.
    pub removed: Vec<DiffEntry>,
    /// Paths present in both whose value/type changed.
    pub changed: Vec<ChangedEntry>,
}

impl BymlDiff {
    /// True when the two trees are structurally identical.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }

    /// Total number of differences.
    pub fn total(&self) -> usize {
        self.added.len() + self.removed.len() + self.changed.len()
    }
}

/// Diff two BYML trees, matching hashes by key and arrays by index.
pub fn diff_byml(old: &Byml, new: &Byml) -> BymlDiff {
    let mut out = BymlDiff::default();
    diff_node("", old, new, &mut out);
    out
}

fn diff_node(path: &str, old: &Byml, new: &Byml, out: &mut BymlDiff) {
    match (old, new) {
        (Byml::Hash(a), Byml::Hash(b)) => {
            let mut keys: Vec<&str> = a
                .iter()
                .map(|(k, _)| k.as_str())
                .chain(b.iter().map(|(k, _)| k.as_str()))
                .collect();
            keys.sort_unstable();
            keys.dedup();
            for k in keys {
                let av = a.iter().find(|(kk, _)| kk == k).map(|(_, v)| v);
                let bv = b.iter().find(|(kk, _)| kk == k).map(|(_, v)| v);
                let child = format!("{path}/{k}");
                match (av, bv) {
                    (Some(x), Some(y)) => diff_node(&child, x, y, out),
                    (Some(x), None) => out.removed.push(entry(&child, x)),
                    (None, Some(y)) => out.added.push(entry(&child, y)),
                    (None, None) => {}
                }
            }
        }
        (Byml::Array(a), Byml::Array(b)) => {
            for i in 0..a.len().max(b.len()) {
                let child = format!("{path}/{i}");
                match (a.get(i), b.get(i)) {
                    (Some(x), Some(y)) => diff_node(&child, x, y, out),
                    (Some(x), None) => out.removed.push(entry(&child, x)),
                    (None, Some(y)) => out.added.push(entry(&child, y)),
                    (None, None) => {}
                }
            }
        }
        _ => {
            if !byml_eq(old, new) {
                out.changed.push(ChangedEntry {
                    path: root_path(path),
                    old: summary(old),
                    new: summary(new),
                });
            }
        }
    }
}

fn entry(path: &str, v: &Byml) -> DiffEntry {
    DiffEntry {
        path: root_path(path),
        value: summary(v),
    }
}

fn root_path(path: &str) -> String {
    if path.is_empty() {
        "/".to_string()
    } else {
        path.to_string()
    }
}

/// Scalar equality with bitwise float comparison (so a changed-detection pass
/// doesn't trip on `NaN != NaN` or `0.0`/`-0.0`).
fn byml_eq(a: &Byml, b: &Byml) -> bool {
    match (a, b) {
        (Byml::F32(x), Byml::F32(y)) => x.to_bits() == y.to_bits(),
        (Byml::F64(x), Byml::F64(y)) => x.to_bits() == y.to_bits(),
        _ => a == b,
    }
}

/// A short, one-line summary of a value (scalars in full; containers as a
/// kind + size).
fn summary(v: &Byml) -> String {
    match v {
        Byml::Null => "null".to_string(),
        Byml::Bool(b) => format!("bool({b})"),
        Byml::I32(n) => format!("s32({n})"),
        Byml::U32(n) => format!("u32({n})"),
        Byml::F32(n) => format!("f32({n})"),
        Byml::I64(n) => format!("s64({n})"),
        Byml::U64(n) => format!("u64({n})"),
        Byml::F64(n) => format!("f64({n})"),
        Byml::String(s) => format!("string({s:?})"),
        Byml::Binary(b) => format!("binary[{}]", b.len()),
        Byml::Array(a) => format!("array[{}]", a.len()),
        Byml::Hash(h) => format!("hash{{{}}}", h.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(pairs: &[(&str, Byml)]) -> Byml {
        Byml::Hash(pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect())
    }

    #[test]
    fn self_diff_is_empty() {
        let t = hash(&[("a", Byml::U32(1)), ("b", Byml::String("x".into()))]);
        assert!(diff_byml(&t, &t).is_empty());
    }

    #[test]
    fn detects_add_remove_change() {
        let old = hash(&[
            ("keep", Byml::U32(1)),
            ("change", Byml::U32(2)),
            ("gone", Byml::String("bye".into())),
        ]);
        let new = hash(&[
            ("keep", Byml::U32(1)),
            ("change", Byml::U32(99)),
            ("fresh", Byml::Bool(true)),
        ]);
        let d = diff_byml(&old, &new);
        assert_eq!(d.added, vec![DiffEntry { path: "/fresh".into(), value: "bool(true)".into() }]);
        assert_eq!(d.removed, vec![DiffEntry { path: "/gone".into(), value: "string(\"bye\")".into() }]);
        assert_eq!(
            d.changed,
            vec![ChangedEntry { path: "/change".into(), old: "u32(2)".into(), new: "u32(99)".into() }]
        );
    }

    #[test]
    fn nested_paths_and_type_change() {
        let old = hash(&[("list", Byml::Array(vec![Byml::U32(1), Byml::U32(2)]))]);
        // index 1 changes type (u32 -> string), index 2 added.
        let new = hash(&[(
            "list",
            Byml::Array(vec![Byml::U32(1), Byml::String("two".into()), Byml::U32(3)]),
        )]);
        let d = diff_byml(&old, &new);
        assert_eq!(
            d.changed,
            vec![ChangedEntry {
                path: "/list/1".into(),
                old: "u32(2)".into(),
                new: "string(\"two\")".into()
            }]
        );
        assert_eq!(d.added, vec![DiffEntry { path: "/list/2".into(), value: "u32(3)".into() }]);
        assert!(d.removed.is_empty());
    }
}
