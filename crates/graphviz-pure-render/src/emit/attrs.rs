//! Attribute access with C Graphviz semantics (`late_int`, `late_double`, `late_string`,
//! `late_nnstring`, `mapbool` in `lib/common/utils.c`) and the C integer scanners they rely on
//! (`strtod` is `graphviz_pure_core::strtod`).

use graphviz_pure_core::AttrMap;
use graphviz_pure_core::strtod::strtod_rest;

pub(crate) use graphviz_pure_core::cnum::{atoi, mapbool};

/// Read-only view of an object's attributes with the `late_*` helpers.
#[derive(Clone, Copy)]
pub(crate) struct Attrs<'a>(pub &'a AttrMap);

impl<'a> Attrs<'a> {
    /// `agget`: the value, or `None` when the attribute is unset.
    pub fn get(&self, k: &str) -> Option<&'a str> {
        self.0.get(k).map(String::as_str)
    }

    /// `agget` with C's "declared but empty" folded into `""`.
    pub fn get_or_empty(&self, k: &str) -> &'a str {
        self.get(k).unwrap_or("")
    }

    pub fn late_double(&self, k: &str, dflt: f64, min: f64) -> f64 {
        match self.get(k).filter(|s| !s.is_empty()).and_then(strtod_rest) {
            Some((v, _)) if v < min => min,
            Some((v, _)) => v,
            None => dflt,
        }
    }

    pub fn late_int(&self, k: &str, dflt: i32, min: i32) -> i32 {
        graphviz_pure_core::cnum::late_int(self.get(k), dflt, min)
    }

    pub fn late_string(&self, k: &str, dflt: &'a str) -> &'a str {
        self.get(k).unwrap_or(dflt)
    }

    pub fn late_nnstring(&self, k: &str, dflt: &'a str) -> &'a str {
        self.get(k).filter(|s| !s.is_empty()).unwrap_or(dflt)
    }

    pub fn mapbool(&self, k: &str) -> bool {
        mapbool(self.get(k))
    }

    /// Non-empty value.
    pub fn nonempty(&self, k: &str) -> Option<&'a str> {
        self.get(k).filter(|s| !s.is_empty())
    }
}

/// `FUNLIMIT` in `lib/common/emit.c`: the size of `parse_style`'s array, its terminator included.
const FUNLIMIT: usize = 64;

/// `parse_style` in `lib/common/emit.c`: splits `style` into entries, each a name followed by
/// its parenthesised arguments. A style of more than `FUNLIMIT - 1` entries gives an empty
/// list after `truncating style`, as a parse error does after its message.
pub(crate) fn parse_style(s: &str) -> Vec<StyleItem> {
    let mut out: Vec<StyleItem> = Vec::new();
    let b = s.as_bytes();
    let mut i = 0;
    let mut in_parens = false;
    let is_delim = |c: u8| matches!(c, b'(' | b')' | b',');
    loop {
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b',') {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        match b[i] {
            b'(' => {
                if in_parens {
                    graphviz_pure_core::diag::agerrorf(&format!(
                        "nesting not allowed in style: {s}\n"
                    ));
                    return Vec::new();
                }
                in_parens = true;
                i += 1;
            }
            b')' => {
                if !in_parens {
                    graphviz_pure_core::diag::agerrorf(&format!("unmatched ')' in style: {s}\n"));
                    return Vec::new();
                }
                in_parens = false;
                i += 1;
            }
            _ => {
                let start = i;
                while i < b.len() && !is_delim(b[i]) {
                    i += 1;
                }
                let tok = s[start..i].to_string();
                if in_parens {
                    if let Some(last) = out.last_mut() {
                        last.args.push(tok);
                    }
                } else {
                    if out.len() == FUNLIMIT - 1 {
                        // C returns its static array unfilled here (#268, docs/porting/crashes.md).
                        graphviz_pure_core::diag::agwarningf(&format!("truncating style '{s}'\n"));
                        return Vec::new();
                    }
                    out.push(StyleItem {
                        name: tok,
                        args: Vec::new(),
                    });
                }
            }
        }
    }
    if in_parens {
        graphviz_pure_core::diag::agerrorf(&format!("unmatched '(' in style: {s}\n"));
        return Vec::new();
    }
    out
}

/// One `style` entry, e.g. `setlinewidth(2)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StyleItem {
    pub name: String,
    pub args: Vec<String>,
}

impl StyleItem {
    pub fn simple(name: &str) -> Self {
        Self {
            name: name.to_string(),
            args: Vec::new(),
        }
    }
}

/// `defaultlinestyle` in `lib/common/emit.c`: `solid`, `setlinewidth(1)`.
pub(crate) fn default_line_style() -> Vec<StyleItem> {
    vec![
        StyleItem::simple("solid"),
        StyleItem {
            name: "setlinewidth".into(),
            args: vec!["1".into()],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_style_with_arguments() {
        let s = parse_style("filled, setlinewidth(2) ,dashed");
        assert_eq!(s.len(), 3);
        assert_eq!(s[1].name, "setlinewidth");
        assert_eq!(s[1].args, vec!["2".to_string()]);
        assert!(parse_style("a(b(c))").is_empty());
    }

    #[test]
    fn parse_style_past_63_items_returns_none() {
        let items = |n: usize| (0..n).map(|i| format!("s{i}(1)")).collect::<Vec<_>>();
        let kept = parse_style(&items(63).join(","));
        assert_eq!(kept.len(), 63);
        assert_eq!(kept[62].args, vec!["1".to_string()]);
        assert!(parse_style(&items(64).join(",")).is_empty());
    }
}
