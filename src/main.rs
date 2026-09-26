//! A language server for SOPS-encrypted files.
//!
//! The editor never sees the plaintext on disk: the server decrypts into the buffer over
//! `workspace/applyEdit` and re-encrypts on the way out. Zed advertises `did_save` and not
//! `will_save_wait_until`, so the write direction is unfinished here — see README.md.

use tower_lsp_server::jsonrpc::Result;
use tower_lsp_server::ls_types::{
    Diagnostic, DiagnosticSeverity, DidOpenTextDocumentParams, InitializeParams, InitializeResult,
    Position, Range, ServerCapabilities, ServerInfo, TextDocumentSyncCapability,
    TextDocumentSyncKind,
};
use tower_lsp_server::{Client, LanguageServer, LspService, Server};

struct Backend {
    client: Client,
}

impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            server_info: Some(ServerInfo {
                name: env!("CARGO_PKG_NAME").to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                ..Default::default()
            },
            offset_encoding: None,
        })
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let doc = params.text_document;
        let diagnostics = if is_sops_encrypted(&doc.text) {
            vec![Diagnostic {
                range: first_line(&doc.text),
                severity: Some(DiagnosticSeverity::INFORMATION),
                source: Some(env!("CARGO_PKG_NAME").to_string()),
                message: "SOPS-encrypted file.".to_string(),
                ..Default::default()
            }]
        } else {
            vec![]
        };
        self.client
            .publish_diagnostics(doc.uri, diagnostics, Some(doc.version))
            .await;
    }
}

/// A zero-width range at the start of a document renders as nothing an editor's reader can
/// see, so the marker spans the first line.
fn first_line(text: &str) -> Range {
    let end = text.lines().next().unwrap_or_default().chars().count();
    Range {
        start: Position::new(0, 0),
        end: Position::new(0, u32::try_from(end).unwrap_or(u32::MAX)),
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

#[tokio::main]
async fn main() {
    let (service, socket) = LspService::new(|client| Backend { client });
    Server::new(tokio::io::stdin(), tokio::io::stdout(), socket)
        .serve(service)
        .await;
}

#[cfg(test)]
mod tests {
    use super::is_sops_encrypted;

    #[test]
    fn recognises_an_encrypted_document() {
        let yaml = "key: ENC[AES256_GCM,data:x]\nsops:\n    mac: ENC[AES256_GCM,data:y]\n";
        assert!(is_sops_encrypted(yaml));
        let json = "{\"key\":\"ENC[...]\",\"sops\":{\"mac\":\"ENC[...]\"}}";
        assert!(is_sops_encrypted(json));
    }

    #[test]
    fn leaves_plain_documents_alone() {
        assert!(!is_sops_encrypted("key: value\n"));
        assert!(!is_sops_encrypted("# How to use sops with mac keychains\n"));
    }
}
