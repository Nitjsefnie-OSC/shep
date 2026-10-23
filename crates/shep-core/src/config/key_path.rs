//! How a key path is spelled in a message: `sinks.local.url`, `rules[1].url`.
//!
//! One spelling for every walk that reports where a string sits, so a dog's
//! `dogs.toml` section and a sheep's per-dog table name a key the same way.

/// Appends `.key`, or `key` at the root. A key that is not a bare TOML key
/// is quoted so the path stays unambiguous.
pub fn push_key(path: &mut String, key: &str) {
    if !path.is_empty() {
        path.push('.');
    }
    let bare = !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if bare {
        path.push_str(key);
    } else {
        path.push_str(&format!("{key:?}"));
    }
}

/// Appends `[index]`.
pub fn push_index(path: &mut String, index: usize) {
    path.push_str(&format!("[{index}]"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_key_is_joined_with_a_dot_and_any_other_is_quoted() {
        let mut path = String::new();
        push_key(&mut path, "env");
        push_key(&mut path, "API-KEY_2");
        push_key(&mut path, "has space");
        push_index(&mut path, 1);
        push_key(&mut path, "");
        assert_eq!(path, "env.API-KEY_2.\"has space\"[1].\"\"");
    }
}
