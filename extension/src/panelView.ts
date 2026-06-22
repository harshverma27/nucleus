// The Nucleus sidebar (Activity Bar webview view). Pure plumbing, no logic:
//
//   • action buttons post `{type:"run", verb}` → we run `nucleus <verb>` in an
//     integrated terminal (the user sees live, colored CLI output).
//   • `{type:"refreshHistory"}` → we run `nucleus history --graph` and post the
//     resulting JSON back; the webview's HistoryPanel draws it.
//
// All counting/validation happens in the Rust CLI; this file only spawns it.

import { execFile } from "child_process";
import { promisify } from "util";

import * as vscode from "vscode";

const execFileAsync = promisify(execFile);

// Verbs the sidebar buttons may run. Whitelisted so a malformed webview message
// can never invoke an arbitrary subcommand.
const VERBS = ["check", "build", "flash", "test"] as const;
type Verb = (typeof VERBS)[number];

function isVerb(v: unknown): v is Verb {
  return typeof v === "string" && (VERBS as readonly string[]).includes(v);
}

export class NucleusPanel implements vscode.WebviewViewProvider {
  static readonly viewId = "nucleus.panel";

  private terminal: vscode.Terminal | undefined;

  constructor(private readonly extensionUri: vscode.Uri) {}

  resolveWebviewView(view: vscode.WebviewView): void {
    view.webview.options = {
      enableScripts: true,
      localResourceRoots: [vscode.Uri.joinPath(this.extensionUri, "dist")],
    };
    view.webview.html = this.render(view.webview);

    view.webview.onDidReceiveMessage((msg) => {
      if (msg?.type === "run" && isVerb(msg.verb)) {
        this.runVerb(msg.verb);
      } else if (msg?.type === "refreshHistory") {
        void this.refreshHistory(view.webview);
      }
    });

    // Populate the chart as soon as the view opens.
    void this.refreshHistory(view.webview);
  }

  /** Run `nucleus <verb>` in a reused "Nucleus" integrated terminal. */
  private runVerb(verb: Verb): void {
    const cli = cliPath();
    if (!this.terminal || this.terminal.exitStatus !== undefined) {
      this.terminal = vscode.window.createTerminal("Nucleus");
    }
    this.terminal.show();
    this.terminal.sendText(`${cli} ${verb}`);
  }

  /** Run `nucleus history --graph` and post the JSON to the webview. */
  private async refreshHistory(webview: vscode.Webview): Promise<void> {
    const cli = cliPath();
    const cwd = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
    try {
      const { stdout } = await execFileAsync(cli, ["history", "--graph"], {
        cwd,
      });
      const data = JSON.parse(stdout.trim() || "{}");
      void webview.postMessage({ type: "history", data });
    } catch (err) {
      void webview.postMessage({ type: "history", data: { schema: "", runs: [] } });
      void vscode.window.showErrorMessage(
        `Nucleus: could not load history (${cli} history --graph): ${err}`
      );
    }
  }

  private render(webview: vscode.Webview): string {
    const script = webview.asWebviewUri(
      vscode.Uri.joinPath(this.extensionUri, "dist", "dashboard.js")
    );
    const style = webview.asWebviewUri(
      vscode.Uri.joinPath(this.extensionUri, "dist", "dashboard.css")
    );
    const nonce = makeNonce();
    const csp = [
      "default-src 'none'",
      `style-src ${webview.cspSource} 'unsafe-inline'`,
      `script-src 'nonce-${nonce}'`,
      `img-src ${webview.cspSource} data:`,
    ].join("; ");

    return `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta http-equiv="Content-Security-Policy" content="${csp}" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <link rel="stylesheet" href="${style}" />
    <title>Nucleus</title>
  </head>
  <body>
    <div id="root"></div>
    <script nonce="${nonce}" src="${script}"></script>
  </body>
</html>`;
  }
}

function cliPath(): string {
  return vscode.workspace
    .getConfiguration("nucleus")
    .get<string>("serverPath", "nucleus");
}

function makeNonce(): string {
  const chars = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
  let nonce = "";
  for (let i = 0; i < 32; i++) {
    nonce += chars.charAt(Math.floor(Math.random() * chars.length));
  }
  return nonce;
}
