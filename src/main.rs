//! A language server for SOPS-encrypted files.
//!
//! Decrypted values are shown, never written. Zed advertises `did_save` and not
//! `will_save_wait_until`, so a server cannot re-encrypt a buffer before the editor flushes it
//! to disk; until that lands, this server does not touch buffer contents at all — see README.md.

mod decrypt;

use std::collections::HashMap;
use std::sync::Mutex;

use tower_lsp_server::jsonrpc::Result;
use tower_lsp_server::ls_types::{
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams, Hover,
    HoverContents, HoverParams, HoverProviderCapability, InitializeParams, InitializeResult,
    MarkupContent, MarkupKind, ServerCapabilities, ServerInfo, TextDocumentSyncCapability,
    TextDocumentSyncKind, Uri,
};
use tower_lsp_server::{LanguageServer, LspService, Server};

struct Backend {
    /// The `sops` binary, from `initialization_options`. It is not always on PATH: a
    /// version-managed install is reachable only through a shim or an absolute path.
    sops: Mutex<String>,
    /// The ciphertext as the editor holds it. Hover reads the key names from here rather than
    /// from disk, so an unsaved edit does not misattribute a value.
    documents: Mutex<HashMap<Uri, String>>,
}

impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        if let Some(path) = sops_path(params.initialization_options.as_ref()) {
            *self.sops.lock().expect("sops lock") = path;
        }
        Ok(InitializeResult {
            server_info: Some(ServerInfo {
                name: env!("CARGO_PKG_NAME").to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                ..Default::default()
            },
            offset_encoding: None,
        })
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    /// Recording the buffer is all an open does. An encrypted file is not a defect, so nothing
    /// here publishes a diagnostic against it.
    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let doc = params.text_document;
        self.documents
            .lock()
            .expect("documents lock")
            .insert(doc.uri, doc.text);
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        // Full sync: the last change carries the whole document.
        if let Some(change) = params.content_changes.into_iter().next_back() {
            self.documents
                .lock()
                .expect("documents lock")
                .insert(params.text_document.uri, change.text);
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.documents
            .lock()
            .expect("documents lock")
            .remove(&params.text_document.uri);
        decrypt::forget(&params.text_document.uri);
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let position = params.text_document_position_params;
        let uri = position.text_document.uri;
        let line_number = position.position.line as usize;

        let Some(text) = self
            .documents
            .lock()
            .expect("documents lock")
            .get(&uri)
            .cloned()
        else {
            return Ok(None);
        };
        if !is_sops_encrypted(&text) {
            return Ok(None);
        }
        let Some(key) = key_at(&text, line_number) else {
            return Ok(None);
        };

        let sops = self.sops.lock().expect("sops lock").clone();
        let value = match decrypt::value_of(&sops, &uri, &key, line_number) {
            Ok(Some(value)) => format!("```\n{value}\n```"),
            Ok(None) => return Ok(None),
            // The reason a decrypt failed is the useful half — a missing key, a stale recipient.
            // It never contains file content.
            Err(reason) => format!("**sops**: {reason}"),
        };

        Ok(Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value,
            }),
            range: None,
        }))
    }
}

/// The `sops.path` an editor passed in `initialization_options`. Nothing between a settings file
/// and this process expands `~`, and a path written by hand is where a `~` appears.
fn sops_path(options: Option<&serde_json::Value>) -> Option<String> {
    let path = options?.get("sops")?.get("path")?.as_str()?.trim();
    (!path.is_empty()).then(|| expand_home(path))
}

fn expand_home(path: &str) -> String {
    let Some(rest) = path.strip_prefix("~/") else {
        return path.to_string();
    };
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() => format!("{}/{rest}", home.trim_end_matches('/')),
        _ => path.to_string(),
    }
}

/// A SOPS file carries a `sops` metadata block holding a `mac`. Both markers are required: a
/// document that merely mentions sops is a document about SOPS, not an encrypted one.
fn is_sops_encrypted(text: &str) -> bool {
    let has_block = text.contains("\nsops:")
        || text.starts_with("sops:")
        || text.contains("\"sops\":")
        || text.contains("[sops]");
    has_block && text.contains("mac")
}

