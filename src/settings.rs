//! Changing the node's own `vork.toml` in place, keeping its comments: the
//! control commands an app (the network app, the desktop utility) uses to
//! set the name, the connections and the owner's devices.

/// The strings of a TOML array `key = [ ... ]` (one line or several).
pub fn list(text: &str, key: &str) -> Vec<String> {
    let Some((start, end)) = list_span(text, key) else { return Vec::new() };
    text[start..end]
        .split(['[', ']', ',', '\n'])
        .map(str::trim)
        .filter(|p| p.starts_with('"'))
        .map(|p| p.trim_matches('"').to_string())
        .collect()
}

/// Byte span of `key = [ ... ]`, from the key to the closing bracket.
fn list_span(text: &str, key: &str) -> Option<(usize, usize)> {
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        let t = line.trim_start();
        if !t.starts_with('#') && t.split('=').next().map(str::trim) == Some(key) && t.contains('[') {
            let open = at + line.find('[')?;
            let close = open + text[open..].find(']')?;
            return Some((at, close + 1));
        }
        at += line.len();
    }
    None
}

/// The text with `key = [ ... ]` set to `items` (added at the end if missing).
pub fn set_list(text: &str, key: &str, items: &[String]) -> String {
    let value = if items.is_empty() {
        format!("{key} = []")
    } else {
        format!("{key} = [\n{}\n]", items.iter().map(|i| format!("    \"{}\",", escape(i))).collect::<Vec<_>>().join("\n"))
    };
    match list_span(text, key) {
        Some((a, b)) => format!("{}{value}{}", &text[..a], &text[b..]),
        None => append(text, &value),
    }
}

/// The text with `key = value` (a TOML value, already written) set: the
/// line replaced, a commented-out `# key = …` taken instead, or appended.
pub fn set_value(text: &str, key: &str, value: &str) -> String {
    let line = format!("{key} = {value}");
    let mut out = String::with_capacity(text.len() + line.len());
    let mut done = false;
    for l in text.split_inclusive('\n') {
        let t = l.trim_start();
        let is = |s: &str| s.split('=').next().map(str::trim) == Some(key) && s.contains('=');
        if !done && !t.starts_with('#') && is(t) {
            out.push_str(&line);
            out.push('\n');
            done = true;
        } else {
            out.push_str(l);
        }
    }
    if !done {
        // A commented-out default: set it where it is.
        let mut again = String::with_capacity(out.len());
        for l in out.split_inclusive('\n') {
            let t = l.trim_start().trim_start_matches('#').trim_start();
            if !done && l.trim_start().starts_with('#') && t.split('=').next().map(str::trim) == Some(key) && t.contains('=') {
                again.push_str(&line);
                again.push('\n');
                done = true;
            } else {
                again.push_str(l);
            }
        }
        out = again;
    }
    if done { out } else { append(&out, &line) }
}

fn append(text: &str, line: &str) -> String {
    let sep = if text.is_empty() || text.ends_with('\n') { "" } else { "\n" };
    format!("{text}{sep}{line}\n")
}

/// A TOML basic string's contents.
pub fn escape(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect::<String>().replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_and_values() {
        let t = "listen = \"0.0.0.0:51820\"\npeers = [\"a:1\"]\n# name = \"laptop\"\nlan = true\n";
        assert_eq!(list(t, "peers"), ["a:1"]);
        let t2 = set_list(t, "peers", &["a:1".into(), "b:2".into()]);
        assert_eq!(list(&t2, "peers"), ["a:1", "b:2"]);
        assert_eq!(list(&set_list(&t2, "devices", &["x".into()]), "devices"), ["x"]);
        let t3 = set_value(&t2, "name", "\"phone\"");
        assert!(t3.contains("\nname = \"phone\"\n") && !t3.contains("# name"));
        let t4 = set_value(&t3, "lan", "false");
        assert!(t4.contains("lan = false") && !t4.contains("lan = true"));
        let t5 = set_value(&t4, "direct", "true");
        assert!(t5.ends_with("direct = true\n"));
        // Still valid TOML.
        let _: toml::Table = t5.parse().unwrap();
    }
}
