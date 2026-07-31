import * as cp from 'node:child_process';
import * as readline from 'node:readline';
import * as vscode from 'vscode';

type AgenaProblem = {
  id: string;
  category: string;
  retry: string;
  recovery: string;
  impact: string;
  user: { fallback: string };
};

class AgenaRequestError extends Error {
  constructor(message: string, readonly problem?: AgenaProblem) {
    super(message);
  }
}

type Pending = {
  resolve: (value: unknown) => void;
  reject: (reason: Error) => void;
};

class AgenaClient {
  private child?: cp.ChildProcessWithoutNullStreams;
  private nextId = 1;
  private pending = new Map<number, Pending>();

  start(command: string, cwd?: string): void {
    if (this.child) {
      return;
    }
    this.child = cp.spawn(command, ['app-server', '--transport', 'stdio'], { cwd });
    const reader = readline.createInterface({ input: this.child.stdout });
    reader.on('line', line => this.handleLine(line));
    this.child.stderr.on('data', chunk => console.debug(`agena: ${chunk}`));
    this.child.on('exit', () => {
      this.child = undefined;
      for (const pending of this.pending.values()) {
        pending.reject(new Error('Agena app-server exited'));
      }
      this.pending.clear();
    });
  }

  async createSession(title: string): Promise<number> {
    const result = await this.request('session/create', { title }) as { session_id: number };
    return result.session_id;
  }

  async submitTurn(sessionId: number, prompt: string): Promise<string> {
    const result = await this.request('message/submit', { session_id: sessionId, prompt }) as { text?: string };
    return result.text ?? '';
  }

  private request(method: string, params: unknown): Promise<unknown> {
    if (!this.child) {
      return Promise.reject(new Error('Agena app-server is not running'));
    }
    const id = this.nextId++;
    const payload = JSON.stringify({ jsonrpc: '2.0', id, method, params });
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.child!.stdin.write(`${payload}\n`);
    });
  }

  private handleLine(line: string): void {
    let message: {
      id?: number;
      result?: unknown;
      error?: {
        message: string;
        data?: { problem?: AgenaProblem };
      };
    };
    try {
      message = JSON.parse(line) as typeof message;
    } catch (diagnostic) {
      console.error('Agena app-server returned an invalid JSON-RPC message', diagnostic);
      return;
    }
    if (typeof message.id !== 'number') {
      return;
    }
    const pending = this.pending.get(message.id);
    if (!pending) {
      return;
    }
    this.pending.delete(message.id);
    if (message.error) {
      const problem = message.error.data?.problem;
      const fallback = problem?.user?.fallback?.trim();
      const reference =
        problem && ['internal', 'data_corruption'].includes(problem.category)
          ? ` Reference: ${problem.id}`
          : '';
      pending.reject(
        new AgenaRequestError(
          fallback ? `${fallback}${reference}` : 'The request could not be completed.',
          problem,
        ),
      );
    } else {
      pending.resolve(message.result);
    }
  }

  dispose(): void {
    this.child?.kill();
    this.child = undefined;
  }
}

export function activate(context: vscode.ExtensionContext): void {
  const client = new AgenaClient();
  context.subscriptions.push({ dispose: () => client.dispose() });
  context.subscriptions.push(vscode.commands.registerCommand('agena.start', () => {
    const command = vscode.workspace.getConfiguration('agena').get<string>('command') ?? 'agena';
    const cwd = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
    client.start(command, cwd);
    vscode.window.showInformationMessage('Agena app-server started');
  }));
  context.subscriptions.push(vscode.commands.registerCommand('agena.prompt', async () => {
    const prompt = await vscode.window.showInputBox({ prompt: 'Prompt Agena' });
    if (!prompt) {
      return;
    }
    const command = vscode.workspace.getConfiguration('agena').get<string>('command') ?? 'agena';
    const cwd = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
    client.start(command, cwd);
    const sessionId = await client.createSession('VS Code');
    const text = await client.submitTurn(sessionId, prompt);
    const document = await vscode.workspace.openTextDocument({ content: text, language: 'markdown' });
    await vscode.window.showTextDocument(document);
  }));
}

export function deactivate(): void {}
