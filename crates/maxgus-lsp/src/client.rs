//! The client itself.
//!
//! [`Client::connect`] is generic over the transport, so the tests drive a
//! real client through an in-memory pipe against a scripted server. Two tokio
//! tasks do the work: one writes queued messages, one decodes incoming frames
//! and routes them — responses to the waiting caller, everything else onto the
//! event channel.

use crate::{
    LspError, Result,
    diagnostics::{Diagnostic, Severity},
    position::{LspPosition, LspRange, PositionEncoding},
    protocol::{
        Decoded, Message, Notification, Request, RequestId, Response, ResponseError, decode,
    },
};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicI64, Ordering},
};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{Mutex, mpsc, oneshot};

/// How long a request waits before giving up.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// How the server syncs document contents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SyncKind {
    /// The server does not want document contents.
    None,
    /// Send the whole document on every change.
    #[default]
    Full,
    /// Send only the changed range.
    Incremental,
}

impl SyncKind {
    fn from_code(code: Option<i64>) -> SyncKind {
        match code {
            Some(0) => SyncKind::None,
            Some(2) => SyncKind::Incremental,
            _ => SyncKind::Full,
        }
    }
}

/// Something the server sent that was not a response to one of our requests.
#[derive(Debug, Clone, PartialEq)]
pub enum ServerEvent {
    /// `textDocument/publishDiagnostics`.
    Diagnostics {
        uri: String,
        diagnostics: Vec<Diagnostic>,
    },
    /// `window/showMessage` or `window/logMessage`.
    Message { severity: Severity, text: String },
    /// A request from the server that the client did not answer itself.
    /// Answer it with [`Client::respond`].
    Request(Request),
    /// A notification the client has no special handling for.
    Notification(Notification),
    /// The transport closed.
    Exited,
}

type Pending = Arc<Mutex<HashMap<RequestId, oneshot::Sender<Result<Value>>>>>;

/// A connection to one language server.
pub struct Client {
    outgoing: mpsc::UnboundedSender<Message>,
    pending: Pending,
    next_id: AtomicI64,
    capabilities: Mutex<Value>,
    encoding: Mutex<PositionEncoding>,
    sync_kind: Mutex<SyncKind>,
    timeout: Duration,
    /// Cleared when the transport closes, so calls fail immediately instead of
    /// waiting out the request timeout.
    alive: Arc<AtomicBool>,
    /// The child process, kept so dropping the client kills the server.
    child: Mutex<Option<tokio::process::Child>>,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("next_id", &self.next_id)
            .finish()
    }
}

impl Client {
    /// Connects over an arbitrary transport, returning the client and the
    /// stream of events the server produces.
    pub fn connect<R, W>(
        reader: R,
        writer: W,
    ) -> (Arc<Client>, mpsc::UnboundedReceiver<ServerEvent>)
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let (outgoing_tx, outgoing_rx) = mpsc::unbounded_channel();
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let alive = Arc::new(AtomicBool::new(true));

        let client = Arc::new(Client {
            outgoing: outgoing_tx.clone(),
            pending: Arc::clone(&pending),
            next_id: AtomicI64::new(1),
            capabilities: Mutex::new(Value::Null),
            encoding: Mutex::new(PositionEncoding::Utf16),
            sync_kind: Mutex::new(SyncKind::Full),
            timeout: DEFAULT_TIMEOUT,
            alive: Arc::clone(&alive),
            child: Mutex::new(None),
        });

