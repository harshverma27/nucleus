// Webview host for the React trace dashboard.
//
// This is pure plumbing: it opens a webview, loads the *same* bundled dashboard
// the standalone browser uses, and tells it which WebSocket to connect to. All
// trace logic lives in the Rust daemon; the dashboard is a thin display client.

import * as vscode from "vscode";

let panel: vscode.WebviewPanel | undefined;

export function openDashboard(context: vscode.ExtensionContext): void {
  if (panel) {
    panel.reveal();
    return;
  }

  panel = vscode.window.createWebviewPanel(
    "nucleusTrace",
    "Nucleus Trace",
    vscode.ViewColumn.Active,
    {
      enableScripts: true,
      retainContextWhenHidden: true,
      localResourceRoots: [vscode.Uri.joinPath(context.extensionUri, "dist")],
    }
  );

  const wsUrl = vscode.workspace
    .getConfiguration("nucleus")
    .get<string>("traceWebSocket", "ws://localhost:7878");

  panel.webview.html = render(panel.webview, context.extensionUri, wsUrl);
  panel.onDidDispose(() => (panel = undefined), null, context.subscriptions);
}

function render(
  webview: vscode.Webview,
  extensionUri: vscode.Uri,
  wsUrl: string
): string {
  const script = webview.asWebviewUri(
    vscode.Uri.joinPath(extensionUri, "dist", "dashboard.js")
  );
  const style = webview.asWebviewUri(
    vscode.Uri.joinPath(extensionUri, "dist", "dashboard.css")
  );
  const nonce = makeNonce();

  // CSP: only our nonce'd script and bundled style; allow the WebSocket
  // connection to the local trace daemon.
  const csp = [
    "default-src 'none'",
    `style-src ${webview.cspSource} 'unsafe-inline'`,
    `script-src 'nonce-${nonce}'`,
    `img-src ${webview.cspSource} data:`,
    "connect-src ws://localhost:* ws://127.0.0.1:*",
  ].join("; ");

  return `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta http-equiv="Content-Security-Policy" content="${csp}" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <link rel="stylesheet" href="${style}" />
    <title>Nucleus Trace</title>
  </head>
  <body>
    <div id="root"></div>
    <script nonce="${nonce}">window.NUCLEUS_WS_URL = ${JSON.stringify(wsUrl)};</script>
    <script nonce="${nonce}" src="${script}"></script>
  </body>
</html>`;
}

function makeNonce(): string {
  const chars = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
  let nonce = "";
  for (let i = 0; i < 32; i++) {
    nonce += chars.charAt(Math.floor(Math.random() * chars.length));
  }
  return nonce;
}