/// The key a line assigns to, for YAML, JSON and TOML alike. A line inside the `sops` metadata
/// block yields nothing: those values are not secrets and reporting them as such teaches the
/// reader the wrong thing.
fn key_at(text: &str, line_number: usize) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    let line = lines.get(line_number)?;
    if in_sops_block(&lines, line_number) {
        return None;
    }
    let (raw_key, rest) = line.split_once(':')?;
    if rest.trim().is_empty() {
        return None;
    }
    let key = raw_key.trim().trim_matches(['"', '\'']);
    if key.is_empty() || key.contains(char::is_whitespace) {
        return None;
    }
    Some(key.to_string())
}

/// Whether a line sits under the top-level `sops:` key, by indentation.
fn in_sops_block(lines: &[&str], line_number: usize) -> bool {
    let indent = |line: &str| line.len() - line.trim_start().len();
    if indent(lines[line_number]) == 0 {
        return lines[line_number].trim_start().starts_with("sops:");
    }
    lines[..line_number]
        .iter()
        .rev()
        .find(|line| !line.trim().is_empty() && indent(line) == 0)
        .is_some_and(|line| line.trim_start().starts_with("sops:"))
}

#[tokio::main]
async fn main() {
    let (service, socket) = LspService::new(|_client| Backend {
        sops: Mutex::new("sops".to_string()),
        documents: Mutex::new(HashMap::new()),
    });
    Server::new(tokio::io::stdin(), tokio::io::stdout(), socket)
        .serve(service)
        .await;
}

#[cfg(test)]
mod tests {
    use super::{is_sops_encrypted, key_at, sops_path};

    const ENCRYPTED: &str = concat!(
        "database_url: ENC[AES256_GCM,data:abc,type:str]\n",
        "nested:\n",
        "    smtp_password: ENC[AES256_GCM,data:def,type:str]\n",
        "sops:\n",
        "    age: []\n",
        "    mac: ENC[AES256_GCM,data:ghi,type:str]\n",
        "    version: 3.13.1\n",
    );

    #[test]
    fn recognises_an_encrypted_document() {
        assert!(is_sops_encrypted(ENCRYPTED));
        assert!(is_sops_encrypted(
            "{\"a\":\"ENC[x]\",\"sops\":{\"mac\":\"ENC[y]\"}}"
        ));
    }

    #[test]
    fn leaves_plain_documents_alone() {
        assert!(!is_sops_encrypted("key: value\n"));
        assert!(!is_sops_encrypted("# How to use sops with mac keychains\n"));
    }

    #[test]
    fn reads_the_key_a_line_assigns_to() {
        assert_eq!(key_at(ENCRYPTED, 0).as_deref(), Some("database_url"));
        assert_eq!(key_at(ENCRYPTED, 2).as_deref(), Some("smtp_password"));
    }

    #[test]
    fn ignores_lines_that_assign_nothing() {
        // `nested:` opens a mapping, and the blank tail is past the end.
        assert_eq!(key_at(ENCRYPTED, 1), None);
        assert_eq!(key_at(ENCRYPTED, 99), None);
    }

    #[test]
    fn reads_the_configured_sops_path() {
        let options = serde_json::json!({"sops": {"path": "/opt/bin/sops"}});
        assert_eq!(sops_path(Some(&options)).as_deref(), Some("/opt/bin/sops"));
    }

    #[test]
    fn expands_a_leading_tilde() {
        // SAFETY: single-threaded test process; no other thread reads the environment.
        unsafe { std::env::set_var("HOME", "/home/someone") };
        let options = serde_json::json!({"sops": {"path": "~/bin/sops"}});
        assert_eq!(
            sops_path(Some(&options)).as_deref(),
            Some("/home/someone/bin/sops")
        );
        let absolute = serde_json::json!({"sops": {"path": "/usr/bin/sops"}});
        assert_eq!(sops_path(Some(&absolute)).as_deref(), Some("/usr/bin/sops"));
    }

    #[test]
    fn falls_back_when_no_path_is_configured() {
        assert_eq!(sops_path(None), None);
        assert_eq!(sops_path(Some(&serde_json::json!({}))), None);
        assert_eq!(
            sops_path(Some(&serde_json::json!({"sops": {"path": "  "}}))),
            None
        );
    }

    #[test]
    fn ignores_the_sops_metadata_block() {
        for line in 3..=6 {
            assert_eq!(key_at(ENCRYPTED, line), None, "line {line} is metadata");
        }
    }
}