        tokio::spawn(write_loop(writer, outgoing_rx, Arc::clone(&alive)));
        tokio::spawn(read_loop(reader, pending, events_tx, outgoing_tx, alive));
        (client, events_rx)
    }

    /// Launches `command` and connects to its stdio.
    pub async fn spawn(
        command: &str,
        args: &[String],
        working_directory: &Path,
    ) -> Result<(Arc<Client>, mpsc::UnboundedReceiver<ServerEvent>)> {
        let mut child = tokio::process::Command::new(command)
            .args(args)
            .current_dir(working_directory)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            // The server's own logs go to the terminal otherwise.
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|source| LspError::Spawn {
                command: command.to_string(),
                source,
            })?;

        let stdin = child.stdin.take().ok_or(LspError::ServerGone)?;
        let stdout = child.stdout.take().ok_or(LspError::ServerGone)?;
        let (client, events) = Client::connect(stdout, stdin);
        *client.child.lock().await = Some(child);
        Ok((client, events))
    }

    /// The position encoding negotiated during initialisation.
    pub async fn encoding(&self) -> PositionEncoding {
        *self.encoding.lock().await
    }

    /// How the server wants document changes delivered.
    pub async fn sync_kind(&self) -> SyncKind {
        *self.sync_kind.lock().await
    }

    /// The server's advertised capabilities.
    pub async fn capabilities(&self) -> Value {
        self.capabilities.lock().await.clone()
    }

    /// True when the server declares support for the given capability key,
    /// e.g. `hoverProvider` or `definitionProvider`.
    pub async fn supports(&self, capability: &str) -> bool {
        let caps = self.capabilities.lock().await;
        match caps.get(capability) {
            None | Some(Value::Null) => false,
            Some(Value::Bool(b)) => *b,
            // An object or anything else means "supported, with options".
            Some(_) => true,
        }
    }

    /// True while the transport is open.
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    /// Sends a notification.
    pub fn notify(&self, method: &str, params: Value) -> Result<()> {
        if !self.is_alive() {
            return Err(LspError::ServerGone);
        }
        self.outgoing
            .send(Message::Notification(Notification {
                method: method.to_string(),
                params,
            }))
            .map_err(|_| LspError::ServerGone)
    }

    /// Sends a request and awaits its response.
    pub async fn request(&self, method: &str, params: Value) -> Result<Value> {
        if !self.is_alive() {
            return Err(LspError::ServerGone);
        }
        let id = RequestId::Number(self.next_id.fetch_add(1, Ordering::Relaxed));
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), tx);

        let sent = self.outgoing.send(Message::Request(Request {
            id: id.clone(),
            method: method.to_string(),
            params,
        }));
        if sent.is_err() {
            self.pending.lock().await.remove(&id);
            return Err(LspError::ServerGone);
        }

        match tokio::time::timeout(self.timeout, rx).await {
            Ok(Ok(result)) => result,
            // The router dropped the sender, which means the transport closed.
            Ok(Err(_)) => Err(LspError::ServerGone),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err(LspError::Timeout(self.timeout))
            }
        }
    }

    /// Answers a request the server made of us.
    pub fn respond(&self, id: RequestId, result: Value) -> Result<()> {
        if !self.is_alive() {
            return Err(LspError::ServerGone);
        }
        self.outgoing
            .send(Message::Response(Response {
                id: Some(id),
                result: Some(result),
                error: None,
            }))
            .map_err(|_| LspError::ServerGone)
    }

    /// Answers a server request with an error.
    pub fn respond_error(&self, id: RequestId, code: i64, message: &str) -> Result<()> {
        if !self.is_alive() {
            return Err(LspError::ServerGone);
        }
        self.outgoing
            .send(Message::Response(Response {
                id: Some(id),
                result: None,
                error: Some(ResponseError {
                    code,
                    message: message.to_string(),
                    data: None,
                }),
            }))
            .map_err(|_| LspError::ServerGone)
    }

    // ---- lifecycle -----------------------------------------------------

    /// Performs the `initialize` handshake and sends `initialized`.
    pub async fn initialize(&self, root: &Path) -> Result<Value> {
        let root_uri = path_to_uri(root);
        let params = json!({
            "processId": std::process::id(),
            "rootUri": root_uri,
            "workspaceFolders": [{ "uri": root_uri, "name": root.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| root.to_string_lossy().into_owned()) }],
            "capabilities": client_capabilities(),
            "clientInfo": { "name": "maxgus", "version": env!("CARGO_PKG_VERSION") },
        });
        let result = self.request("initialize", params).await?;

        let capabilities = result.get("capabilities").cloned().unwrap_or(Value::Null);
        // The server may pick an encoding from the ones we offered.
        let encoding = capabilities
            .get("positionEncoding")
            .and_then(Value::as_str)
            .map(PositionEncoding::from_wire_name)
            .unwrap_or(PositionEncoding::Utf16);
        let sync_kind = document_sync_kind(&capabilities);

        *self.capabilities.lock().await = capabilities;
        *self.encoding.lock().await = encoding;
        *self.sync_kind.lock().await = sync_kind;

        self.notify("initialized", json!({}))?;
        Ok(result)
    }

    /// Sends `shutdown` then `exit`, the orderly way to stop a server.
    pub async fn shutdown(&self) -> Result<()> {
        // A server that fails to answer `shutdown` still gets `exit`.
        let result = self.request("shutdown", Value::Null).await;
        self.notify("exit", Value::Null).ok();
        if let Some(child) = self.child.lock().await.as_mut() {
            // Give it a moment, then make sure it is gone.
            let _ = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
            child.start_kill().ok();
        }
        result.map(|_| ())
    }

    // ---- document synchronisation --------------------------------------

    pub fn did_open(&self, uri: &str, language: &str, version: i64, text: &str) -> Result<()> {
        self.notify(
            "textDocument/didOpen",
            json!({ "textDocument": {
                "uri": uri, "languageId": language, "version": version, "text": text
            }}),
        )
    }

    /// Sends the whole document, for a server using full sync.
    pub fn did_change_full(&self, uri: &str, version: i64, text: &str) -> Result<()> {
        self.notify(
            "textDocument/didChange",
            json!({
                "textDocument": { "uri": uri, "version": version },
                "contentChanges": [{ "text": text }]
            }),
        )
    }

    /// Sends one changed range, for a server using incremental sync.
    pub fn did_change_incremental(
        &self,
        uri: &str,
        version: i64,
        range: LspRange,
        text: &str,
    ) -> Result<()> {
        self.notify(
            "textDocument/didChange",
            json!({
                "textDocument": { "uri": uri, "version": version },
                "contentChanges": [{ "range": range, "text": text }]
            }),
        )
    }

    pub fn did_save(&self, uri: &str, text: Option<&str>) -> Result<()> {
        let mut params = json!({ "textDocument": { "uri": uri } });
        if let Some(text) = text {
            params["text"] = Value::String(text.to_string());
        }
        self.notify("textDocument/didSave", params)
    }

    pub fn did_close(&self, uri: &str) -> Result<()> {
        self.notify(
            "textDocument/didClose",
            json!({ "textDocument": { "uri": uri } }),
        )
    }

    // ---- language features ---------------------------------------------

    fn document_position(uri: &str, position: LspPosition) -> Value {
        json!({ "textDocument": { "uri": uri }, "position": position })
    }

    pub async fn hover(&self, uri: &str, position: LspPosition) -> Result<Value> {
        self.request("textDocument/hover", Self::document_position(uri, position))
            .await
    }

    pub async fn definition(&self, uri: &str, position: LspPosition) -> Result<Value> {
        self.request(
            "textDocument/definition",
            Self::document_position(uri, position),
        )
        .await
    }

    pub async fn references(&self, uri: &str, position: LspPosition) -> Result<Value> {
        let mut params = Self::document_position(uri, position);
        params["context"] = json!({ "includeDeclaration": true });
        self.request("textDocument/references", params).await
    }

    pub async fn completion(&self, uri: &str, position: LspPosition) -> Result<Value> {
        self.request(
            "textDocument/completion",
            Self::document_position(uri, position),
        )
        .await
    }

    pub async fn signature_help(&self, uri: &str, position: LspPosition) -> Result<Value> {
        self.request(
            "textDocument/signatureHelp",
            Self::document_position(uri, position),
        )
        .await
    }

    pub async fn rename(&self, uri: &str, position: LspPosition, new_name: &str) -> Result<Value> {
        let mut params = Self::document_position(uri, position);
        params["newName"] = Value::String(new_name.to_string());
        self.request("textDocument/rename", params).await
    }

    pub async fn formatting(
        &self,
        uri: &str,
        tab_size: usize,
        insert_spaces: bool,
    ) -> Result<Value> {
        self.request(
            "textDocument/formatting",
            json!({
                "textDocument": { "uri": uri },
                "options": { "tabSize": tab_size, "insertSpaces": insert_spaces }
            }),
        )
        .await
    }

    pub async fn code_action(
        &self,
        uri: &str,
        range: LspRange,
        diagnostics: &[Diagnostic],
    ) -> Result<Value> {
        let diagnostics: Vec<Value> = diagnostics
            .iter()
            .map(|d| json!({ "range": d.range, "severity": d.severity as i64, "message": d.message }))
            .collect();
        self.request(
            "textDocument/codeAction",
            json!({
                "textDocument": { "uri": uri },
                "range": range,
                "context": { "diagnostics": diagnostics }
            }),
        )
        .await
    }

    pub async fn document_symbols(&self, uri: &str) -> Result<Value> {
        self.request(
            "textDocument/documentSymbol",
            json!({ "textDocument": { "uri": uri } }),
        )
        .await
    }

    pub async fn workspace_symbols(&self, query: &str) -> Result<Value> {
        self.request("workspace/symbol", json!({ "query": query }))
            .await
    }
}

