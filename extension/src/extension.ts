// Nucleus VS Code extension — a thin client. It contains zero business logic:
// it spawns `nucleus lsp` for diagnostics/hover/completion, and hosts a sidebar
// whose buttons shell out to the `nucleus` CLI (check / build / flash / test).
// All intelligence lives in the Rust binary.

import * as vscode from "vscode";
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
  TransportKind,
} from "vscode-languageclient/node";

import { NucleusPanel } from "./panelView";

let client: LanguageClient | undefined;

export function activate(context: vscode.ExtensionContext): void {
  // The sidebar: action buttons + test-history chart. Zero logic — buttons run
  // `nucleus <verb>` in a terminal; the chart renders `nucleus history --graph`.
  context.subscriptions.push(
    vscode.window.registerWebviewViewProvider(
      NucleusPanel.viewId,
      new NucleusPanel(context.extensionUri)
    )
  );

  const config = vscode.workspace.getConfiguration("nucleus");
  const command = config.get<string>("serverPath", "nucleus");

  // Run `nucleus lsp`, speaking LSP over stdio.
  const serverOptions: ServerOptions = {
    run: { command, args: ["lsp"], transport: TransportKind.stdio },
    debug: { command, args: ["lsp"], transport: TransportKind.stdio },
  };

  // Only attach to stm32.toml files (by name, not language id).
  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ scheme: "file", pattern: "**/stm32.toml" }],
    synchronize: {
      fileEvents: vscode.workspace.createFileSystemWatcher("**/stm32.toml"),
    },
  };

  client = new LanguageClient(
    "nucleus",
    "Nucleus Language Server",
    serverOptions,
    clientOptions
  );

  // Starts the server lazily and registers it for disposal on deactivate.
  client.start().catch((err) => {
    void vscode.window.showErrorMessage(
      `Nucleus: failed to start language server (${command} lsp): ${err}`
    );
  });
  context.subscriptions.push({ dispose: () => void client?.stop() });
}

export function deactivate(): Thenable<void> | undefined {
  return client?.stop();
}
