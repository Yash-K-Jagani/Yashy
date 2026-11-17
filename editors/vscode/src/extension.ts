import * as vscode from 'vscode';
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
} from 'vscode-languageclient/node';

let client: LanguageClient | undefined;

function compiler(): string {
  return vscode.workspace.getConfiguration('yashyc').get<string>('path') ?? 'yashyc';
}

function extraArgs(): string[] {
  return vscode.workspace.getConfiguration('yashyc').get<string[]>('extraArgs') ?? [];
}

export function activate(context: vscode.ExtensionContext): void {
  // The server is the compiler's own verb. No editor-only parser and no second
  // copy of the language lives in this extension.
  const serverOptions: ServerOptions = {
    run: { command: compiler(), args: ['lsp', '--stdio', ...extraArgs()] },
    debug: { command: compiler(), args: ['lsp', '--stdio', ...extraArgs()] },
  };
  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ scheme: 'file', language: 'yashy' }],
    synchronize: { fileEvents: vscode.workspace.createFileSystemWatcher('**/*.ya') },
  };
  client = new LanguageClient('yashy', 'Yashy', serverOptions, clientOptions);
  context.subscriptions.push(client.start());

  context.subscriptions.push(
    vscode.commands.registerTextEditorCommand('yashyc.check', async (editor) => {
      await editor.document.save();
      await run(editor.document, 'check');
    }),
    vscode.commands.registerCommand('yashyc.test', async () => {
      const folder = vscode.workspace.workspaceFolders?.[0];
      if (!folder) {
        void vscode.window.showErrorMessage('Open a Yashy project before running tests.');
        return;
      }
      await runInTerminal(compiler(), ['test', folder.uri.fsPath], 'yashyc test');
    }),
    vscode.commands.registerCommand('yashyc.doctor', async () => {
      await runInTerminal(compiler(), ['doctor'], 'yashyc doctor');
    }),
    vscode.tasks.registerTaskProvider('yashy', {
      provideTasks: () =>
        (vscode.workspace.getConfiguration('yashyc').get<string[]>('tasks') ?? []).map((name) =>
          task(name),
        ),
      resolveTask: (_, name) => task(name.label),
    }),
  );
}

function task(name: string): vscode.Task {
  const definition = { type: 'yashy', task: name };
  const execution = new vscode.ProcessExecution(compiler(), ['task', name]);
  return new vscode.Task(definition, vscode.TaskScope.Workspace, name, 'yashy', execution);
}

async function run(document: vscode.TextDocument, verb: string): Promise<void> {
  if (document.languageId !== 'yashy' || document.isUntitled) {
    void vscode.window.showErrorMessage('Save a .ya file before checking it.');
    return;
  }
  await runInTerminal(compiler(), [verb, document.uri.fsPath], `yashyc ${verb}`);
}

async function runInTerminal(command: string, args: string[], name: string): Promise<void> {
  const terminal = vscode.window.createTerminal(name);
  terminal.show();
  terminal.sendText([command, ...args].map(quote).join(' '));
}

function quote(value: string): string {
  return /\s/.test(value) ? JSON.stringify(value) : value;
}

export function deactivate(): Thenable<void> | undefined {
  return client?.stop();
}