/// The capabilities `maxgus` advertises.
fn client_capabilities() -> Value {
    json!({
        "general": { "positionEncodings": ["utf-16", "utf-8", "utf-32"] },
        "textDocument": {
            "synchronization": { "didSave": true, "willSave": false, "dynamicRegistration": false },
            "publishDiagnostics": { "relatedInformation": false },
            "hover": { "contentFormat": ["plaintext", "markdown"] },
            "completion": {
                "completionItem": { "snippetSupport": false, "documentationFormat": ["plaintext"] }
            },
            "definition": { "linkSupport": false },
            "references": {},
            "documentSymbol": { "hierarchicalDocumentSymbolSupport": true },
            "formatting": {},
            "rename": { "prepareSupport": false },
            "codeAction": { "codeActionLiteralSupport": {
                "codeActionKind": { "valueSet": ["quickfix", "refactor", "source"] }
            }},
            "signatureHelp": {}
        },
        "workspace": {
            "workspaceFolders": true,
            "symbol": {},
            "applyEdit": true,
            "configuration": true
        }
    })
}

/// Extracts the sync kind from a server's capabilities, which may be a bare
/// number or an object with a `change` field.
fn document_sync_kind(capabilities: &Value) -> SyncKind {
    match capabilities.get("textDocumentSync") {
        Some(Value::Number(n)) => SyncKind::from_code(n.as_i64()),
        Some(object) => SyncKind::from_code(object.get("change").and_then(Value::as_i64)),
        None => SyncKind::Full,
    }
}

/// Converts a filesystem path into a `file://` URI.
pub fn path_to_uri(path: &Path) -> String {
    match url::Url::from_file_path(path) {
        Ok(url) => url.to_string(),
        // `from_file_path` only fails for a relative path; encode it by hand
        // rather than losing the location entirely.
        Err(()) => {
            let text = path.to_string_lossy();
            let encoded =
                percent_encoding::utf8_percent_encode(&text, percent_encoding::NON_ALPHANUMERIC);
            format!("file:///{encoded}")
        }
    }
}

/// Converts a `file://` URI back into a path.
pub fn uri_to_path(uri: &str) -> Option<std::path::PathBuf> {
    url::Url::parse(uri).ok()?.to_file_path().ok()
}

