//! The tower-lsp server shell.
//!
//! Keeps an in-memory copy of every open document and, on each edit, runs
//! [`crate::analysis`] and publishes diagnostics. Hover and completion delegate
//! to the same pure functions. All hardware logic lives in the analysis layer
//! (and below it, the compiler/db) — this file is plumbing.

use std::collections::HashMap;

use tokio::sync::Mutex;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

use crate::analysis;

struct Backend {
    client: Client,
    /// Open documents, keyed by URI. FULL text sync keeps these authoritative.
    documents: Mutex<HashMap<Url, String>>,
}

impl Backend {
    fn new(client: Client) -> Backend {
        Backend {
            client,
            documents: Mutex::new(HashMap::new()),
        }
    }

    /// Re-analyze `uri` and push diagnostics to the client.
    async fn publish(&self, uri: Url, text: &str, version: Option<i32>) {
        let diags = analysis::diagnostics(text);
        self.client.publish_diagnostics(uri, diags, version).await;
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            server_info: Some(ServerInfo {
                name: "nucleus-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec!["\"".to_string()]),
                    ..CompletionOptions::default()
                }),
                ..ServerCapabilities::default()
            },
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "nucleus-lsp ready")
            .await;
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let doc = params.text_document;
        self.documents
            .lock()
            .await
            .insert(doc.uri.clone(), doc.text.clone());
        self.publish(doc.uri, &doc.text, Some(doc.version)).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        // FULL sync: the last change carries the entire new document.
        let Some(change) = params.content_changes.into_iter().last() else {
            return;
        };
        let uri = params.text_document.uri;
        self.documents
            .lock()
            .await
            .insert(uri.clone(), change.text.clone());
        self.publish(uri, &change.text, Some(params.text_document.version))
            .await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.documents
            .lock()
            .await
            .remove(&params.text_document.uri);
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let pos = params.text_document_position_params;
        let docs = self.documents.lock().await;
        let Some(text) = docs.get(&pos.text_document.uri) else {
            return Ok(None);
        };
        Ok(analysis::hover(text, pos.position))
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let pos = params.text_document_position;
        let docs = self.documents.lock().await;
        let Some(text) = docs.get(&pos.text_document.uri) else {
            return Ok(None);
        };
        let items = analysis::completion(text, pos.position);
        Ok((!items.is_empty()).then_some(CompletionResponse::Array(items)))
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

/// Run the language server over stdio until the client disconnects. Blocks on a
/// fresh Tokio runtime, so callers (the CLI) stay synchronous.
pub fn run_stdio() -> std::io::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let stdin = tokio::io::stdin();
        let stdout = tokio::io::stdout();
        let (service, socket) = LspService::new(Backend::new);
        Server::new(stdin, stdout, socket).serve(service).await;
    });
    Ok(())
}
