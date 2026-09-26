//! Decryption, delegated to the `sops` binary.
//!
//! Nothing here writes a file. The plaintext lives in this process's memory for as long as the
//! document is open, and `forget` drops it when the editor closes the document.

use std::collections::HashMap;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use tower_lsp_server::ls_types::Uri;

/// Decrypted lines per file, invalidated when the file on disk changes.
type Cache = HashMap<String, (Option<SystemTime>, Vec<String>)>;

fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The plaintext `key` assigns to on `line_number`, or `None` when the decrypted document does
/// not carry that key unambiguously. Reporting the wrong value is worse than reporting none.
pub fn value_of(uri: &Uri, key: &str, line_number: usize) -> Result<Option<String>, String> {
    let path = file_path(uri).ok_or("not a local file")?;
    let lines = decrypted(&path)?;

    let same_line = lines.get(line_number).and_then(|line| value_for(line, key));
    if same_line.is_some() {
        return Ok(same_line);
    }
    // sops preserves key order, so the same index is the common case. A document whose
    // formatting shifted is looked up by name instead, and only when the name is unique.
    let mut matches = lines.iter().filter_map(|line| value_for(line, key));
    match (matches.next(), matches.next()) {
        (Some(value), None) => Ok(Some(value)),
        _ => Ok(None),
    }
}

pub fn forget(uri: &Uri) {
    if let Some(path) = file_path(uri) {
        cache().lock().expect("cache lock").remove(&path);
    }
}

/// The decrypted document, from the cache when the file has not changed since.
fn decrypted(path: &str) -> Result<Vec<String>, String> {
    let mtime = modified(path);
    if let Some((cached_mtime, lines)) = cache().lock().expect("cache lock").get(path)
        && *cached_mtime == mtime
    {
        return Ok(lines.clone());
    }

    let output = Command::new("sops")
        .arg("decrypt")
        .arg(path)
        .output()
        .map_err(|err| format!("could not run sops: {err}"))?;
    if !output.status.success() {
        return Err(first_meaningful_line(&String::from_utf8_lossy(
            &output.stderr,
        )));
    }
    let lines: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_string)
        .collect();

    cache()
        .lock()
        .expect("cache lock")
        .insert(path.to_string(), (mtime, lines.clone()));
    Ok(lines)
}

fn modified(path: &str) -> Option<SystemTime> {
    std::fs::metadata(path)
        .and_then(|stat| stat.modified())
        .ok()
}

/// The value a line assigns, when the line assigns to `key`.
fn value_for(line: &str, key: &str) -> Option<String> {
    let (raw_key, rest) = line.split_once(':')?;
    if raw_key.trim().trim_matches(['"', '\'']) != key {
        return None;
    }
    let value = rest.trim().trim_end_matches(',').trim_matches(['"', '\'']);
    if value.is_empty() {
        return None;
    }
    Some(value.to_string())
}

/// sops reports a failure across several lines, headed by a blank one. The first line with
/// content is the part that names the cause.
fn first_meaningful_line(stderr: &str) -> String {
    stderr
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("decryption failed")
        .to_string()
}

/// The local path a `file:` URI names, percent-decoded.
fn file_path(uri: &Uri) -> Option<String> {
    if uri.scheme().as_str() != "file" {
        return None;
    }
    Some(uri.path().decode().to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::{first_meaningful_line, value_for};

    #[test]
    fn reads_the_value_for_a_matching_key() {
        assert_eq!(
            value_for("api_token: sk-demo-0123456789", "api_token").as_deref(),
            Some("sk-demo-0123456789")
        );
        assert_eq!(
            value_for("    \"token\": \"abc\",", "token").as_deref(),
            Some("abc")
        );
    }

    #[test]
    fn refuses_a_line_that_assigns_to_another_key() {
        assert_eq!(value_for("api_token: secret", "database_url"), None);
        assert_eq!(value_for("nested:", "nested"), None);
    }

    #[test]
    fn names_the_cause_of_a_failure() {
        let stderr = "\n\nFailed to get the data key required to decrypt the SOPS file.\n\nGroup 0: FAILED\n";
        assert_eq!(
            first_meaningful_line(stderr),
            "Failed to get the data key required to decrypt the SOPS file."
        );
        assert_eq!(first_meaningful_line(""), "decryption failed");
    }
}