/// Drains the outgoing queue onto the transport.
async fn write_loop<W: AsyncWrite + Unpin>(
    mut writer: W,
    mut queue: mpsc::UnboundedReceiver<Message>,
    alive: Arc<AtomicBool>,
) {
    while let Some(message) = queue.recv().await {
        if writer.write_all(&message.encode()).await.is_err() {
            break;
        }
        if writer.flush().await.is_err() {
            break;
        }
    }
    alive.store(false, Ordering::Release);
}

/// Decodes incoming frames and routes them.
async fn read_loop<R: AsyncRead + Unpin>(
    mut reader: R,
    pending: Pending,
    events: mpsc::UnboundedSender<ServerEvent>,
    outgoing: mpsc::UnboundedSender<Message>,
    alive: Arc<AtomicBool>,
) {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        // Drain everything already buffered before reading more.
        loop {
            match decode(&buffer) {
                Ok(Decoded::Message(message, used)) => {
                    buffer.drain(..used);
                    route(*message, &pending, &events, &outgoing).await;
                }
                Ok(Decoded::Incomplete) => break,
                Err(_) => {
                    // A frame we cannot parse desynchronises the stream; the
                    // only safe recovery is to stop.
                    alive.store(false, Ordering::Release);
                    let _ = events.send(ServerEvent::Exited);
                    return;
                }
            }
        }
        match reader.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => buffer.extend_from_slice(&chunk[..n]),
        }
    }
    // Mark the transport dead before waking anyone, so a caller that retries
    // on being woken fails immediately rather than waiting out the timeout.
    alive.store(false, Ordering::Release);
    // Wake every caller still waiting, rather than leaving them to time out.
    for (_, sender) in pending.lock().await.drain() {
        let _ = sender.send(Err(LspError::ServerGone));
    }
    let _ = events.send(ServerEvent::Exited);
}

