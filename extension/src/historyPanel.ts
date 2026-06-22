// Webview host for the dashboard's history mode (M9).
//
// Pure plumbing, mirroring tracePanel.ts: it runs `nucleus history --graph`,
// injects the resulting trend JSON as `window.NUCLEUS_TREND`, and loads the
// same bundled dashboard. All trend computation happens in the Rust CLI; the
// webview only renders.

import { execFile } from "child_process";
import { promisify } from "util";

import * as vscode from "vscode";

const execFileAsync = promisify(execFile);

let panel: vscode.WebviewPanel | undefined;

export async function openHistory(
  context: vscode.ExtensionContext
): Promise<void> {
  const cli = vscode.workspace
    .getConfiguration("nucleus")
    .get<string>("serverPath", "nucleus");
  const cwd = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;

  let trendJson: string;
  try {
    const { stdout } = await execFileAsync(cli, ["history", "--graph"], { cwd });
    trendJson = stdout.trim() || "{}";
    JSON.parse(trendJson); // validate before injecting
  } catch (err) {
    void vscode.window.showErrorMessage(
      `Nucleus: could not load history (${cli} history --graph): ${err}`
    );
    return;
  }

  if (panel) {
    panel.reveal();
  } else {
    panel = vscode.window.createWebviewPanel(
      "nucleusHistory",
      "Nucleus History",
      vscode.ViewColumn.Active,
      {
        enableScripts: true,
        retainContextWhenHidden: true,
        localResourceRoots: [vscode.Uri.joinPath(context.extensionUri, "dist")],
      }
    );
    panel.onDidDispose(() => (panel = undefined), null, context.subscriptions);
  }

  panel.webview.html = render(panel.webview, context.extensionUri, trendJson);
}

function render(
  webview: vscode.Webview,
  extensionUri: vscode.Uri,
  trendJson: string
): string {
  const script = webview.asWebviewUri(
    vscode.Uri.joinPath(extensionUri, "dist", "dashboard.js")
  );
  const style = webview.asWebviewUri(
    vscode.Uri.joinPath(extensionUri, "dist", "dashboard.css")
  );
  const nonce = makeNonce();

  // No live WebSocket in history mode, but the bundle still resolves one; keep
  // connect-src for parity so toggling to Trace works if a daemon is running.
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
    <title>Nucleus History</title>
  </head>
  <body>
    <div id="root"></div>
    <script nonce="${nonce}">window.NUCLEUS_TREND = JSON.parse(${JSON.stringify(
      trendJson
    )});</script>
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