/// Delivers one decoded message to whoever should see it.
async fn route(
    message: Message,
    pending: &Pending,
    events: &mpsc::UnboundedSender<ServerEvent>,
    outgoing: &mpsc::UnboundedSender<Message>,
) {
    match message {
        Message::Response(response) => {
            let Some(id) = response.id else { return };
            let Some(sender) = pending.lock().await.remove(&id) else {
                // A response to a request we already gave up on.
                return;
            };
            let result = match response.error {
                Some(e) => Err(LspError::ServerError {
                    code: e.code,
                    message: e.message,
                }),
                None => Ok(response.result.unwrap_or(Value::Null)),
            };
            let _ = sender.send(result);
        }
        Message::Notification(notification) => {
            let event = match notification.method.as_str() {
                "textDocument/publishDiagnostics" => {
                    let uri = notification
                        .params
                        .get("uri")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let diagnostics = notification
                        .params
                        .get("diagnostics")
                        .and_then(Value::as_array)
                        .map(|list| list.iter().filter_map(Diagnostic::from_json).collect())
                        .unwrap_or_default();
                    ServerEvent::Diagnostics { uri, diagnostics }
                }
                "window/showMessage" | "window/logMessage" => ServerEvent::Message {
                    severity: Severity::from_code(
                        notification.params.get("type").and_then(Value::as_i64),
                    ),
                    text: notification
                        .params
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                },
                _ => ServerEvent::Notification(notification),
            };
            let _ = events.send(event);
        }
        Message::Request(request) => {
            // Answer the housekeeping requests ourselves; a server blocks
            // waiting for these and would otherwise never finish starting up.
            let auto = match request.method.as_str() {
                "client/registerCapability" | "client/unregisterCapability" => Some(Value::Null),
                "window/workDoneProgress/create" => Some(Value::Null),
                "workspace/configuration" => {
                    // One null per requested item: we hold no settings.
                    let count = request
                        .params
                        .get("items")
                        .and_then(Value::as_array)
                        .map_or(0, Vec::len);
                    Some(Value::Array(vec![Value::Null; count]))
                }
                _ => None,
            };
            match auto {
                Some(result) => {
                    let _ = outgoing.send(Message::Response(Response {
                        id: Some(request.id),
                        result: Some(result),
                        error: None,
                    }));
                }
                None => {
                    let _ = events.send(ServerEvent::Request(request));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::DuplexStream;

    /// The other end of the pipe: reads what the client sends and replies with
    /// whatever the test scripts.
    struct MockServer {
        reader: DuplexStream,
        writer: DuplexStream,
        buffer: Vec<u8>,
    }

    impl MockServer {
        /// Reads one message the client sent.
        async fn recv(&mut self) -> Message {
            loop {
                if let Decoded::Message(message, used) = decode(&self.buffer).unwrap() {
                    self.buffer.drain(..used);
                    return *message;
                }
                let mut chunk = [0u8; 4096];
                let n = self
                    .reader
                    .read(&mut chunk)
                    .await
                    .expect("pipe stayed open");
                assert_ne!(n, 0, "the client closed the connection");
                self.buffer.extend_from_slice(&chunk[..n]);
            }
        }

        /// Reads one message and asserts it is a request for `method`.
        async fn expect_request(&mut self, method: &str) -> Request {
            match self.recv().await {
                Message::Request(r) if r.method == method => r,
                other => panic!("expected a `{method}` request, got {other:?}"),
            }
        }

        async fn expect_notification(&mut self, method: &str) -> Notification {
            match self.recv().await {
                Message::Notification(n) if n.method == method => n,
                other => panic!("expected a `{method}` notification, got {other:?}"),
            }
        }

        async fn send(&mut self, message: Message) {
            self.writer.write_all(&message.encode()).await.unwrap();
            self.writer.flush().await.unwrap();
        }

        async fn reply(&mut self, id: RequestId, result: Value) {
            self.send(Message::Response(Response {
                id: Some(id),
                result: Some(result),
                error: None,
            }))
            .await;
        }

        async fn reply_error(&mut self, id: RequestId, code: i64, message: &str) {
            self.send(Message::Response(Response {
                id: Some(id),
                result: None,
                error: Some(ResponseError {
                    code,
                    message: message.into(),
                    data: None,
                }),
            }))
            .await;
        }

        async fn notify(&mut self, method: &str, params: Value) {
            self.send(Message::Notification(Notification {
                method: method.into(),
                params,
            }))
            .await;
        }

        /// Closes the pipe, as a crashed server would.
        fn hang_up(self) {}
    }

    fn connected() -> (
        Arc<Client>,
        mpsc::UnboundedReceiver<ServerEvent>,
        MockServer,
    ) {
        let (client_reader, server_writer) = tokio::io::duplex(64 * 1024);
        let (server_reader, client_writer) = tokio::io::duplex(64 * 1024);
        let (client, events) = Client::connect(client_reader, client_writer);
        (
            client,
            events,
            MockServer {
                reader: server_reader,
                writer: server_writer,
                buffer: Vec::new(),
            },
        )
    }

    #[tokio::test]
    async fn a_request_is_matched_with_its_response() {
        let (client, _events, mut server) = connected();
        let task = {
            let client = Arc::clone(&client);
            tokio::spawn(async move { client.request("test/method", json!({"a": 1})).await })
        };
        let request = server.expect_request("test/method").await;
        assert_eq!(request.params, json!({"a": 1}));
        server.reply(request.id, json!({"ok": true})).await;
        assert_eq!(task.await.unwrap().unwrap(), json!({"ok": true}));
    }

    #[tokio::test]
    async fn responses_are_routed_by_id_even_when_they_arrive_out_of_order() {
        let (client, _events, mut server) = connected();
        let first = {
            let c = Arc::clone(&client);
            tokio::spawn(async move { c.request("first", Value::Null).await })
        };
        let a = server.expect_request("first").await;
        let second = {
            let c = Arc::clone(&client);
            tokio::spawn(async move { c.request("second", Value::Null).await })
        };
        let b = server.expect_request("second").await;
        assert_ne!(a.id, b.id, "each request gets a fresh id");

        // Answer the second one first.
        server.reply(b.id, json!("b")).await;
        server.reply(a.id, json!("a")).await;
        assert_eq!(first.await.unwrap().unwrap(), json!("a"));
        assert_eq!(second.await.unwrap().unwrap(), json!("b"));
    }

    #[tokio::test]
    async fn a_server_error_response_becomes_an_error() {
        let (client, _events, mut server) = connected();
        let task = {
            let c = Arc::clone(&client);
            tokio::spawn(async move { c.request("bad", Value::Null).await })
        };
        let request = server.expect_request("bad").await;
        server
            .reply_error(request.id, -32601, "method not found")
            .await;
        let err = task.await.unwrap().unwrap_err();
        assert!(matches!(err, LspError::ServerError { code: -32601, .. }));
        assert!(err.to_string().contains("method not found"));
    }

    #[tokio::test]
    async fn a_pending_request_fails_when_the_server_disappears() {
        let (client, _events, mut server) = connected();
        let task = {
            let c = Arc::clone(&client);
            tokio::spawn(async move { c.request("hangs", Value::Null).await })
        };
        server.expect_request("hangs").await;
        server.hang_up();
        assert!(matches!(task.await.unwrap(), Err(LspError::ServerGone)));
    }

    #[tokio::test]
    async fn a_slow_server_times_out_rather_than_hanging() {
        let (client_reader, _server_writer) = tokio::io::duplex(1024);
        let (mut server_reader, client_writer) = tokio::io::duplex(1024);
        let (client, _events) = Client::connect(client_reader, client_writer);
        // Reach into the client to shorten the wait; the production default is
        // ten seconds, which no test should sit through.
        let client = Arc::new(Client {
            timeout: Duration::from_millis(50),
            ..unwrap_arc(client)
        });

        let task = {
            let c = Arc::clone(&client);
            tokio::spawn(async move { c.request("ignored", Value::Null).await })
        };
        // Drain the request so the write side does not block, then never reply.
        let mut sink = [0u8; 1024];
        let _ = server_reader.read(&mut sink).await;
        assert!(matches!(task.await.unwrap(), Err(LspError::Timeout(_))));
    }

    /// Takes an `Arc<Client>` apart so a test can rebuild it with a different
    /// timeout. Sound only because the test holds the sole reference.
    fn unwrap_arc(client: Arc<Client>) -> Client {
        Arc::try_unwrap(client).unwrap_or_else(|_| panic!("the test holds the only reference"))
    }

    #[tokio::test]
    async fn the_initialize_handshake_records_what_the_server_declared() {
        let (client, _events, mut server) = connected();
        let task = {
            let c = Arc::clone(&client);
            tokio::spawn(async move { c.initialize(Path::new("/tmp/project")).await })
        };
        let request = server.expect_request("initialize").await;
        assert_eq!(
            request.params["rootUri"].as_str().unwrap(),
            "file:///tmp/project"
        );
        assert!(request.params["capabilities"]["textDocument"]["hover"].is_object());

        server
            .reply(
                request.id,
                json!({ "capabilities": {
                    "positionEncoding": "utf-8",
                    "textDocumentSync": 2,
                    "hoverProvider": true,
                    "definitionProvider": { "workDoneProgress": false },
                    "renameProvider": false
                }}),
            )
            .await;
        task.await.unwrap().unwrap();

        // `initialized` follows the handshake.
        server.expect_notification("initialized").await;

        assert_eq!(client.encoding().await, PositionEncoding::Utf8);
        assert_eq!(client.sync_kind().await, SyncKind::Incremental);
        assert!(client.supports("hoverProvider").await);
        assert!(
            client.supports("definitionProvider").await,
            "an options object counts"
        );
        assert!(!client.supports("renameProvider").await, "explicitly false");
        assert!(!client.supports("codeLensProvider").await, "absent");
    }

    #[tokio::test]
    async fn initialisation_defaults_when_the_server_says_little() {
        let (client, _events, mut server) = connected();
        let task = {
            let c = Arc::clone(&client);
            tokio::spawn(async move { c.initialize(Path::new("/tmp")).await })
        };
        let request = server.expect_request("initialize").await;
        server
            .reply(request.id, json!({ "capabilities": {} }))
            .await;
        task.await.unwrap().unwrap();
        assert_eq!(
            client.encoding().await,
            PositionEncoding::Utf16,
            "the protocol default"
        );
        assert_eq!(client.sync_kind().await, SyncKind::Full);
    }

    #[tokio::test]
    async fn diagnostics_arrive_as_events() {
        let (_client, mut events, mut server) = connected();
        server
            .notify(
                "textDocument/publishDiagnostics",
                json!({
                    "uri": "file:///a.rs",
                    "diagnostics": [{
                        "range": {"start": {"line": 1, "character": 0}, "end": {"line": 1, "character": 4}},
                        "severity": 1,
                        "message": "boom"
                    }]
                }),
            )
            .await;
        let event = events.recv().await.unwrap();
        let ServerEvent::Diagnostics { uri, diagnostics } = event else {
            panic!("expected diagnostics, got {event:?}");
        };
        assert_eq!(uri, "file:///a.rs");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].message, "boom");
        assert_eq!(diagnostics[0].severity, Severity::Error);
    }

    #[tokio::test]
    async fn a_malformed_diagnostic_is_skipped_without_dropping_the_rest() {
        let (_client, mut events, mut server) = connected();
        server
            .notify(
                "textDocument/publishDiagnostics",
                json!({
                    "uri": "file:///a.rs",
                    "diagnostics": [
                        {"message": "no range"},
                        {
                            "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
                            "message": "good"
                        }
                    ]
                }),
            )
            .await;
        let ServerEvent::Diagnostics { diagnostics, .. } = events.recv().await.unwrap() else {
            panic!()
        };
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].message, "good");
    }

    #[tokio::test]
    async fn server_messages_arrive_as_events() {
        let (_client, mut events, mut server) = connected();
        server
            .notify(
                "window/showMessage",
                json!({ "type": 2, "message": "heads up" }),
            )
            .await;
        let ServerEvent::Message { severity, text } = events.recv().await.unwrap() else {
            panic!()
        };
        assert_eq!(severity, Severity::Warning);
        assert_eq!(text, "heads up");
    }

    #[tokio::test]
    async fn an_unrecognised_notification_is_passed_through() {
        let (_client, mut events, mut server) = connected();
        server.notify("$/progress", json!({ "token": 1 })).await;
        let ServerEvent::Notification(n) = events.recv().await.unwrap() else {
            panic!()
        };
        assert_eq!(n.method, "$/progress");
    }

    #[tokio::test]
    async fn housekeeping_requests_are_answered_automatically() {
        let (_client, _events, mut server) = connected();
        server
            .send(Message::Request(Request {
                id: RequestId::Number(100),
                method: "client/registerCapability".into(),
                params: json!({ "registrations": [] }),
            }))
            .await;
        match server.recv().await {
            Message::Response(r) => {
                assert_eq!(r.id, Some(RequestId::Number(100)));
                assert!(r.error.is_none());
            }
            other => panic!("expected an automatic response, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_configuration_request_gets_one_null_per_item() {
        let (_client, _events, mut server) = connected();
        server
            .send(Message::Request(Request {
                id: RequestId::Number(5),
                method: "workspace/configuration".into(),
                params: json!({ "items": [{"section": "a"}, {"section": "b"}] }),
            }))
            .await;
        let Message::Response(r) = server.recv().await else {
            panic!()
        };
        assert_eq!(r.result, Some(json!([null, null])));
    }

    #[tokio::test]
    async fn other_server_requests_are_handed_to_the_application() {
        let (client, mut events, mut server) = connected();
        server
            .send(Message::Request(Request {
                id: RequestId::Number(9),
                method: "workspace/applyEdit".into(),
                params: json!({ "edit": {} }),
            }))
            .await;
        let ServerEvent::Request(request) = events.recv().await.unwrap() else {
            panic!("expected the request to be forwarded")
        };
        assert_eq!(request.method, "workspace/applyEdit");
        client
            .respond(request.id, json!({ "applied": false }))
            .unwrap();
        let Message::Response(r) = server.recv().await else {
            panic!()
        };
        assert_eq!(r.result, Some(json!({ "applied": false })));
    }

    #[tokio::test]
    async fn an_application_can_refuse_a_server_request() {
        let (client, mut events, mut server) = connected();
        server
            .send(Message::Request(Request {
                id: RequestId::Number(3),
                method: "workspace/applyEdit".into(),
                params: Value::Null,
            }))
            .await;
        let ServerEvent::Request(request) = events.recv().await.unwrap() else {
            panic!()
        };
        client
            .respond_error(request.id, -32601, "not supported")
            .unwrap();
        let Message::Response(r) = server.recv().await else {
            panic!()
        };
        assert_eq!(r.error.unwrap().code, -32601);
    }

    #[tokio::test]
    async fn document_lifecycle_notifications_carry_the_right_shape() {
        let (client, _events, mut server) = connected();
        client
            .did_open("file:///a.rs", "rust", 1, "fn main() {}")
            .unwrap();
        let n = server.expect_notification("textDocument/didOpen").await;
        assert_eq!(n.params["textDocument"]["languageId"], "rust");
        assert_eq!(n.params["textDocument"]["version"], 1);
        assert_eq!(n.params["textDocument"]["text"], "fn main() {}");

        client
            .did_change_full("file:///a.rs", 2, "fn main() { }")
            .unwrap();
        let n = server.expect_notification("textDocument/didChange").await;
        assert_eq!(n.params["textDocument"]["version"], 2);
        assert_eq!(n.params["contentChanges"][0]["text"], "fn main() { }");
        assert!(
            n.params["contentChanges"][0].get("range").is_none(),
            "full sync sends no range"
        );

        let range = LspRange::new(LspPosition::new(0, 3), LspPosition::new(0, 7));
        client
            .did_change_incremental("file:///a.rs", 3, range, "new")
            .unwrap();
        let n = server.expect_notification("textDocument/didChange").await;
        assert_eq!(
            n.params["contentChanges"][0]["range"]["start"]["character"],
            3
        );
        assert_eq!(n.params["contentChanges"][0]["text"], "new");

        client.did_save("file:///a.rs", Some("saved")).unwrap();
        let n = server.expect_notification("textDocument/didSave").await;
        assert_eq!(n.params["text"], "saved");

        client.did_close("file:///a.rs").unwrap();
        let n = server.expect_notification("textDocument/didClose").await;
        assert_eq!(n.params["textDocument"]["uri"], "file:///a.rs");
    }

    #[tokio::test]
    async fn did_save_omits_the_text_when_the_server_does_not_want_it() {
        let (client, _events, mut server) = connected();
        client.did_save("file:///a.rs", None).unwrap();
        let n = server.expect_notification("textDocument/didSave").await;
        assert!(n.params.get("text").is_none());
    }

    #[tokio::test]
    async fn language_feature_requests_send_the_documented_parameters() {
        let (client, _events, mut server) = connected();
        let position = LspPosition::new(4, 2);

        macro_rules! check {
            ($call:expr, $method:literal, $check:expr) => {{
                let c = Arc::clone(&client);
                let task = tokio::spawn(async move {
                    let client = c;
                    $call(client).await
                });
                let request = server.expect_request($method).await;
                #[allow(clippy::redundant_closure_call)]
                $check(&request.params);
                server.reply(request.id, Value::Null).await;
                task.await.unwrap().unwrap();
            }};
        }

        check!(
            |c: Arc<Client>| async move { c.hover("file:///a.rs", position).await },
            "textDocument/hover",
            |p: &Value| {
                assert_eq!(p["textDocument"]["uri"], "file:///a.rs");
                assert_eq!(p["position"]["line"], 4);
                assert_eq!(p["position"]["character"], 2);
            }
        );
        check!(
            |c: Arc<Client>| async move { c.definition("file:///a.rs", position).await },
            "textDocument/definition",
            |p: &Value| assert_eq!(p["position"]["line"], 4)
        );
        check!(
            |c: Arc<Client>| async move { c.references("file:///a.rs", position).await },
            "textDocument/references",
            |p: &Value| assert_eq!(p["context"]["includeDeclaration"], true)
        );
        check!(
            |c: Arc<Client>| async move { c.completion("file:///a.rs", position).await },
            "textDocument/completion",
            |p: &Value| assert_eq!(p["textDocument"]["uri"], "file:///a.rs")
        );
        check!(
            |c: Arc<Client>| async move { c.signature_help("file:///a.rs", position).await },
            "textDocument/signatureHelp",
            |p: &Value| assert_eq!(p["position"]["character"], 2)
        );
        check!(
            |c: Arc<Client>| async move { c.rename("file:///a.rs", position, "renamed").await },
            "textDocument/rename",
            |p: &Value| assert_eq!(p["newName"], "renamed")
        );
        check!(
            |c: Arc<Client>| async move { c.formatting("file:///a.rs", 4, true).await },
            "textDocument/formatting",
            |p: &Value| {
                assert_eq!(p["options"]["tabSize"], 4);
                assert_eq!(p["options"]["insertSpaces"], true);
            }
        );
        check!(
            |c: Arc<Client>| async move { c.document_symbols("file:///a.rs").await },
            "textDocument/documentSymbol",
            |p: &Value| assert_eq!(p["textDocument"]["uri"], "file:///a.rs")
        );
        check!(
            |c: Arc<Client>| async move { c.workspace_symbols("needle").await },
            "workspace/symbol",
            |p: &Value| assert_eq!(p["query"], "needle")
        );
    }

    #[tokio::test]
    async fn a_code_action_request_carries_the_diagnostics_in_range() {
        let (client, _events, mut server) = connected();
        let range = LspRange::new(LspPosition::new(1, 0), LspPosition::new(1, 10));
        let diagnostic = Diagnostic::new(range, Severity::Warning, "unused");
        let task = {
            let c = Arc::clone(&client);
            tokio::spawn(async move { c.code_action("file:///a.rs", range, &[diagnostic]).await })
        };
        let request = server.expect_request("textDocument/codeAction").await;
        assert_eq!(
            request.params["context"]["diagnostics"][0]["message"],
            "unused"
        );
        assert_eq!(request.params["context"]["diagnostics"][0]["severity"], 2);
        server.reply(request.id, json!([])).await;
        task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn shutting_down_sends_shutdown_then_exit() {
        let (client, _events, mut server) = connected();
        let task = {
            let c = Arc::clone(&client);
            tokio::spawn(async move { c.shutdown().await })
        };
        let request = server.expect_request("shutdown").await;
        server.reply(request.id, Value::Null).await;
        server.expect_notification("exit").await;
        task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn the_event_stream_reports_the_server_going_away() {
        let (_client, mut events, server) = connected();
        server.hang_up();
        assert_eq!(events.recv().await, Some(ServerEvent::Exited));
    }

    #[tokio::test]
    async fn sending_after_the_server_is_gone_is_an_error_not_a_panic() {
        let (client, mut events, server) = connected();
        server.hang_up();
        // Wait for the reader to notice.
        assert_eq!(events.recv().await, Some(ServerEvent::Exited));
        assert!(matches!(
            client.request("anything", Value::Null).await,
            Err(LspError::ServerGone)
        ));
    }

    #[tokio::test]
    async fn spawning_a_missing_executable_reports_which_one() {
        let err = Client::spawn("maxgus-no-such-language-server", &[], Path::new("/tmp"))
            .await
            .unwrap_err();
        assert!(matches!(err, LspError::Spawn { .. }));
        assert!(err.to_string().contains("maxgus-no-such-language-server"));
    }

    #[test]
    fn paths_and_uris_round_trip() {
        let path = Path::new("/tmp/maxgus/src/main.rs");
        let uri = path_to_uri(path);
        assert_eq!(uri, "file:///tmp/maxgus/src/main.rs");
        assert_eq!(uri_to_path(&uri).unwrap(), path);
    }

    #[test]
    fn uris_escape_the_characters_that_need_it() {
        let uri = path_to_uri(Path::new("/tmp/with space/a#b.rs"));
        assert!(!uri.contains(' '), "a raw space would break the URI: {uri}");
        assert_eq!(
            uri_to_path(&uri).unwrap(),
            Path::new("/tmp/with space/a#b.rs")
        );
    }

    #[test]
    fn a_non_file_uri_has_no_path() {
        assert!(uri_to_path("untitled:Untitled-1").is_none());
        assert!(uri_to_path("not a uri").is_none());
    }

    #[test]
    fn sync_kinds_parse_from_both_capability_shapes() {
        assert_eq!(
            document_sync_kind(&json!({"textDocumentSync": 0})),
            SyncKind::None
        );
        assert_eq!(
            document_sync_kind(&json!({"textDocumentSync": 1})),
            SyncKind::Full
        );
        assert_eq!(
            document_sync_kind(&json!({"textDocumentSync": 2})),
            SyncKind::Incremental
        );
        assert_eq!(
            document_sync_kind(&json!({"textDocumentSync": {"openClose": true, "change": 2}})),
            SyncKind::Incremental
        );
        assert_eq!(
            document_sync_kind(&json!({})),
            SyncKind::Full,
            "unstated means full"
        );
    }

    #[test]
    fn the_advertised_capabilities_cover_the_features_we_call() {
        let caps = client_capabilities();
        for feature in [
            "hover",
            "completion",
            "definition",
            "references",
            "formatting",
            "rename",
            "codeAction",
        ] {
            assert!(
                caps["textDocument"].get(feature).is_some(),
                "`{feature}` is called but not advertised"
            );
        }
        assert_eq!(caps["general"]["positionEncodings"][0], "utf-16");
    }
}
